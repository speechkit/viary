//! An X11 session (or XWayland, where it applies): the active window and
//! synthetic keys through XTest.

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

/// The keycode that types `keysym` in the current layout.
fn keycode(conn: &RustConnection, keysym: u32) -> Result<Keycode, String> {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let map = conn
        .get_keyboard_mapping(min, max - min + 1)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?;
    let per = usize::from(map.keysyms_per_keycode).max(1);
    map.keysyms
        .chunks(per)
        .position(|syms| syms.contains(&keysym))
        .and_then(|i| u8::try_from(i).ok())
        .map(|i| min + i)
        .ok_or_else(|| format!("no key types keysym {keysym:#x}"))
}

/// Presses `keysyms` in order, then releases them in reverse: a shortcut.
pub fn press(keysyms: &[u32]) -> Result<(), String> {
    let (conn, root) = connect()?;
    let codes = keysyms
        .iter()
        .map(|&k| keycode(&conn, k))
        .collect::<Result<Vec<_>, _>>()?;
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

/// The active window, its process, and its class ("firefox").
pub fn active_window() -> Option<(Window, i32, String)> {
    let (conn, root) = connect().ok()?;
    let active = atom(&conn, "_NET_ACTIVE_WINDOW").ok()?;
    let window = conn
        .get_property(false, root, active, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
        .filter(|&w| w != 0)?;
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

/// Asks the window manager to raise and focus `window`.
pub fn activate(window: Window) -> bool {
    let Ok((conn, root)) = connect() else {
        return false;
    };
    let Ok(active) = atom(&conn, "_NET_ACTIVE_WINDOW") else {
        return false;
    };
    // Source 2: a pager, which window managers obey without focus-stealing
    // checks.
    let event = ClientMessageEvent::new(32, window, active, [2, x11rb::CURRENT_TIME, 0, 0, 0]);
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    )
    .is_ok()
        && conn.flush().is_ok()
}
