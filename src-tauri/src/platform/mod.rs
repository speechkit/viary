//! The parts of Viary that talk to the operating system directly: the
//! hold-to-talk key, typing into other apps, the clipboard, and privacy
//! permissions. Each system has a module with the same items:
//!
//! - `apps`: [`TargetApp`](apps::TargetApp), `frontmost()`, `activate()`
//! - `focus`: [`Focus`](focus::Focus), `focused(pid)`
//! - `hotkey`: [`HotkeyEvent`](hotkey::HotkeyEvent) and
//!   [`HotkeyListener`](hotkey::HotkeyListener)
//! - `keys`: `paste(app)` and `undo(app)`
//! - `pasteboard`: `save()`, `set_text()`, `set_transient_text()`, `restore()`
//! - `permissions`: [`Permissions`](permissions::Permissions), `check()`,
//!   `can_type()`, `request()`, `open_settings()`

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(target_os = "linux"))]
mod desktop {
    //! The desktop choices only Linux has; elsewhere there is nothing to set.

    /// What the setup window shows about the desktop: nothing here.
    pub fn desktop() -> serde_json::Value {
        serde_json::Value::Null
    }

    pub fn bind_shortcut() -> Result<(), String> {
        Err("this system has no talk shortcut to bind".into())
    }

    pub fn set_typing(_method: &str) -> Result<(), String> {
        Err("this system has one way to type".into())
    }

    pub fn install_extension() -> Result<(), String> {
        Err("GNOME Shell extensions are for Linux".into())
    }
}
#[cfg(not(target_os = "linux"))]
pub use desktop::*;

/// The keyboard layout in use, where it bears on the talk key.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Keyboard {
    /// Right Alt is AltGr, which types characters (Windows).
    pub alt_gr: bool,
}

/// The keyboard layout, on Windows, where Right Alt can be the talk key.
pub fn keyboard() -> Option<Keyboard> {
    #[cfg(target_os = "windows")]
    return Some(Keyboard {
        alt_gr: windows::layout_has_altgr(),
    });
    #[cfg(not(target_os = "windows"))]
    None
}

/// The talk key's name where the desktop, not Viary, decides the keys:
/// GNOME's shortcut, which the user can change in GNOME Settings.
pub fn hotkey_label() -> Option<String> {
    #[cfg(target_os = "linux")]
    return hotkey::trigger();
    #[cfg(not(target_os = "linux"))]
    None
}

/// The system Viary was built for, as the web views name it.
pub const NAME: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "windows") {
    "windows"
} else {
    "linux"
};
