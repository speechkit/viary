//! Linux, as GNOME runs it. On Wayland, Viary's GNOME Shell extension
//! hears the talk shortcut, types, names the focused app, and draws the
//! pill: Wayland lets no app do these itself. On X11, Viary grabs the
//! shortcut, types through XTest, and shows its own pill window. The
//! clipboard goes through X11 (XWayland on Wayland) on both.

pub mod apps;
pub mod extension;
pub mod focus;
pub mod hotkey;
pub mod keys;
pub mod pasteboard;
pub mod permissions;
mod x11;

use std::sync::OnceLock;

use serde::Serialize;
use tauri::AppHandle;

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

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Call once at startup, before the hotkey listener.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
    extension::listen(app);
}

fn app() -> Option<&'static AppHandle> {
    APP.get()
}

/// The name Viary's session bus connection owns. Viary's extension answers
/// only the client that owns it.
const CLIENT_NAME: &str = "app.viary.App";

/// The session bus, connected once: the pill's level alone calls it
/// twenty times a second.
async fn session_bus() -> zbus::Result<zbus::Connection> {
    static BUS: tokio::sync::OnceCell<zbus::Connection> = tokio::sync::OnceCell::const_new();
    BUS.get_or_try_init(|| async {
        let connection = zbus::Connection::session().await?;
        // Viary serves nothing, but zbus wants a server up before a name
        // is owned; calls to it are then answered, not lost.
        let _ = connection.object_server();
        if let Err(error) = connection.request_name(CLIENT_NAME).await {
            tracing::warn!(%error, "cannot own {CLIENT_NAME}; the GNOME Shell extension will not answer");
        }
        Ok::<_, zbus::Error>(connection)
    })
    .await
    .cloned()
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
    extension: extension::Status,
}

/// The desktop's state for the web views.
pub fn desktop() -> serde_json::Value {
    serde_json::to_value(Status {
        session: if is_wayland() { "wayland" } else { "x11" },
        extension: extension::status(),
    })
    .unwrap_or_default()
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
