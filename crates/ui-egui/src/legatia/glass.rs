//! LegatIA: the side panels as frosted glass over the design, like the LegatIA glass cards. The
//! canvas is rendered under the panels too ([`widen`]) and painted behind them, in a slot [`reserve`]
//! keeps before they are laid out ([`page`], [`canvas`]). Where the cards are it is painted blurred
//! ([`frost`]), under the cards' glass: a fill lighter at the top and darker below, and the LegatIA
//! glass ring (a hairline lit at the top and the bottom, [`ring`]). Between the cards the design
//! shows as is.
//!
//! The blur comes from each finished render: averaged down [`STEP`] times and box-blurred, a small
//! picture that the GPU stretches back over the cards.

use std::f32::consts::{FRAC_PI_2, PI};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::epaint::{Mesh, RectShape};
use egui::layers::ShapeIdx;
use egui::{Color32, ColorImage, Context, CornerRadius, LayerId, Painter, Pos2, Rect, Shape, TextureHandle, TextureId, TextureOptions, Ui, pos2};

use super::{PANEL_GAP, PANEL_RADIUS, RAIL_GAP};

/// Device pixels per pixel of the blurred picture.
const STEP: usize = 12;
/// The box blur's radius (in blurred pixels) and passes: about the LegatIA cards' blur.
const RADIUS: usize = 5;
const PASSES: usize = 3;
/// The glass ring's width (the LegatIA `--glass-ring-width`).
const RING: f32 = 1.4;

/// This frame's slot behind the panels and what goes in it.
struct Slot {
    layer: LayerId,
    idx: ShapeIdx,
    /// The window under the tab bar: panels and canvas.
    full: Rect,
    /// The canvas between the panels ([`crate::canvas::CanvasState::rect`]), as last painted.
    central: Rect,
    narrow: bool,
    bg: Option<Color32>,
    image: Option<(TextureId, Rect)>,
}

/// The blurred picture of the last render, and how much of it the render covers (it is padded to
/// whole [`STEP`]s).
struct Frost {
    texture: TextureHandle,
    covered: [f32; 2],
}

static SLOT: Mutex<Option<Slot>> = Mutex::new(None);
static FROST: Mutex<Option<Frost>> = Mutex::new(None);
/// Whether the last frame painted the cards' glass (the panels' frames then leave their fill to it).
static GLAZED: AtomicBool = AtomicBool::new(false);

/// Keeps the place behind the panels (called before they are laid out, see [`super::backdrop`]).
pub(super) fn reserve(ui: &Ui, full: Rect) {
    let idx = ui.painter().add(Shape::Noop);
    let mut guard = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let (central, narrow) = guard.as_ref().map_or((full, false), |s| (s.central, s.narrow));
    *guard = Some(Slot { layer: ui.layer_id(), idx, full, central, narrow, bg: None, image: None });
}

/// Whether the panels sit on the glass painted behind them (see [`super::panel_frame`]).
pub(super) fn glazed() -> bool {
    GLAZED.load(Ordering::Relaxed)
}

/// The render's origin and width in pixels (`visible_w`) grown to cover the panels on either side of
/// `central`, at `scale` pixels per page unit and `zoom` points per page unit, within `max_side`
/// pixels (the panels' far edges stay uncovered when the window is too wide for it). A narrow
/// window keeps its own layout (no glass).
pub fn widen(central: Rect, narrow: bool, ox: f64, visible_w: u32, scale: f64, zoom: f64, max_side: u32) -> (f64, u32) {
    let guard = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(slot) = guard.as_ref() else { return (ox, visible_w) };
    let left = f64::from((central.min.x - slot.full.min.x).max(0.0));
    let right = f64::from((slot.full.max.x - central.max.x).max(0.0));
    if narrow || left + right < 1.0 || zoom <= 0.0 {
        return (ox, visible_w);
    }
    let per_point = scale / zoom;
    let room = f64::from(max_side.saturating_sub(visible_w));
    let fit = (room / ((left + right) * per_point)).min(1.0);
    let (left, right) = (left * fit, right * fit);
    (ox - left / zoom, visible_w + ((left + right) * per_point).floor() as u32)
}

