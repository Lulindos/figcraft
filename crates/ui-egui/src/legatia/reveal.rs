//! LegatIA: a animação "construindo ao vivo", para demonstrar um design.
//!
//! Não executa nada no design: as pranchetas que já existem (as selecionadas, ou todas as da página) são desenhadas de novo enquanto um
//! enxame de cursores genéricos (sem marca de nenhuma IA: as marcas só aparecem quando um agente de verdade mexe no arquivo) cria cada
//! camada à vista, como numa ferramenta de verdade. Nada surge do nada:
//!
//! - cada quadro, forma, ícone ou imagem é **arrastado** (alças e "L × A" ao vivo), com um clique no começo e no fim; o quadro nasce cinza
//!   e reto e vai à cor e ao raio finais (um chip mostra o hex e o raio subindo); ícones e imagens aparecem dentro da caixa arrastada;
//! - os textos são **digitados** com ritmo de gente (pausas nos espaços e na pontuação) e no fim o estilo é aplicado (a fonte aparece);
//! - uma camada só começa quando a camada que a contém já existe, e cada cursor pega a próxima tarefa livre quando termina a sua: o tempo
//!   não é dividido de antemão, ele sai do trabalho (como no Swarm Cursor). O enxame cresce com o tamanho do design (de 6 a 40 cursores);
//! - começa pela primeira prancheta e a **câmera** faz o tour, acompanhando onde o enxame está trabalhando; no fim se afasta e mostra tudo;
//!   a lista de camadas e o painel de propriedades seguem o quadro da vez (a seleção anda junto);
//! - controles discretos embaixo: **pausar**, **velocidade** (0,5×, 1×, 2×, 3×) e **parar**; Esc também para.
//!
//! O documento não é tocado (nada no desfazer, nada alterado): a linha de fundo tira uma cópia (barata: o documento é persistente), muda
//! nela só o que está no meio da animação e desenha a parte visível (`figcraft_render::render_area_skipping`) na resolução da tela. A vista
//! e a seleção mudam durante a demonstração e, no fim, as de antes voltam.
//!
//! A ideia vem do Swarm Rebuild (cursores coloridos reconstruindo uma página), reescrita do zero: aquele projeto não tem licença.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use figcraft_doc::{Document, NodeId, NodeKind};

use crate::FigApp;
use crate::theme::{Tokens, doc_c32, regular, semibold};

/// As cores do enxame (o azul da LegatIA primeiro). Nada de marcas de IA.
const PALETTE: [Color32; 8] = [
    Color32::from_rgb(0x00, 0x6F, 0xFE),
    Color32::from_rgb(0x22, 0xC5, 0xE0),
    Color32::from_rgb(0xA7, 0x8B, 0xFA),
    Color32::from_rgb(0xFB, 0x92, 0x3C),
    Color32::from_rgb(0xF4, 0x72, 0xB6),
    Color32::from_rgb(0x34, 0xD3, 0x99),
    Color32::from_rgb(0xFA, 0xCC, 0x15),
    Color32::from_rgb(0xF8, 0x71, 0x71),
];

/// Depois de criado, quanto o quadro leva para arredondar os cantos e ir à cor final.
const SETTLE: f32 = 0.6;
/// Um ícone ou uma imagem aparece dentro da caixa arrastada neste tempo (e as peças de um ícone com ele).
const FADE_IN: f32 = 0.3;
/// Depois de digitado, quanto dura o "estilo aplicado" (a faixa de seleção e a fonte).
const STYLE: f32 = 0.45;
/// Antes de o enxame começar, e depois da última coisa (o enxame gira) e o sumiço.
const LEAD_IN: f32 = 0.35;
const HOLD: f32 = 1.6;
const FADE: f32 = 0.7;
/// Até este tamanho (nos dois lados) uma camada é uma peça só: um ícone é arrastado inteiro e as partes dele aparecem junto.
const SMALL: f32 = 48.0;
/// O cinza de onde as cores saem.
const PLACEHOLDER: figcraft_color::Color = figcraft_color::Color { r: 0.84, g: 0.85, b: 0.88, a: 1.0 };
/// No máximo de pranchetas de uma vez.
const MAX_ROOTS: usize = 24;
const SPEEDS: [f32; 4] = [0.5, 1.0, 2.0, 3.0];

// ---------- o roteiro (puro: não mexe em nada)

/// O que cada camada é, para o roteiro decidir como criá-la.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Class {
    /// Quadros, cartões, seções, formas: arrastados, depois ganham cor e cantos.
    Container,
    /// Texto com tantas letras: digitado.
    Text(usize),
    /// O resto (vetores, imagens, linhas): arrastado e aparece dentro da caixa.
    Other,
}

#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub class: Class,
    /// O retângulo na página.
    pub rect: Rect,
    /// A camada que a contém (índice na mesma lista; sempre antes dela).
    pub parent: Option<usize>,
    /// Em qual prancheta está (índice nas pranchetas).
    pub root: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Aparece (opacidade).
    Pop,
    /// O quadro foi criado arrastando: arredonda e ganha a cor.
    Settle,
    /// Digitado.
    Type,
}

/// A vida de uma camada no desenho: começa em `start` e leva `dur` para ficar como no arquivo.
#[derive(Clone, Copy, Debug)]
pub struct Anim {
    pub start: f32,
    pub dur: f32,
    pub mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Um quadro arrastado (e depois cor e cantos).
    Frame,
    /// Um ícone, imagem ou vetor arrastado.
    Place,
    /// Um texto digitado.
    Text,
}

/// Uma tarefa de um cursor: chega em `arrive`, faz o gesto até `work_end` (o arrasto ou a digitação) e fica livre em `free`.
#[derive(Clone, Copy, Debug)]
struct Action {
    node: usize,
    cursor: usize,
    kind: Kind,
    arrive: f32,
    work_end: f32,
    free: f32,
    rect: Rect,
    root: usize,
}

/// Um trecho do caminho de um cursor (coordenadas da página).
struct Leg {
    depart: f32,
    arrive: f32,
    from: Pos2,
    to: Pos2,
    /// Reto (arrastando ou seguindo o texto): sem arco nem tremidinha.
    straight: bool,
}

struct Cursor {
    legs: Vec<Leg>,
    start: Pos2,
    /// Cada cursor tem a sua fase (a tremidinha da mão, o balanço parado e o ponto da órbita no fim).
    phase: f32,
    color: Color32,
}

struct Plan {
    actions: Vec<Action>,
    anims: Vec<Anim>,
    cursors: Vec<Cursor>,
    /// As tarefas de cada cursor (índices em `actions`, na ordem) e os cliques dele (instantes, em ordem).
    by_cursor: Vec<Vec<usize>>,
    clicks: Vec<Vec<f32>>,
    /// Quando a última coisa termina.
    end: f32,
}

