//! Whether the focused UI element can take typed text, from UI Automation.
//!
//! As on macOS, Viary only reports "not editable" when the app names a
//! control that takes no text, such as a button or a list. Anything
//! unknown gets the paste: pasting into nothing is harmless, while missing
//! a real field is not.

use std::{sync::mpsc, time::Duration};

use windows::Win32::{
    System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
    UI::Accessibility::{
        CUIAutomation, IUIAutomation, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
        UIA_CONTROLTYPE_ID, UIA_ImageControlTypeId, UIA_ListControlTypeId,
        UIA_ListItemControlTypeId, UIA_MenuBarControlTypeId, UIA_MenuItemControlTypeId,
        UIA_RadioButtonControlTypeId, UIA_SliderControlTypeId, UIA_TableControlTypeId,
        UIA_TabItemControlTypeId, UIA_TextControlTypeId, UIA_TreeControlTypeId,
        UIA_TreeItemControlTypeId,
    },
};

/// Where dictated text can go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// A text field, or something Viary cannot rule out as one.
    Editable,
    /// A control that takes no text.
    NotEditable,
}

/// Control types that never take typed text. Panes, groups, documents,
/// and custom controls are not here: browsers and terminals report those
/// for their text areas.
const NOT_TEXT: &[UIA_CONTROLTYPE_ID] = &[
    UIA_ButtonControlTypeId,
    UIA_CheckBoxControlTypeId,
    UIA_ImageControlTypeId,
    UIA_ListControlTypeId,
    UIA_ListItemControlTypeId,
    UIA_MenuBarControlTypeId,
    UIA_MenuItemControlTypeId,
    UIA_RadioButtonControlTypeId,
    UIA_SliderControlTypeId,
    UIA_TabItemControlTypeId,
    UIA_TableControlTypeId,
    UIA_TextControlTypeId,
    UIA_TreeControlTypeId,
    UIA_TreeItemControlTypeId,
];

/// An app that does not answer UI Automation must not hold up the paste.
const ASK_FOR: Duration = Duration::from_millis(300);

/// Classifies the focused element. UI Automation follows the keyboard
/// focus across apps, so `pid` is only logged.
pub fn focused(pid: i32) -> Focus {
    let (sender, received) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("viary-focus".into())
        .spawn(move || {
            let _ = sender.send(control_type());
        });
    if spawned.is_err() {
        return Focus::Editable;
    }
    match received.recv_timeout(ASK_FOR) {
        Ok(Some(kind)) => {
            tracing::debug!(pid, kind = kind.0, "focused element");
            if NOT_TEXT.contains(&kind) {
                Focus::NotEditable
            } else {
                Focus::Editable
            }
        }
        Ok(None) => {
            tracing::debug!(pid, "no focused element reported; pasting anyway");
            Focus::Editable
        }
        Err(_) => {
            tracing::debug!(pid, "UI Automation did not answer; pasting anyway");
            Focus::Editable
        }
    }
}

fn control_type() -> Option<UIA_CONTROLTYPE_ID> {
    // SAFETY: COM is initialized for this thread and released after the
    // automation object is dropped.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
        let kind = (|| {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let element = automation.GetFocusedElement().ok()?;
            element.CurrentControlType().ok()
        })();
        CoUninitialize();
        kind
    }
}
