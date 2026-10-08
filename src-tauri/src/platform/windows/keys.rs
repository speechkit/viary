//! Synthetic Ctrl+V and Ctrl+Z, sent to whichever window has focus.
//!
//! Windows apps match shortcuts by virtual key, which is the same on every
//! layout: Ctrl+V is VK_CONTROL with 'V' on AZERTY and on Russian alike.

use tauri::AppHandle;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_CONTROL,
};

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