/// Quantos cursores para tantas tarefas: um para cada 7, de 6 a 40.
pub fn cursor_count(tasks: usize) -> usize {
    tasks.div_ceil(7).clamp(6, 40)
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Sai rápido e chega devagar (como a mão de verdade levando o mouse a um ponto).
fn ease_fast(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let s = ease(x);
    1.0 - (1.0 - s) * (1.0 - s) * (1.0 - 0.3 * s)
}

/// Quanto tempo um texto de `chars` letras leva para ser digitado.
pub fn type_secs(chars: usize) -> f32 {
    (chars as f32 * 0.045).clamp(0.35, 1.6)
}

/// Quanto dura o arrasto de uma caixa deste tamanho: as grandes levam mais.
pub fn drag_secs(rect: Rect) -> f32 {
    (0.26 + 0.0002 * (rect.width() + rect.height())).clamp(0.26, 0.75)
}

/// Quanto o cursor leva para ir de um ponto a outro (em relação ao tamanho da prancheta): perto é rápido, longe não demora tanto mais.
fn travel_secs(from: Pos2, to: Pos2, diag: f32) -> f32 {
    (0.16 + 0.5 * ((to - from).length() / diag.max(1.0)).sqrt()).clamp(0.16, 0.75)
}

/// Um número de 0 a 1 que parece sorteado, sempre o mesmo para a mesma entrada.
fn noise(seed: u64, i: u64) -> f32 {
    let mut x = seed ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// Quantas letras de `text` já foram digitadas em `p` (0 a 1) do tempo de digitação: com ritmo de gente, cada letra leva um tempo um pouco
/// diferente e há uma pausa depois de espaço e pontuação. `seed` deixa o ritmo igual a cada quadro.
pub fn typed(text: &str, p: f32, seed: u64) -> usize {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() || p <= 0.0 {
        return 0;
    }
    if p >= 1.0 {
        return chars.len();
    }
    let weight = |i: usize| {
        let pause = if i > 0 && matches!(chars[i - 1], ' ' | ',' | '.' | ':' | ';' | '!' | '?' | '\n') { 0.9 } else { 0.0 };
        0.55 + 0.9 * noise(seed, i as u64) + pause
    };
    let total: f32 = (0..chars.len()).map(weight).sum();
    let goal = p * total;
    let mut acc = 0.0;
    for i in 0..chars.len() {
        acc += weight(i);
        if acc > goal {
            return i;
        }
    }
    chars.len()
}

/// As camadas que não ganham gesto próprio: as partes de uma peça pequena (um ícone) aparecem com ela.
fn attached(infos: &[Info]) -> Vec<bool> {
    let mut out = vec![false; infos.len()];
    for i in 0..infos.len() {
        if let Some(p) = infos[i].parent.filter(|p| *p < i) {
            let parent = &infos[p];
            let small = parent.parent.is_some() && !matches!(parent.class, Class::Text(_)) && parent.rect.width() <= SMALL && parent.rect.height() <= SMALL;
            out[i] = out[p] || small;
        }
    }
    out
}

/// Quem faz o quê e quando. Cada camada vira uma tarefa; cada tarefa vai para o cursor que pode chegar lá primeiro (quem está livre e
/// perto), e só começa depois de a camada que a contém existir. A duração total sai daí, não é fixada antes.
fn plan(infos: &[Info], roots: &[Rect]) -> Plan {
    let n = infos.len();
    let glued = attached(infos);
    let tasks = glued.iter().filter(|g| !**g).count();
    let k = cursor_count(tasks);
    let first = roots.first().copied().unwrap_or(Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0)));
    let mut cursors: Vec<Cursor> = (0..k)
        .map(|c| {
            // Os cursores entram de todos os lados da primeira prancheta, um pouco para fora dela
            let ang = c as f32 / k as f32 * std::f32::consts::TAU + 0.4;
            let start = first.center() + vec2(ang.cos() * first.width() * 0.62, ang.sin() * first.height() * 0.62);
            Cursor { legs: Vec::new(), start, phase: c as f32 * 2.399, color: PALETTE[c % PALETTE.len()] }
        })
        .collect();
    let mut free: Vec<f32> = (0..k).map(|c| LEAD_IN + 0.5 * noise(3, c as u64)).collect();
    let mut at: Vec<Pos2> = cursors.iter().map(|c| c.start).collect();
    let mut ready = vec![0.0_f32; n];
    let mut anims = vec![Anim { start: f32::MAX, dur: 0.0, mode: Mode::Pop }; n];
    let mut actions = Vec::with_capacity(tasks);
    let mut by_cursor = vec![Vec::new(); k];
    let mut clicks = vec![Vec::new(); k];
    let mut end = 0.0_f32;

    // Uma perna do caminho do cursor `c`: chega a `to` em `arrive`, saindo `lead` antes (sem pisar na perna anterior)
    let go = |cursors: &mut Vec<Cursor>, c: usize, arrive: f32, to: Pos2, straight: bool, lead: f32| {
        let cur = &mut cursors[c];
        let (prev_at, prev_pos) = cur.legs.last().map_or((0.0, cur.start), |l| (l.arrive, l.to));
        let depart = (arrive - lead).max(prev_at).min(arrive);
        cur.legs.push(Leg { depart, arrive, from: prev_pos, to, straight });
    };

    for i in 0..n {
        let info = infos[i];
        let parent_ready = info.parent.filter(|p| *p < i).map_or(0.0, |p| ready[p]);
        if glued[i] {
            anims[i] = Anim { start: parent_ready, dur: FADE_IN, mode: Mode::Pop };
            ready[i] = parent_ready;
            end = end.max(parent_ready + FADE_IN);
            continue;
        }
        let r = info.rect;
        let target = match info.class {
            Class::Text(_) => pos2(r.min.x + 2.0, r.center().y),
            _ => r.min,
        };
        let diag = roots.get(info.root).map_or(first.size().length(), |b| b.size().length());
        // O cursor que chega primeiro (com uma preferência leve por quem está perto)
        let (c, arrive, travel) = (0..k)
            .map(|c| {
                let travel = travel_secs(at[c], target, diag);
                let arrive = (free[c] + travel).max(parent_ready + 0.05);
                (c, arrive, travel)
            })
            .min_by(|a, b| (a.1 + 0.3 * a.2).partial_cmp(&(b.1 + 0.3 * b.2)).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or((0, parent_ready, 0.3));
        let arrive = arrive + 0.12 * noise(7, i as u64);
        go(&mut cursors, c, arrive, target, false, travel);
        clicks[c].push(arrive);
        let action = match info.class {
            Class::Text(chars) => {
                let dur = type_secs(chars) * (0.85 + 0.3 * noise(11, i as u64));
                let caret_end = pos2((r.max.x - 2.0).max(target.x), target.y);
                go(&mut cursors, c, arrive + dur, caret_end, true, dur);
                anims[i] = Anim { start: arrive, dur, mode: Mode::Type };
                ready[i] = arrive;
                free[c] = arrive + dur + 0.12;
                at[c] = caret_end;
                end = end.max(arrive + dur + STYLE);
                Action { node: i, cursor: c, kind: Kind::Text, arrive, work_end: arrive + dur, free: free[c], rect: r, root: info.root }
            }
            class => {
                let dd = drag_secs(r) * (0.9 + 0.2 * noise(13, i as u64));
                let e = arrive + dd;
                go(&mut cursors, c, e, r.max, true, dd);
                clicks[c].push(e);
                let frame = class == Class::Container;
                anims[i] = if frame { Anim { start: e, dur: SETTLE, mode: Mode::Settle } } else { Anim { start: e, dur: FADE_IN, mode: Mode::Pop } };
                ready[i] = e;
                free[c] = e + if frame { 0.22 } else { 0.08 };
                at[c] = r.max;
                end = end.max(e + if frame { SETTLE } else { FADE_IN });
                Action { node: i, cursor: c, kind: if frame { Kind::Frame } else { Kind::Place }, arrive, work_end: e, free: free[c], rect: r, root: info.root }
            }
        };
        by_cursor[c].push(actions.len());
        actions.push(action);
    }
    Plan { actions, anims, cursors, by_cursor, clicks, end }
}

/// Onde o cursor está no instante `t` (coordenadas da página) e se está andando: voa em arco de um ponto a outro, com uma tremidinha de
/// mão, e freia passando um pouco do ponto; arrasta e segue o texto em linha reta; e depois da última coisa o enxame se junta em volta das
/// pranchetas e gira (como no Swarm Rebuild).
fn cursor_at(c: &Cursor, t: f32, bounds: Rect, end: f32) -> (Pos2, bool) {
    let mut pos = c.start;
    let mut moving = false;
    for (i, leg) in c.legs.iter().enumerate() {
        if t < leg.depart {
            break;
        }
        let raw = if leg.arrive > leg.depart { ((t - leg.depart) / (leg.arrive - leg.depart)).clamp(0.0, 1.0) } else { 1.0 };
        moving = raw < 1.0;
        if leg.straight {
            pos = leg.from + (leg.to - leg.from) * ease(raw);
        } else {
            // Um arco: o ponto de controle sai do meio do caminho para o lado (cada trecho para um lado, com curvas diferentes)
            let k = ease_fast(raw);
            let mid = leg.from + (leg.to - leg.from) * 0.5;
            let along = leg.to - leg.from;
            let side = if i % 2 == 0 { 1.0 } else { -1.0 };
            let bend = 0.1 + 0.12 * noise(c.phase.to_bits() as u64, i as u64);
            let ctrl = mid + vec2(-along.y, along.x) * (bend * side);
            let u = 1.0 - k;
            pos = pos2(u * u * leg.from.x + 2.0 * u * k * ctrl.x + k * k * leg.to.x, u * u * leg.from.y + 2.0 * u * k * ctrl.y + k * k * leg.to.y);
            // A freada: no fim passa um pouquinho do ponto e volta
            let brake = ((raw - 0.72) / 0.28).clamp(0.0, 1.0);
            pos += along * (0.035 * (brake * std::f32::consts::PI).sin());
            // A tremidinha some quando o cursor chega
            let wobble = 2.0 * (raw * (1.0 - raw)) * 4.0 * (t * 9.0 + c.phase).sin();
            pos += vec2(wobble, wobble * 0.6);
        }
    }
    if t > end {
        // O enxame se junta e gira em volta das pranchetas
        let k = ease((t - end) / 0.8);
        let r = bounds.width().min(bounds.height()) * 0.42;
        let ang = c.phase + (t - end) * 2.2;
        let orbit = bounds.center() + vec2(ang.cos(), ang.sin()) * r;
        pos = pos + (orbit - pos) * k;
        moving = true;
    }
    (pos, moving)
}

/// O que o cursor está fazendo agora, para a etiqueta (nada quando está só andando).
fn activity(plan: &Plan, c: usize, t: f32) -> Option<&'static str> {
    let list = plan.by_cursor.get(c)?;
    // As tarefas de um cursor estão em ordem e não se sobrepõem: a última que já começou é a única que pode estar acontecendo
    let idx = list.partition_point(|a| plan.actions[*a].arrive <= t);
    let a = &plan.actions[*list.get(idx.checked_sub(1)?)?];
    if t >= a.free {
        return None;
    }
    Some(match (a.kind, t < a.work_end) {
        (Kind::Frame, true) => "Criando",
        (Kind::Frame, false) => "Cores",
        (Kind::Place, _) => "Inserindo",
        (Kind::Text, true) => "Digitando",
        (Kind::Text, false) => return None,
    })
}

