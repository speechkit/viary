//! An X11 session (or XWayland, where it applies): the active window and
//! synthetic keys through XTest.

use std::time::{Duration, Instant};

use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{
            AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, KEY_PRESS_EVENT,
            KEY_RELEASE_EVENT, Keycode, Window,
        },
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
};

pub const CONTROL_L: u32 = 0xffe3;
pub const KEY_V: u32 = 0x76;
pub const KEY_Z: u32 = 0x7a;

fn connect() -> Result<(RustConnection, Window), String> {
    let (conn, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let root = conn.setup().roots[screen].root;
    Ok((conn, root))
}

fn atom(conn: &RustConnection, name: &str) -> Result<u32, String> {
    Ok(conn
        .intern_atom(false, name.as_bytes())
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .atom)
}

/// Keysyms that would turn Ctrl+V into another shortcut while held: Shift,
/// Alt, Super, Meta, and AltGr (ISO_Level3_Shift). The talk shortcut's Alt
/// can still be down when its Space comes up. Ctrl is not here: Ctrl+V is
/// still Ctrl+V.
const IN_THE_WAY: [u32; 9] = [0xffe1, 0xffe2, 0xffe9, 0xffea, 0xffeb, 0xffec, 0xffe7, 0xffe8, 0xfe03];

/// How long to wait for those keys to come up before giving up.
const WAIT_FOR_KEYS: Duration = Duration::from_millis(1500);

/// The keyboard mapping: which keysyms each keycode types.
struct KeyMap {
    min: Keycode,
    per: usize,
    keysyms: Vec<u32>,
}

impl KeyMap {
    fn read(conn: &RustConnection) -> Result<Self, String> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            min,
            per: usize::from(map.keysyms_per_keycode).max(1),
            keysyms: map.keysyms,
        })
    }

    /// The keycode that types `keysym` in the current layout.
    fn code(&self, keysym: u32) -> Option<Keycode> {
        self.keysyms
            .chunks(self.per)
            .position(|syms| syms.contains(&keysym))
            .and_then(|i| u8::try_from(i).ok())
            .map(|i| self.min + i)
    }
}

/// Waits until none of [`IN_THE_WAY`] is held; false if one still is.
fn keys_released(conn: &RustConnection, map: &KeyMap) -> Result<bool, String> {
    let codes: Vec<Keycode> = IN_THE_WAY.iter().filter_map(|&k| map.code(k)).collect();
    let deadline = Instant::now() + WAIT_FOR_KEYS;
    loop {
        let keys = conn
            .query_keymap()
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .keys;
        let held = codes
            .iter()
            .any(|&code| keys[usize::from(code / 8)] & (1 << (code % 8)) != 0);
        if !held {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Presses `keysyms` in order, then releases them in reverse: a shortcut.
/// Waits first for keys that would change the shortcut to come up.
pub fn press(keysyms: &[u32]) -> Result<(), String> {
    let (conn, root) = connect()?;
    let map = KeyMap::read(&conn)?;
    let codes = keysyms
        .iter()
        .map(|&k| map.code(k).ok_or_else(|| format!("no key types keysym {k:#x}")))
        .collect::<Result<Vec<_>, _>>()?;
    if !keys_released(&conn, &map)? {
        return Err("a modifier key is still held".into());
    }
    let send = |kind: u8, code: Keycode| {
        conn.xtest_fake_input(kind, code, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .map(drop)
            .map_err(|e| e.to_string())
    };
    for &code in &codes {
        send(KEY_PRESS_EVENT, code)?;
    }
    for &code in codes.iter().rev() {
        send(KEY_RELEASE_EVENT, code)?;
    }
    conn.flush().map_err(|e| e.to_string())
}

/// The active window on `conn`, if any.
fn active_on(conn: &RustConnection, root: Window, active: u32) -> Option<Window> {
    conn.get_property(false, root, active, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
        .filter(|&w| w != 0)
}

/// The active window, its process, and its class ("firefox").
pub fn active_window() -> Option<(Window, i32, String)> {
    let (conn, root) = connect().ok()?;
    let active = atom(&conn, "_NET_ACTIVE_WINDOW").ok()?;
    let window = active_on(&conn, root, active)?;
    let pid_atom = atom(&conn, "_NET_WM_PID").ok()?;
    let pid = conn
        .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().and_then(|mut v| v.next()))
        .and_then(|p| i32::try_from(p).ok())
        .unwrap_or(0);
    // WM_CLASS holds the instance and the class, NUL-separated.
    let class = conn
        .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| {
            let parts: Vec<&[u8]> = r.value.split(|&b| b == 0).filter(|p| !p.is_empty()).collect();
            parts.last().map(|p| String::from_utf8_lossy(p).into_owned()).unwrap_or_default()
        })
        .unwrap_or_default();
    Some((window, pid, class))
}

/// What [`activate`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    /// The window was active already.
    Already,
    /// The window manager brought it forward.
    Moved,
    /// It is not active.
    Failed,
}

/// Asks the window manager to raise and focus `window`, and waits up to
/// `timeout` for it to: the window manager moves the focus in its own
/// time.
pub fn activate(window: Window, timeout: Duration) -> Activation {
    let Ok((conn, root)) = connect() else {
        return Activation::Failed;
    };
    let Ok(active) = atom(&conn, "_NET_ACTIVE_WINDOW") else {
        return Activation::Failed;
    };
    if active_on(&conn, root, active) == Some(window) {
        return Activation::Already;
    }
    // Source 2: a pager, which window managers obey without focus-stealing
    // checks.
    let event = ClientMessageEvent::new(32, window, active, [2, x11rb::CURRENT_TIME, 0, 0, 0]);
    let sent = conn
        .send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )
        .is_ok()
        && conn.flush().is_ok();
    if !sent {
        return Activation::Failed;
    }
    let deadline = Instant::now() + timeout;
    loop {
        if active_on(&conn, root, active) == Some(window) {
            return Activation::Moved;
        }
        if Instant::now() >= deadline {
            return Activation::Failed;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}
