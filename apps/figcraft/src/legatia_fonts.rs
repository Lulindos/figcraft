//! LegatIA: the interface in Satoshi, from the files the installer put in the user's fonts folder
//! (`install.ps1` downloads them from Fontshare: the licence doesn't allow shipping them). Without
//! them the interface keeps FigCraft's own typeface.

use std::path::PathBuf;

/// Where Windows keeps the fonts of one user (no administrator needed).
fn fonts_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Microsoft").join("Windows").join("Fonts"))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Regular for the text, Medium for titles and buttons: the weights the site uses.
pub fn load(ctx: &egui::Context) {
    let Some(dir) = fonts_dir() else { return };
    let (Ok(regular), Ok(medium)) = (std::fs::read(dir.join("LegatIA-Satoshi-Regular.ttf")), std::fs::read(dir.join("LegatIA-Satoshi-Medium.ttf"))) else {
        return;
    };
    figcraft_ui_egui::legatia::fonts::set(ctx, regular, medium);
}