/// The page colour, under the panels too (the canvas paints it at `central`).
pub fn page(ctx: &Context, central: Rect, narrow: bool, bg: Color32) {
    update(ctx, |slot| {
        slot.central = central;
        slot.narrow = narrow;
        slot.bg = Some(bg);
        slot.image = None;
    });
}

/// The canvas picture (`texture` at `rect`), under the panels too.
pub fn canvas(ctx: &Context, texture: TextureId, rect: Rect) {
    update(ctx, |slot| slot.image = Some((texture, rect)));
}

/// Blurs a finished render for the cards and the dock.
pub fn frost(ctx: &Context, image: &ColorImage) {
    let [w, h] = image.size;
    if w == 0 || h == 0 || !crate::theme::Tokens::get(ctx).dark {
        return;
    }
    let (fw, fh) = (w.div_ceil(STEP), h.div_ceil(STEP));
    let mut px = vec![[0.0f32; 4]; fw * fh];
    for by in 0..fh {
        for bx in 0..fw {
            // Every other pixel of the block: enough for a picture that is blurred anyway.
            let (mut sum, mut n) = ([0u32; 4], 0u32);
            for y in (by * STEP..((by + 1) * STEP).min(h)).step_by(2) {
                for x in (bx * STEP..((bx + 1) * STEP).min(w)).step_by(2) {
                    let p = image.pixels[y * w + x];
                    sum[0] += u32::from(p.r());
                    sum[1] += u32::from(p.g());
                    sum[2] += u32::from(p.b());
                    sum[3] += u32::from(p.a());
                    n += 1;
                }
            }
            let n = n.max(1) as f32;
            px[by * fw + bx] = sum.map(|s| s as f32 / n);
        }
    }
    for _ in 0..PASSES {
        blur(&mut px, fw, fh, 1, fw);
        blur(&mut px, fh, fw, fw, 1);
    }
    let pixels = px.iter().map(|c| Color32::from_rgba_premultiplied(c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8)).collect();
    let picture = ColorImage::new([fw, fh], pixels);
    let covered = [w as f32 / (fw * STEP) as f32, h as f32 / (fh * STEP) as f32];
    let mut guard = FROST.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_mut() {
        Some(f) => {
            f.texture.set(picture, TextureOptions::LINEAR);
            f.covered = covered;
        }
        None => *guard = Some(Frost { texture: ctx.load_texture("legatia-frost", picture, TextureOptions::LINEAR), covered }),
    }
}

/// One box blur pass along lines of `len` pixels `step` apart; `lines` of them, `stride` apart.
fn blur(px: &mut [[f32; 4]], len: usize, lines: usize, step: usize, stride: usize) {
    let r = RADIUS as isize;
    let width = (2 * RADIUS + 1) as f32;
    let mut line = vec![[0.0f32; 4]; len];
    for l in 0..lines {
        let at = |i: isize| l * stride + i.clamp(0, len as isize - 1) as usize * step;
        let mut sum = [0.0f32; 4];
        for i in -r..=r {
            let p = px[at(i)];
            for c in 0..4 {
                sum[c] += p[c];
            }
        }
        for (i, out) in line.iter_mut().enumerate() {
            *out = sum.map(|s| s / width);
            let (add, sub) = (px[at(i as isize + r + 1)], px[at(i as isize - r)]);
            for c in 0..4 {
                sum[c] += add[c] - sub[c];
            }
        }
        for (i, p) in line.iter().enumerate() {
            px[l * stride + i * step] = *p;
        }
    }
}

