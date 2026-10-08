//! The app the user is dictating into. X11 names the active window;
//! Wayland tells other apps nothing, so there only Viary's extension can
//! say, and without it the paste goes wherever the focus is.

use super::{extension, is_wayland, x11};

#[derive(Debug, Clone, Default)]
pub struct TargetApp {
    pub pid: i32,
    pub name: String,
    /// The X11 window, or 0.
    pub window: u32,
}

impl TargetApp {
    pub fn is_self(&self) -> bool {
        self.pid == std::process::id().cast_signed()
    }
}

pub fn frontmost() -> Option<TargetApp> {
    if is_wayland() {
        // Only the shell knows; its extension may say.
        if !extension::active() {
            return None;
        }
        let (pid, name) = extension::focused_app()?;
        return Some(TargetApp { pid, name, window: 0 });
    }
    let (window, pid, name) = x11::active_window()?;
    Some(TargetApp { pid, name, window })
}

/// Brings `target` back to the front; on Wayland, trusts that it still is.
pub fn activate(target: &TargetApp) -> bool {
    if target.window == 0 || is_wayland() {
        return true;
    }
    if frontmost().is_some_and(|app| app.window == target.window) {
        return true;
    }
    x11::activate(target.window)
}

/// Whether the system keeps Viary's keystrokes from `target`: never here.
pub fn is_protected(_target: &TargetApp) -> bool {
    false
}
