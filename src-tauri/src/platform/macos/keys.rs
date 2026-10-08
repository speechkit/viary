//! Synthetic ⌘V and ⌘Z, posted to whichever app has focus. Posting events
//! to other apps needs the Accessibility permission.
//!
//! Apps match shortcuts by the character a key types, not by its position,
//! so the key for "v" or "z" is looked up in the current keyboard layout:
//! on AZERTY the ANSI Z key types "w", and ⌘ on it would close the window.

use std::{ffi::c_void, sync::mpsc, time::Duration};

use core_foundation::{
    base::{CFRelease, CFTypeRef},
    data::{CFDataGetBytePtr, CFDataRef},
    string::CFStringRef,
};
use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use tauri::AppHandle;

// Virtual key codes on the ANSI layout, used when the layout cannot be read.
const ANSI_V: CGKeyCode = 9;
const ANSI_Z: CGKeyCode = 6;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> CFTypeRef;
    fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> CFTypeRef;
    fn TISGetInputSourceProperty(source: CFTypeRef, key: CFStringRef) -> CFTypeRef;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        virtual_key: u16,
        action: u16,
        modifier_state: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        chars: *mut u16,
    ) -> i32;
}

const KEY_ACTION_DOWN: u16 = 0;
const NO_DEAD_KEYS: u32 = 1;
/// `cmdKey >> 8`: translate as with ⌘ held, so layouts that switch to
/// QWERTY for shortcuts ("Dvorak - QWERTY ⌘") are honoured.
const COMMAND_MODIFIER: u32 = 1;

/// The key that types `target` with ⌘ in the layout of `source`.
///
/// # Safety
///
/// `source` is a live TISInputSource; call on the main thread.
unsafe fn key_in(source: CFTypeRef, target: char) -> Option<CGKeyCode> {
    if source.is_null() {
        return None;
    }
    // SAFETY: `source` is live; the property is a CFData owned by it.
    let data = unsafe { TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) };
    if data.is_null() {
        return None; // Input methods such as Pinyin have no key layout.
    }
    // SAFETY: the layout data outlives this call, owned by `source`.
    let layout = unsafe { CFDataGetBytePtr(data as CFDataRef) }.cast::<c_void>();
    // SAFETY: reads a global the system keeps current.
    let keyboard = u32::from(unsafe { LMGetKbdType() });
    (0..128_u16).find(|&code| {
        let mut dead = 0_u32;
        let mut chars = [0_u16; 4];
        let mut len = 0_usize;
        // SAFETY: `layout` is a valid 'uchr' table and the buffers fit.
        let status = unsafe {
            UCKeyTranslate(
                layout,
                code,
                KEY_ACTION_DOWN,
                COMMAND_MODIFIER,
                keyboard,
                NO_DEAD_KEYS,
                &raw mut dead,
                chars.len(),
                &raw mut len,
                chars.as_mut_ptr(),
            )
        };
        status == 0
            && len == 1
            && char::from_u32(u32::from(chars[0]))
                .is_some_and(|c| c.eq_ignore_ascii_case(&target))
    })
}

/// Looks `target` up in the current layout, then in the ASCII-capable one
/// the system uses for shortcuts under non-Latin layouts. Main thread only.
fn lookup(target: char) -> Option<CGKeyCode> {
    // SAFETY: each copy returns a +1 reference released below.
    unsafe {
        for copy in [
            TISCopyCurrentKeyboardLayoutInputSource as unsafe extern "C" fn() -> CFTypeRef,
            TISCopyCurrentASCIICapableKeyboardLayoutInputSource,
        ] {
            let source = copy();
            let found = key_in(source, target);
            if !source.is_null() {
                CFRelease(source);
            }
            if found.is_some() {
                return found;
            }
        }
    }
    None
}

/// The key code for `target`, read on the main thread (the input source
/// APIs require it), or `fallback` if that fails.
fn key_for(app: &AppHandle, target: char, fallback: CGKeyCode) -> CGKeyCode {
    let (sender, received) = mpsc::channel();
    let dispatched = app.run_on_main_thread(move || {
        let _ = sender.send(lookup(target));
    });
    if dispatched.is_err() {
        return fallback;
    }
    match received.recv_timeout(Duration::from_secs(1)) {
        Ok(Some(code)) => code,
        _ => {
            tracing::warn!(%target, "cannot find the key in the keyboard layout; using the ANSI key");
            fallback
        }
    }
}

fn command(key: CGKeyCode) -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|()| "cannot create an event source".to_owned())?;
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), key, down)
            .map_err(|()| "cannot create a key event".to_owned())?;
        // Only ⌘: a hotkey still reported as held must not change the shortcut.
        event.set_flags(CGEventFlags::CGEventFlagCommand);
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}

/// ⌘V. Call off the main thread.
pub fn paste(app: &AppHandle) -> Result<(), String> {
    command(key_for(app, 'v', ANSI_V))
}

/// ⌘Z. Call off the main thread.
pub fn undo(app: &AppHandle) -> Result<(), String> {
    command(key_for(app, 'z', ANSI_Z))
}

#[cfg(test)]
mod tests {
    /// Prints the keys found in the current layout. Run by hand with
    /// `cargo test keys -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the live keyboard layout"]
    fn current_layout_keys() {
        println!("v={:?} z={:?}", super::lookup('v'), super::lookup('z'));
    }
}