// ---------- o que muda em cada quadro (feito na cópia do documento, na linha de fundo)

/// As primeiras `n` letras do texto (por letra, não por byte).
fn prefix(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// A cópia do documento no instante `t`: o que ainda não entrou fica na lista `skip`, e o que está no meio da animação ganha o estado
/// dela (raio dos cantos, cor, opacidade, texto até a letra N). O que já terminou fica como está no arquivo.
fn frame_at(base: &Document, order: &[NodeId], anims: &[Anim], t: f32) -> (Document, Vec<NodeId>) {
    let mut doc = base.clone();
    let mut skip = Vec::new();
    for (id, a) in order.iter().zip(anims) {
        if t < a.start {
            skip.push(*id);
            continue;
        }
        if t >= a.start + a.dur {
            continue;
        }
        let raw = ((t - a.start) / a.dur.max(0.001)).clamp(0.0, 1.0);
        let p = ease(raw);
        let (Some(orig), Some(node)) = (base.node(*id), doc.node_mut(*id)) else { continue };
        match a.mode {
            Mode::Pop => node.opacity = orig.opacity * p,
            Mode::Settle => {
                // Os cantos arredondam do reto ao raio final, e as cores vão do cinza à cor final
                for (i, r) in orig.corners.radii.iter().enumerate() {
                    node.corners.radii[i] = r * f64::from(p);
                }
                for (paint, was) in node.fills.iter_mut().zip(&orig.fills) {
                    match (&mut paint.kind, &was.kind) {
                        (figcraft_color::PaintKind::Solid { color }, figcraft_color::PaintKind::Solid { color: end }) => *color = PLACEHOLDER.lerp(*end, p),
                        _ => paint.opacity = was.opacity * p,
                    }
                }
                for (paint, was) in node.strokes.iter_mut().zip(&orig.strokes) {
                    paint.opacity = was.opacity * p;
                }
            }
            Mode::Type => {
                if let (NodeKind::Text(data), NodeKind::Text(whole)) = (&mut node.kind, &orig.kind) {
                    let shown = typed(&whole.characters, raw, id.0);
                    data.characters = prefix(&whole.characters, shown);
                    data.runs = whole.runs.iter().filter(|r| r.start < shown).map(|r| figcraft_doc::StyleRun { end: r.end.min(shown), ..r.clone() }).collect();
                }
            }
        }
    }
    (doc, skip)
}

// ---------- o estado da animação

struct Rendered {
    size: [usize; 2],
    rgba: Vec<u8>,
    /// O pedaço da página que este desenho cobre.
    area: Rect,
}

/// O que se sabe de uma camada para os chips (cor final, raio, fonte).
#[derive(Clone, Default)]
struct Detail {
    fill: Option<Color32>,
    hex: String,
    radius: f32,
    font: String,
}

type View = (f64, f64, f64);

/// O que volta ao fim: a vista e a seleção de antes.
struct Restore {
    selection: Vec<NodeId>,
    view: View,
}

struct Show {
    doc_uid: u64,
    page: NodeId,
    /// O retângulo de todas as pranchetas, e o de cada uma, na página.
    bounds: Rect,
    roots: Vec<Rect>,
    order: Vec<NodeId>,
    details: Vec<Detail>,
    texts: Vec<String>,
    plan: Plan,
    bg: Color32,
    /// O relógio da animação (anda com a velocidade e para na pausa).
    clock: f32,
    last_tick: Instant,
    speed: usize,
    paused: bool,
    /// A câmera: a vista geral, onde ela está agora, para onde vai, e se a pessoa mexeu na vista (aí a câmera para de seguir).
    overview: View,
    cam: View,
    cam_goal: View,
    cam_tick: Instant,
    camera_free: bool,
    before: Restore,
    selected: Option<usize>,
    selected_at: f32,
    /// O rastro de cada cursor (posições na página, com o instante).
    trails: Vec<VecDeque<(Pos2, f32)>>,
    /// Pedidos de desenho (o instante, a escala e o pedaço da página) e as respostas da linha de fundo.
    requests: Sender<(f32, f64, figcraft_geom::Rect)>,
    results: Receiver<Rendered>,
    last_sent: Option<Instant>,
    texture: Option<(egui::TextureHandle, Rect)>,
}

static SHOW: Mutex<Option<Show>> = Mutex::new(None);
static RESTORE: Mutex<Option<Restore>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<Show>> {
    SHOW.lock().unwrap_or_else(|e| e.into_inner())
}

/// Está rodando.
pub fn running() -> bool {
    state().is_some()
}

/// Acaba a animação (a vista e a seleção de antes voltam no próximo quadro).
fn finish(guard: &mut Option<Show>) {
    if let Some(show) = guard.take() {
        *RESTORE.lock().unwrap_or_else(|e| e.into_inner()) = Some(show.before);
    }
}

pub fn stop() {
    finish(&mut state());
}

/// O fundo da página aberta (o que fica atrás das pranchetas).
fn page_bg(app: &FigApp, ctx: &egui::Context) -> Color32 {
    app.session
        .ds()
        .ok()
        .and_then(|ds| match ds.doc.node(ds.page).map(|n| &n.kind) {
            Some(NodeKind::Page { background, .. }) => Some(doc_c32(ctx, *background)),
            _ => None,
        })
        .unwrap_or_else(|| Tokens::get(ctx).canvas)
}

fn rect_of(b: figcraft_geom::Rect) -> Rect {
    Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))
}

