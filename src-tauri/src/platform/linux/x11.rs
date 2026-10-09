//! An X11 session (or XWayland, where it applies): the active window,
//! synthetic keys through XTest, and what the clipboard offers.

use std::time::{Duration, Instant};

use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xproto::{
            AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask,
            KEY_PRESS_EVENT, KEY_RELEASE_EVENT, Keycode, Window, WindowClass,
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

    /// Every keycode that types `keysym`: a layout often puts a modifier
    /// on more than one key (ISO_Level3_Shift on 92 and 108).
    fn codes(&self, keysym: u32) -> impl Iterator<Item = Keycode> + '_ {
        self.keysyms
            .chunks(self.per)
            .enumerate()
            .filter(move |(_, syms)| syms.contains(&keysym))
            .filter_map(|(i, _)| u8::try_from(i).ok())
            .map(|i| self.min + i)
    }

    /// The keycode that types `keysym` in the current layout.
    fn code(&self, keysym: u32) -> Option<Keycode> {
        self.codes(keysym).next()
    }
}

/// Which keys are down, a bit per keycode.
fn keys_down(conn: &RustConnection) -> Result<[u8; 32], String> {
    Ok(conn
        .query_keymap()
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .keys)
}

fn is_down(keys: &[u8; 32], code: Keycode) -> bool {
    keys[usize::from(code / 8)] & (1 << (code % 8)) != 0
}

/// Waits until none of [`IN_THE_WAY`] is held; false if one still is.
fn keys_released(conn: &RustConnection, map: &KeyMap) -> Result<bool, String> {
    let codes: Vec<Keycode> = IN_THE_WAY.iter().flat_map(|&k| map.codes(k)).collect();
    let deadline = Instant::now() + WAIT_FOR_KEYS;
    loop {
        let keys = keys_down(conn)?;
        if !codes.iter().any(|&code| is_down(&keys, code)) {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Presses `keysyms` in order, then releases them in reverse: a shortcut.
/// Waits first for keys that would change the shortcut to come up. A
/// modifier the user still holds (the talk shortcut's Ctrl) is left as it
/// is: releasing it would take it from their next shortcut.
pub fn press(keysyms: &[u32]) -> Result<(), String> {
    let (conn, root) = connect()?;
    let map = KeyMap::read(&conn)?;
    if !keys_released(&conn, &map)? {
        return Err("a modifier key is still held".into());
    }
    let keys = keys_down(&conn)?;
    let mut codes = Vec::with_capacity(keysyms.len());
    for (i, &keysym) in keysyms.iter().enumerate() {
        let modifier = i + 1 < keysyms.len();
        if modifier && map.codes(keysym).any(|code| is_down(&keys, code)) {
            continue;
        }
        codes.push(map.code(keysym).ok_or_else(|| format!("no key types keysym {keysym:#x}"))?);
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

/// How long the clipboard's owner has to say what it offers.
const ASK_OWNER_FOR: Duration = Duration::from_millis(300);

/// The formats the clipboard offers ("image/png", "text/html",
/// "UTF8_STRING"...), asked of its owner once; empty when nothing is on it.
/// `None` when the owner did not answer in time.
pub fn clipboard_targets() -> Option<Vec<String>> {
    let (conn, root) = connect().ok()?;
    let window = conn.generate_id().ok()?;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        window,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    )
    .ok()?;
    let clipboard = atom(&conn, "CLIPBOARD").ok()?;
    let targets = atom(&conn, "TARGETS").ok()?;
    let property = atom(&conn, "VIARY_TARGETS").ok()?;
    conn.convert_selection(window, clipboard, targets, property, x11rb::CURRENT_TIME)
        .ok()?;
    conn.flush().ok()?;
    let deadline = Instant::now() + ASK_OWNER_FOR;
    let answered = loop {
        match conn.poll_for_event().ok()? {
            Some(Event::SelectionNotify(event)) if event.requestor == window => break event,
            Some(_) => continue,
            None if Instant::now() >= deadline => return None,
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    // No owner, or one that would not say.
    if answered.property == x11rb::NONE {
        return Some(Vec::new());
    }
    let atoms: Vec<u32> = conn
        .get_property(true, window, property, AtomEnum::ATOM, 0, 1024)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .collect();
    // Every name asked for at once, then the answers read.
    let asked: Vec<_> = atoms.iter().filter_map(|&a| conn.get_atom_name(a).ok()).collect();
    Some(
        asked
            .into_iter()
            .filter_map(|cookie| cookie.reply().ok())
            .map(|reply| String::from_utf8_lossy(&reply.name).into_owned())
            .collect(),
    )
}
