//! LegatIA: the side panels as frosted glass over the design. The canvas is rendered under the panels
//! too ([`widen`]) and painted behind them, in a slot [`reserve`] keeps before they are laid out
//! ([`page`], [`canvas`]); where the cards are it is painted blurred ([`frost`]), and the cards'
//! translucent fill ([`super::panel_frame`]) goes over it. Between the cards the design shows as is.
//!
//! The blur comes from each finished render: averaged down [`STEP`] times and box-blurred, a small
//! picture that the GPU stretches back over the cards.

use std::sync::Mutex;

use egui::epaint::RectShape;
use egui::layers::ShapeIdx;
use egui::{Color32, ColorImage, Context, CornerRadius, LayerId, Painter, Rect, Shape, TextureHandle, TextureId, TextureOptions, Ui, pos2};

use super::{PANEL_GAP, PANEL_RADIUS, RAIL_GAP};

/// Device pixels per pixel of the blurred picture.
const STEP: usize = 8;
/// The box blur's radius (in blurred pixels) and passes: about a 20-point blur at 2x.
const RADIUS: usize = 4;
const PASSES: usize = 3;

/// This frame's slot behind the panels and what goes in it.
struct Slot {
    layer: LayerId,
    idx: ShapeIdx,
    /// The window under the tab bar: panels and canvas.
    full: Rect,
    /// The canvas between the panels ([`crate::canvas::CanvasState::rect`]).
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

/// Keeps the place behind the panels (called before they are laid out, see [`super::backdrop`]).
pub(super) fn reserve(ui: &Ui, full: Rect) {
    let idx = ui.painter().add(Shape::Noop);
    *SLOT.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Slot { layer: ui.layer_id(), idx, full, central: full, narrow: false, bg: None, image: None });
}

/// The render's origin and width in pixels (`visible_w`) grown to cover the panels on either side of
/// `central`, at `scale` pixels per page unit and `zoom` points per page unit. A narrow window keeps
/// its own layout (no glass).
pub fn widen(central: Rect, narrow: bool, ox: f64, visible_w: u32, scale: f64, zoom: f64) -> (f64, u32) {
    let guard = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(slot) = guard.as_ref() else { return (ox, visible_w) };
    let left = (central.min.x - slot.full.min.x).max(0.0);
    let right = (slot.full.max.x - central.max.x).max(0.0);
    if narrow || left + right < 1.0 || zoom <= 0.0 {
        return (ox, visible_w);
    }
    let per_point = scale / zoom;
    (ox - f64::from(left) / zoom, visible_w.saturating_add((f64::from(left + right) * per_point).ceil() as u32))
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

/// Blurs a finished render for the cards.
pub fn frost(ctx: &Context, image: &ColorImage) {
    let [w, h] = image.size;
    let panels = SLOT.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|s| !s.narrow && s.central.width() < s.full.width() - 1.0);
    if w == 0 || h == 0 || !panels {
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
    let mut shapes = Vec::new();
    if let Some(bg) = slot.bg {
        shapes.push(Shape::rect_filled(slot.full, CornerRadius::ZERO, bg));
    }
    if let Some((texture, rect)) = slot.image {
        shapes.push(Shape::image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE));
        let frost = FROST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(frost) = frost.as_ref().filter(|_| !slot.narrow) {
            let uv = |p: egui::Pos2| {
                pos2((p.x - rect.min.x) / rect.width() * frost.covered[0], (p.y - rect.min.y) / rect.height() * frost.covered[1])
            };
            for card in cards(slot.full, slot.central) {
                let shape = RectShape::filled(card, CornerRadius::same(PANEL_RADIUS), Color32::WHITE)
                    .with_texture(frost.texture.id(), Rect::from_min_max(uv(card.min), uv(card.max)));
                shapes.push(shape.into());
            }
        }
    }
    Painter::new(ctx.clone(), slot.layer, slot.full).set(slot.idx, Shape::Vec(shapes));
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
