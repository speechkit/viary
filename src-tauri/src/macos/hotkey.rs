//! Hold-to-talk: a listen-only event tap that reports when the chosen
//! modifier goes down and up.
//!
//! Modifiers such as fn cannot be registered as global shortcuts, so Viary
//! watches `flagsChanged` events instead. A listen-only tap needs the Input
//! Monitoring permission; until it is granted, creating the tap fails and
//! the listener tries again every two seconds.

use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};

use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult,
};

use crate::settings::Hotkey;

/// What the listener reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Down,
    Up,
    /// Another key was pressed while the hotkey was held: probably a
    /// shortcut such as fn+arrow, not a dictation.
    OtherKey,
}

// NX_SECONDARYFNMASK, NX_DEVICERALTKEYMASK, NX_DEVICERCMDKEYMASK.
const FN: u64 = 0x0080_0000;
const RIGHT_OPTION: u64 = 0x0000_0040;
const RIGHT_COMMAND: u64 = 0x0000_0010;

fn encode(hotkey: Hotkey) -> u8 {
    match hotkey {
        Hotkey::Fn => 0,
        Hotkey::RightOption => 1,
        Hotkey::RightCommand => 2,
    }
}

fn mask(code: u8) -> u64 {
    match code {
        1 => RIGHT_OPTION,
        2 => RIGHT_COMMAND,
        _ => FN,
    }
}

/// The running listener. Change the key with [`set_hotkey`](Self::set_hotkey).
#[derive(Clone)]
pub struct HotkeyListener {
    hotkey: Arc<AtomicU8>,
    active: Arc<AtomicBool>,
}

impl HotkeyListener {
    /// Starts listening on a thread of its own.
    pub fn spawn(hotkey: Hotkey, on_event: impl Fn(HotkeyEvent) + Send + 'static) -> Self {
        let listener = Self {
            hotkey: Arc::new(AtomicU8::new(encode(hotkey))),
            active: Arc::new(AtomicBool::new(false)),
        };
        let shared = listener.clone();
        let spawned = std::thread::Builder::new()
            .name("viary-hotkey".into())
            .spawn(move || shared.run(&on_event));
        if let Err(error) = spawned {
            tracing::error!(%error, "cannot start the hotkey listener");
        }
        listener
    }

    pub fn set_hotkey(&self, hotkey: Hotkey) {
        self.hotkey.store(encode(hotkey), Ordering::SeqCst);
    }

    /// Whether the tap is installed, which means Input Monitoring is granted.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    fn run(&self, on_event: &dyn Fn(HotkeyEvent)) {
        let mut warned = false;
        loop {
            let held = Cell::new(false);
            let installed = CGEventTap::with_enabled(
                CGEventTapLocation::Session,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                vec![CGEventType::FlagsChanged, CGEventType::KeyDown],
                |_, kind, event| {
                    match kind {
                        CGEventType::FlagsChanged => {
                            let down = event.get_flags().bits()
                                & mask(self.hotkey.load(Ordering::Relaxed))
                                != 0;
                            if down != held.get() {
                                held.set(down);
                                on_event(if down {
                                    HotkeyEvent::Down
                                } else {
                                    HotkeyEvent::Up
                                });
                            }
                        }
                        CGEventType::KeyDown if held.get() => on_event(HotkeyEvent::OtherKey),
                        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
                            // macOS turned the tap off. The release may never
                            // be seen, so end a dictation in progress, then
                            // leave the run loop to install a fresh tap.
                            tracing::warn!(?kind, "hotkey listener disabled; reinstalling");
                            if held.replace(false) {
                                on_event(HotkeyEvent::Up);
                            }
                            CFRunLoop::get_current().stop();
                        }
                        _ => {}
                    }
                    CallbackResult::Keep
                },
                || {
                    self.active.store(true, Ordering::SeqCst);
                    tracing::info!("hotkey listener installed");
                    CFRunLoop::run_current();
                },
            );
            self.active.store(false, Ordering::SeqCst);
            if installed.is_err() && !warned {
                warned = true;
                tracing::warn!("cannot watch the keyboard yet; waiting for Input Monitoring");
            }
            // A tap macOS disabled comes back quickly; a missing permission
            // is polled.
            let pause = if installed.is_ok() {
                Duration::from_millis(200)
            } else {
                Duration::from_secs(2)
            };
            std::thread::sleep(pause);
        }
    }
}
