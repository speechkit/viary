//! Hold-to-talk: a low-level keyboard hook that reports when the chosen key
//! goes down and up.
//!
//! The hook only watches; every key still reaches the app in front. One
//! catch: releasing Alt or Win alone opens the app's menu bar or the Start
//! menu. So while the hotkey is held, Viary types an unassigned key once,
//! which Windows counts as "used with another key", as AutoHotkey does.
//!
//! The hook procedure only records the key and hands any event to a thread
//! of its own: Windows removes a low-level hook, without telling anyone,
//! when it is slow to answer.

use std::{
    sync::{
        Arc, Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc,
    },
    time::Duration,
};

use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
            KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
            VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
            LLKHF_INJECTED, MSG, SetTimer, SetWindowsHookExW, TranslateMessage,
            UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
            WM_TIMER,
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

/// How often the hook is installed afresh, while no key is held, in case
/// Windows removed it (LowLevelHooksTimeout), in milliseconds.
const REINSTALL_EVERY: u32 = 60_000;

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

    /// Forgets the modifiers Windows no longer reports down, other than
    /// `current`, the key being reported. The hook misses key-ups that go
    /// to the lock screen (Win+L), the secure desktop (Ctrl+Alt+Del), or an
    /// app running as administrator; without this, a Win left "held" would
    /// make a lone Ctrl start a dictation.
    fn forget_released(&mut self, current: VIRTUAL_KEY, is_down: impl Fn(VIRTUAL_KEY) -> bool) {
        let [left_ctrl, right_ctrl] = &mut self.ctrl;
        let [left_win, right_win] = &mut self.win;
        for (vk, held) in [
            (VK_RMENU, &mut self.right_alt),
            (VK_LCONTROL, left_ctrl),
            (VK_RCONTROL, right_ctrl),
            (VK_LWIN, left_win),
            (VK_RWIN, right_win),
        ] {
            if vk != current && *held && !is_down(vk) {
                *held = false;
            }
        }
    }

    /// Whether any key the hotkeys use is down.
    fn any(&self) -> bool {
        self.hotkey || self.right_alt || self.ctrl.contains(&true) || self.win.contains(&true)
    }
}

/// Whether Windows reports `vk` down. Inside the hook this is the state
/// before the key being reported.
fn is_down(vk: VIRTUAL_KEY) -> bool {
    // SAFETY: a plain query.
    unsafe { GetAsyncKeyState(i32::from(vk.0)) < 0 }
}

struct Hook {
    hotkey: Arc<AtomicU8>,
    /// To the thread that acts on events, off the hook procedure.
    events: mpsc::Sender<HotkeyEvent>,
    held: Mutex<Held>,
}

impl Hook {
    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
    let mut held = hook.held();
    held.forget_released(vk, is_down);
    let modifier = held.track(vk, down);
    let now = held.of(key);
    let event = if now != held.hotkey {
        held.hotkey = now;
        Some(if now { HotkeyEvent::Down } else { HotkeyEvent::Up })
    } else if !modifier && down && now {
        // Key repeat sends more downs; any non-modifier key down counts.
        Some(HotkeyEvent::OtherKey)
    } else {
        None
    };
    drop(held);
    if let Some(event) = event {
        let _ = hook.events.send(event);
    }
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
        let (events, received) = mpsc::channel();
        let installed = HOOK.set(Hook {
            hotkey: listener.hotkey.clone(),
            events,
            held: Mutex::default(),
        });
        if installed.is_err() {
            tracing::error!("the hotkey listener is already running");
            return listener;
        }
        let acting = std::thread::Builder::new()
            .name("viary-hotkey-events".into())
            .spawn(move || {
                for event in received {
                    if event == HotkeyEvent::Down {
                        tap(MASK_KEY);
                    }
                    on_event(event);
                }
            });
        if let Err(error) = acting {
            tracing::error!(%error, "cannot start the hotkey listener");
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

fn install() -> Option<HHOOK> {
    // SAFETY: a low-level hook needs no module handle; it is removed when
    // replaced or when the message loop ends.
    match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(procedure), None, 0) } {
        Ok(hook) => Some(hook),
        Err(error) => {
            tracing::warn!(%error, "cannot install the keyboard hook");
            None
        }
    }
}

/// Whether the user holds Ctrl, as the hook saw it: Viary's own
/// keystrokes are not counted, as Windows' key state would count them.
pub fn ctrl_held() -> bool {
    HOOK.get().is_some_and(|hook| hook.held().ctrl.contains(&true))
}

/// Whether a key the hotkeys use is down, so the hook must stay as it is.
fn keys_held() -> bool {
    HOOK.get().is_some_and(|hook| hook.held().any())
}

fn run(active: &AtomicBool) {
    let mut hook = loop {
        if let Some(hook) = install() {
            break hook;
        }
        std::thread::sleep(Duration::from_secs(2));
    };
    active.store(true, Ordering::SeqCst);
    tracing::info!("hotkey listener installed");
    // SAFETY: the standard message loop for this thread, with a thread
    // timer (no window) that posts WM_TIMER to it.
    unsafe {
        let _ = SetTimer(None, 0, REINSTALL_EVERY, None);
        let mut message = MSG::default();
        while GetMessageW(&raw mut message, None, 0, 0).as_bool() {
            if message.message == WM_TIMER {
                // Windows gives no sign that it removed the hook: install
                // it again, then remove the old one if it is still there.
                if !keys_held()
                    && let Some(fresh) = install()
                {
                    let _ = UnhookWindowsHookEx(hook);
                    hook = fresh;
                }
                continue;
            }
            let _ = TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
    active.store(false, Ordering::SeqCst);
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

    #[test]
    fn a_missed_key_up_is_forgotten() {
        let mut held = Held::default();
        held.track(VK_LWIN, true);
        // Win+L: the key-up went to the lock screen. Then Ctrl alone.
        held.forget_released(VK_LCONTROL, |_| false);
        held.track(VK_LCONTROL, true);
        assert!(!held.of(Key::CtrlWin));
        assert!(held.any());
    }

    #[test]
    fn keys_still_down_are_kept() {
        let mut held = Held::default();
        held.track(VK_LWIN, true);
        held.forget_released(VK_LCONTROL, |vk| vk == VK_LWIN);
        held.track(VK_LCONTROL, true);
        assert!(held.of(Key::CtrlWin));
        // The key being reported is not asked about: Windows has not
        // recorded it yet.
        held.forget_released(VK_LWIN, |_| false);
        assert!(held.win[0]);
    }
}
