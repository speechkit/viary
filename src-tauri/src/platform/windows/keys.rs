//! Synthetic Ctrl+V and Ctrl+Z, sent to whichever window has focus.
//!
//! Windows apps match shortcuts by virtual key, which is the same on every
//! layout: Ctrl+V is VK_CONTROL with 'V' on AZERTY and on Russian alike.

use std::time::{Duration, Instant};

use tauri::AppHandle;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RMENU,
    VK_RSHIFT, VK_RWIN,
};

/// Keys that would turn Ctrl+V into another shortcut while held: the app
/// would get Ctrl+Win+V, or Ctrl+Alt+V (AltGr+V on many layouts). The talk
/// key can still be down when a quick dictation ends. Ctrl is not here:
/// Ctrl+V is still Ctrl+V.
const IN_THE_WAY: [VIRTUAL_KEY; 6] = [VK_LWIN, VK_RWIN, VK_LMENU, VK_RMENU, VK_LSHIFT, VK_RSHIFT];

/// How long to wait for those keys to come up before leaving the text on
/// the clipboard instead.
const WAIT_FOR_KEYS: Duration = Duration::from_millis(1500);

/// Waits until none of [`IN_THE_WAY`] is held; false if one still is.
fn keys_released() -> bool {
    let deadline = Instant::now() + WAIT_FOR_KEYS;
    loop {
        // SAFETY: plain queries.
        let held = IN_THE_WAY.iter().any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0);
        if !held {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                ..Default::default()
            },
        },
    }
}

fn control(letter: u8) -> Result<(), String> {
    if !keys_released() {
        return Err("a modifier key is still held".into());
    }
    let letter = VIRTUAL_KEY(u16::from(letter));
    let inputs = [
        key(VK_CONTROL, false),
        key(letter, false),
        key(letter, true),
        key(VK_CONTROL, true),
    ];
    // SAFETY: the inputs are fully initialized keyboard events.
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        // Blocked by UIPI: the window in front runs as administrator.
        Err(windows::core::Error::from_win32().to_string())
    }
}

/// Ctrl+V.
pub fn paste(_: &AppHandle) -> Result<(), String> {
    control(b'V')
}

/// Ctrl+Z.
pub fn undo(_: &AppHandle) -> Result<(), String> {
    control(b'Z')
}
