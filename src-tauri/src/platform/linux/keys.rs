//! Synthetic Ctrl+V and Ctrl+Z: XTest on X11; on Wayland, Viary's GNOME
//! Shell extension or the RemoteDesktop portal, whichever the user chose.
//! Keysyms, not keycodes, so the layout decides which key types "v".

use tauri::AppHandle;

use super::{Typing, extension, is_wayland, prefs, remote, x11};

fn control(letter: u32) -> Result<(), String> {
    let keys = [x11::CONTROL_L, letter];
    if !is_wayland() {
        return x11::press(&keys);
    }
    match prefs().typing {
        Typing::Extension if letter == x11::KEY_V => extension::paste(),
        Typing::Extension => extension::undo(),
        Typing::Portal => remote::press(&keys),
        Typing::Clipboard => Err("Viary leaves the text on the clipboard".into()),
    }
}

/// Ctrl+V.
pub fn paste(_: &AppHandle) -> Result<(), String> {
    control(x11::KEY_V)
}

/// Ctrl+Z.
pub fn undo(_: &AppHandle) -> Result<(), String> {
    control(x11::KEY_Z)
}
