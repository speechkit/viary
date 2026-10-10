//! What the desktop lets Viary do: hear the talk shortcut, and type.

use serde::Serialize;

use super::{extension, is_wayland};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    /// Viary can type into other apps.
    pub accessibility: bool,
    /// Viary hears the talk shortcut.
    pub input_monitoring: bool,
}

pub fn check() -> Permissions {
    Permissions {
        accessibility: can_type(),
        input_monitoring: super::hotkey::bound(),
    }
}

/// Whether Viary can send Ctrl+V: always on X11; on Wayland, while its
/// GNOME Shell extension runs.
pub fn can_type() -> bool {
    !is_wayland() || extension::active()
}

/// What to do when [`can_type`] is false.
pub const ALLOW_TYPING: &str = "Ctrl+V to paste";

/// Nothing to ask GNOME for: the setup window installs the extension.
pub fn request(_kind: &str) {}

/// Opens GNOME Settings at `kind`'s page.
pub fn open_settings(kind: &str) {
    let panel = match kind {
        // Input device and volume.
        "microphone" => "sound",
        _ => return,
    };
    if let Err(error) = std::process::Command::new("gnome-control-center").arg(panel).spawn() {
        tracing::warn!(%error, "cannot open GNOME Settings");
    }
}

/// What is missing for dictation, for the indicator, if anything.
pub fn missing(hotkey_active: bool) -> Option<&'static str> {
    if is_wayland() && !extension::active() {
        return Some("turn on Viary's GNOME Shell extension");
    }
    (!hotkey_active).then_some("another app holds Ctrl+Alt+Space")
}