fn kurbo_of(r: Rect) -> figcraft_geom::Rect {
    figcraft_geom::Rect::new(f64::from(r.min.x), f64::from(r.min.y), f64::from(r.max.x), f64::from(r.max.y))
}

/// Por onde começar: as pranchetas selecionadas (as que têm camadas) ou, sem seleção, todas as da página, na ordem de leitura.
pub fn pick_roots(doc: &Document, page: NodeId, selection: &[NodeId]) -> Vec<NodeId> {
    let has_kids = |id: &NodeId| doc.node(*id).is_some_and(|n| n.visible && !n.children.is_empty());
    let mut roots: Vec<NodeId> = selection.iter().copied().filter(has_kids).collect();
    if roots.is_empty() {
        roots = doc.node(page).map(|p| p.children.iter().copied().filter(has_kids).collect()).unwrap_or_default();
    }
    // Ordem de leitura: de cima para baixo (em faixas de 400 unidades), da esquerda para a direita
    let pos = |id: &NodeId| doc.world_render_bounds(*id).map_or((0.0, 0.0), |b| (b.y0, b.x0));
    roots.sort_by(|a, b| {
        let (pa, pb) = (pos(a), pos(b));
        ((pa.0 / 400.0).floor(), pa.1).partial_cmp(&((pb.0 / 400.0).floor(), pb.1)).unwrap_or(std::cmp::Ordering::Equal)
    });
    roots.truncate(MAX_ROOTS);
    roots
}

/// As camadas, prancheta por prancheta (na ordem de leitura) e, dentro de cada uma, por nível (a prancheta, os filhos, os netos…), com a
/// camada que contém cada uma e o número da prancheta.
pub fn layers(doc: &Document, roots: &[NodeId]) -> Vec<(NodeId, Option<usize>, usize)> {
    let mut out: Vec<(NodeId, Option<usize>, usize)> = Vec::new();
    for (r, root) in roots.iter().enumerate() {
        out.push((*root, None, r));
        let mut level: Vec<usize> = vec![out.len() - 1];
        while !level.is_empty() && out.len() < 20_000 {
            let mut next = Vec::new();
            for idx in level {
                let id = out[idx].0;
                if let Some(n) = doc.node(id) {
                    for c in n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|k| k.visible)) {
                        out.push((c, Some(idx), r));
                        next.push(out.len() - 1);
                    }
                }
            }
            level = next;
        }
    }
    out
}

fn class_of(doc: &Document, id: NodeId) -> Class {
    let Some(n) = doc.node(id) else { return Class::Other };
    match &n.kind {
        NodeKind::Text(t) => Class::Text(t.characters.chars().count()),
        NodeKind::Frame | NodeKind::Group | NodeKind::Section { .. } | NodeKind::Rectangle | NodeKind::Ellipse { .. } | NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. } => Class::Container,
        _ => Class::Other,
    }
}

fn detail_of(doc: &Document, id: NodeId, ctx: &egui::Context) -> Detail {
    let Some(n) = doc.node(id) else { return Detail::default() };
    let fill = n.fills.iter().find(|p| p.visible).and_then(|p| match &p.kind {
        figcraft_color::PaintKind::Solid { color } => Some(*color),
        _ => None,
    });
    let font = match &n.kind {
        NodeKind::Text(t) => {
            let family = t.style.font_family.trim();
            if family.is_empty() { String::new() } else { format!("{family} · {}", t.style.font_size.round() as i64) }
        }
        _ => String::new(),
    };
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Detail {
        fill: fill.map(|c| doc_c32(ctx, c)),
        hex: fill.map(|c| format!("#{:02X}{:02X}{:02X}", byte(c.r), byte(c.g), byte(c.b))).unwrap_or_default(),
        radius: n.corners.radii.iter().copied().fold(0.0_f64, f64::max) as f32,
        font,
    }
}

/// A vista que enquadra `r` (com folga), no zoom máximo de 200%.
fn fit_view(app: &FigApp, r: Rect) -> Option<View> {
    let ds = app.session.ds().ok()?;
    let mut v = ds.view.clone();
    v.fit(kurbo_of(r), 2.0);
    Some((v.cx, v.cy, v.zoom))
}

