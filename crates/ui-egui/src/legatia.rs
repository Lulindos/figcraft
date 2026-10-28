//! LegatIA: the LegatIA colours and wordmark over FigCraft's tokens (fork Lulindos/figcraft, branch
//! `legatia`). Kept in a file of its own so upstream updates don't touch it: `theme.rs` only calls
//! [`tokens`] when it installs a theme and [`paint_wordmark`] when it draws the name. See LEGATIA.md.
//!
//! Tokens are rewritten with struct update syntax (`..t`): a token upstream adds later keeps
//! FigCraft's value until it is mapped here, instead of breaking the build.

use egui::{Color32, FontId, Painter, Pos2};

use crate::theme::Tokens;

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
        border: rgb(0x1F, 0x1F, 0x23),
        border_strong: rgb(0x2E, 0x2E, 0x33),
        text: rgb(0xFF, 0xFF, 0xFF),
        text_secondary: rgb(0xB3, 0xB3, 0xB8),
        // At least 3:1 on panels, fields and selected rows, dimmer than text_secondary.
        text_tertiary: rgb(0x8A, 0x8A, 0x90),
        icon: rgb(0xF5, 0xF5, 0xF7),
        icon_secondary: rgb(0xA6, 0xA6, 0xAD),
        hover: rgb(0x17, 0x17, 0x1A),
        pressed: rgb(0x22, 0x22, 0x27),
        field: rgb(0x14, 0x14, 0x17),
        field_hover_border: rgb(0x33, 0x33, 0x39),
        accent: BLUE_500,
        accent_hover: BLUE_400,
        accent_on: Color32::WHITE,
        accent_text: BLUE_300,
        selected_row: BLUE_900,
        selected_row_child: BLUE_950,
        danger: rgb(0xF8, 0x71, 0x71),
        warning: rgb(0xFB, 0x92, 0x3C),
        tab_bar: rgb(0x00, 0x00, 0x00),
        tab_active: rgb(0x0A, 0x0A, 0x0B),
        shadow: Color32::from_black_alpha(160),
        menu: rgb(0x0E, 0x0E, 0x10),
        menu_hover: BLUE_600,
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
