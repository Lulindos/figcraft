//! LegatIA: the LegatIA colours and wordmark over FigCraft's tokens (fork Lulindos/figcraft, branch
//! `legatia`). Kept in a file of its own so upstream updates don't touch it: `theme.rs` only calls
//! [`tokens`] when it installs a theme and [`paint_wordmark`] when it draws the name. See LEGATIA.md.
//!
//! The page around the web editor (LegatIAdm) also puts its own buttons and files in it ([`host`])
//! and the LegatIA typeface in the interface ([`fonts`]); the panels float as frosted glass cards over
//! the design ([`panel_frame`], [`backdrop`], [`glass`]), with no lines between their sections ([`divider`]).
//!
//! Tokens are rewritten with struct update syntax (`..t`): a token upstream adds later keeps
//! FigCraft's value until it is mapped here, instead of breaking the build.

use egui::{Color32, CornerRadius, FontId, Frame, Margin, Painter, Pos2, Rect, Stroke, Ui};

use crate::theme::Tokens;

pub mod fonts;
pub mod glass;
pub mod host;

/// The LegatIA blue (#006FFE) and the steps of its scale used by the UI (LegatIA design system).
pub const BLUE_300: Color32 = Color32::from_rgb(0x75, 0xB1, 0xFF);
pub const BLUE_400: Color32 = Color32::from_rgb(0x3D, 0x92, 0xFF);
pub const BLUE_500: Color32 = Color32::from_rgb(0x00, 0x6F, 0xFE);
pub const BLUE_600: Color32 = Color32::from_rgb(0x00, 0x59, 0xCC);
pub const BLUE_900: Color32 = Color32::from_rgb(0x05, 0x29, 0x57);
pub const BLUE_950: Color32 = Color32::from_rgb(0x04, 0x19, 0x34);

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// FigCraft's tokens in LegatIA's colours: black surfaces with hairline borders and the brand blue
/// for accents, selection and focus in the dark theme; only the green accents turn blue in the light one.
pub fn tokens(t: Tokens) -> Tokens {
    if t.dark { dark(t) } else { light(t) }
}

fn dark(t: Tokens) -> Tokens {
    Tokens {
        bg: rgb(0x0A, 0x0A, 0x0B),
        panel: rgb(0x0A, 0x0A, 0x0B),
        canvas: rgb(0x00, 0x00, 0x00),
        // The LegatIA tokens: hairlines are white at 10% and 16% (--glass-border, -hover), text white
        // and white at 70% and 60% (fg, fg-soft, fg-mute), neutral greys with no tint.
        border: Color32::from_white_alpha(26),
        border_strong: Color32::from_white_alpha(41),
        text: rgb(0xFF, 0xFF, 0xFF),
        text_secondary: rgb(0xB3, 0xB3, 0xB3),
        // At least 3:1 on panels, fields and selected rows, dimmer than text_secondary.
        text_tertiary: rgb(0x99, 0x99, 0x99),
        icon: rgb(0xF5, 0xF5, 0xF5),
        icon_secondary: rgb(0x99, 0x99, 0x99),
        // Translucent white, as the LegatIA glass (--glass-bg-*): the same over the panels' glass
        // as over the opaque menus.
        hover: Color32::from_white_alpha(20),
        pressed: Color32::from_white_alpha(31),
        field: Color32::from_white_alpha(15),
        field_hover_border: Color32::from_white_alpha(31),
        accent: BLUE_500,
        accent_hover: BLUE_400,
        accent_on: Color32::WHITE,
        accent_text: BLUE_300,
        selected_row: Color32::from_rgba_unmultiplied(0x00, 0x6F, 0xFE, 0x48),
        selected_row_child: Color32::from_rgba_unmultiplied(0x00, 0x6F, 0xFE, 0x22),
        danger: rgb(0xF8, 0x71, 0x71),
        warning: rgb(0xFB, 0x92, 0x3C),
        tab_bar: rgb(0x00, 0x00, 0x00),
        tab_active: Color32::from_white_alpha(14),
        shadow: Color32::from_black_alpha(160),
        menu: rgb(0x11, 0x11, 0x13),
        menu_hover: BLUE_500,
        focus: BLUE_400,
        ..t
    }
}

