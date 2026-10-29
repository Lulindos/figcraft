//! LegatIA: the side panels, the tab bar and the tools' dock as frosted glass over the design (dark
//! theme). The canvas is rendered under the panels and the tab bar too ([`widen`]) and painted behind
//! them, in a slot [`reserve`] keeps before the tab bar ([`page`], [`canvas`]). Under the cards, the
//! bar and the dock it is painted blurred ([`frost`]), with a translucent fill over it: lighter at the
//! top and darker below on the cards, like the LegatIA glass cards. Between the cards the design
//! shows as is.
//!
//! The blur comes from each finished render: averaged down [`STEP`] times and box-blurred, a small
//! picture that the GPU stretches back over the glass.

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

/// This frame's slot behind the tab bar and the panels, and what goes in it.
struct Slot {
    layer: LayerId,
    idx: ShapeIdx,
    /// The whole window, the tab bar included.
    window: Rect,
    /// The window under the tab bar: panels and canvas ([`area`]).
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

impl Frost {
    /// Where `part` of a render shown at `image` lies in the blurred picture.
    fn uv(&self, image: Rect, part: Rect) -> Rect {
        let at = |p: Pos2| pos2((p.x - image.min.x) / image.width() * self.covered[0], (p.y - image.min.y) / image.height() * self.covered[1]);
        Rect::from_min_max(at(part.min), at(part.max))
    }
}

static SLOT: Mutex<Option<Slot>> = Mutex::new(None);
static FROST: Mutex<Option<Frost>> = Mutex::new(None);
/// Whether the last frame painted the glass (the panels' and the tab bar's frames then leave their
/// fill to it).
static GLAZED: AtomicBool = AtomicBool::new(false);

/// Keeps the place behind the tab bar and the panels, the canvas colour until the canvas fills it
/// (called before the tab bar is laid out, see `tabs::tab_bar`).
pub fn reserve(ui: &Ui) {
    let window = ui.max_rect();
    let idx = ui.painter().add(Shape::rect_filled(window, CornerRadius::ZERO, crate::theme::Tokens::get(ui.ctx()).canvas));
    let mut guard = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let (full, central, narrow) = guard.as_ref().map_or((window, window, false), |s| (s.full, s.central, s.narrow));
    *guard = Some(Slot { layer: ui.layer_id(), idx, window, full, central, narrow, bg: None, image: None });
}

/// Where the panels and the canvas go: the window under the tab bar (see [`super::backdrop`]).
pub(super) fn area(full: Rect) {
    if let Some(slot) = SLOT.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        slot.full = full;
    }
}

/// Whether the panels and the tab bar sit on the glass painted behind them (see [`super::panel_frame`]).
pub(super) fn glazed() -> bool {
    GLAZED.load(Ordering::Relaxed)
}

/// The tab bar's fill: clear over the glass, `fill` otherwise.
pub fn bar(fill: Color32) -> Color32 {
    if glazed() { Color32::TRANSPARENT } else { fill }
}

/// The render's origin and size in pixels (`visible`) grown to cover the panels on either side of
/// `central` and the tab bar above it, at `scale` pixels per page unit and `zoom` points per page
/// unit, within `max_side` pixels (the far edges stay uncovered when the window is too big for it).
/// Only under the glass; a narrow window keeps its own layout.
pub fn widen(central: Rect, narrow: bool, origin: (f64, f64), visible: (u32, u32), scale: f64, zoom: f64, max_side: u32) -> ((f64, f64), (u32, u32)) {
    let guard = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(slot) = guard.as_ref() else { return (origin, visible) };
    if narrow || zoom <= 0.0 || !glazed() {
        return (origin, visible);
    }
    let per_point = scale / zoom;
    let grow = |before: f32, after: f32, at: f64, size: u32| {
        let (before, after) = (f64::from(before.max(0.0)), f64::from(after.max(0.0)));
        if before + after < 1.0 {
            return (at, size);
        }
        let fit = (f64::from(max_side.saturating_sub(size)) / ((before + after) * per_point)).min(1.0);
        (at - before * fit / zoom, size + ((before + after) * fit * per_point).floor() as u32)
    };
    let (x, w) = grow(central.min.x - slot.full.min.x, slot.full.max.x - central.max.x, origin.0, visible.0);
    let (y, h) = grow(central.min.y - slot.window.min.y, 0.0, origin.1, visible.1);
    ((x, y), (w, h))
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

/// Blurs a finished render for the glass.
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
    let t = crate::theme::Tokens::get(ctx);
    let cards = if slot.narrow { Vec::new() } else { cards(slot.full, slot.central) };
    let header = Rect::from_min_max(slot.window.min, pos2(slot.window.max.x, slot.full.min.y));
    let mut shapes = vec![Shape::rect_filled(slot.window, CornerRadius::ZERO, t.canvas)];
    if let Some(bg) = slot.bg {
        shapes.push(Shape::rect_filled(slot.window, CornerRadius::ZERO, bg));
    }
    if let Some((texture, rect)) = slot.image.filter(|_| t.dark) {
        shapes.push(Shape::image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE));
        if let Some(frost) = FROST.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let blurred = |part: Rect, radius: u8| -> Shape {
                RectShape::filled(part, CornerRadius::same(radius), Color32::WHITE).with_texture(frost.texture.id(), frost.uv(rect, part)).into()
            };
            shapes.extend(cards.iter().map(|c| blurred(*c, PANEL_RADIUS)));
            shapes.push(blurred(header, 0));
        }
    }
    if t.dark {
        shapes.extend(cards.iter().map(|c| Shape::mesh(glass_fill(*c, f32::from(PANEL_RADIUS)))));
        shapes.push(Shape::rect_filled(header, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(8, 8, 9, 0xC0)));
    }
    GLAZED.store(t.dark && !cards.is_empty(), Ordering::Relaxed);
    Painter::new(ctx.clone(), slot.layer, slot.window).set(slot.idx, Shape::Vec(shapes));
}

/// The tools' dock at `rect` (rounded by `radius`) as a small glass card over the design: the blurred
/// design under it and the glass fill. False in the light theme or before a render (the toolbar then
/// paints its own panel).
pub fn dock(painter: &Painter, rect: Rect, radius: f32) -> bool {
    let slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let frost = FROST.lock().unwrap_or_else(|e| e.into_inner());
    let (Some((_, image)), Some(frost)) = (slot.as_ref().and_then(|s| s.image), frost.as_ref()) else { return false };
    if !crate::theme::Tokens::get(painter.ctx()).dark {
        return false;
    }
    painter.add(RectShape::filled(rect, CornerRadius::same(radius as u8), Color32::WHITE).with_texture(frost.texture.id(), frost.uv(image, rect)));
    painter.add(Shape::mesh(glass_fill(rect, radius)));
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

/// The glass over the blurred design: lighter at the top (white at 8% in the LegatIA cards), darker
/// from three quarters down; dark enough for white text over a white design.
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
