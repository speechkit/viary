//! Whether the focused UI element can take typed text, from the
//! Accessibility API.
//!
//! Many apps answer the Accessibility API poorly: terminals draw their own
//! text, and Chromium and Electron apps expose nothing until asked. So
//! Viary only reports "not editable" when the app names a control that
//! takes no text, such as a button or a file list. Anything unknown gets
//! the paste: pasting into nothing is harmless, while missing a real field
//! is not.

use std::ffi::c_void;

use core_foundation::{
    base::{CFRelease, CFTypeRef, TCFType},
    boolean::CFBoolean,
    string::{CFString, CFStringRef},
};

type AXUIElementRef = *const c_void;
type AXError = i32;

const AX_SUCCESS: AXError = 0;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementIsAttributeSettable(
        element: AXUIElementRef,
        attribute: CFStringRef,
        settable: *mut u8,
    ) -> AXError;
}

/// Where dictated text can go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// A text field, or something Viary cannot rule out as one.
    Editable,
    /// A control that takes no text.
    NotEditable,
}

/// Roles that never take typed text. Windows and groups are not here:
/// web views and terminals report those for their text areas.
const NOT_TEXT: &[&str] = &[
    "AXButton",
    "AXList",
    "AXTable",
    "AXOutline",
    "AXStaticText",
    "AXImage",
    "AXMenuBar",
    "AXMenuItem",
    "AXCheckBox",
    "AXRadioButton",
    "AXPopUpButton",
    "AXSlider",
];

struct Owned(CFTypeRef);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: we own this reference, from a Copy or Create call.
            unsafe { CFRelease(self.0) };
        }
    }
}

fn copy(element: AXUIElementRef, name: &str) -> Result<Owned, AXError> {
    let attribute = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: `element` is a live AXUIElement and `value` is an out pointer.
    let error = unsafe {
        AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &raw mut value)
    };
    if error == AX_SUCCESS && !value.is_null() {
        Ok(Owned(value))
    } else {
        Err(error)
    }
}

fn settable(element: AXUIElementRef, name: &str) -> bool {
    let attribute = CFString::new(name);
    let mut settable = 0_u8;
    // SAFETY: `element` is a live AXUIElement and `settable` an out pointer.
    let error = unsafe {
        AXUIElementIsAttributeSettable(element, attribute.as_concrete_TypeRef(), &raw mut settable)
    };
    error == AX_SUCCESS && settable != 0
}

/// The focused element of app `pid`, or of whichever app has focus.
fn focused_element(pid: i32) -> Result<Owned, AXError> {
    if pid > 0 {
        // SAFETY: returns a new reference that `app` releases.
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        // Chromium and Electron build their accessibility tree only when
        // asked; this is what screen readers set. Other apps ignore it.
        let manual = CFString::new("AXManualAccessibility");
        // SAFETY: `app` is live; the value is a CFBoolean.
        unsafe {
            AXUIElementSetAttributeValue(
                app.0,
                manual.as_concrete_TypeRef(),
                CFBoolean::true_value().as_CFTypeRef(),
            );
        }
        if let Ok(element) = copy(app.0, "AXFocusedUIElement") {
            return Ok(element);
        }
    }
    // SAFETY: returns a new reference that `system` releases.
    let system = Owned(unsafe { AXUIElementCreateSystemWide() });
    copy(system.0, "AXFocusedUIElement")
}

/// Classifies the focused element of app `pid`.
pub fn focused(pid: i32) -> Focus {
    let element = match focused_element(pid) {
        Ok(element) => element,
        Err(error) => {
            tracing::debug!(pid, error, "no focused element reported; pasting anyway");
            return Focus::Editable;
        }
    };
    if settable(element.0, "AXSelectedTextRange") {
        tracing::debug!(pid, "focused element takes text");
        return Focus::Editable;
    }
    let Ok(role) = copy(element.0, "AXRole") else {
        return Focus::Editable;
    };
    // SAFETY: AXRole is always a CFString; `get_rule` retains it for the wrapper.
    let role = unsafe { CFString::wrap_under_get_rule(role.0.cast()) }.to_string();
    tracing::debug!(pid, %role, "focused element");
    if NOT_TEXT.contains(&role.as_str()) {
        Focus::NotEditable
    } else {
        Focus::Editable
    }
}

#[cfg(test)]
mod tests {
    /// Prints what the frontmost app reports. Run by hand with
    /// `cargo test focus -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the live desktop"]
    fn frontmost_app_focus() {
        let app = crate::macos::apps::frontmost().unwrap();
        let trusted = crate::macos::permissions::accessibility();
        let focus = super::focused(app.pid);
        println!(
            "app={} pid={} trusted={trusted} focus={focus:?}",
            app.name, app.pid
        );
    }
}
