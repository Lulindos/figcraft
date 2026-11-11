//! LegatIA: a animação "construindo ao vivo", para demonstrar um design.
//!
//! Não executa nada no design: um quadro que já existe é desenhado de novo, camada por camada, enquanto um enxame de cursores
//! genéricos (sem marca de nenhuma IA: as marcas só aparecem quando um agente de verdade mexe no arquivo) voa até cada camada e ela aparece. As camadas entram por nível (primeiro as seções e os cartões,
//! por último os textos e os ícones) e ficam sempre no lugar final, então nada se mexe. O desenho é o do renderizador
//! (`figcraft_render::render_node_skipping`), feito numa linha de fundo; o documento não é tocado, nada vai para o desfazer e nada
//! é salvo. Esc (ou o botão Parar) termina e o quadro de verdade volta, igual ao último quadro da animação.
//!
//! A ideia vem do Swarm Rebuild (cursores coloridos reconstruindo uma página), reescrita do zero: aquele projeto não tem licença.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, MutexGuard};
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
/// Antes da primeira camada e depois da última (os cursores entram e saem).
const LEAD_IN: f32 = 0.5;
const HOLD: f32 = 1.4;
const FADE: f32 = 0.7;

/// Uma batida: o instante em que algumas camadas (as de `order[from..to]`) aparecem, e o cursor que chega a elas.
struct Beat {
    at: f32,
    to: usize,
    cursor: usize,
    /// O retângulo da primeira camada da batida (coordenadas da página), onde o cursor chega e o pulso aparece.
    target: Rect,
}

/// Um trecho do caminho de um cursor (coordenadas da página).
struct Leg {
    depart: f32,
    arrive: f32,
    from: Pos2,
    to: Pos2,
}

struct Cursor {
    legs: Vec<Leg>,
    start: Pos2,
    /// Cada cursor tem a sua fase (a tremidinha da mão e o ponto da órbita no fim).
    phase: f32,
}

struct Rendered {
    count: usize,
    size: [usize; 2],
    rgba: Vec<u8>,
}

