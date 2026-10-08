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