fn update(ctx: &Context, f: impl FnOnce(&mut Slot)) {
    let mut guard = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(slot) = guard.as_mut() else { return };
    f(slot);
    let dark = crate::theme::Tokens::get(ctx).dark;
    let cards = if slot.narrow { Vec::new() } else { cards(slot.full, slot.central) };
    let mut shapes = Vec::new();
    if let Some(bg) = slot.bg {
        shapes.push(Shape::rect_filled(slot.full, CornerRadius::ZERO, bg));
    }
    if let Some((texture, rect)) = slot.image {
        shapes.push(Shape::image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE));
        let frost = FROST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(frost) = frost.as_ref() {
            let uv = |p: Pos2| pos2((p.x - rect.min.x) / rect.width() * frost.covered[0], (p.y - rect.min.y) / rect.height() * frost.covered[1]);
            for card in &cards {
                let shape = RectShape::filled(*card, CornerRadius::same(PANEL_RADIUS), Color32::WHITE)
                    .with_texture(frost.texture.id(), Rect::from_min_max(uv(card.min), uv(card.max)));
                shapes.push(shape.into());
            }
        }
    }
    if dark {
        for card in &cards {
            shapes.push(Shape::mesh(glass_fill(*card, f32::from(PANEL_RADIUS))));
            shapes.push(Shape::mesh(ring(*card, f32::from(PANEL_RADIUS))));
        }
    }
    GLAZED.store(dark && !cards.is_empty(), Ordering::Relaxed);
    Painter::new(ctx.clone(), slot.layer, slot.full).set(slot.idx, Shape::Vec(shapes));
}

/// The tools' dock at `rect` (rounded by `radius`) as a small glass card over the design: the blurred
/// design under it, the glass fill and the ring. False in the light theme or before a render (the
/// toolbar then paints its own panel).
pub fn dock(painter: &Painter, rect: Rect, radius: f32) -> bool {
    let slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let frost = FROST.lock().unwrap_or_else(|e| e.into_inner());
    let (Some((_, image)), Some(frost)) = (slot.as_ref().and_then(|s| s.image), frost.as_ref()) else { return false };
    if !crate::theme::Tokens::get(painter.ctx()).dark {
        return false;
    }
    let uv = |p: Pos2| pos2((p.x - image.min.x) / image.width() * frost.covered[0], (p.y - image.min.y) / image.height() * frost.covered[1]);
    let blurred = RectShape::filled(rect, CornerRadius::same(radius as u8), Color32::WHITE)
        .with_texture(frost.texture.id(), Rect::from_min_max(uv(rect.min), uv(rect.max)));
    painter.add(blurred);
    painter.add(Shape::mesh(glass_fill(rect, radius)));
    painter.add(Shape::mesh(ring(rect, radius)));
    true
}

/// The cards around `central` in `full`, as [`super::panel_frame`] places them: the rail and the
/// layers panel on the left, the properties panel on the right.
fn cards(full: Rect, central: Rect) -> Vec<Rect> {
    let gap = f32::from(PANEL_GAP);
    let rail = 56.0 + RAIL_GAP;
    let (top, bottom) = (full.min.y, full.max.y - gap);
    let card = |x0: f32, x1: f32| (x1 - x0 > 1.0).then(|| Rect::from_min_max(pos2(x0, top), pos2(x1, bottom)));
    let left = central.min.x - full.min.x;
    let mut out = Vec::new();
    if left >= rail - 0.5 {
        out.extend(card(full.min.x + gap, full.min.x + rail));
        out.extend(card(full.min.x + rail + gap, central.min.x - gap));
    }
    out.extend(card(central.max.x + gap, full.max.x - gap));
    out
}

