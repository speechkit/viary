//! The talk shortcut, Ctrl+Alt+Space, reported going down and up:
//!
//! - Wayland: Viary's GNOME Shell extension grabs it once it runs, and
//!   reports it as its `Talk` signal (see [`super::extension`]).
//! - X11: the global shortcut plugin grabs it.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use super::{app, is_wayland};
use crate::settings::Hotkey;

/// What the listener reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Down,
    Up,
    /// Another key while the shortcut is held: GNOME does not report it.
    #[expect(dead_code, reason = "GNOME does not report other keys")]
    OtherKey,
}

/// The talk shortcut, as GNOME writes accelerators.
pub const ACCELERATOR: &str = "<Control><Alt>space";

type OnEvent = Arc<dyn Fn(HotkeyEvent) + Send + Sync>;

static ON_EVENT: OnceLock<OnEvent> = OnceLock::new();

/// Whether Viary hears the shortcut: grabbed on X11, or by the extension.
static BOUND: AtomicBool = AtomicBool::new(false);

/// Reports the shortcut going down or up.
pub(super) fn report(event: HotkeyEvent) {
    if let Some(on_event) = ON_EVENT.get() {
        on_event(event);
    }
}

/// Records whether Viary hears the shortcut, and shows it if that changed.
pub(super) fn set_bound(bound: bool) {
    if BOUND.swap(bound, Ordering::SeqCst) != bound
        && let Some(app) = app()
    {
        crate::ui::refresh(app);
    }
}

#[derive(Clone)]
pub struct HotkeyListener;

impl HotkeyListener {
    pub fn spawn(_hotkey: Hotkey, on_event: impl Fn(HotkeyEvent) + Send + Sync + 'static) -> Self {
        if ON_EVENT.set(Arc::new(on_event)).is_err() {
            tracing::error!("the hotkey listener is already running");
        } else if !is_wayland() {
            grab_x11();
        }
        Self
    }

    /// The shortcut is fixed: Ctrl+Alt+Space.
    pub fn set_hotkey(&self, _hotkey: Hotkey) {}

    pub fn is_active(&self) -> bool {
        bound()
    }
}

/// Whether Viary hears the shortcut.
pub fn bound() -> bool {
    BOUND.load(Ordering::SeqCst)
}

fn grab_x11() {
    let Some(app) = app() else {
        return;
    };
    let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
    let grabbed = app.global_shortcut().on_shortcut(shortcut, |_, _, event| {
        report(if event.state() == ShortcutState::Pressed {
            HotkeyEvent::Down
        } else {
            HotkeyEvent::Up
        });
    });
    if let Err(error) = &grabbed {
        tracing::warn!(%error, "another app holds Ctrl+Alt+Space");
    }
    set_bound(grabbed.is_ok());
}
