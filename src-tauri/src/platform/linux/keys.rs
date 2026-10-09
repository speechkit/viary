//! Synthetic Ctrl+V and Ctrl+Z: XTest on X11; on Wayland, Viary's GNOME
//! Shell extension or the RemoteDesktop portal, whichever the user chose.
//! Keysyms, not keycodes, so the layout decides which key types "v".

use std::time::Duration;

use tauri::AppHandle;

use super::{Typing, extension, hotkey, is_wayland, prefs, remote, x11};

/// The portal cannot say which keys are down, and GNOME reports the talk
/// shortcut released when its first key comes up: Alt can still be held.
/// Typing waits until this long after the release, so Ctrl+V does not
/// arrive as Ctrl+Alt+V. (X11 and the extension ask which keys are down.)
const SETTLE: Duration = Duration::from_millis(400);

fn settle() {
    if let Some(since) = hotkey::since_release()
        && since < SETTLE
    {
        std::thread::sleep(SETTLE - since);
    }
}

fn control(letter: u32) -> Result<(), String> {
    let keys = [x11::CONTROL_L, letter];
    if !is_wayland() {
        return x11::press(&keys);
    }
    match prefs().typing {
        Typing::Extension if letter == x11::KEY_V => extension::paste(),
        Typing::Extension => extension::undo(),
        Typing::Portal => {
            settle();
            remote::press(&keys)
        }
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
