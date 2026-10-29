//! LegatIA: what the page around the editor (LegatIAdm, the editor in an `<iframe>`) puts in it.
//!
//! The page describes its buttons and files with `figcraft.host({...})` (the web shell's
//! `window.figcraft`, [`set`]); the editor draws them in its own places and hands the clicks back
//! as events the shell passes to `figcraft.onhost` ([`take_events`]):
//!
//! - `primary`: the button in Share's place (Salvar na LegatIA);
//! - `buttons`: buttons left of the theme toggle (Sair da tela cheia);
//! - `files`: the page's files on the home screen, instead of the recent files;
//! - `newFile`: new files are the page's (the tab bar's + asks it);
//! - `dockSlot`: points kept empty at the end of the tools' dock, where the page draws its own
//!   button over the editor (the agent orb); `figcraft.dockSlot()` says where ([`dock_slot_rect`]);
//! - `glass`: `"clear"` (panels translucent over the design, no blur) or `"off"` (solid panels, as
//!   FigCraft's); anything else, the frosted glass ([`super::glass`]).
//!
//! Events: `{type: "button", id}`, `{type: "open", id}` (a file on the home screen) and
//! `{type: "new"}`. Without a host (the desktop app, the plain web build) nothing changes.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Host {
    pub primary: Option<Button>,
    pub buttons: Vec<Button>,
    pub files: Option<Files>,
    pub new_file: bool,
    pub dock_slot: f32,
    pub glass: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Button {
    pub id: String,
    pub label: String,
    pub tip: String,
    /// `fullscreen` or `exitFullscreen` for the tab bar's buttons.
    pub icon: String,
    /// Shown as working (the label says what): not clickable.
    pub busy: bool,
    pub disabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Files {
    pub title: String,
    /// The new file card (empty: none).
    pub new_label: String,
    pub new_detail: String,
    /// Said when there are no files.
    pub empty: String,
    pub loading: bool,
    pub items: Vec<File>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct File {
    pub id: String,
    pub name: String,
    /// The line under the name ("Versão 12 · ontem").
    pub detail: String,
    /// A tag on the picture ("Editando: Ana"), empty for none.
    pub badge: String,
    /// The file's picture, a PNG in base64 (empty: a file glyph).
    pub thumbnail: String,
}

static HOST: Mutex<Option<Arc<Host>>> = Mutex::new(None);
static EVENTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());
static SHOW_EDITOR: Mutex<bool> = Mutex::new(false);
/// Where the dock's slot was drawn, and in which pass.
static DOCK_SLOT: Mutex<Option<(Rect, u64)>> = Mutex::new(None);

/// The page opened or made a file (`figcraft.showEditor()`): the home screen gives way to it.
pub fn show_editor() {
    *SHOW_EDITOR.lock().unwrap_or_else(|e| e.into_inner()) = true;
}

/// Whether the page asked for the editor since the last call (the home screen asks every frame).
pub fn take_show_editor() -> bool {
    std::mem::take(&mut *SHOW_EDITOR.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The page's description (`null`: no host).
pub fn set(host: Value) -> Result<(), String> {
    let host = if host.is_null() { None } else { Some(Arc::new(serde_json::from_value::<Host>(host).map_err(|e| e.to_string())?)) };
    *HOST.lock().unwrap_or_else(|e| e.into_inner()) = host;
    Ok(())
}

pub fn get() -> Option<Arc<Host>> {
    HOST.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The clicks since the last call, for the page.
pub fn take_events() -> Vec<Value> {
    std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()))
}

fn emit(event: Value) {
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).push(event);
}

/// The `thumbnail.png` inside a `.figcraft` file, for the page to keep beside it.
pub fn thumbnail_png(file: &[u8]) -> Option<Vec<u8>> {
    figcraft_format::thumbnail::thumbnail_png(std::io::Cursor::new(file))
}

/// The width the page keeps at the end of the tools' dock (0: none), with the gap before it.
pub fn dock_slot_width() -> f32 {
    get().map_or(0.0, |h| if h.dock_slot > 0.0 { h.dock_slot + 8.0 } else { 0.0 })
}

/// Keeps the page's slot at the end of the dock `pill` (called while the dock is drawn): a divider
/// and an empty square the page draws over.
pub fn dock_slot(ui: &Ui, t: &Tokens, pill: Rect) {
    let w = get().map_or(0.0, |h| h.dock_slot);
    if w <= 0.0 {
        return;
    }
    let r = Rect::from_center_size(pos2(pill.max.x - 8.0 - w / 2.0, pill.center().y), vec2(w, w));
    let x = r.min.x - 6.0;
    ui.painter().line_segment([pos2(x, pill.center().y - 14.0), pos2(x, pill.center().y + 14.0)], Stroke::new(1.0, t.border));
    *DOCK_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = Some((r, ui.ctx().cumulative_pass_nr()));
}

/// Where the page's slot is now, in points (null when the dock wasn't drawn in the last passes:
/// another toolbar, the home screen, the UI hidden).
pub fn dock_slot_rect(ctx: &egui::Context) -> Option<Rect> {
    let (r, pass) = (*DOCK_SLOT.lock().unwrap_or_else(|e| e.into_inner()))?;
    (pass + 2 >= ctx.cumulative_pass_nr()).then_some(r)
}

/// The tab bar's +: true when the page makes new files (it was told), so the editor opens none.
pub fn new_file() -> bool {
    if get().is_some_and(|h| h.new_file) {
        emit(json!({"type": "new"}));
        return true;
    }
    false
}

/// The LegatIA call to action (light pill with the orange glow, as the platform's CtaButton).
const CTA_FILL: Color32 = Color32::from_rgb(0xD1, 0xD1, 0xD1);
const CTA_GLOW: Color32 = Color32::from_rgb(0xFF, 0x70, 0x37);

/// The page's primary button in Share's place, at the right of the right panel's top bar `top`;
/// its rect, or `None` without one (Share is drawn).
pub fn primary_button(ui: &mut Ui, t: &Tokens, top: Rect) -> Option<Rect> {
    let b = get()?.primary.clone()?;
    let label = ui.painter().layout_no_wrap(b.label.clone(), semibold(11.5), Color32::BLACK);
    let w = (label.size().x + 28.0).max(64.0);
    let r = Rect::from_min_size(pos2(top.max.x - 14.0 - w, top.center().y - 15.0), vec2(w, 30.0));
    let idle = b.busy || b.disabled;
    let resp = ui.interact(r, ui.scope_id().with(("legatia_primary", &b.id)), if idle { Sense::hover() } else { Sense::click() });
    let resp = if b.tip.is_empty() { resp } else { resp.on_hover_text(b.tip.as_str()) };
    crate::a11y::button(&resp, &b.label);
    let pill = CornerRadius::same(15);
    if !b.disabled {
        let glow = egui::Shadow { offset: [5, 0], blur: if resp.hovered() { 18 } else { 12 }, spread: 0, color: CTA_GLOW.gamma_multiply(0.7) };
        ui.painter().add(glow.as_shape(r, pill));
    }
    ui.painter().rect(r, pill, if b.disabled { t.pressed } else { CTA_FILL }, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
    let ink = if b.disabled { t.text_secondary } else if b.busy { Color32::from_gray(70) } else { Color32::BLACK };
    ui.painter().galley(r.center() - label.size() / 2.0, label, ink);
    if resp.clicked() {
        emit(json!({"type": "button", "id": b.id}));
    }
    Some(r)
}

/// The page's buttons left of the theme toggle (`theme_r`); the rect from the leftmost one to
/// the toggle (the tab bar's free space ends there).
pub fn tab_bar_buttons(ui: &mut Ui, t: &Tokens, theme_r: Rect) -> Rect {
    let Some(host) = get() else { return theme_r };
    let mut left = theme_r;
    for b in &host.buttons {
        let r = left.translate(vec2(-(left.width() + 4.0), 0.0));
        let resp = ui.interact(r, ui.scope_id().with(("legatia_tab_button", &b.id)), Sense::click());
        let resp = if b.tip.is_empty() { resp } else { resp.on_hover_text(b.tip.as_str()) };
        crate::a11y::button(&resp, if b.label.is_empty() { &b.tip } else { &b.label });
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        match b.icon.as_str() {
            "fullscreen" => paint_fullscreen(ui, r, false, t.icon),
            "exitFullscreen" => paint_fullscreen(ui, r, true, t.icon),
            _ => {
                ui.painter().text(r.center(), Align2::CENTER_CENTER, &b.label, regular(11.0), t.icon);
            }
        }
        if resp.clicked() {
            emit(json!({"type": "button", "id": b.id}));
        }
        left = r;
    }
    left.union(theme_r)
}

/// Four corners: pointing out (enter full screen) or in (`exit`).
fn paint_fullscreen(ui: &Ui, r: Rect, exit: bool, color: Color32) {
    let box_ = r.shrink(8.0);
    let arm = 4.0;
    let stroke = Stroke::new(1.4, color);
    for (corner, dx, dy) in
        [(box_.left_top(), 1.0, 1.0), (box_.right_top(), -1.0, 1.0), (box_.left_bottom(), 1.0, -1.0), (box_.right_bottom(), -1.0, -1.0)]
    {
        let elbow = if exit { corner + vec2(arm * dx, arm * dy) } else { corner };
        let (a, b) = if exit {
            (elbow - vec2(arm * dx, 0.0), elbow - vec2(0.0, arm * dy))
        } else {
            (elbow + vec2(arm * dx, 0.0), elbow + vec2(0.0, arm * dy))
        };
        ui.painter().add(Shape::line(vec![a, elbow, b], stroke));
    }
}

/// The page's files, when it gave them (the home screen shows them instead of the recent files).
pub fn files() -> Option<Files> {
    get()?.files.clone()
}

/// The new file card was chosen.
pub fn new_file_chosen() {
    emit(json!({"type": "new"}));
}

/// Pictures by file id, with the hash of the base64 they came from.
#[derive(Clone, Default)]
struct Pictures(HashMap<String, (u64, Option<egui::TextureHandle>)>);

fn picture(ui: &Ui, file: &File) -> Option<egui::TextureHandle> {
    if file.thumbnail.is_empty() {
        return None;
    }
    let id = Id::new("legatia_host_pictures");
    let mut pictures: Pictures = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    file.thumbnail.hash(&mut hasher);
    let hash = hasher.finish();
    if let Some((_, texture)) = pictures.0.get(&file.id).filter(|(h, _)| *h == hash) {
        return texture.clone();
    }
    let texture = base64::engine::general_purpose::STANDARD
        .decode(&file.thumbnail)
        .ok()
        .and_then(|png| figcraft_format::thumbnail::decode_thumbnail(&png, 720))
        .map(|p| {
            let image = egui::ColorImage::from_rgba_unmultiplied([p.width as usize, p.height as usize], &p.rgba);
            ui.ctx().load_texture(format!("legatia-file-{}", file.id), image, egui::TextureOptions::LINEAR)
        });
    pictures.0.insert(file.id.clone(), (hash, texture.clone()));
    ui.data_mut(|d| d.insert_temp(id, pictures));
    texture
}

/// The files as cards, as many across as fit (the recent files' grid); a click opens one.
pub fn file_grid(ui: &mut Ui, t: &Tokens, files: &Files) {
    ui.add_space(28.0);
    ui.label(egui::RichText::new(&files.title).font(semibold(12.0)).color(t.text_secondary));
    ui.add_space(16.0);
    if files.items.is_empty() {
        let said = if files.loading { "…" } else { files.empty.as_str() };
        ui.label(egui::RichText::new(said).font(regular(12.0)).color(t.text_secondary));
        return;
    }
    let gap = crate::home_recents::GAP;
    let (columns, card_w) = crate::home_recents::grid_columns(ui.available_width());
    let (picture_h, card_h) = crate::home_recents::card_heights(card_w);
    for row in files.items.chunks(columns) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), card_h), Sense::hover());
        for (i, file) in row.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(r.min.x + i as f32 * (card_w + gap), r.min.y), vec2(card_w, card_h));
            let resp = ui.interact(rect, Id::new(("legatia_host_file", &file.id)), Sense::click());
            crate::a11y::button(&resp, &file.name);
            if ui.is_rect_visible(rect) {
                card(ui, t, rect, picture_h, &resp, file);
            }
            if resp.clicked() {
                emit(json!({"type": "open", "id": file.id}));
            }
        }
        ui.add_space(gap);
    }
}

