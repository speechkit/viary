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

/// The system Viary was built for, as the web views name it.
pub const NAME: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "windows") {
    "windows"
} else {
    "linux"
};
