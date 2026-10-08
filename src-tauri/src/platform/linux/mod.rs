//! Linux, as GNOME runs it: the talk shortcut GNOME hands over, typing
//! through the RemoteDesktop portal (Wayland) or XTest (X11), the clipboard
//! through X11 (XWayland on Wayland), and results as notifications where
//! Wayland keeps the pill from floating.

pub mod apps;
pub mod focus;
pub mod hotkey;
pub mod keys;
pub mod notify;
pub mod pasteboard;
pub mod permissions;
mod remote;
mod x11;

use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::lock;

/// How the user pastes text left on the clipboard.
pub const PASTE_HINT: &str = "Ctrl+V to paste";
/// The key that undoes an insertion.
pub const UNDO_KEY: &str = "Ctrl+Z";

/// Whether the session is Wayland, where apps cannot type into or look at
/// other windows. Otherwise X11.
pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland")
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// How Viary types into other apps on Wayland. X11 needs no choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Typing {
    /// Through GNOME's RemoteDesktop portal: GNOME asks once.
    Portal,
    /// Ctrl+V is left to the user.
    #[default]
    Clipboard,
}

/// What Viary keeps about the desktop, beside the settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Prefs {
    typing: Typing,
    /// Restores the RemoteDesktop permission without asking again.
    restore_token: Option<String>,
}

struct Desktop {
    app: AppHandle,
    file: PathBuf,
    prefs: Mutex<Prefs>,
}

static DESKTOP: OnceLock<Desktop> = OnceLock::new();

/// Call once at startup, before the hotkey listener.
pub fn init(app: &AppHandle, config_dir: &std::path::Path) {
    let file = config_dir.join("desktop.json");
    let prefs = crate::json_store::load(&file, "desktop preferences");
    let _ = DESKTOP.set(Desktop {
        app: app.clone(),
        file,
        prefs: Mutex::new(prefs),
    });
}

fn prefs() -> Prefs {
    DESKTOP.get().map(|d| lock(&d.prefs).clone()).unwrap_or_default()
}

fn change_prefs(edit: impl FnOnce(&mut Prefs)) {
    if let Some(desktop) = DESKTOP.get() {
        let mut prefs = lock(&desktop.prefs);
        edit(&mut prefs);
        crate::json_store::save(&desktop.file, &*prefs, "desktop preferences");
    }
}

fn app() -> Option<&'static AppHandle> {
    DESKTOP.get().map(|d| &d.app)
}

/// What the setup window and Settings show about the desktop.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    /// `wayland` or `x11`.
    session: &'static str,
    shortcut: hotkey::Status,
    typing: Typing,
    /// GNOME already allowed typing through the portal.
    portal_allowed: bool,
}

/// The desktop's state for the web views.
pub fn desktop() -> serde_json::Value {
    let prefs = prefs();
    serde_json::to_value(Status {
        session: if is_wayland() { "wayland" } else { "x11" },
        shortcut: hotkey::status(),
        typing: prefs.typing,
        portal_allowed: prefs.restore_token.is_some(),
    })
    .unwrap_or_default()
}

/// Asks GNOME for the talk shortcut: its dialog through the portal, or a
/// custom shortcut where the portal is missing.
pub fn bind_shortcut() -> Result<(), String> {
    hotkey::bind()
}

/// Chooses how Viary types. The portal asks GNOME now, so its dialog
/// comes while the user is choosing, not mid-dictation.
pub fn set_typing(method: &str) -> Result<(), String> {
    let typing = match method {
        "portal" => Typing::Portal,
        "clipboard" => Typing::Clipboard,
        _ => return Err(format!("unknown typing method {method}")),
    };
    if typing == Typing::Portal {
        remote::allow()?;
    }
    change_prefs(|p| p.typing = typing);
    Ok(())
}
