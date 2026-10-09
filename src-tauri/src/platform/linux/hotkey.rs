//! The talk shortcut, Ctrl+Alt+Space, which GNOME hands to Viary:
//!
//! - Wayland with the GlobalShortcuts portal (GNOME 48 and later): GNOME
//!   asks the user once, then reports the shortcut going down and up.
//! - Wayland without it (Ubuntu 24.04, GNOME 46): a GNOME custom shortcut
//!   runs `viary --toggle`, which tells the running Viary through a socket.
//!   GNOME reports no release there, so talking is a toggle.
//! - X11: the global shortcut plugin grabs the key, down and up.

use std::{
    io::Read,
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use serde::Serialize;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use super::{app, is_wayland};
use crate::{lock, settings::Hotkey};

/// What the listener reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Down,
    Up,
    /// Another key while the shortcut is held: GNOME does not report it.
    #[expect(dead_code, reason = "GNOME does not report other keys")]
    OtherKey,
    /// A press where GNOME reports no release: start, or stop and insert.
    Toggle,
}

const TALK: &str = "talk";
/// The shortcut as the XDG shortcuts specification writes it.
const TRIGGER: &str = "CTRL+ALT+space";
/// As GNOME's custom shortcuts write it.
const GNOME_BINDING: &str = "<Control><Alt>space";
const CUSTOM: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/viary/";

/// How the shortcut reaches Viary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Down and up: hold to talk.
    Hold,
    /// Presses only: press to start, press again to insert.
    Toggle,
    /// Not known yet.
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub mode: Mode,
    /// GNOME hands the shortcut to Viary.
    pub bound: bool,
}

type OnEvent = Arc<dyn Fn(HotkeyEvent) + Send + Sync>;

struct Listener {
    on_event: OnEvent,
    status: Mutex<Status>,
    /// The portal and its session, for binding later from the setup window.
    portal: tokio::sync::Mutex<Option<(GlobalShortcuts, ashpd::desktop::Session<GlobalShortcuts>)>>,
}

static LISTENER: OnceLock<Arc<Listener>> = OnceLock::new();

pub fn status() -> Status {
    LISTENER.get().map(|l| *lock(&l.status)).unwrap_or_default()
}

fn set_status(listener: &Listener, status: Status) {
    *lock(&listener.status) = status;
    if let Some(app) = app() {
        crate::ui::refresh(app);
    }
}

#[derive(Clone)]
pub struct HotkeyListener;

impl HotkeyListener {
    pub fn spawn(_hotkey: Hotkey, on_event: impl Fn(HotkeyEvent) + Send + Sync + 'static) -> Self {
        let listener = Arc::new(Listener {
            on_event: Arc::new(on_event),
            status: Mutex::default(),
            portal: tokio::sync::Mutex::new(None),
        });
        if LISTENER.set(listener.clone()).is_err() {
            tracing::error!("the hotkey listener is already running");
            return Self;
        }
        if is_wayland() {
            tauri::async_runtime::spawn(watch_portal(listener));
        } else {
            grab_x11(&listener);
        }
        Self
    }

    /// The shortcut is GNOME's to change, in GNOME Settings.
    pub fn set_hotkey(&self, _hotkey: Hotkey) {}

    pub fn is_active(&self) -> bool {
        status().bound
    }
}

fn grab_x11(listener: &Arc<Listener>) {
    let Some(app) = app() else {
        return;
    };
    let on_event = listener.on_event.clone();
    let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
    let grabbed = app.global_shortcut().on_shortcut(shortcut, move |_, _, event| {
        on_event(if event.state() == ShortcutState::Pressed {
            HotkeyEvent::Down
        } else {
            HotkeyEvent::Up
        });
    });
    let bound = match grabbed {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, "another app holds Ctrl+Alt+Space");
            false
        }
    };
    set_status(listener, Status { mode: Mode::Hold, bound });
}

/// Opens a GlobalShortcuts session and reports the shortcut while it lives.
/// Without the portal, falls back to the custom shortcut and its socket.
async fn watch_portal(listener: Arc<Listener>) {
    match open_portal(&listener).await {
        Ok(()) => {}
        Err(error) => {
            tracing::info!(%error, "no GlobalShortcuts portal; using a GNOME custom shortcut");
            set_status(&listener, Status { mode: Mode::Toggle, bound: custom_bound() });
            listen_socket(listener);
        }
    }
}

