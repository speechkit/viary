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

/// Whether Windows reports `vk` down. Inside the keyboard hook this is the
/// state before the key being reported.
fn is_down(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    // SAFETY: a plain query.
    unsafe { GetAsyncKeyState(i32::from(vk.0)) < 0 }
}

/// A key going down, or up with `KEYEVENTF_KEYUP` in `flags`, for SendInput.
fn key_input(
    vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT};

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
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

/// Whether a keyboard layout the user has installed has AltGr: Right Alt
/// then types characters (é, @, ł) with other keys, so holding it to talk
/// would get in the way. Every layout is checked, not only Viary's own:
/// Windows can keep a layout per app, and the user switches between them.
pub fn layout_has_altgr() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, GetKeyboardLayoutList, HKL};

    // SAFETY: the list is read into a buffer of the length passed.
    let layouts = unsafe {
        let count = usize::try_from(GetKeyboardLayoutList(None)).unwrap_or(0);
        let mut layouts = vec![HKL::default(); count];
        let read = usize::try_from(GetKeyboardLayoutList(Some(&mut layouts))).unwrap_or(0);
        layouts.truncate(read);
        if layouts.is_empty() {
            layouts.push(GetKeyboardLayout(0));
        }
        layouts
    };
    layouts.into_iter().any(has_altgr)
}

/// Whether `layout` has AltGr, read from the layout itself: some key
/// types a character with Ctrl+Alt held, which is what AltGr sends.
fn has_altgr(layout: windows::Win32::UI::Input::KeyboardAndMouse::HKL) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MAPVK_VK_TO_VSC, MapVirtualKeyExW, ToUnicodeEx, VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RMENU,
    };

    /// ToUnicodeEx: leave the keyboard's dead-key state alone.
    const KEEP_STATE: u32 = 0x4;

    let mut state = [0_u8; 256];
    for vk in [VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RMENU] {
        state[usize::from(vk.0)] = 0x80;
    }
    // SAFETY: plain queries; the buffers outlive the calls.
    unsafe {
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
