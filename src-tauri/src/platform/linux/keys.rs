//! Synthetic Ctrl+V and Ctrl+Z: XTest on X11; on Wayland, Viary's GNOME
//! Shell extension. Keysyms, not keycodes, so the layout decides which
//! key types "v".

use tauri::AppHandle;

use super::{extension, is_wayland, x11};

/// Ctrl+V.
pub fn paste(_: &AppHandle) -> Result<(), String> {
    if is_wayland() { extension::paste() } else { x11::press(&[x11::CONTROL_L, x11::KEY_V]) }
}

/// Ctrl+Z.
pub fn undo(_: &AppHandle) -> Result<(), String> {
    if is_wayland() { extension::undo() } else { x11::press(&[x11::CONTROL_L, x11::KEY_Z]) }
}
