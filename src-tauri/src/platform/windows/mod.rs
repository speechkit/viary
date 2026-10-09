//! Windows: the hold-to-talk key, typing into other apps, the clipboard,
//! and the microphone privacy setting.

pub mod apps;
pub mod focus;
pub mod hotkey;
pub mod keys;
pub mod pasteboard;
pub mod permissions;

/// How the user pastes text left on the clipboard.
pub const PASTE_HINT: &str = "Ctrl+V to paste";
/// The key that undoes an insertion.
pub const UNDO_KEY: &str = "Ctrl+Z";

/// A Rust string as a NUL-terminated UTF-16 buffer for Win32 calls.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Whether the taskbar uses the dark theme, as Settings › Personalization ›
/// Colors sets it. Windows 10 and 11 default to dark.
pub fn dark_taskbar() -> bool {
    use windows::{
        Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        core::PCWSTR,
    };

    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    let mut light = 0_u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: a DWORD read into a DWORD, with its size.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut light).cast()),
            Some(&raw mut size),
        )
    };
    read.is_err() || light == 0
}

/// Whether the keyboard layout in use has AltGr: Right Alt then types
/// characters (é, @, ł) with other keys, so holding it to talk would get
/// in the way. Read from the layout itself: some key types a character
/// with Ctrl+Alt held, which is what AltGr sends.
pub fn layout_has_altgr() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardLayout, MAPVK_VK_TO_VSC, MapVirtualKeyExW, ToUnicodeEx, VK_CONTROL, VK_LCONTROL,
        VK_MENU, VK_RMENU,
    };

    /// ToUnicodeEx: leave the keyboard's dead-key state alone.
    const KEEP_STATE: u32 = 0x4;

    let mut state = [0_u8; 256];
    for vk in [VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RMENU] {
        state[usize::from(vk.0)] = 0x80;
    }
    // SAFETY: plain queries; the buffers outlive the calls.
    unsafe {
        let layout = GetKeyboardLayout(0);
        // Digits, letters, and the punctuation keys.
        (0x30_u32..=0x5A).chain(0xBA..=0xE2).any(|vk| {
            let scan = MapVirtualKeyExW(vk, MAPVK_VK_TO_VSC, Some(layout));
            if scan == 0 {
                return false;
            }
            let mut typed = [0_u16; 4];
            let n = ToUnicodeEx(vk, scan, &state, &mut typed, KEEP_STATE, Some(layout));
            // A dead key (negative) counts: AltGr+key starts an accent.
            n < 0 || (n > 0 && typed[0] >= 0x20)
        })
    }
}
