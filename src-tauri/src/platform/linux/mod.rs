//! Linux, as GNOME runs it: the talk shortcut GNOME hands over; typing
//! through Viary's GNOME Shell extension or the RemoteDesktop portal
//! (Wayland), or XTest (X11); the clipboard through X11 (XWayland on
//! Wayland); and the pill drawn by the extension, or results as
//! notifications where Wayland keeps a window from floating.

pub mod apps;
pub mod extension;
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
    /// Through Viary's GNOME Shell extension: no prompts.
    Extension,
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

/// The name Viary's session bus connection owns. Viary's extension answers
/// only the client that owns it.
const CLIENT_NAME: &str = "app.viary.App";

/// The session bus, connected once: the pill's level alone calls it
/// twenty times a second.
async fn session_bus() -> ashpd::zbus::Result<ashpd::zbus::Connection> {
    static BUS: tokio::sync::OnceCell<ashpd::zbus::Connection> = tokio::sync::OnceCell::const_new();
    BUS.get_or_try_init(|| async {
        let connection = ashpd::zbus::Connection::session().await?;
        if let Err(error) = connection.request_name(CLIENT_NAME).await {
            tracing::warn!(%error, "cannot own {CLIENT_NAME}; the GNOME Shell extension will not answer");
        }
        Ok::<_, ashpd::zbus::Error>(connection)
    })
    .await
    .cloned()
}

/// Whether `message` came from the client that owns `name` now. Any client
/// on the session bus can send a signal naming any interface, so a signal
/// is only trusted from the service it claims to be from.
async fn sent_by(message: &ashpd::zbus::Message, name: &str) -> bool {
    let Some(sender) = message.header().sender().map(ToString::to_string) else {
        return false;
    };
    let Ok(connection) = session_bus().await else {
        return false;
    };
    let owner = connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &(name,),
        )
        .await;
    owner
        .ok()
        .and_then(|reply| reply.body().deserialize::<String>().ok())
        .is_some_and(|owner| owner == sender)
}

/// Runs `gsettings` with `args`; its output, or its error.
fn gsettings(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("gsettings")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// A string list setting, as `gsettings get` prints it: `['a', 'b']`, or
/// `@as []` when empty.
fn parse_list(printed: &str) -> Vec<String> {
    printed
        .trim()
        .trim_start_matches("@as")
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|item| item.trim().trim_matches('\'').to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Reads the string list `key` of `schema`.
fn gsettings_list(schema: &str, key: &str) -> Result<Vec<String>, String> {
    gsettings(&["get", schema, key]).map(|printed| parse_list(&printed))
}

/// Sets the string list `key` of `schema` to `items`.
fn gsettings_set_list(schema: &str, key: &str, items: &[String]) -> Result<(), String> {
    let quoted: Vec<String> = items.iter().map(|item| format!("'{item}'")).collect();
    gsettings(&["set", schema, key, &format!("[{}]", quoted.join(", "))]).map(drop)
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
    extension: extension::Status,
}

/// The desktop's state for the web views.
pub fn desktop() -> serde_json::Value {
    let prefs = prefs();
    serde_json::to_value(Status {
        session: if is_wayland() { "wayland" } else { "x11" },
        shortcut: hotkey::status(),
        typing: prefs.typing,
        portal_allowed: prefs.restore_token.is_some(),
        extension: extension::status(),
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
        "extension" if extension::active() => Typing::Extension,
        "extension" => return Err("Viary's extension is not running yet. Log out and back in first.".into()),
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

/// Installs Viary's GNOME Shell extension, which runs from the next login.
pub fn install_extension() -> Result<(), String> {
    extension::install()
}

/// Whether the shell draws the pill: Wayland, with the extension running.
pub fn shell_pill() -> bool {
    is_wayland() && extension::active()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_gsettings_list_is_read_without_its_quotes() {
        assert_eq!(super::parse_list("@as []"), Vec::<String>::new());
        assert_eq!(
            super::parse_list("['ding@rastersoft.com', 'viary@viary.app']\n"),
            ["ding@rastersoft.com", "viary@viary.app"]
        );
    }
}
