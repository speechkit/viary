//! Hold-to-talk: a low-level keyboard hook that reports when the chosen key
//! goes down and up.
//!
//! The hook only watches; every key still reaches the app in front. One
//! catch: releasing Alt or Win alone opens the app's menu bar or the Start
//! menu. So while the hotkey is held, Viary types an unassigned key once,
//! which Windows counts as "used with another key", as AutoHotkey does.

use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};

use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
            SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
            VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT,
            LLKHF_INJECTED, MSG, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
            WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    },
};

use crate::settings::Hotkey;

/// What the listener reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Down,
    Up,
    /// Another key was pressed while the hotkey was held: probably a
    /// shortcut such as Ctrl+Win+D, not a dictation.
    OtherKey,
}

/// An unassigned virtual key: typed to keep a lone Alt or Win release
/// from opening a menu.
const MASK_KEY: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    RightAlt,
    CtrlWin,
}

fn encode(hotkey: Hotkey) -> u8 {
    match hotkey {
        Hotkey::CtrlWin => 1,
        // Keys from other systems, in settings copied from them.
        _ => 0,
    }
}

fn decode(code: u8) -> Key {
    if code == 1 { Key::CtrlWin } else { Key::RightAlt }
}

/// Which modifiers are down, as the hook has seen them.
#[derive(Default)]
struct Held {
    right_alt: bool,
    ctrl: [bool; 2],
    win: [bool; 2],
    /// Whether the hotkey counts as down.
    hotkey: bool,
}

impl Held {
    fn of(&self, key: Key) -> bool {
        match key {
            Key::RightAlt => self.right_alt,
            Key::CtrlWin => self.ctrl.contains(&true) && self.win.contains(&true),
        }
    }

    /// Records a modifier going down or up; false for other keys.
    fn track(&mut self, vk: VIRTUAL_KEY, down: bool) -> bool {
        match vk {
            VK_RMENU => self.right_alt = down,
            VK_LCONTROL => self.ctrl[0] = down,
            VK_RCONTROL => self.ctrl[1] = down,
            VK_LWIN => self.win[0] = down,
            VK_RWIN => self.win[1] = down,
            // Left Alt, Shift, and the generic codes: modifiers, but they
            // do not end a dictation as "another key".
            VK_LMENU | VK_MENU | VK_LSHIFT | VK_RSHIFT | VK_SHIFT | VK_CONTROL => {}
            _ => return false,
        }
        true
    }
}

struct Hook {
    hotkey: Arc<AtomicU8>,
    on_event: Box<dyn Fn(HotkeyEvent) + Send + Sync>,
    held: std::sync::Mutex<Held>,
}

/// The hook procedure is a plain function, so the listener it reports to
/// lives here. There is one per process.
static HOOK: OnceLock<Hook> = OnceLock::new();

unsafe extern "system" fn procedure(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && let Some(hook) = HOOK.get()
    {
        // SAFETY: for WH_KEYBOARD_LL, lparam points at a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Viary's own keystrokes (the mask key, Ctrl+V) are not the user's.
        if info.flags.0 & LLKHF_INJECTED.0 == 0 {
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let up = matches!(wparam.0 as u32, WM_KEYUP | WM_SYSKEYUP);
            if down || up {
                observe(hook, VIRTUAL_KEY(info.vkCode as u16), down);
            }
        }
    }
    // SAFETY: passes the event on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn observe(hook: &Hook, vk: VIRTUAL_KEY, down: bool) {
    let key = decode(hook.hotkey.load(Ordering::Relaxed));
    let mut held = hook.held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !held.track(vk, down) {
        // Key repeat sends more downs; any non-modifier key down counts.
        if down && held.hotkey {
            drop(held);
            (hook.on_event)(HotkeyEvent::OtherKey);
        }
        return;
    }
    let now = held.of(key);
    if now == held.hotkey {
        return;
    }
    held.hotkey = now;
    drop(held);
    if now {
        // Typed after the hook returns, so the order of keys is kept.
        std::thread::spawn(|| tap(MASK_KEY));
    }
    (hook.on_event)(if now { HotkeyEvent::Down } else { HotkeyEvent::Up });
}

fn key_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
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

fn tap(vk: VIRTUAL_KEY) {
    let inputs = [key_input(vk, KEYBD_EVENT_FLAGS(0)), key_input(vk, KEYEVENTF_KEYUP)];
    // SAFETY: the inputs are fully initialized keyboard events.
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

/// The running listener. Change the key with [`set_hotkey`](Self::set_hotkey).
#[derive(Clone)]
pub struct HotkeyListener {
    hotkey: Arc<AtomicU8>,
    active: Arc<AtomicBool>,
}

impl HotkeyListener {
    /// Starts listening on a thread of its own, which runs the message
    /// loop the hook needs.
    pub fn spawn(hotkey: Hotkey, on_event: impl Fn(HotkeyEvent) + Send + Sync + 'static) -> Self {
        let listener = Self {
            hotkey: Arc::new(AtomicU8::new(encode(hotkey))),
            active: Arc::new(AtomicBool::new(false)),
        };
        let installed = HOOK.set(Hook {
            hotkey: listener.hotkey.clone(),
            on_event: Box::new(on_event),
            held: std::sync::Mutex::default(),
        });
        if installed.is_err() {
            tracing::error!("the hotkey listener is already running");
            return listener;
        }
        let active = listener.active.clone();
        let spawned = std::thread::Builder::new()
            .name("viary-hotkey".into())
            .spawn(move || run(&active));
        if let Err(error) = spawned {
            tracing::error!(%error, "cannot start the hotkey listener");
        }
        listener
    }

    pub fn set_hotkey(&self, hotkey: Hotkey) {
        self.hotkey.store(encode(hotkey), Ordering::SeqCst);
    }

    /// Whether the hook is installed.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

fn run(active: &AtomicBool) {
    loop {
        // SAFETY: a low-level hook needs no module handle; it is removed
        // below when the message loop ends.
        match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(procedure), None, 0) } {
            Ok(hook) => {
                active.store(true, Ordering::SeqCst);
                tracing::info!("hotkey listener installed");
                let mut message = MSG::default();
                // SAFETY: the standard message loop for this thread.
                unsafe {
                    while GetMessageW(&raw mut message, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&raw const message);
                        DispatchMessageW(&raw const message);
                    }
                    let _ = UnhookWindowsHookEx(hook);
                }
                active.store(false, Ordering::SeqCst);
            }
            Err(error) => tracing::warn!(%error, "cannot install the keyboard hook"),
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_win_needs_both_keys() {
        let mut held = Held::default();
        held.track(VK_LCONTROL, true);
        assert!(!held.of(Key::CtrlWin));
        held.track(VK_RWIN, true);
        assert!(held.of(Key::CtrlWin));
        held.track(VK_LCONTROL, false);
        assert!(!held.of(Key::CtrlWin));
    }

    #[test]
    fn letters_are_not_modifiers() {
        let mut held = Held::default();
        assert!(!held.track(VIRTUAL_KEY(u16::from(b'A')), true));
        assert!(held.track(VK_RMENU, true));
        assert!(held.of(Key::RightAlt));
    }
}