/// The card's glass over the blurred design: lighter at the top (white at 8% in the LegatIA cards),
/// darker from three quarters down; dark enough for white text over a white design.
fn glass_fill(rect: Rect, radius: f32) -> Mesh {
    let top = [40.0, 40.0, 44.0, 0.76];
    let bottom = [16.0, 16.0, 18.0, 0.88];
    let colour = |y: f32| {
        let k = ((y - rect.min.y) / (rect.height() * 0.76)).clamp(0.0, 1.0);
        let c: [f32; 4] = std::array::from_fn(|i| top[i] + (bottom[i] - top[i]) * k);
        Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, (c[3] * 255.0) as u8)
    };
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    // Rows: close together along the corners, every 8 points down the sides.
    let mut ys: Vec<f32> = (0..=12).map(|i| rect.min.y + r * i as f32 / 12.0).collect();
    let mut y = rect.min.y + r + 8.0;
    while y < rect.max.y - r {
        ys.push(y);
        y += 8.0;
    }
    ys.extend((0..=12).map(|i| rect.max.y - r + r * i as f32 / 12.0));
    let inset = |y: f32| {
        let d = if y < rect.min.y + r { rect.min.y + r - y } else if y > rect.max.y - r { y - (rect.max.y - r) } else { 0.0 };
        r - (r * r - d.min(r) * d.min(r)).sqrt()
    };
    let mut mesh = Mesh::default();
    for (i, y) in ys.iter().enumerate() {
        let c = colour(*y);
        mesh.colored_vertex(pos2(rect.min.x + inset(*y), *y), c);
        mesh.colored_vertex(pos2(rect.max.x - inset(*y), *y), c);
        if i > 0 {
            let k = (2 * i) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    mesh
}

/// The LegatIA glass ring around `rect` (rounded by `radius`): a [`RING`]-wide hairline, white at
/// 45% at the top and the bottom, fading out in the middle (`--glass-ring-gradient`).
pub(super) fn ring(rect: Rect, radius: f32) -> Mesh {
    const STOPS: [(f32, f32); 6] = [(0.0, 0.45), (0.2, 0.15), (0.4, 0.0), (0.6, 0.0), (0.8, 0.15), (1.0, 0.45)];
    let alpha = |y: f32| {
        let k = ((y - rect.min.y) / rect.height()).clamp(0.0, 1.0);
        let i = STOPS.iter().position(|s| s.0 >= k).unwrap_or(STOPS.len() - 1).max(1);
        let ((k0, a0), (k1, a1)) = (STOPS[i - 1], STOPS[i]);
        a0 + (a1 - a0) * ((k - k0) / (k1 - k0)).clamp(0.0, 1.0)
    };
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let outer = outline(rect, r);
    let inner = outline(rect.shrink(RING), (r - RING).max(0.0));
    let mut mesh = Mesh::default();
    for (o, i) in outer.iter().zip(&inner) {
        let c = Color32::from_white_alpha((alpha(o.y) * 255.0) as u8);
        mesh.colored_vertex(*o, c);
        mesh.colored_vertex(*i, c);
    }
    let n = outer.len() as u32;
    for i in 0..n {
        let j = (i + 1) % n;
        mesh.add_triangle(2 * i, 2 * i + 1, 2 * j);
        mesh.add_triangle(2 * i + 1, 2 * j + 1, 2 * j);
    }
    mesh
}

/// Points around `rect` rounded by `r`, clockwise from the top-left corner, with the straight sides
/// cut every 8% of the height so a vertical gradient along them keeps its stops.
fn outline(rect: Rect, r: f32) -> Vec<Pos2> {
    const ARC: usize = 8;
    const SIDE: usize = 12;
    let arc = |cx: f32, cy: f32, from: f32| (0..=ARC).map(move |i| {
        let a = from + FRAC_PI_2 * i as f32 / ARC as f32;
        pos2(cx + r * a.cos(), cy + r * a.sin())
    });
    let side = |x: f32, y0: f32, y1: f32| (1..SIDE).map(move |i| pos2(x, y0 + (y1 - y0) * i as f32 / SIDE as f32));
    let (l, t, rr, b) = (rect.min.x + r, rect.min.y + r, rect.max.x - r, rect.max.y - r);
    let mut out: Vec<Pos2> = arc(l, t, PI).collect();
    out.extend(arc(rr, t, 1.5 * PI));
    out.extend(side(rect.max.x, t, b));
    out.extend(arc(rr, b, 0.0));
    out.extend(arc(l, b, FRAC_PI_2));
    out.extend(side(rect.min.x, b, t));
    out
}
