//! What Windows lets Viary do. Desktop apps need no permission to watch the
//! keyboard or type into other windows; the microphone is the one switch,
//! under Settings › Privacy & security › Microphone.

use serde::Serialize;
use windows::{
    Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW},
    core::PCWSTR,
};

use super::wide;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    /// Always true: kept so every system answers in the same shape.
    pub accessibility: bool,
    /// Always true, as above.
    pub input_monitoring: bool,
    /// "Microphone access" and "Let desktop apps access your microphone".
    pub microphone: bool,
}

const CONSENT: &str =
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

/// Whether the consent store says "Deny" under `key`. A missing value is
/// not a refusal.
fn denied(key: &str) -> bool {
    let key = wide(key);
    let name = wide("Value");
    let mut buffer = [0_u16; 16];
    let mut size = size_of_val(&buffer) as u32;
    // SAFETY: the buffer and its size are passed together.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut size),
        )
    };
    if read.is_err() {
        return false;
    }
    let len = (size as usize / 2).saturating_sub(1).min(buffer.len());
    String::from_utf16_lossy(&buffer[..len]) == "Deny"
}

pub fn check() -> Permissions {
    Permissions {
        accessibility: true,
        input_monitoring: true,
        microphone: !denied(CONSENT) && !denied(&format!(r"{CONSENT}\NonPackaged")),
    }
}

/// Whether Viary may send Ctrl+V to other apps: always.
pub fn can_type() -> bool {
    true
}

/// What to allow when [`can_type`] is false; never shown on Windows.
pub const ALLOW_TYPING: &str = "Ctrl+V to paste";

/// Windows has no prompt to show; the settings page is the way.
pub fn request(_kind: &str) {}

/// Opens the Settings page for `kind`: `microphone`, or `taskbar`, where
/// tray icons can be kept in view.
pub fn open_settings(kind: &str) {
    let page = match kind {
        "microphone" => "ms-settings:privacy-microphone",
        "taskbar" => "ms-settings:taskbar",
        _ => return,
    };
    // Explorer hands the URI to Settings, without a console window.
    if let Err(error) = std::process::Command::new("explorer")
        .arg(page)
        .spawn()
    {
        tracing::warn!(%error, "cannot open Settings");
    }
}

/// What is missing for dictation, for the tray icon, if anything.
pub fn missing(hotkey_active: bool) -> Option<&'static str> {
    if !check().microphone {
        Some("microphone blocked")
    } else if !hotkey_active {
        Some("the talk key is not working")
    } else {
        None
    }
}
