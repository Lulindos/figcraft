//! LegatIA: the interface in the LegatIA typeface (Satoshi). Its licence doesn't allow shipping the
//! files with the editor (the releases are public), so the page hands them over at runtime
//! (`figcraft.uiFonts(regular, bold)`, the web shell): [`set`] keeps them and installs the fonts
//! again, and `theme::install_fonts` puts them first through [`install`] every time it runs.

use std::sync::{Arc, Mutex};

use egui::{FontData, FontDefinitions, FontFamily};

struct UiFonts {
    regular: Arc<FontData>,
    bold: Arc<FontData>,
}

static FONTS: Mutex<Option<UiFonts>> = Mutex::new(None);
/// `install_fonts`' last arguments, to install again the same way.
static LAST: Mutex<(bool, u8)> = Mutex::new((false, 0));

/// The interface's text in `regular`, its titles and buttons in `bold` (font files).
pub fn set(ctx: &egui::Context, regular: Vec<u8>, bold: Vec<u8>) {
    *FONTS.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(UiFonts { regular: Arc::new(FontData::from_owned(regular)), bold: Arc::new(FontData::from_owned(bold)) });
    let (system, cjk) = *LAST.lock().unwrap_or_else(|e| e.into_inner());
    crate::theme::install_fonts(ctx, system, cjk);
    ctx.request_repaint();
}

/// Called by `install_fonts` before it hands `fonts` to egui: the page's faces lead their families
/// (FigCraft's stay behind them for what they lack).
pub(crate) fn install(fonts: &mut FontDefinitions, system: bool, cjk: u8) {
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = (system, cjk);
    let guard = FONTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(ui) = guard.as_ref() else { return };
    fonts.font_data.insert("legatia-ui".into(), ui.regular.clone());
    fonts.font_data.insert("legatia-ui-bold".into(), ui.bold.clone());
    if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
        list.insert(0, "legatia-ui".into());
    }
    for family in ["semibold", "brand"] {
        if let Some(list) = fonts.families.get_mut(&FontFamily::Name(family.into())) {
            list.insert(0, "legatia-ui-bold".into());
        }
    }
}
