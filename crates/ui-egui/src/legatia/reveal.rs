//! LegatIA: a animação "construindo ao vivo", para demonstrar um design.
//!
//! Não executa nada no design: as pranchetas que já existem (as selecionadas, ou todas as da página) são desenhadas de novo, camada por
//! camada, enquanto um enxame de cursores genéricos (sem marca de nenhuma IA: as marcas só aparecem quando um agente de verdade mexe no
//! arquivo) cria tudo à vista: **arrasta** um retângulo para fazer cada quadro (com o tamanho aparecendo), os cantos **arredondam** e as
//! cores **trocam** do cinza para a cor final, os textos são **digitados** letra por letra, e o resto entra em seguida. As camadas ficam
//! sempre no lugar final, então nada se mexe.
//!
//! O documento não é tocado (nada no desfazer, nada alterado): a linha de fundo tira uma cópia (barata: o documento é persistente) e muda
//! nela só o que está no meio da animação (raio dos cantos, cor, opacidade, texto até a letra N) antes de desenhar, deixando de fora o que
//! ainda não entrou (`figcraft_render::render_area_skipping`). Esc ou o botão Parar termina e o desenho de verdade volta, igual ao último
//! quadro da animação.
//!
//! A ideia vem do Swarm Rebuild (cursores coloridos reconstruindo uma página), reescrita do zero: aquele projeto não tem licença.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use figcraft_doc::{Document, NodeId, NodeKind};

use crate::FigApp;
use crate::theme::{Tokens, doc_c32, regular, semibold};

/// O enxame: cada cursor com um papel e uma cor (as cores da LegatIA e três de apoio). Nada de marcas de IA.
const SWARM: [(&str, Color32); 4] = [
    ("Estrutura", Color32::from_rgb(0x00, 0x6F, 0xFE)),
    ("Cores", Color32::from_rgb(0x22, 0xC5, 0xE0)),
    ("Texto", Color32::from_rgb(0xA7, 0x8B, 0xFA)),
    ("Detalhes", Color32::from_rgb(0xFB, 0x92, 0x3C)),
];

/// Quanto o cursor leva para ir de uma camada à outra.
const GLIDE: f32 = 0.32;
/// Quanto dura o arrastar que cria um quadro.
const DRAG: f32 = 0.55;
/// Depois de criado, quanto o quadro leva para arredondar os cantos e ir à cor final.
const SETTLE: f32 = 0.7;
/// Uma camada comum entra (aparece) neste tempo.
const POP: f32 = 0.28;
/// Antes da primeira batida e depois da última (os cursores entram e saem).
const LEAD_IN: f32 = 0.5;
const HOLD: f32 = 1.4;
const FADE: f32 = 0.7;
/// O cinza de onde as cores saem.
const PLACEHOLDER: figcraft_color::Color = figcraft_color::Color { r: 0.84, g: 0.85, b: 0.88, a: 1.0 };
/// No máximo de pranchetas de uma vez.
const MAX_ROOTS: usize = 24;

// ---------- o roteiro (puro: não mexe em nada)

/// O que cada camada é, para o roteiro decidir o que fazer com ela.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Class {
    /// Quadros, cartões, seções, formas: criados arrastando.
    Container,
    /// Texto com tantas letras: digitado.
    Text(usize),
    /// O resto (vetores, imagens, linhas): só entra.
    Other,
}

#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub class: Class,
    /// O retângulo na página.
    pub rect: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Pop,
    /// O quadro foi criado arrastando: depois que aparece, arredonda e ganha a cor.
    Settle,
    /// Digitado.
    Type,
}

/// A vida de uma camada: aparece em `start` e leva `dur` para terminar o que faz.
#[derive(Clone, Copy, Debug)]
pub struct Anim {
    pub start: f32,
    pub dur: f32,
    pub mode: Mode,
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
    /// Cada cursor tem a sua fase (a tremidinha da mão e o ponto da órbita no fim).
    phase: f32,
}

/// Um quadro sendo arrastado: o retângulo cresce do canto de cima à esquerda ao de baixo à direita entre `from` e `to`.
struct DragBox {
    rect: Rect,
    from: f32,
    to: f32,
    cursor: usize,
}

/// Um texto sendo digitado: o traço do cursor de texto anda pela caixa entre `start` e `start + dur`.
struct TypeRun {
    rect: Rect,
    start: f32,
    dur: f32,
    cursor: usize,
}

