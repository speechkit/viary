//! What the desktop lets Viary do: hear the talk shortcut, and type.

use serde::Serialize;

use super::{Typing, is_wayland, prefs};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    /// Viary can type into other apps.
    pub accessibility: bool,
    /// GNOME hands the talk shortcut to Viary.
    pub input_monitoring: bool,
}

pub fn check() -> Permissions {
    Permissions {
        accessibility: can_type(),
        input_monitoring: super::hotkey::status().bound,
    }
}

/// Whether Viary can send Ctrl+V: always on X11; on Wayland, once the user
/// chose the portal and GNOME allowed it.
pub fn can_type() -> bool {
    !is_wayland() || {
        let prefs = prefs();
        prefs.typing == Typing::Portal && prefs.restore_token.is_some()
    }
}

/// What to do when [`can_type`] is false.
pub const ALLOW_TYPING: &str = "Ctrl+V to paste";

/// GNOME asks through its own dialogs, from the setup window.
pub fn request(_kind: &str) {}

/// Opens GNOME Settings at `kind`'s page.
pub fn open_settings(kind: &str) {
    let panel = match kind {
        "microphone" => "privacy",
        "inputMonitoring" => "keyboard",
        _ => return,
    };
    if let Err(error) = std::process::Command::new("gnome-control-center").arg(panel).spawn() {
        tracing::warn!(%error, "cannot open GNOME Settings");
    }
}