fn card(ui: &Ui, t: &Tokens, rect: Rect, picture_h: f32, resp: &egui::Response, file: &File) {
    let p = ui.painter();
    let radius = CornerRadius::same(12);
    p.rect_filled(rect, radius, if resp.hovered() { t.hover } else { t.panel });
    let area = Rect::from_min_size(rect.min, vec2(rect.width(), picture_h)).shrink(1.0);
    let top = CornerRadius { nw: 11, ne: 11, sw: 0, se: 0 };
    p.rect_filled(area, top, t.canvas);
    match picture(ui, file) {
        Some(texture) => {
            let size = texture.size_vec2();
            let inner = area.shrink(area.height() * 0.06);
            let fit = (inner.width() / size.x.max(1.0)).min(inner.height() / size.y.max(1.0));
            let image = Rect::from_center_size(inner.center(), size * fit);
            p.image(texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => icons::paint(p, Rect::from_center_size(area.center(), vec2(28.0, 28.0)), Icon::Page, t.icon_secondary),
    }
    if !file.badge.is_empty() {
        let galley = p.layout_no_wrap(file.badge.clone(), semibold(10.5), t.accent_on);
        let badge = Rect::from_min_size(area.min + vec2(10.0, 10.0), galley.size() + vec2(16.0, 8.0));
        p.rect_filled(badge, CornerRadius::same(10), t.accent);
        p.galley(badge.min + vec2(8.0, 4.0), galley, t.accent_on);
    }
    p.line_segment([pos2(rect.min.x, area.max.y + 0.5), pos2(rect.max.x, area.max.y + 0.5)], Stroke::new(1.0, t.border));
    let footer = Rect::from_min_max(pos2(rect.min.x, area.max.y + 1.0), rect.max);
    let (x, w) = (rect.min.x + 16.0, rect.width() - 32.0);
    let name = crate::widgets::elide_font(ui, &file.name, w, semibold(12.5));
    p.text(pos2(x, footer.center().y - 8.0), Align2::LEFT_CENTER, name, semibold(12.5), t.text);
    let detail = crate::widgets::elide_font(ui, &file.detail, w, regular(11.5));
    p.text(pos2(x, footer.center().y + 9.0), Align2::LEFT_CENTER, detail, regular(11.5), t.text_secondary);
    let border = if resp.hovered() { t.border_strong } else { t.border };
    p.rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    if resp.has_focus() {
        p.rect_stroke(rect, radius, Stroke::new(2.0, t.focus), StrokeKind::Outside);
    }
}