fn light(t: Tokens) -> Tokens {
    Tokens {
        accent: BLUE_500,
        accent_hover: BLUE_600,
        accent_on: Color32::WHITE,
        // 5.9:1 on white: links and labels stay legible.
        accent_text: BLUE_600,
        selected_row: rgb(0xEB, 0xF4, 0xFF),
        selected_row_child: rgb(0xF5, 0xF9, 0xFF),
        menu_hover: BLUE_600,
        focus: BLUE_600,
        ..t
    }
}

/// The LegatIA wordmark: "Legat" in the text colour and "IA" in the accent, as the LegatIA logo
/// sets it. `font` is FigCraft's brand face at the requested size.
pub fn paint_wordmark(painter: &Painter, left_center: Pos2, font: FontId, t: &Tokens) {
    let legat = painter.layout_no_wrap("Legat".into(), font.clone(), t.text);
    let width = legat.size().x;
    painter.galley(Pos2::new(left_center.x, left_center.y - legat.size().y / 2.0), legat, t.text);
    painter.text(Pos2::new(left_center.x + width, left_center.y), egui::Align2::LEFT_CENTER, "IA", font, t.accent_text);
}

/// Between the panels, and around them.
const PANEL_GAP: i8 = 6;
/// The cards' corners.
const PANEL_RADIUS: u8 = 18;

/// The side panels' frame: a card floating over the design, rounded like the LegatIA cards (18).
/// In the dark theme its glass (the blurred design and a translucent fill) is painted behind it ([`glass`]), so
/// the frame itself is clear; until then, and in the light theme, it has a fill and a hairline.
/// `left`, `right`: the gap on that side (always one above and below).
pub fn panel_frame(t: &Tokens, left: bool, right: bool) -> Frame {
    let (fill, line) = if t.dark && glass::glazed() {
        (Color32::TRANSPARENT, Color32::TRANSPARENT)
    } else if t.dark {
        (rgb(0x11, 0x11, 0x13), Color32::from_white_alpha(20))
    } else {
        (Color32::from_rgba_unmultiplied(t.panel.r(), t.panel.g(), t.panel.b(), 0xD0), t.border)
    };
    Frame::NONE
        .fill(fill)
        .stroke(Stroke::new(1.0, line))
        .corner_radius(CornerRadius::same(PANEL_RADIUS))
        .outer_margin(Margin { left: if left { PANEL_GAP } else { 0 }, right: if right { PANEL_GAP } else { 0 }, top: PANEL_GAP, bottom: PANEL_GAP })
}

/// Where the floating panels go (the window under the tab bar): the design and its glass go behind
/// them ([`glass`]), in the place kept before the tab bar.
pub fn backdrop(ui: &Ui, _t: &Tokens) {
    glass::area(ui.available_rect_before_wrap());
}

/// The lines between a panel's sections and rows: none, the cards' spacing separates them.
pub fn divider(_t: &Tokens) -> Color32 {
    Color32::TRANSPARENT
}

/// The active item of the left rail, as the LegatIA menu's: a pill of light glass (dark theme); the
/// selected-row fill in the light one.
pub fn active_item(painter: &Painter, rect: Rect, t: &Tokens) {
    if !t.dark {
        painter.rect_filled(rect, CornerRadius::same(6), t.selected_row);
        return;
    }
    painter.rect_filled(rect, CornerRadius::same((rect.height() / 2.0) as u8), Color32::from_white_alpha(16));
}

/// A secondary (not filled) button's edge: in the dark theme a light glass fill with no line, as the
/// secondary buttons of LegatIA and macOS; the outline in the light theme.
pub fn button_edge(painter: &Painter, rect: Rect, t: &Tokens) {
    if t.dark {
        painter.rect_filled(rect, CornerRadius::same(6), Color32::from_white_alpha(18));
    } else {
        painter.rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    }
}

/// The active rail item's icon: white on the dark glass, the accent in the light theme.
pub fn active_icon(t: &Tokens) -> Color32 {
    if t.dark { t.icon } else { t.accent_text }
}

/// What the left rail grows by: its gap from the window edge ([`panel_frame`]), so its labels keep their room.
pub const RAIL_GAP: f32 = PANEL_GAP as f32;
