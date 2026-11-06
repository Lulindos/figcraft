//! LegatIA: the folder-sync program that ships beside the app (`legatia-sync/`: its own `node`
//! and `sync.mjs`) opens with the app and ends with it.
//!
//! It only runs while the app runs, so an update can replace its files: the app ends it before it
//! exits (the update helper waits for the app, then copies over the folder), and it also ends by
//! itself when its parent is gone (`LEGATIA_PARENT_PID`). A copy already answering on its port (one
//! started by hand from the repository, say) is left alone. Nothing happens when the folder isn't
//! there (a build from source).

use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

static CHILD: Mutex<Option<Child>> = Mutex::new(None);

fn port() -> u16 {
    std::env::var("LEGATIA_SYNC_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(47821)
}

fn answering(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(300)).is_ok()
}

/// The folder with the program, beside the executable.
fn folder() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.join("legatia-sync");
    dir.is_dir().then_some(dir)
}

/// Starts the program when it is shipped and not already running.
pub fn start() {
    let Some(dir) = folder() else { return };
    let node = dir.join(if cfg!(windows) { "node.exe" } else { "node" });
    let script = dir.join("sync.mjs");
    if !node.is_file() || !script.is_file() || answering(port()) {
        return;
    }
    let mut command = Command::new(node);
    command
        .arg("--disable-warning=MODULE_TYPELESS_PACKAGE_JSON")
        .arg(script)
        .current_dir(&dir)
        .env("LEGATIA_PARENT_PID", std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // No console window: the app is a window program and so is its helper.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    match command.spawn() {
        Ok(child) => *CHILD.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(child),
        Err(e) => log::warn!("legatia-sync did not start: {e}"),
    }
}

/// Ends the program this app started, and waits until its files are free.
pub fn stop() {
    let child = CHILD.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
}