/// Uma batida: o instante em que um grupo de camadas entra, e o cursor que chega a elas.
struct Beat {
    at: f32,
    cursor: usize,
    /// O retângulo da primeira camada da batida, onde o pulso aparece.
    target: Rect,
}

struct Plan {
    beats: Vec<Beat>,
    anims: Vec<Anim>,
    cursors: Vec<Cursor>,
    drags: Vec<DragBox>,
    types: Vec<TypeRun>,
    /// Quando a última coisa termina.
    end: f32,
}

/// Em quantas batidas a criação se divide: uma por camada até um limite (as camadas de um grupo entram juntas).
pub fn beat_count(layers: usize) -> usize {
    layers.clamp(1, 110)
}

/// Os instantes das batidas: começa devagar e aperta, e dura de 5 a 20 s (cerca de 0,17 s por batida).
pub fn beat_times(beats: usize) -> Vec<f32> {
    let total = (beats as f32 * 0.17).clamp(5.0, 20.0);
    (0..beats).map(|k| LEAD_IN + total * (k as f32 / beats as f32).powf(0.95)).collect()
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Quanto tempo um texto de `chars` letras leva para ser digitado.
pub fn type_secs(chars: usize) -> f32 {
    (chars as f32 * 0.04).clamp(0.3, 1.4)
}

/// O que cada camada faz e quando, e por onde cada cursor passa. `bounds` é o retângulo de todas as pranchetas.
fn plan(infos: &[Info], bounds: Rect) -> Plan {
    let n = infos.len();
    let beats_n = beat_count(n);
    let times = beat_times(beats_n);
    let k_cursors = SWARM.len();
    let mut cursors: Vec<Cursor> = (0..k_cursors)
        .map(|i| {
            // Cada cursor entra de um canto diferente, fora das pranchetas
            let corner = [(-0.12, -0.1), (1.1, -0.1), (-0.12, 1.1), (1.1, 1.1)][i % 4];
            Cursor { legs: Vec::new(), start: pos2(bounds.min.x + bounds.width() * corner.0, bounds.min.y + bounds.height() * corner.1), phase: i as f32 * std::f32::consts::FRAC_PI_2 }
        })
        .collect();
    let mut busy = vec![0.0_f32; k_cursors];
    let mut anims = vec![Anim { start: f32::MAX, dur: 0.0, mode: Mode::Pop }; n];
    let (mut beats, mut drags, mut types) = (Vec::with_capacity(beats_n), Vec::new(), Vec::new());
    let mut end = 0.0_f32;

    // Uma perna do caminho do cursor `c`: chega a `to` em `arrive` (sai antes, sem pisar na perna anterior)
    let go = |cursors: &mut Vec<Cursor>, c: usize, arrive: f32, to: Pos2, straight: bool, lead: f32| {
        let cur = &mut cursors[c];
        let (prev_at, prev_pos) = cur.legs.last().map_or((0.0, cur.start), |l| (l.arrive, l.to));
        let depart = (arrive - lead).max(prev_at).min(arrive);
        cur.legs.push(Leg { depart, arrive, from: prev_pos, to, straight });
    };

    for (k, at) in times.iter().enumerate() {
        let at = *at;
        let from = (k * n / beats_n).min(n - 1);
        let to = ((k + 1) * n / beats_n).max(from + 1).min(n);
        let first = infos[from];
        let c = k % k_cursors;
        let min = first.rect.min;
        let big_enough = first.rect.width() >= 30.0 && first.rect.height() >= 20.0;
        let mut hero = Mode::Pop;
        let mut hero_dur = POP;
        match first.class {
            Class::Container if big_enough && busy[c] <= at - DRAG - 0.05 => {
                // O cursor vai ao canto e arrasta até o canto oposto: o quadro nasce no fim do arrasto
                go(&mut cursors, c, at - DRAG, first.rect.min, false, GLIDE);
                go(&mut cursors, c, at, first.rect.max, true, DRAG);
                drags.push(DragBox { rect: first.rect, from: at - DRAG, to: at, cursor: c });
                busy[c] = at + 0.1;
                hero = Mode::Settle;
                hero_dur = SETTLE;
            }
            Class::Text(chars) if chars >= 3 && busy[c] <= at - 0.05 => {
                let dur = type_secs(chars);
                let line = pos2(min.x + 3.0, first.rect.center().y);
                go(&mut cursors, c, at, line, false, GLIDE);
                go(&mut cursors, c, at + dur, pos2(first.rect.max.x.min(min.x + 3.0 + first.rect.width() * 0.95), line.y), true, dur);
                types.push(TypeRun { rect: first.rect, start: at, dur, cursor: c });
                busy[c] = at + dur;
                hero = Mode::Type;
                hero_dur = dur;
            }
            _ => {
                go(&mut cursors, c, at, first.rect.center(), false, GLIDE);
                busy[c] = busy[c].max(at);
            }
        }
        anims[from] = Anim { start: at, dur: hero_dur, mode: hero };
        end = end.max(at + hero_dur);
        for a in anims.iter_mut().take(to).skip(from + 1) {
            *a = Anim { start: at, dur: POP, mode: Mode::Pop };
        }
        end = end.max(at + POP);
        beats.push(Beat { at, cursor: c, target: first.rect });
    }
    Plan { beats, anims, cursors, drags, types, end }
}

/// Onde o cursor está no instante `t` (coordenadas da página): voa em arco de uma camada à outra, com uma tremidinha de mão; arrasta e
/// segue o texto em linha reta; e depois da última coisa o enxame se junta no meio das pranchetas e gira (como no Swarm Rebuild).
fn cursor_at(c: &Cursor, t: f32, bounds: Rect, end: f32) -> Pos2 {
    let mut pos = c.start;
    for (i, leg) in c.legs.iter().enumerate() {
        if t < leg.depart {
            break;
        }
        let k = if leg.arrive > leg.depart { ease((t - leg.depart) / (leg.arrive - leg.depart)) } else { 1.0 };
        if leg.straight {
            pos = leg.from + (leg.to - leg.from) * k;
        } else {
            // Um arco: o ponto de controle sai do meio do caminho para o lado (cada trecho para um lado)
            let mid = leg.from + (leg.to - leg.from) * 0.5;
            let along = leg.to - leg.from;
            let side = if i % 2 == 0 { 1.0 } else { -1.0 };
            let ctrl = mid + vec2(-along.y, along.x) * (0.18 * side);
            let u = 1.0 - k;
            pos = pos2(u * u * leg.from.x + 2.0 * u * k * ctrl.x + k * k * leg.to.x, u * u * leg.from.y + 2.0 * u * k * ctrl.y + k * k * leg.to.y);
            // A tremidinha some quando o cursor chega
            let wobble = 2.5 * (k * (1.0 - k)) * 4.0 * (t * 9.0 + c.phase).sin();
            pos += vec2(wobble, wobble * 0.6);
        }
    }
    if t > end {
        // O enxame se junta e gira em volta das pranchetas
        let k = ease((t - end) / 0.7);
        let r = bounds.width().min(bounds.height()) * 0.42;
        let ang = c.phase + (t - end) * 2.2;
        let orbit = bounds.center() + vec2(ang.cos(), ang.sin()) * r;
        pos = pos + (orbit - pos) * k;
    }
    pos
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
        let p = ease((t - a.start) / a.dur.max(0.001));
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
                    let total = whole.characters.chars().count();
                    let shown = ((total as f32) * p).ceil() as usize;
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
}

struct Show {
    doc_uid: u64,
    page: NodeId,
    /// O retângulo de todas as pranchetas na página.
    bounds: Rect,
    plan: Plan,
    started: Instant,
    bg: Color32,
    /// Pedidos de desenho (o instante e a escala) e as respostas da linha de fundo.
    requests: Sender<(f32, f64)>,
    results: Receiver<Rendered>,
    last_sent: Option<Instant>,
    texture: Option<egui::TextureHandle>,
}

static SHOW: Mutex<Option<Show>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<Show>> {
    SHOW.lock().unwrap_or_else(|e| e.into_inner())
}

/// Está rodando.
pub fn running() -> bool {
    state().is_some()
}

pub fn stop() {
    *state() = None;
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

/// As camadas das pranchetas por nível (as pranchetas primeiro, depois os filhos, os netos…), cada nível na ordem das camadas.
pub fn levels(doc: &Document, roots: &[NodeId]) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = roots.to_vec();
    let mut level: Vec<NodeId> = roots.to_vec();
    while !level.is_empty() && out.len() < 20_000 {
        let mut next = Vec::new();
        for id in &level {
            if let Some(n) = doc.node(*id) {
                next.extend(n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|k| k.visible)));
            }
        }
        out.extend(next.iter().copied());
        level = next;
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

/// Começa a animação das pranchetas selecionadas (ou de todas as da página). Só olha o desenho: nada no arquivo muda.
pub fn start(app: &mut FigApp, ctx: &egui::Context) {
    if cfg!(test) {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let (doc, page, uid) = (ds.doc.clone(), ds.page, ds.uid);
    let roots = pick_roots(&doc, page, &ds.selection);
    if roots.is_empty() {
        app.toast("Não achei uma prancheta com camadas para demonstrar nesta página.");
        return;
    }
    let order = levels(&doc, &roots);
    if order.len() < 3 {
        app.toast("Esta prancheta tem poucas camadas para uma demonstração.");
        return;
    }
    let Some(bounds_k) = roots.iter().filter_map(|r| figcraft_render::export_bounds(&doc, *r)).reduce(|a, b| a.union(b)) else { return };
    let bounds = rect_of(bounds_k);
    // Enquadra as pranchetas na tela (só a vista, o arquivo não muda)
    app.run("select.set", serde_json::json!({"ids": roots}));
    app.run("view.zoomSelection", serde_json::json!({}));

    let infos: Vec<Info> = order.iter().map(|id| Info { class: class_of(&doc, *id), rect: doc.world_render_bounds(*id).map(rect_of).filter(|r| r.width() > 0.0 && r.height() > 0.0).unwrap_or(bounds) }).collect();
    let plan = plan(&infos, bounds);
    let bg = page_bg(app, ctx);

    // A linha de fundo desenha as pranchetas no instante pedido; só o último pedido importa
    let (req_tx, req_rx) = channel::<(f32, f64)>();
    let (res_tx, res_rx) = channel::<Rendered>();
    let worker_doc: Arc<Document> = doc.clone();
    let anims = plan.anims.clone();
    let ctx2 = ctx.clone();
    let _ = std::thread::Builder::new().name("legatia-reveal".into()).spawn(move || {
        while let Ok(mut want) = req_rx.recv() {
            while let Ok(newer) = req_rx.try_recv() {
                want = newer;
            }
            let (t, scale) = want;
            let (frame, skip) = frame_at(&worker_doc, &order, &anims, t);
            if let Some(pix) = figcraft_render::render_area_skipping(&frame, &roots, bounds_k, scale, &skip) {
                let rgba = figcraft_render::to_rgba8(&pix);
                if res_tx.send(Rendered { size: [pix.width() as usize, pix.height() as usize], rgba }).is_err() {
                    return;
                }
                ctx2.request_repaint();
            }
        }
    });
    *state() = Some(Show { doc_uid: uid, page, bounds, plan, started: Instant::now(), bg, requests: req_tx, results: res_rx, last_sent: None, texture: None });
    ctx.request_repaint();
}

// ---------- o desenho por cima do canvas

/// A setinha do cursor com a ponta em `tip`.
fn arrow(painter: &Painter, tip: Pos2, color: Color32, alpha: f32) {
    let pts = [(0.0, 0.0), (0.0, 17.0), (4.5, 13.2), (8.0, 20.0), (10.6, 18.8), (7.2, 12.2), (12.5, 12.2)];
    let points: Vec<Pos2> = pts.iter().map(|(x, y)| tip + vec2(*x, *y)).collect();
    let fade = |c: Color32| c.gamma_multiply(alpha);
    // A seta não é convexa: o corpo e a cauda são pintados em duas peças, e o contorno vai por cima
    let at = |i: usize| points[i];
    painter.add(Shape::convex_polygon(vec![at(0), at(1), at(2), at(6)], fade(color), Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![at(2), at(3), at(4), at(5)], fade(color), Stroke::NONE));
    painter.add(Shape::closed_line(points, Stroke::new(1.4, fade(Color32::WHITE))));
}

fn name_tag(painter: &Painter, ctx: &egui::Context, at: Pos2, name: &str, color: Color32, alpha: f32) {
    let galley = painter.layout_no_wrap(name.to_string(), semibold(11.0), Color32::WHITE.gamma_multiply(alpha));
    // A foto do agente à esquerda (só se o nome for de um agente que conhecemos: aqui, nunca)
    let known = crate::legatia::agent_marks::brand_of(name).is_some();
    let photo = if known { 18.0 } else { 0.0 };
    let size = vec2(galley.size().x + 16.0 + if known { photo + 2.0 } else { 0.0 }, galley.size().y.max(photo) + 8.0);
    let rect = Rect::from_min_size(at + vec2(11.0, 20.0), size);
    painter.rect_filled(rect, CornerRadius::same(13), color.gamma_multiply(alpha));
    if known {
        crate::legatia::agent_marks::paint(painter, ctx, pos2(rect.min.x + 4.0 + photo / 2.0, rect.center().y), photo / 2.0, name, alpha);
    }
    painter.galley(pos2(rect.min.x + 8.0 + if known { photo + 2.0 } else { 0.0 }, rect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
}

/// A camada pulsa um instante quando o cursor chega (o contorno cresce e some).
fn pulse(painter: &Painter, rect: Rect, color: Color32, age: f32) {
    if !(0.0..0.5).contains(&age) {
        return;
    }
    let k = age / 0.5;
    let grow = 2.0 + 8.0 * ease(k);
    painter.rect_stroke(rect.expand(grow), CornerRadius::same(4), Stroke::new(2.0, color.gamma_multiply(1.0 - k)), StrokeKind::Outside);
}

/// O retângulo que está sendo arrastado: contorno na cor do cursor, quatro alças e o tamanho (largura × altura) num rótulo.
fn drag_box(painter: &Painter, rect: Rect, color: Color32, size_label: &str) {
    painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.6, color), StrokeKind::Middle);
    painter.rect_filled(rect, CornerRadius::ZERO, color.gamma_multiply(0.10));
    for corner in [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()] {
        let h = Rect::from_center_size(corner, vec2(7.0, 7.0));
        painter.rect_filled(h, CornerRadius::ZERO, Color32::WHITE);
        painter.rect_stroke(h, CornerRadius::ZERO, Stroke::new(1.2, color), StrokeKind::Middle);
    }
    let galley = painter.layout_no_wrap(size_label.to_string(), semibold(10.5), Color32::WHITE);
    let tag = Rect::from_min_size(pos2(rect.center().x - galley.size().x / 2.0 - 7.0, rect.max.y + 8.0), galley.size() + vec2(14.0, 6.0));
    painter.rect_filled(tag, CornerRadius::same(4), color);
    painter.galley(tag.min + vec2(7.0, 3.0), galley, Color32::WHITE);
}

/// O que a animação põe por cima do canvas: as pranchetas sendo criadas, os cursores e o aviso. Chamado a cada quadro.
pub fn draw(app: &FigApp, painter: &Painter, ctx: &egui::Context) {
    let mut guard = state();
    let Some(show) = guard.as_mut() else { return };
    // Outro arquivo ou outra página: a animação acaba
    let still_here = app.session.ds().is_ok_and(|d| d.uid == show.doc_uid && d.page == show.page);
    let t = show.started.elapsed().as_secs_f32();
    let end = show.plan.end;
    if !still_here || t > end + HOLD + FADE || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        *guard = None;
        ctx.request_repaint();
        return;
    }
    let to_screen_page = |p: Pos2| app.to_screen(f64::from(p.x), f64::from(p.y));
    let (min, max) = (to_screen_page(show.bounds.min), to_screen_page(show.bounds.max));
    let screen = Rect::from_min_max(min, max);
    let ppp = ctx.pixels_per_point();
    let page_scale = screen.width() / show.bounds.width().max(1.0);

    // Pede o desenho deste instante à linha de fundo (ela junta os pedidos: só o último conta)
    let want_scale = {
        let s = f64::from(screen.width() * ppp) / f64::from(show.bounds.width().max(1.0));
        // Sem passar de ~2400 px de lado
        s.min(2400.0 / f64::from(show.bounds.width().max(show.bounds.height()).max(1.0))).max(0.03)
    };
    if show.last_sent.is_none_or(|at| at.elapsed().as_millis() >= 40) {
        let _ = show.requests.send((t, want_scale));
        show.last_sent = Some(Instant::now());
    }
    while let Ok(r) = show.results.try_recv() {
        let image = egui::ColorImage::from_rgba_unmultiplied(r.size, &r.rgba);
        match show.texture.as_mut() {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => show.texture = Some(ctx.load_texture("legatia-reveal", image, egui::TextureOptions::LINEAR)),
        }
    }

    // Cobre as pranchetas de verdade com o fundo da página e põe por cima o que já foi criado
    let alpha_out = if t > end + HOLD { (1.0 - (t - end - HOLD) / FADE).clamp(0.0, 1.0) } else { 1.0 };
    painter.rect_filled(screen.expand(48.0 * page_scale), 0.0, show.bg);
    if let Some(tex) = &show.texture {
        painter.image(tex.id(), screen, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }

    let to_screen = |p: Pos2| pos2(screen.min.x + (p.x - show.bounds.min.x) * page_scale, screen.min.y + (p.y - show.bounds.min.y) * page_scale);
    let on_screen = |r: Rect| Rect::from_min_max(to_screen(r.min), to_screen(r.max));
    // Os quadros sendo arrastados: o retângulo cresce do canto de cima ao de baixo, com o tamanho
    for d in &show.plan.drags {
        if (d.from..d.to).contains(&t) {
            let p = ease((t - d.from) / (d.to - d.from).max(0.001));
            let corner = d.rect.min + (d.rect.max - d.rect.min) * p;
            let r = Rect::from_min_max(d.rect.min, corner);
            let label = format!("{} × {}", r.width().round() as i64, r.height().round() as i64);
            drag_box(painter, on_screen(r), SWARM[d.cursor % SWARM.len()].1, &label);
        }
    }
    // Os textos sendo digitados: o traço do cursor de texto anda pela caixa, piscando
    for run in &show.plan.types {
        if (run.start..run.start + run.dur).contains(&t) && ((t * 7.0) as i64) % 2 == 0 {
            let p = ((t - run.start) / run.dur.max(0.001)).clamp(0.0, 1.0);
            let x = run.rect.min.x + 3.0 + (run.rect.width() * 0.95) * p;
            let h = run.rect.height().clamp(10.0, 60.0);
            let (a, b) = (to_screen(pos2(x, run.rect.center().y - h / 2.0)), to_screen(pos2(x, run.rect.center().y + h / 2.0)));
            painter.line_segment([a, b], Stroke::new(1.8, SWARM[run.cursor % SWARM.len()].1));
        }
    }
    for beat in &show.plan.beats {
        pulse(painter, on_screen(beat.target), SWARM[beat.cursor % SWARM.len()].1, t - beat.at);
    }
    for (i, c) in show.plan.cursors.iter().enumerate() {
        let tip = to_screen(cursor_at(c, t, show.bounds, end));
        let (name, color) = SWARM[i % SWARM.len()];
        arrow(painter, tip, color, alpha_out);
        name_tag(painter, ctx, tip, name, color, alpha_out);
    }

    // O aviso e o botão para parar
    let hud = Rect::from_center_size(pos2(app.canvas.rect.center().x, app.canvas.rect.min.y + 34.0), vec2(300.0, 34.0));
    painter.rect_filled(hud, CornerRadius::same(17), Color32::from_rgba_unmultiplied(16, 18, 24, 220));
    painter.text(hud.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, "Demonstração: nada muda no arquivo", regular(11.5), Color32::from_white_alpha(220));
    painter.text(hud.right_center() - vec2(14.0, 0.0), Align2::RIGHT_CENTER, "Esc", semibold(11.0), Color32::from_white_alpha(150));
    let mut stop_now = false;
    egui::Area::new(egui::Id::new("legatia_reveal_stop")).order(egui::Order::Foreground).fixed_pos(pos2(hud.max.x + 8.0, hud.min.y + 3.0)).show(ctx, |ui| {
        let b = egui::Button::new(egui::RichText::new("Parar").font(semibold(11.5)).color(Color32::WHITE)).corner_radius(CornerRadius::same(14)).fill(Color32::from_rgba_unmultiplied(16, 18, 24, 220)).stroke(Stroke::new(1.0, Color32::from_white_alpha(40)));
        stop_now = ui.add(b).clicked();
    });
    if stop_now {
        *guard = None;
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(class: Class, x: f32, y: f32, w: f32, h: f32) -> Info {
        Info { class, rect: Rect::from_min_size(pos2(x, y), vec2(w, h)) }
    }

    #[test]
    fn the_beats_run_from_five_to_twenty_seconds_and_speed_up() {
        let t = beat_times(30);
        assert_eq!(t.len(), 30);
        assert!(t.windows(2).all(|w| w[1] >= w[0]), "sempre para a frente");
        assert!(*beat_times(3).last().unwrap() >= LEAD_IN + 3.0);
        assert!(*beat_times(110).last().unwrap() <= LEAD_IN + 20.0);
        assert_eq!(beat_count(1), 1);
        assert_eq!(beat_count(9000), 110);
    }

    #[test]
    fn text_is_typed_in_a_time_that_grows_with_its_length_up_to_a_limit() {
        assert_eq!(type_secs(1), 0.3);
        assert!((type_secs(20) - 0.8).abs() < 1e-5);
        assert_eq!(type_secs(5000), 1.4);
    }

    #[test]
    fn typing_shows_whole_letters_not_bytes() {
        assert_eq!(prefix("Olá, mundo", 3), "Olá");
        assert_eq!(prefix("ação", 2), "aç");
        assert_eq!(prefix("abc", 10), "abc");
    }

    #[test]
    fn the_plan_drags_frames_types_texts_and_pops_the_rest_in_order() {
        // Duas pranchetas, cartões dentro, textos dentro, e uns detalhes
        let mut infos = vec![info(Class::Container, 0.0, 0.0, 400.0, 300.0), info(Class::Container, 500.0, 0.0, 400.0, 300.0)];
        for i in 0..6 {
            infos.push(info(Class::Container, 20.0 + 60.0 * i as f32, 20.0, 50.0, 40.0));
        }
        for i in 0..6 {
            infos.push(info(Class::Text(12 + i), 20.0, 100.0 + 20.0 * i as f32, 200.0, 16.0));
        }
        for _ in 0..6 {
            infos.push(info(Class::Other, 10.0, 10.0, 8.0, 8.0));
        }
        let p = plan(&infos, Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 300.0)));
        // Toda camada tem uma hora para entrar, e as pranchetas (as primeiras) entram antes do resto
        assert!(p.anims.iter().all(|a| a.start.is_finite() && a.start < 1.0e6));
        assert!(p.anims[0].start <= p.anims[infos.len() - 1].start);
        // Há quadros arrastados e textos digitados, e cada um termina antes de a animação acabar
        assert!(!p.drags.is_empty() && !p.types.is_empty());
        assert!(p.drags.iter().all(|d| d.to <= p.end && d.from < d.to));
        assert!(p.types.iter().all(|r| r.start + r.dur <= p.end + 1e-3));
        // Um cursor nunca é mandado a duas coisas ao mesmo tempo: o arrasto e a digitação de um mesmo cursor não se sobrepõem
        for c in 0..SWARM.len() {
            let mut spans: Vec<(f32, f32)> = p.drags.iter().filter(|d| d.cursor == c).map(|d| (d.from, d.to)).chain(p.types.iter().filter(|r| r.cursor == c).map(|r| (r.start, r.start + r.dur))).collect();
            spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            assert!(spans.windows(2).all(|w| w[0].1 <= w[1].0 + 1e-3), "cursor {c}: {spans:?}");
        }
    }

    #[test]
    fn a_cursor_flies_between_its_stops_drags_straight_and_the_swarm_gathers_at_the_end() {
        let bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let c = Cursor {
            start: pos2(0.0, 0.0),
            phase: 0.0,
            legs: vec![
                Leg { depart: 1.0, arrive: 2.0, from: pos2(0.0, 0.0), to: pos2(100.0, 0.0), straight: false },
                Leg { depart: 3.0, arrive: 4.0, from: pos2(100.0, 0.0), to: pos2(100.0, 50.0), straight: true },
            ],
        };
        assert_eq!(cursor_at(&c, 0.5, bounds, 9.0), pos2(0.0, 0.0));
        // No meio do voo já saiu do ponto de partida e ainda não chegou (e o arco o tira da reta)
        let mid = cursor_at(&c, 1.5, bounds, 9.0);
        assert!(mid.x > 20.0 && mid.x < 80.0 && mid.y.abs() > 1.0, "{mid:?}");
        let stop = cursor_at(&c, 2.5, bounds, 9.0);
        assert!((stop - pos2(100.0, 0.0)).length() < 0.01, "{stop:?}");
        // Arrastando: em linha reta
        let drag = cursor_at(&c, 3.5, bounds, 9.0);
        assert!((drag.x - 100.0).abs() < 0.01 && drag.y > 10.0 && drag.y < 40.0, "{drag:?}");
        assert!((cursor_at(&c, 4.0, bounds, 9.0) - pos2(100.0, 50.0)).length() < 0.01);
        // Depois da última coisa vai para a órbita em volta das pranchetas
        let orbit = cursor_at(&c, 12.0, bounds, 9.0);
        assert!((orbit - bounds.center()).length() > 100.0, "{orbit:?}");
    }
}
