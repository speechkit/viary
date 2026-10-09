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
    time::{Duration, Instant},
};

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use serde::Serialize;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use super::{app, gsettings, gsettings_list, gsettings_set_list, is_wayland};
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

/// When GNOME last reported the shortcut released, or pressed in toggle
/// mode, which reports no release.
static RELEASED: Mutex<Option<Instant>> = Mutex::new(None);

/// How long ago GNOME last reported the shortcut released (or pressed, in
/// toggle mode); `None` if it has not.
pub fn since_release() -> Option<Duration> {
    lock(&RELEASED).map(|at| at.elapsed())
}

/// The talk shortcut's keys as GNOME has them, for the UI to name: the
/// user can change them in GNOME Settings. `None` until GNOME says, or on
/// X11, where Viary grabs Ctrl+Alt+Space itself.
static TRIGGER: Mutex<Option<String>> = Mutex::new(None);

pub fn trigger() -> Option<String> {
    lock(&TRIGGER).clone()
}

/// Records the shortcut's keys; true if they changed.
fn set_trigger(trigger: Option<&str>) -> bool {
    let label = trigger.map(label).filter(|label| !label.is_empty());
    let mut current = lock(&TRIGGER);
    let changed = *current != label;
    *current = label;
    changed
}

/// The talk shortcut's trigger among the portal's `shortcuts`.
fn talk_trigger(shortcuts: &[ashpd::desktop::global_shortcuts::Shortcut]) -> Option<&str> {
    shortcuts.iter().find(|s| s.id() == TALK).map(|s| s.trigger_description())
}

/// A shortcut as people read it: GNOME's "<Control><Alt>space" is
/// "Ctrl+Alt+Space". Text already in that form is kept.
fn label(trigger: &str) -> String {
    let mut rest = trigger.trim();
    if !rest.starts_with('<') {
        return rest.to_owned();
    }
    let mut keys = Vec::new();
    while let Some(after) = rest.strip_prefix('<')
        && let Some((modifier, tail)) = after.split_once('>')
    {
        keys.push(match modifier {
            "Control" | "Primary" | "Ctrl" => "Ctrl".to_owned(),
            "Super" => "Super".to_owned(),
            other => other.to_owned(),
        });
        rest = tail;
    }
    let mut chars = rest.chars();
    if let Some(first) = chars.next() {
        keys.push(first.to_uppercase().chain(chars).collect());
    }
    keys.join("+")
}

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
            on_event: Arc::new(move |event: HotkeyEvent| {
                if matches!(event, HotkeyEvent::Up | HotkeyEvent::Toggle) {
                    *lock(&RELEASED) = Some(Instant::now());
                }
                on_event(event);
            }),
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
            read_custom_trigger();
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
    set_trigger(talk_trigger(listed.shortcuts()));
    let mut activated = std::pin::pin!(proxy.receive_activated().await?);
    let mut deactivated = std::pin::pin!(proxy.receive_deactivated().await?);
    let mut changed = std::pin::pin!(proxy.receive_shortcuts_changed().await?);
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
    // Changed in GNOME Settings: name the new keys.
    let renamed = async {
        while let Some(event) = changed.next().await {
            if set_trigger(talk_trigger(event.shortcuts()))
                && let Some(app) = app()
            {
                crate::ui::refresh(app);
            }
        }
    };
    futures_util::future::join3(downs, ups, renamed).await;
    Ok(())
}

/// Asks GNOME for the shortcut, from the setup window.
pub fn bind() -> Result<(), String> {
    let listener = LISTENER.get().ok_or("the hotkey listener is not running")?.clone();
    match status().mode {
        Mode::Toggle => {
            add_custom()?;
            read_custom_trigger();
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
            set_trigger(talk_trigger(bound.shortcuts()));
            let bound = bound.shortcuts().iter().any(|s| s.id() == TALK);
            drop(portal);
            set_status(&listener, Status { mode: Mode::Hold, bound });
            Ok::<(), String>(())
        }),
    }
}

// ---------------------------------------------------------------------------
// The custom shortcut and its socket (GNOME 46)

/// How long a connection may take to say "toggle": the socket is read one
/// connection at a time, so one left open must not hold up the next.
const READ_FOR: Duration = Duration::from_millis(500);

/// Where the running Viary listens for `viary --toggle`: the user's runtime
/// directory, or else a folder of the user's own in the shared temporary
/// one, where other users could otherwise reach or squat the socket.
fn socket_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Some(PathBuf::from(dir).join("viary.sock"));
    }
    private_temp_dir().map(|dir| dir.join("viary.sock"))
}

fn private_temp_dir() -> Option<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    // /proc/self belongs to this process's user.
    let uid = std::fs::metadata("/proc/self").ok()?.uid();
    let dir = std::env::temp_dir().join(format!("viary-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            tracing::warn!(%error, "cannot make a folder for the shortcut socket");
            return None;
        }
    }
    // Made by someone else first, or opened up since: not ours to use.
    let meta = std::fs::symlink_metadata(&dir).ok()?;
    (meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0).then_some(dir)
}

/// `viary --toggle` handed over as a second launch, where the socket is
/// not listening: the same as through it.
pub fn toggle() {
    if let Some(listener) = LISTENER.get() {
        toggled(listener);
    }
}

/// The custom shortcut was pressed. Its keys are read again after: GNOME
/// does not say when the user changes them, and a press with new keys is
/// the first sign.
fn toggled(listener: &Listener) {
    (listener.on_event)(HotkeyEvent::Toggle);
    if read_custom_trigger()
        && let Some(app) = app()
    {
        crate::ui::refresh(app);
    }
}

/// `viary --toggle`: tells the running Viary, and returns whether one was
/// there to tell. Called before the app starts.
pub fn send_toggle() -> bool {
    let Some(path) = socket_path() else {
        return false;
    };
    UnixStream::connect(path)
        .and_then(|mut stream| std::io::Write::write_all(&mut stream, b"toggle"))
        .is_ok()
}

fn listen_socket(listener: Arc<Listener>) {
    let Some(path) = socket_path() else {
        tracing::warn!("no private folder for the custom shortcut's socket");
        return;
    };
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
                if stream.set_read_timeout(Some(READ_FOR)).is_err() {
                    continue;
                }
                let mut word = String::new();
                if (&stream).take(16).read_to_string(&mut word).is_ok() && word == "toggle" {
                    toggled(&listener);
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

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";

fn custom_list() -> Vec<String> {
    gsettings_list(MEDIA_KEYS, "custom-keybindings").unwrap_or_default()
}

/// Reads the custom shortcut's keys; true if they changed.
fn read_custom_trigger() -> bool {
    let schema = format!("{MEDIA_KEYS}.custom-keybinding:{CUSTOM}");
    let binding = custom_bound()
        .then(|| gsettings(&["get", &schema, "binding"]).ok())
        .flatten();
    set_trigger(binding.as_deref().map(|b| b.trim_matches('\'')))
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
    gsettings_set_list(MEDIA_KEYS, "custom-keybindings", &list)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_gnome_accelerator_reads_as_keys() {
        assert_eq!(super::label("<Control><Alt>space"), "Ctrl+Alt+Space");
        assert_eq!(super::label("<Super>F9"), "Super+F9");
        assert_eq!(super::label("Ctrl+Alt+Space"), "Ctrl+Alt+Space");
        assert_eq!(super::label(""), "");
    }

    #[test]
    fn the_command_quotes_the_path() {
        assert!(super::command().ends_with("' --toggle"));
    }
}