async fn open_portal(listener: &Arc<Listener>) -> ashpd::Result<()> {
    let proxy = GlobalShortcuts::new().await?;
    let session = proxy.create_session(Default::default()).await?;
    let listed = proxy.list_shortcuts(&session, Default::default()).await?.response()?;
    let bound = listed.shortcuts().iter().any(|s| s.id() == TALK);
    let mut activated = std::pin::pin!(proxy.receive_activated().await?);
    let mut deactivated = std::pin::pin!(proxy.receive_deactivated().await?);
    *listener.portal.lock().await = Some((proxy, session));
    set_status(listener, Status { mode: Mode::Hold, bound });

    let on_event = listener.on_event.clone();
    let downs = async {
        while let Some(event) = activated.next().await {
            if event.shortcut_id() == TALK {
                on_event(HotkeyEvent::Down);
            }
        }
    };
    let ups = async {
        while let Some(event) = deactivated.next().await {
            if event.shortcut_id() == TALK {
                on_event(HotkeyEvent::Up);
            }
        }
    };
    futures_util::future::join(downs, ups).await;
    Ok(())
}

/// Asks GNOME for the shortcut, from the setup window.
pub fn bind() -> Result<(), String> {
    let listener = LISTENER.get().ok_or("the hotkey listener is not running")?.clone();
    match status().mode {
        Mode::Toggle => {
            add_custom()?;
            set_status(&listener, Status { mode: Mode::Toggle, bound: custom_bound() });
            Ok(())
        }
        Mode::Hold if !is_wayland() => Ok(()),
        _ => tauri::async_runtime::block_on(async {
            let portal = listener.portal.lock().await;
            let (proxy, session) = portal.as_ref().ok_or("GNOME has not answered yet")?;
            let shortcut = NewShortcut::new(TALK, "Talk: hold to speak, release to type")
                .preferred_trigger(TRIGGER);
            let bound = proxy
                .bind_shortcuts(session, &[shortcut], None, Default::default())
                .await
                .and_then(|request| request.response())
                .map_err(|e| e.to_string())?;
            let bound = bound.shortcuts().iter().any(|s| s.id() == TALK);
            drop(portal);
            set_status(&listener, Status { mode: Mode::Hold, bound });
            Ok::<(), String>(())
        }),
    }
}

// ---------------------------------------------------------------------------
// The custom shortcut and its socket (GNOME 46)

/// Where the running Viary listens for `viary --toggle`.
fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("viary.sock")
}

/// `viary --toggle`: tells the running Viary, and returns whether one was
/// there to tell. Called before the app starts.
pub fn send_toggle() -> bool {
    UnixStream::connect(socket_path())
        .and_then(|mut stream| std::io::Write::write_all(&mut stream, b"toggle"))
        .is_ok()
}

fn listen_socket(listener: Arc<Listener>) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let socket = match UnixListener::bind(&path) {
        Ok(socket) => socket,
        Err(error) => {
            tracing::warn!(%error, "cannot listen for the custom shortcut");
            return;
        }
    };
    let _ = std::thread::Builder::new()
        .name("viary-toggle".into())
        .spawn(move || {
            for stream in socket.incoming().flatten() {
                let mut word = String::new();
                if (&stream).take(16).read_to_string(&mut word).is_ok() && word == "toggle" {
                    (listener.on_event)(HotkeyEvent::Toggle);
                }
            }
        });
}

/// The command the custom shortcut runs: this executable, or the AppImage
/// it was started from.
fn command() -> String {
    let exe = std::env::var("APPIMAGE")
        .ok()
        .or_else(|| std::env::current_exe().ok().map(|p| p.display().to_string()))
        .unwrap_or_else(|| "viary".into());
    format!("'{exe}' --toggle")
}

fn gsettings(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("gsettings")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";

fn custom_list() -> Vec<String> {
    gsettings(&["get", MEDIA_KEYS, "custom-keybindings"])
        .unwrap_or_default()
        .trim_start_matches("@as")
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|p| p.trim().trim_matches('\'').to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

fn custom_bound() -> bool {
    custom_list().iter().any(|p| p == CUSTOM)
}

fn add_custom() -> Result<(), String> {
    let schema = format!("{MEDIA_KEYS}.custom-keybinding:{CUSTOM}");
    gsettings(&["set", &schema, "name", "Viary: talk"])?;
    gsettings(&["set", &schema, "command", &command()])?;
    gsettings(&["set", &schema, "binding", GNOME_BINDING])?;
    let mut list = custom_list();
    if !list.iter().any(|p| p == CUSTOM) {
        list.push(CUSTOM.to_owned());
    }
    let value = format!(
        "[{}]",
        list.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(", ")
    );
    gsettings(&["set", MEDIA_KEYS, "custom-keybindings", &value])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_command_quotes_the_path() {
        assert!(super::command().ends_with("' --toggle"));
    }
}