/// Começa a animação das pranchetas selecionadas (ou de todas as da página). Só olha o desenho: nada no arquivo muda.
pub fn start(app: &mut FigApp, ctx: &egui::Context) {
    if cfg!(test) {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let (doc, page, uid, selection_before) = (ds.doc.clone(), ds.page, ds.uid, ds.selection.clone());
    let view_before = (ds.view.cx, ds.view.cy, ds.view.zoom);
    let roots = pick_roots(&doc, page, &ds.selection);
    if roots.is_empty() {
        app.toast("Não achei uma prancheta com camadas para demonstrar nesta página.");
        return;
    }
    let list = layers(&doc, &roots);
    if list.len() < 3 {
        app.toast("Esta prancheta tem poucas camadas para uma demonstração.");
        return;
    }
    let root_rects: Vec<Rect> = roots.iter().map(|r| figcraft_render::export_bounds(&doc, *r).map(rect_of).unwrap_or(Rect::NOTHING)).collect();
    let Some(bounds_k) = roots.iter().filter_map(|r| figcraft_render::export_bounds(&doc, *r)).reduce(|a, b| a.union(b)) else { return };
    let bounds = rect_of(bounds_k);
    let (Some(overview), Some(first_view)) = (fit_view(app, bounds), fit_view(app, root_rects[0])) else { return };

    let order: Vec<NodeId> = list.iter().map(|(id, _, _)| *id).collect();
    let infos: Vec<Info> = list
        .iter()
        .map(|(id, parent, root)| Info {
            class: class_of(&doc, *id),
            rect: doc.world_render_bounds(*id).map(rect_of).filter(|r| r.width() > 0.0 && r.height() > 0.0).unwrap_or(root_rects[*root]),
            parent: *parent,
            root: *root,
        })
        .collect();
    let details: Vec<Detail> = order.iter().map(|id| detail_of(&doc, *id, ctx)).collect();
    let texts: Vec<String> = order
        .iter()
        .map(|id| match doc.node(*id).map(|n| &n.kind) {
            Some(NodeKind::Text(t)) => t.characters.clone(),
            _ => String::new(),
        })
        .collect();
    let plan = plan(&infos, &root_rects);
    let bg = page_bg(app, ctx);

    // A linha de fundo desenha o pedaço visível das pranchetas no instante pedido; só o último pedido importa
    let (req_tx, req_rx) = channel::<(f32, f64, figcraft_geom::Rect)>();
    let (res_tx, res_rx) = channel::<Rendered>();
    let worker_doc: Arc<Document> = doc.clone();
    let (worker_order, anims, worker_roots) = (order.clone(), plan.anims.clone(), roots.clone());
    let ctx2 = ctx.clone();
    let _ = std::thread::Builder::new().name("legatia-reveal".into()).spawn(move || {
        while let Ok(mut want) = req_rx.recv() {
            while let Ok(newer) = req_rx.try_recv() {
                want = newer;
            }
            let (t, scale, area) = want;
            let (frame, skip) = frame_at(&worker_doc, &worker_order, &anims, t);
            if let Some(pix) = figcraft_render::render_area_skipping(&frame, &worker_roots, area, scale, &skip) {
                let rgba = figcraft_render::to_rgba8(&pix);
                if res_tx.send(Rendered { size: [pix.width() as usize, pix.height() as usize], rgba, area: rect_of(area) }).is_err() {
                    return;
                }
                ctx2.request_repaint();
            }
        }
    });
    let cursors = plan.cursors.len();
    // Começa na primeira prancheta, sem seleção (os painéis seguem o quadro da vez)
    app.run("select.none", serde_json::json!({}));
    if let Ok(ds) = app.session.ds_mut() {
        (ds.view.cx, ds.view.cy, ds.view.zoom) = first_view;
    }
    *state() = Some(Show {
        doc_uid: uid,
        page,
        bounds,
        roots: root_rects,
        order,
        details,
        texts,
        plan,
        bg,
        clock: 0.0,
        last_tick: Instant::now(),
        speed: 1,
        paused: false,
        overview,
        cam: first_view,
        cam_goal: first_view,
        cam_tick: Instant::now(),
        camera_free: false,
        before: Restore { selection: selection_before, view: view_before },
        selected: None,
        selected_at: f32::MIN,
        trails: vec![VecDeque::new(); cursors],
        requests: req_tx,
        results: res_rx,
        last_sent: None,
        texture: None,
    });
    ctx.request_repaint();
}

/// Para onde a câmera quer olhar: o que o enxame está fazendo agora (as tarefas mais recentes em andamento), ao menos meia prancheta.
/// Sem nada em andamento, fica onde está.
fn camera_focus(show: &Show) -> Option<Rect> {
    let t = show.clock;
    let mut live: Vec<&Action> = show.plan.actions.iter().filter(|a| a.arrive - 0.25 <= t && t <= a.free + 0.35).collect();
    if live.is_empty() {
        return None;
    }
    live.sort_by(|a, b| b.arrive.partial_cmp(&a.arrive).unwrap_or(std::cmp::Ordering::Equal));
    live.truncate(14);
    let focus = live.iter().map(|a| a.rect).reduce(|a, b| a.union(b))?;
    let root = show.roots.get(live[0].root).copied().unwrap_or(show.bounds);
    let min = root.size() * 0.5;
    Some(Rect::from_center_size(focus.center(), vec2(focus.width().max(min.x), focus.height().max(min.y))).expand2(focus.size() * 0.06))
}

/// A cada quadro (com o app editável): a câmera faz o tour atrás do enxame, a seleção (e com ela a lista de camadas e o painel de
/// propriedades) segue o quadro criado por último, e no fim a vista e a seleção de antes voltam.
pub fn frame(app: &mut FigApp, ctx: &egui::Context) {
    if cfg!(test) {
        return;
    }
    if let Some(r) = RESTORE.lock().unwrap_or_else(|e| e.into_inner()).take() {
        if let Ok(ds) = app.session.ds_mut() {
            (ds.view.cx, ds.view.cy, ds.view.zoom) = r.view;
        }
        app.run("select.set", serde_json::json!({"ids": r.selection}));
        return;
    }
    let (focus, wide) = {
        let guard = state();
        let Some(show) = guard.as_ref() else { return };
        (camera_focus(show), show.clock > show.plan.end - 0.2)
    };
    let goal = if wide { None } else { focus.and_then(|f| fit_view(app, f)) };
    let (select, camera) = {
        let mut guard = state();
        let Some(show) = guard.as_mut() else { return };
        let t = show.clock;
        // A pessoa rolou ou deu zoom: a câmera para de seguir (a demonstração continua)
        if ctx.input(|i| i.smooth_scroll_delta != egui::Vec2::ZERO || i.zoom_delta() != 1.0) {
            show.camera_free = true;
        }
        // O quadro da vez: o último quadro grande que começou a ser criado (troca no máximo a cada 0,7 s, para os painéis não piscarem)
        let mut select = None;
        if t - show.selected_at >= 0.7 {
            let current = show
                .plan
                .actions
                .iter()
                .enumerate()
                .filter(|(_, a)| a.arrive <= t && a.kind == Kind::Frame && a.rect.width() >= 60.0)
                .max_by(|a, b| a.1.arrive.partial_cmp(&b.1.arrive).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i);
            if current.is_some() && current != show.selected {
                show.selected = current;
                show.selected_at = t;
                select = current.map(|i| show.order[show.plan.actions[i].node]);
            }
        }
        if wide {
            show.cam_goal = show.overview;
        } else if let Some(g) = goal {
            show.cam_goal = g;
        }
        let dt = show.cam_tick.elapsed().as_secs_f64().min(0.1);
        show.cam_tick = Instant::now();
        let camera = (!show.camera_free).then(|| {
            let (cur, goal) = (show.cam, show.cam_goal);
            let k = 1.0 - (-dt * 1.7).exp();
            let next = (cur.0 + (goal.0 - cur.0) * k, cur.1 + (goal.1 - cur.1) * k, (cur.2.ln() + (goal.2.ln() - cur.2.ln()) * k).exp());
            show.cam = next;
            next
        });
        (select, camera)
    };
    if let Some(c) = camera {
        if let Ok(ds) = app.session.ds_mut() {
            (ds.view.cx, ds.view.cy, ds.view.zoom) = c;
        }
    }
    if let Some(id) = select {
        app.run("select.set", serde_json::json!({"ids": [id]}));
    }
    ctx.request_repaint();
}

// ---------- o desenho por cima do canvas

/// A setinha do cursor com a ponta em `tip`, do tamanho `scale` (encolhe um pouco no clique).
fn arrow(painter: &Painter, tip: Pos2, color: Color32, alpha: f32, scale: f32) {
    let pts = [(0.0, 0.0), (0.0, 17.0), (4.5, 13.2), (8.0, 20.0), (10.6, 18.8), (7.2, 12.2), (12.5, 12.2)];
    let points: Vec<Pos2> = pts.iter().map(|(x, y)| tip + vec2(*x, *y) * scale).collect();
    let fade = |c: Color32| c.gamma_multiply(alpha);
    // Uma sombra leve embaixo
    let shadow: Vec<Pos2> = points.iter().map(|p| *p + vec2(1.2, 1.8)).collect();
    painter.add(Shape::convex_polygon(vec![shadow[0], shadow[1], shadow[2], shadow[6]], Color32::from_black_alpha((60.0 * alpha) as u8), Stroke::NONE));
    // A seta não é convexa: o corpo e a cauda são pintados em duas peças, e o contorno vai por cima
    let at = |i: usize| points[i];
    painter.add(Shape::convex_polygon(vec![at(0), at(1), at(2), at(6)], fade(color), Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![at(2), at(3), at(4), at(5)], fade(color), Stroke::NONE));
    painter.add(Shape::closed_line(points, Stroke::new(1.4, fade(Color32::WHITE))));
}

/// Os rótulos já desenhados neste quadro: um rótulo que cairia em cima de outro não é desenhado (com o enxame grande, viraria uma pilha).
fn free_spot(placed: &mut Vec<Rect>, rect: Rect) -> bool {
    if placed.iter().any(|p| p.expand(2.0).intersects(rect)) {
        return false;
    }
    placed.push(rect);
    true
}

fn name_tag(painter: &Painter, placed: &mut Vec<Rect>, at: Pos2, name: &str, color: Color32, alpha: f32) {
    let galley = painter.layout_no_wrap(name.to_string(), semibold(10.0), Color32::WHITE.gamma_multiply(alpha));
    let rect = Rect::from_min_size(at + vec2(12.0, 19.0), galley.size() + vec2(12.0, 6.0));
    if !free_spot(placed, rect) {
        return;
    }
    painter.rect_filled(rect, CornerRadius::same(10), color.gamma_multiply(alpha));
    painter.galley(pos2(rect.min.x + 6.0, rect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
}

/// O retângulo que está sendo arrastado: contorno na cor do cursor, quatro alças e, se couber, o tamanho (largura × altura) num rótulo.
fn drag_box(painter: &Painter, placed: &mut Vec<Rect>, rect: Rect, color: Color32, size_label: Option<&str>) {
    painter.rect_filled(rect, CornerRadius::ZERO, color.gamma_multiply(0.10));
    painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.4, color), StrokeKind::Middle);
    if rect.width() > 24.0 && rect.height() > 18.0 {
        for corner in [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()] {
            let h = Rect::from_center_size(corner, vec2(6.0, 6.0));
            painter.rect_filled(h, CornerRadius::ZERO, Color32::WHITE);
            painter.rect_stroke(h, CornerRadius::ZERO, Stroke::new(1.1, color), StrokeKind::Middle);
        }
    }
    let Some(label) = size_label else { return };
    let galley = painter.layout_no_wrap(label.to_string(), semibold(10.0), Color32::WHITE);
    let tag = Rect::from_min_size(pos2(rect.center().x - galley.size().x / 2.0 - 6.0, rect.max.y + 7.0), galley.size() + vec2(12.0, 5.0));
    if !free_spot(placed, tag) {
        return;
    }
    painter.rect_filled(tag, CornerRadius::same(4), color);
    painter.galley(tag.min + vec2(6.0, 2.5), galley, Color32::WHITE);
}

fn chip_bg(alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(16, 18, 24, (225.0 * alpha) as u8)
}

/// O chip ao lado de um quadro recém-criado: a cor indo do cinza à final (com o hex) e o raio dos cantos subindo.
fn inspector_chip(painter: &Painter, near: Rect, d: &Detail, p: f32, alpha: f32) {
    if d.fill.is_none() && d.radius <= 0.0 {
        return;
    }
    let ink = Color32::from_white_alpha((235.0 * alpha) as u8);
    let mut text = String::new();
    if d.fill.is_some() {
        text.push_str(&d.hex);
    }
    if d.radius > 0.0 {
        if !text.is_empty() {
            text.push_str("   ");
        }
        text.push_str(&format!("Raio {}", (d.radius * p).round() as i64));
    }
    let galley = painter.layout_no_wrap(text, regular(10.5), ink);
    let swatch = d.fill.map(|f| lerp_color(Color32::from_rgb(214, 217, 224), f, p));
    let w = galley.size().x + 16.0 + if swatch.is_some() { 18.0 } else { 0.0 };
    let chip = Rect::from_min_size(pos2(near.max.x + 10.0, near.min.y), vec2(w, 22.0));
    painter.rect_filled(chip, CornerRadius::same(6), chip_bg(alpha));
    painter.rect_stroke(chip, CornerRadius::same(6), Stroke::new(1.0, Color32::from_white_alpha((30.0 * alpha) as u8)), StrokeKind::Inside);
    let mut x = chip.min.x + 8.0;
    if let Some(c) = swatch {
        let s = Rect::from_center_size(pos2(x + 6.0, chip.center().y), vec2(12.0, 12.0));
        painter.rect_filled(s, CornerRadius::same(3), c.gamma_multiply(alpha));
        painter.rect_stroke(s, CornerRadius::same(3), Stroke::new(1.0, Color32::from_white_alpha((150.0 * alpha) as u8)), StrokeKind::Inside);
        x += 18.0;
    }
    painter.galley(pos2(x, chip.center().y - galley.size().y / 2.0), galley, ink);
}

/// O texto acabou de ser digitado: a faixa de seleção passa por cima e, se pedido, a fonte aparece num chip ("Satoshi · 32").
fn style_applied(painter: &Painter, rect: Rect, font: Option<&str>, color: Color32, age: f32) {
    if !(0.0..STYLE).contains(&age) {
        return;
    }
    let a = (1.0 - age / STYLE).clamp(0.0, 1.0);
    painter.rect_filled(rect, CornerRadius::same(2), color.gamma_multiply(0.22 * a));
    let Some(font) = font.filter(|f| !f.is_empty()) else { return };
    let galley = painter.layout_no_wrap(format!("Aa  {font}"), regular(10.5), Color32::from_white_alpha((235.0 * a) as u8));
    let chip = Rect::from_min_size(pos2(rect.min.x, rect.min.y - 26.0), galley.size() + vec2(14.0, 8.0));
    painter.rect_filled(chip, CornerRadius::same(6), chip_bg(a));
    painter.galley(chip.min + vec2(7.0, 4.0), galley, Color32::WHITE);
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// O que a animação põe por cima do canvas: as pranchetas sendo criadas, os gestos, os cursores e os controles. Chamado a cada quadro.
pub fn draw(app: &FigApp, painter: &Painter, ctx: &egui::Context) {
    let mut guard = state();
    let Some(show) = guard.as_mut() else { return };
    // O relógio anda com a velocidade e para na pausa
    let dt = show.last_tick.elapsed().as_secs_f32().min(0.1);
    show.last_tick = Instant::now();
    if !show.paused {
        show.clock += dt * SPEEDS[show.speed];
    }
    let t = show.clock;
    let end = show.plan.end;
    // Outro arquivo ou outra página: a animação acaba
    let still_here = app.session.ds().is_ok_and(|d| d.uid == show.doc_uid && d.page == show.page);
    if !still_here || t > end + HOLD + FADE || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        finish(&mut guard);
        ctx.request_repaint();
        return;
    }
    let page_to_screen = |p: Pos2| app.to_screen(f64::from(p.x), f64::from(p.y));
    let screen_all = Rect::from_min_max(page_to_screen(show.bounds.min), page_to_screen(show.bounds.max));
    let page_scale = screen_all.width() / show.bounds.width().max(1.0);
    let ppp = ctx.pixels_per_point();
    let to_screen = |p: Pos2| pos2(screen_all.min.x + (p.x - show.bounds.min.x) * page_scale, screen_all.min.y + (p.y - show.bounds.min.y) * page_scale);
    let on_screen = |r: Rect| Rect::from_min_max(to_screen(r.min), to_screen(r.max));

    // Pede o desenho da parte visível (com folga, para a câmera andar) na resolução da tela
    if show.last_sent.is_none_or(|at| at.elapsed().as_millis() >= 40) {
        let canvas = app.canvas.rect;
        let (a, b) = (app.to_page(canvas.min), app.to_page(canvas.max));
        let visible = Rect::from_min_max(pos2(a.x as f32, a.y as f32), pos2(b.x as f32, b.y as f32));
        let area = visible.expand2(visible.size() * 0.15).intersect(show.bounds.expand(48.0));
        if area.width() > 1.0 && area.height() > 1.0 {
            let longest = f64::from(area.width().max(area.height()));
            let scale = f64::from(page_scale * ppp).min(2600.0 / longest.max(1.0)).max(0.02);
            let _ = show.requests.send((t, scale, kurbo_of(area)));
            show.last_sent = Some(Instant::now());
        }
    }
    while let Ok(r) = show.results.try_recv() {
        let image = egui::ColorImage::from_rgba_unmultiplied(r.size, &r.rgba);
        match show.texture.as_mut() {
            Some((tex, area)) => {
                tex.set(image, egui::TextureOptions::LINEAR);
                *area = r.area;
            }
            None => show.texture = Some((ctx.load_texture("legatia-reveal", image, egui::TextureOptions::LINEAR), r.area)),
        }
    }

    // Cobre as pranchetas de verdade com o fundo da página e põe por cima o que já foi criado
    let alpha_out = if t > end + HOLD { (1.0 - (t - end - HOLD) / FADE).clamp(0.0, 1.0) } else { 1.0 };
    painter.rect_filled(screen_all.expand(48.0 * page_scale), 0.0, show.bg);
    if let Some((tex, area)) = &show.texture {
        painter.image(tex.id(), on_screen(*area), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    let canvas = app.canvas.rect.expand(40.0);
    let mut placed: Vec<Rect> = Vec::new();

    // Os gestos: caixas sendo arrastadas, o traço do texto sendo digitado e o estilo aplicado
    let (mut settles, mut styled): (Vec<&Action>, Vec<&Action>) = (Vec::new(), Vec::new());
    for a in &show.plan.actions {
        if t < a.arrive || t > a.work_end + SETTLE + 0.4 {
            continue;
        }
        let color = show.plan.cursors[a.cursor].color;
        match a.kind {
            Kind::Frame | Kind::Place if t < a.work_end => {
                let p = ease((t - a.arrive) / (a.work_end - a.arrive).max(0.001));
                let r = Rect::from_min_max(a.rect.min, a.rect.min + (a.rect.max - a.rect.min) * p);
                let screen = on_screen(r);
                if !canvas.intersects(screen) {
                    continue;
                }
                let label = (a.kind == Kind::Frame && on_screen(a.rect).width() >= 140.0).then(|| format!("{} × {}", r.width().round() as i64, r.height().round() as i64));
                drag_box(painter, &mut placed, screen, color, label.as_deref());
            }
            Kind::Frame if t < a.work_end + SETTLE + 0.35 && a.rect.width() >= 120.0 && a.rect.height() >= 48.0 => settles.push(a),
            Kind::Text if t < a.work_end => {
                if ((t * 7.0) as i64) % 2 != 0 {
                    continue;
                }
                let raw = ((t - a.arrive) / (a.work_end - a.arrive).max(0.001)).clamp(0.0, 1.0);
                let text = show.texts.get(a.node).map_or("", String::as_str);
                let frac = typed(text, raw, show.order[a.node].0) as f32 / text.chars().count().max(1) as f32;
                let x = a.rect.min.x + 2.0 + (a.rect.width() - 2.0) * frac;
                let h = a.rect.height().clamp(6.0, 60.0);
                painter.line_segment([to_screen(pos2(x, a.rect.center().y - h / 2.0)), to_screen(pos2(x, a.rect.center().y + h / 2.0))], Stroke::new(1.6, color));
            }
            Kind::Text if t < a.work_end + STYLE => styled.push(a),
            _ => {}
        }
    }
    // Os chips só para os mais recentes (com o enxame grande, chips demais virariam ruído)
    settles.sort_by(|a, b| b.work_end.partial_cmp(&a.work_end).unwrap_or(std::cmp::Ordering::Equal));
    for a in settles.iter().take(3) {
        let age = t - a.work_end;
        let fade = if age > SETTLE { (1.0 - (age - SETTLE) / 0.35).clamp(0.0, 1.0) } else { 1.0 };
        if let Some(d) = show.details.get(a.node) {
            inspector_chip(painter, on_screen(a.rect), d, ease(age / SETTLE), fade);
        }
    }
    styled.sort_by(|a, b| b.work_end.partial_cmp(&a.work_end).unwrap_or(std::cmp::Ordering::Equal));
    for (k, a) in styled.iter().enumerate() {
        let font = (k < 2).then(|| show.details.get(a.node).map_or("", |d| d.font.as_str()));
        style_applied(painter, on_screen(a.rect), font, show.plan.cursors[a.cursor].color, t - a.work_end);
    }

    // Os cursores: rastro curto, balanço quando parados, clique (encolhe e solta uma onda) e a etiqueta com o que estão fazendo
    for (i, c) in show.plan.cursors.iter().enumerate() {
        let (page_pos, moving) = cursor_at(c, t, show.bounds, end);
        let trail = &mut show.trails[i];
        trail.push_back((page_pos, t));
        while trail.front().is_some_and(|(_, at)| t - at > 0.1 || *at > t) || trail.len() > 10 {
            trail.pop_front();
        }
        let mut tip = to_screen(page_pos);
        if !canvas.contains(tip) {
            continue;
        }
        if moving && trail.len() > 2 {
            let pts: Vec<Pos2> = trail.iter().map(|(p, _)| to_screen(*p)).collect();
            for (k, seg) in pts.windows(2).enumerate() {
                let a = (k as f32 + 1.0) / pts.len() as f32;
                painter.line_segment([seg[0], seg[1]], Stroke::new(1.8 * a, c.color.gamma_multiply(0.3 * a * alpha_out)));
            }
        }
        if !moving {
            // Parado: um balanço leve, como a mão no mouse
            tip += vec2((t * 2.1 + c.phase).sin(), (t * 1.7 + c.phase).cos()) * 1.2;
        }
        let clicks = &show.plan.clicks[i];
        let since = clicks.partition_point(|at| *at <= t).checked_sub(1).map_or(f32::MAX, |k| t - clicks[k]);
        let press = if (0.0..0.12).contains(&since) { 0.86 } else { 1.0 };
        if (0.0..0.35).contains(&since) {
            let k = since / 0.35;
            painter.circle_stroke(tip, 3.0 + 12.0 * ease(k), Stroke::new(1.5, c.color.gamma_multiply((1.0 - k) * alpha_out)));
        }
        arrow(painter, tip, c.color, alpha_out, press);
        if let Some(label) = activity(&show.plan, i, t) {
            name_tag(painter, &mut placed, tip, label, c.color, alpha_out);
        }
    }

    // Os controles, discretos, no alto do canvas: pausar, velocidade e parar, com a barrinha do progresso
    let (mut stop_now, mut toggle_pause, mut next_speed) = (false, false, false);
    let top = pos2(app.canvas.rect.center().x, app.canvas.rect.min.y + 14.0);
    let area = egui::Area::new(egui::Id::new("legatia_reveal_controls")).order(egui::Order::Foreground).pivot(egui::Align2::CENTER_TOP).fixed_pos(top).show(ctx, |ui| {
        egui::Frame::new().fill(chip_bg(1.0)).corner_radius(CornerRadius::same(18)).inner_margin(egui::Margin::symmetric(6, 5)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let button = |label: &str| {
                    egui::Button::new(egui::RichText::new(label).font(semibold(11.5)).color(Color32::WHITE)).corner_radius(CornerRadius::same(13)).fill(Color32::from_white_alpha(14)).stroke(Stroke::NONE).min_size(vec2(34.0, 26.0))
                };
                toggle_pause = ui.add(button(if show.paused { "Continuar" } else { "Pausar" })).clicked();
                let speed = match SPEEDS[show.speed] {
                    s if s < 1.0 => "0,5×".to_string(),
                    s => format!("{}×", s as i64),
                };
                next_speed = ui.add(button(&speed)).on_hover_text("Velocidade").clicked();
                stop_now = ui.add(button("Parar")).on_hover_text("Parar (Esc)").clicked();
            });
        });
    });
    let pill = area.response.rect;
    let progress = (t / (end + HOLD).max(0.001)).clamp(0.0, 1.0);
    let track = Rect::from_min_size(pos2(pill.min.x + 14.0, pill.max.y + 5.0), vec2((pill.width() - 28.0).max(0.0), 2.0));
    painter.rect_filled(track, CornerRadius::same(1), Color32::from_black_alpha(40));
    painter.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * progress, 2.0)), CornerRadius::same(1), PALETTE[0]);
    if toggle_pause {
        show.paused = !show.paused;
    }
    if next_speed {
        show.speed = (show.speed + 1) % SPEEDS.len();
    }
    if stop_now {
        finish(&mut guard);
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(class: Class, x: f32, y: f32, w: f32, h: f32, parent: Option<usize>, root: usize) -> Info {
        Info { class, rect: Rect::from_min_size(pos2(x, y), vec2(w, h)), parent, root }
    }

    /// Duas pranchetas com cartões, textos e ícones (com peças).
    fn sample() -> (Vec<Info>, Vec<Rect>) {
        let roots = vec![Rect::from_min_size(pos2(0.0, 0.0), vec2(1440.0, 900.0)), Rect::from_min_size(pos2(1600.0, 0.0), vec2(1440.0, 900.0))];
        let mut infos = Vec::new();
        for (r, b) in roots.iter().enumerate() {
            let root = infos.len();
            infos.push(info(Class::Container, b.min.x, b.min.y, b.width(), b.height(), None, r));
            for i in 0..8 {
                let x = b.min.x + 40.0 + 170.0 * i as f32;
                let card = infos.len();
                infos.push(info(Class::Container, x, 100.0, 150.0, 200.0, Some(root), r));
                infos.push(info(Class::Text(14 + i), x + 10.0, 120.0, 120.0, 18.0, Some(card), r));
                let icon = infos.len();
                infos.push(info(Class::Container, x + 10.0, 160.0, 24.0, 24.0, Some(card), r));
                infos.push(info(Class::Other, x + 12.0, 162.0, 20.0, 20.0, Some(icon), r));
                infos.push(info(Class::Other, x + 14.0, 164.0, 10.0, 10.0, Some(icon), r));
            }
        }
        (infos, roots)
    }

    #[test]
    fn the_swarm_grows_with_the_design() {
        assert_eq!(cursor_count(3), 6);
        assert_eq!(cursor_count(140), 20);
        assert_eq!(cursor_count(100_000), 40);
    }

    #[test]
    fn text_is_typed_in_a_time_that_grows_with_its_length_up_to_a_limit() {
        assert_eq!(type_secs(1), 0.35);
        assert!((type_secs(20) - 0.9).abs() < 1e-5);
        assert_eq!(type_secs(5000), 1.6);
    }

    #[test]
    fn typing_shows_whole_letters_not_bytes() {
        assert_eq!(prefix("Olá, mundo", 3), "Olá");
        assert_eq!(prefix("ação", 2), "aç");
        assert_eq!(prefix("abc", 10), "abc");
    }

    #[test]
    fn typing_has_a_human_rhythm_but_always_moves_forward_and_finishes() {
        let text = "Bom dia, time. Três entregas esta semana.";
        let seed = 42;
        let counts: Vec<usize> = (0..=100).map(|i| typed(text, i as f32 / 100.0, seed)).collect();
        assert_eq!(counts[0], 0);
        assert_eq!(*counts.last().unwrap(), text.chars().count());
        assert!(counts.windows(2).all(|w| w[1] >= w[0]), "nunca apaga");
        assert_eq!(typed(text, 0.37, seed), typed(text, 0.37, seed));
        let linear: Vec<usize> = (0..=100).map(|i| ((i as f32 / 100.0) * text.chars().count() as f32) as usize).collect();
        assert!(counts.iter().zip(&linear).any(|(a, b)| a != b));
    }

    #[test]
    fn every_layer_is_created_by_a_gesture_after_the_layer_that_holds_it() {
        let (infos, roots) = sample();
        let p = plan(&infos, &roots);
        // Toda camada tem hora para aparecer; as peças de um ícone aparecem com ele (sem gesto próprio); o resto tem um gesto
        assert!(p.anims.iter().all(|a| a.start.is_finite() && a.start < 1.0e6));
        let glued = attached(&infos);
        assert_eq!(glued.iter().filter(|g| **g).count(), 2 * 8 * 2, "as duas peças de cada ícone");
        assert_eq!(p.actions.len(), infos.len() - 32);
        let mut done = vec![false; infos.len()];
        for a in &p.actions {
            done[a.node] = true;
        }
        assert!((0..infos.len()).all(|i| done[i] || glued[i]), "nada surge sem ser criado");
        // Ninguém começa antes de quem o contém existir (o pai existe quando o arrasto dele termina)
        for (i, inf) in infos.iter().enumerate() {
            if let Some(par) = inf.parent {
                let parent_ready = p.actions.iter().find(|a| a.node == par).map_or(p.anims[par].start, |a| if a.kind == Kind::Text { a.arrive } else { a.work_end });
                let own = p.actions.iter().find(|a| a.node == i).map_or(p.anims[i].start, |a| a.arrive);
                assert!(own + 1e-4 >= parent_ready, "camada {i} começou em {own}, o pai existe em {parent_ready}");
            }
        }
        // Um cursor nunca faz duas coisas ao mesmo tempo, e as tarefas dele ficam em ordem
        for list in &p.by_cursor {
            assert!(list.windows(2).all(|w| p.actions[w[0]].free <= p.actions[w[1]].arrive + 1e-4));
        }
        // O tempo sai do trabalho: um enxame grande faz ~50 tarefas em poucos segundos (nada de um relógio fixo)
        assert!(p.cursors.len() >= 6);
        assert!(p.end > 2.0 && p.end < 20.0, "{}", p.end);
        // A primeira prancheta começa antes da segunda
        let first_of = |r: usize| p.actions.iter().filter(|a| a.root == r).map(|a| a.arrive).fold(f32::MAX, f32::min);
        assert!(first_of(0) <= first_of(1));
        // A etiqueta diz o que o cursor faz no meio do arrasto e da digitação
        let d = p.actions.iter().find(|a| a.kind == Kind::Frame).unwrap();
        assert_eq!(activity(&p, d.cursor, (d.arrive + d.work_end) / 2.0), Some("Criando"));
        let ty = p.actions.iter().find(|a| a.kind == Kind::Text).unwrap();
        assert_eq!(activity(&p, ty.cursor, (ty.arrive + ty.work_end) / 2.0), Some("Digitando"));
    }

    #[test]
    fn a_cursor_flies_brakes_past_the_point_drags_straight_and_the_swarm_gathers_at_the_end() {
        let bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let c = Cursor {
            start: pos2(0.0, 0.0),
            phase: 0.0,
            color: PALETTE[0],
            legs: vec![
                Leg { depart: 1.0, arrive: 2.0, from: pos2(0.0, 0.0), to: pos2(100.0, 0.0), straight: false },
                Leg { depart: 3.0, arrive: 4.0, from: pos2(100.0, 0.0), to: pos2(100.0, 50.0), straight: true },
            ],
        };
        assert_eq!(cursor_at(&c, 0.5, bounds, 9.0), (pos2(0.0, 0.0), false));
        let (mid, moving) = cursor_at(&c, 1.5, bounds, 9.0);
        assert!(moving && mid.x > 20.0 && mid.y.abs() > 1.0, "{mid:?}");
        let (late, _) = cursor_at(&c, 1.85, bounds, 9.0);
        assert!(late.x > 99.0, "{late:?}");
        let (stop, moving) = cursor_at(&c, 2.5, bounds, 9.0);
        assert!(!moving && (stop - pos2(100.0, 0.0)).length() < 0.01, "{stop:?}");
        let (drag, _) = cursor_at(&c, 3.5, bounds, 9.0);
        assert!((drag.x - 100.0).abs() < 0.01 && drag.y > 10.0 && drag.y < 40.0, "{drag:?}");
        let (orbit, _) = cursor_at(&c, 12.0, bounds, 9.0);
        assert!((orbit - bounds.center()).length() > 100.0, "{orbit:?}");
    }
}
