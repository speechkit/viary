//! The privacy permissions Viary needs, and the System Settings panes that
//! grant them.

use core_foundation::{
    base::TCFType,
    boolean::CFBoolean,
    dictionary::{CFDictionary, CFDictionaryRef},
    string::CFString,
};
use serde::Serialize;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    /// Posting ⌘V into other apps, and reading which field has focus.
    pub accessibility: bool,
    /// Watching the hold-to-talk key.
    pub input_monitoring: bool,
}

pub fn check() -> Permissions {
    // SAFETY: both functions only read the process's TCC state.
    unsafe {
        Permissions {
            accessibility: AXIsProcessTrusted(),
            input_monitoring: CGPreflightListenEventAccess(),
        }
    }
}

/// Whether Viary may post ⌘V to other apps: Accessibility.
pub fn can_type() -> bool {
    // SAFETY: reads TCC state.
    unsafe { AXIsProcessTrusted() }
}

/// What to allow when [`can_type`] is false.
pub const ALLOW_TYPING: &str = "Allow Accessibility to paste";

/// Shows the system prompt for `kind`, which adds Viary to the list in
/// System Settings.
pub fn request(kind: &str) {
    match kind {
        "accessibility" => {
            let key = CFString::new("AXTrustedCheckOptionPrompt");
            let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
            // SAFETY: `options` is a valid CFDictionary for the call.
            unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) };
        }
        "inputMonitoring" => {
            // SAFETY: asks TCC; returns at once.
            unsafe { CGRequestListenEventAccess() };
        }
        _ => {}
    }
}

/// Opens the System Settings pane for `kind`.
pub fn open_settings(kind: &str) {
    let pane = match kind {
        "accessibility" => "Privacy_Accessibility",
        "inputMonitoring" => "Privacy_ListenEvent",
        "microphone" => "Privacy_Microphone",
        _ => return,
    };
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{pane}");
    if let Err(error) = std::process::Command::new("open").arg(url).status() {
        tracing::warn!(%error, "cannot open System Settings");
    }
}

/// What is missing for dictation, for the menu bar icon, if anything.
pub fn missing(hotkey_active: bool) -> Option<&'static str> {
    if !hotkey_active {
        Some("allow Input Monitoring")
    } else if !can_type() {
        Some("allow Accessibility to paste")
    } else {
        None
    }
}