struct Show {
    doc_uid: u64,
    page: NodeId,
    /// O retângulo do quadro na página.
    bounds: Rect,
    beats: Vec<Beat>,
    cursors: Vec<Cursor>,
    /// Quanto dura a revelação (o último instante de batida).
    reveal_end: f32,
    started: Instant,
    bg: Color32,
    /// Pedidos de desenho (quantas camadas já entraram, e a escala) e as respostas da linha de fundo.
    requests: Sender<(usize, f64)>,
    results: Receiver<Rendered>,
    asked: usize,
    scale_sent: bool,
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

/// O fundo da página aberta (o que fica atrás do quadro).
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

/// Por onde começar: o quadro selecionado, ou o primeiro quadro (com camadas dentro) da página.
fn pick_frame(doc: &Document, page: NodeId, selection: &[NodeId]) -> Option<NodeId> {
    let has_kids = |id: NodeId| doc.node(id).is_some_and(|n| n.visible && !n.children.is_empty());
    selection.iter().copied().find(|id| has_kids(*id)).or_else(|| doc.node(page)?.children.iter().copied().find(|id| has_kids(*id)))
}

/// As camadas do quadro por nível (o quadro primeiro, depois os filhos dele, os netos…), cada nível na ordem das camadas.
pub fn levels(doc: &Document, frame: NodeId) -> Vec<NodeId> {
    let mut out = vec![frame];
    let mut level = vec![frame];
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

/// Em quantas batidas a revelação se divide: uma por camada até um limite (os grandes entram em grupos).
pub fn beat_count(layers: usize) -> usize {
    layers.clamp(1, 150)
}

/// Os instantes das batidas (em segundos desde o começo): começa devagar e aperta, e dura de 5 a 14 s.
pub fn beat_times(beats: usize) -> Vec<f32> {
    let total = (beats as f32 * 0.085).clamp(5.0, 14.0);
    (0..beats).map(|k| LEAD_IN + total * (k as f32 / beats as f32).powf(0.9)).collect()
}

/// Começa a animação do quadro selecionado (ou do primeiro quadro da página). Só olha o desenho: nada no arquivo muda.
pub fn start(app: &mut FigApp, ctx: &egui::Context) {
    if cfg!(test) {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let (doc, page, uid) = (ds.doc.clone(), ds.page, ds.uid);
    let Some(frame) = pick_frame(&doc, page, &ds.selection) else {
        app.toast("Não achei um quadro com camadas para demonstrar nesta página.");
        return;
    };
    let order = levels(&doc, frame);
    if order.len() < 3 {
        app.toast("Este quadro tem poucas camadas para uma demonstração.");
        return;
    }
    let Some(b) = figcraft_render::export_bounds(&doc, frame) else { return };
    let bounds = rect_of(b);
    // Enquadra o quadro na tela (só a vista, o arquivo não muda)
    app.run("select.set", serde_json::json!({"ids": [frame]}));
    app.run("view.zoomSelection", serde_json::json!({}));

    let n = order.len();
    let beats_n = beat_count(n);
    let times = beat_times(beats_n);
    let mut beats = Vec::with_capacity(beats_n);
    let mut cursors: Vec<Cursor> = (0..SWARM.len())
        .map(|i| {
            // Cada cursor entra de um canto diferente, fora do quadro
            let corner = [(-0.15, -0.1), (1.12, -0.1), (-0.15, 1.1), (1.12, 1.1)][i % 4];
            Cursor { legs: Vec::new(), start: pos2(bounds.min.x + bounds.width() * corner.0, bounds.min.y + bounds.height() * corner.1), phase: i as f32 * std::f32::consts::FRAC_PI_2 }
        })
        .collect();
    for (k, at) in times.iter().enumerate() {
        let (from, to) = (k * n / beats_n, (k + 1) * n / beats_n);
        let first = order[from.min(n - 1)];
        let target = doc.world_render_bounds(first).map(rect_of).filter(|r| r.width() > 0.0 && r.height() > 0.0).unwrap_or(bounds);
        let cursor = k % cursors.len();
        let c = &mut cursors[cursor];
        let (prev_at, prev_pos) = c.legs.last().map_or((0.0, c.start), |l| (l.arrive, l.to));
        let arrive = *at;
        let depart = (arrive - GLIDE).max(prev_at);
        c.legs.push(Leg { depart, arrive, from: prev_pos, to: target.center() });
        beats.push(Beat { at: arrive, to: to.max(from + 1).min(n), cursor, target });
    }
    let reveal_end = beats.last().map_or(0.0, |b| b.at);
    let bg = page_bg(app, ctx);

    // A linha de fundo desenha o quadro com as camadas que já entraram; só o último pedido importa
    let (req_tx, req_rx) = channel::<(usize, f64)>();
    let (res_tx, res_rx) = channel::<Rendered>();
    let (worker_doc, skip_order) = (doc.clone(), order);
    let ctx2 = ctx.clone();
    let _ = std::thread::Builder::new().name("legatia-reveal".into()).spawn(move || {
        while let Ok(mut want) = req_rx.recv() {
            while let Ok(newer) = req_rx.try_recv() {
                want = newer;
            }
            let (count, scale) = want;
            let skip: Vec<NodeId> = skip_order.iter().skip(count).copied().collect();
            if let Some(pix) = figcraft_render::render_node_skipping(&worker_doc, frame, scale, &skip) {
                let rgba = figcraft_render::to_rgba8(&pix);
                if res_tx.send(Rendered { count, size: [pix.width() as usize, pix.height() as usize], rgba }).is_err() {
                    return;
                }
                ctx2.request_repaint();
            }
        }
    });
    *state() = Some(Show { doc_uid: uid, page, bounds, beats, cursors, reveal_end, started: Instant::now(), bg, requests: req_tx, results: res_rx, asked: usize::MAX, scale_sent: false, texture: None });
    ctx.request_repaint();
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Onde o cursor está no instante `t` (coordenadas da página): voa em arco de uma camada à outra, com uma tremidinha de mão, e depois da
/// última o enxame se junta no meio do quadro e gira (como no Swarm Rebuild) enquanto some.
fn cursor_at(c: &Cursor, t: f32, bounds: Rect, reveal_end: f32) -> Pos2 {
    let mut pos = c.start;
    for (i, leg) in c.legs.iter().enumerate() {
        if t < leg.depart {
            break;
        }
        let k = if leg.arrive > leg.depart { ease((t - leg.depart) / (leg.arrive - leg.depart)) } else { 1.0 };
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
    if t > reveal_end {
        // O enxame se junta e gira em volta do quadro
        let k = ease((t - reveal_end) / 0.7);
        let r = bounds.width().min(bounds.height()) * 0.42;
        let ang = c.phase + (t - reveal_end) * 2.2;
        let orbit = bounds.center() + vec2(ang.cos(), ang.sin()) * r;
        pos = pos + (orbit - pos) * k;
    }
    pos
}

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
    // A foto do agente à esquerda (se não conhecemos a marca, só o nome)
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

/// O que a animação põe por cima do canvas: o quadro revelado, os cursores e o aviso. Chamado a cada quadro.
pub fn draw(app: &FigApp, painter: &Painter, ctx: &egui::Context) {
    let mut guard = state();
    let Some(show) = guard.as_mut() else { return };
    // Outro arquivo ou outra página: a animação acaba
    let still_here = app.session.ds().is_ok_and(|d| d.uid == show.doc_uid && d.page == show.page);
    let t = show.started.elapsed().as_secs_f32();
    if !still_here || t > show.reveal_end + HOLD + FADE || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        *guard = None;
        ctx.request_repaint();
        return;
    }
    let (min, max) = (app.to_screen(f64::from(show.bounds.min.x), f64::from(show.bounds.min.y)), app.to_screen(f64::from(show.bounds.max.x), f64::from(show.bounds.max.y)));
    let screen = Rect::from_min_max(min, max);
    let ppp = ctx.pixels_per_point();

    // Quantas camadas já entraram agora; pede o desenho (a linha de fundo junta os pedidos)
    let revealed = show.beats.iter().rev().find(|b| b.at <= t).map_or(0, |b| b.to);
    let want_scale = {
        let s = f64::from(screen.width() * ppp) / f64::from(show.bounds.width().max(1.0));
        // Sem passar de ~2400 px de lado
        s.min(2400.0 / f64::from(show.bounds.width().max(show.bounds.height()).max(1.0))).max(0.05)
    };
    if show.asked != revealed || !show.scale_sent {
        let _ = show.requests.send((revealed, want_scale));
        show.asked = revealed;
        show.scale_sent = true;
    }
    while let Ok(r) = show.results.try_recv() {
        let image = egui::ColorImage::from_rgba_unmultiplied(r.size, &r.rgba);
        match show.texture.as_mut() {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => show.texture = Some(ctx.load_texture("legatia-reveal", image, egui::TextureOptions::LINEAR)),
        }
        let _ = r.count;
    }

    // Cobre o quadro de verdade com o fundo da página e põe por cima o que já foi revelado
    let alpha_out = if t > show.reveal_end + HOLD { (1.0 - (t - show.reveal_end - HOLD) / FADE).clamp(0.0, 1.0) } else { 1.0 };
    painter.rect_filled(screen.expand(48.0 * (screen.width() / show.bounds.width().max(1.0))), 0.0, show.bg);
    if let Some(tex) = &show.texture {
        painter.image(tex.id(), screen, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }

    let page_scale = screen.width() / show.bounds.width().max(1.0);
    let to_screen = |p: Pos2| pos2(screen.min.x + (p.x - show.bounds.min.x) * page_scale, screen.min.y + (p.y - show.bounds.min.y) * page_scale);
    for beat in &show.beats {
        let color = SWARM[beat.cursor % SWARM.len()].1;
        let r = Rect::from_min_max(to_screen(beat.target.min), to_screen(beat.target.max));
        pulse(painter, r, color, t - beat.at);
    }
    for (i, c) in show.cursors.iter().enumerate() {
        let tip = to_screen(cursor_at(c, t, show.bounds, show.reveal_end));
        let (name, color) = SWARM[i % SWARM.len()];
        arrow(painter, tip, color, alpha_out);
        name_tag(painter, ctx, tip, name, color, alpha_out);
    }

    // O aviso e o botão para parar
    let hud = Rect::from_center_size(pos2(app.canvas.rect.center().x, app.canvas.rect.min.y + 34.0), vec2(300.0, 34.0));
    painter.rect_filled(hud, CornerRadius::same(17), Color32::from_rgba_unmultiplied(16, 18, 24, 220));
    painter.text(hud.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, "Demonstração: nada muda no arquivo", regular(11.5), Color32::from_white_alpha(220));
    painter.text(hud.right_center() - vec2(14.0, 0.0), Align2::RIGHT_CENTER, "Esc", semibold(11.0), Color32::from_white_alpha(150));
    // O botão para parar (além do Esc)
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

    #[test]
    fn the_beats_run_from_five_to_fourteen_seconds_and_speed_up() {
        let t = beat_times(30);
        assert_eq!(t.len(), 30);
        assert!(t.windows(2).all(|w| w[1] >= w[0]), "sempre para a frente");
        let span = t.last().unwrap() - t[0];
        assert!((4.0..=14.0).contains(&span), "{span}");
        // Poucas camadas ainda duram pelo menos uns 5 s; muitas, no máximo 14
        assert!(*beat_times(3).last().unwrap() >= LEAD_IN + 3.0);
        assert!(*beat_times(150).last().unwrap() <= LEAD_IN + 14.0);
    }

    #[test]
    fn the_layers_come_in_a_beat_each_up_to_a_limit() {
        assert_eq!(beat_count(1), 1);
        assert_eq!(beat_count(40), 40);
        assert_eq!(beat_count(9000), 150);
    }

    #[test]
    fn a_cursor_flies_between_its_stops_waits_at_each_and_the_swarm_gathers_at_the_end() {
        let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
        let c = Cursor {
            start: pos2(0.0, 0.0),
            phase: 0.0,
            legs: vec![Leg { depart: 1.0, arrive: 2.0, from: pos2(0.0, 0.0), to: pos2(100.0, 0.0) }, Leg { depart: 3.0, arrive: 4.0, from: pos2(100.0, 0.0), to: pos2(100.0, 50.0) }],
        };
        assert_eq!(cursor_at(&c, 0.5, bounds, 9.0), pos2(0.0, 0.0));
        // No meio do trecho já saiu do ponto de partida e ainda não chegou (e o arco o tira da reta)
        let mid = cursor_at(&c, 1.5, bounds, 9.0);
        assert!(mid.x > 20.0 && mid.x < 80.0 && mid.y.abs() > 1.0, "{mid:?}");
        // Chegou: fica parado na camada
        let stop = cursor_at(&c, 2.5, bounds, 9.0);
        assert!((stop - pos2(100.0, 0.0)).length() < 0.01, "{stop:?}");
        assert!((cursor_at(&c, 4.0, bounds, 9.0) - pos2(100.0, 50.0)).length() < 0.01);
        // Depois da última batida vai para a órbita em volta do quadro
        let orbit = cursor_at(&c, 12.0, bounds, 9.0);
        assert!((orbit - bounds.center()).length() > 100.0, "{orbit:?}");
    }
}
