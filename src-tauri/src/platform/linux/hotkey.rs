//! The talk shortcut: not connected yet.

use crate::settings::Hotkey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(dead_code, reason = "the GNOME shortcut that sends these is not connected yet")]
pub enum HotkeyEvent {
    Down,
    Up,
    OtherKey,
}

#[derive(Clone)]
pub struct HotkeyListener;

impl HotkeyListener {
    pub fn spawn(_hotkey: Hotkey, _on_event: impl Fn(HotkeyEvent) + Send + Sync + 'static) -> Self {
        tracing::warn!("the talk shortcut is not available on Linux yet");
        Self
    }

    pub fn set_hotkey(&self, _hotkey: Hotkey) {}

    pub fn is_active(&self) -> bool {
        false
    }
}
