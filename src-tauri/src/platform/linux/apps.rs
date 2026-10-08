//! The app the user is dictating into. X11 names the active window;
//! Wayland tells other apps nothing, so there the target is unknown and
//! the paste goes wherever the focus is.

use super::{is_wayland, x11};

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
        return None;
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
