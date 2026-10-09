//! Viary's GNOME Shell extension (`gnome-extension/` in the repository):
//! installing it, and asking it to paste, name the focused app, and show
//! the pill. Its pill buttons come back as a `PillAction` signal.
//!
//! GNOME on Wayland loads a newly installed extension only at the next
//! login, so "installed" and "active" are told apart.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

use ashpd::zbus::{self, MatchRule, MessageStream, message::Type};
use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;

use crate::{
    App,
    dictation::{Msg, PillAction, PillView},
    lock,
};

const UUID: &str = "viary@viary.app";
const BUS: &str = "app.viary.Shell";
const PATH: &str = "/app/viary/Shell";

const FILES: [(&str, &str); 3] = [
    ("metadata.json", include_str!("../../../../gnome-extension/viary@viary.app/metadata.json")),
    ("extension.js", include_str!("../../../../gnome-extension/viary@viary.app/extension.js")),
    ("stylesheet.css", include_str!("../../../../gnome-extension/viary@viary.app/stylesheet.css")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Missing,
    /// On disk, waiting for the next login.
    Installed,
    /// Running in the shell.
    Active,
}

fn dir() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))?;
    Some(data.join("gnome-shell/extensions").join(UUID))
}

/// Whether the extension answers: asked once, then kept current by
/// [`watch_owner`] as the shell's name comes and goes. The pill's level and
/// every snapshot ask, so asking must not wait on the bus.
static ACTIVE: AtomicU8 = AtomicU8::new(UNKNOWN);
const UNKNOWN: u8 = 0;
const NO: u8 = 1;
const YES: u8 = 2;

/// Records whether the extension answers; true if that changed.
fn set_active(active: bool) -> bool {
    let state = if active { YES } else { NO };
    ACTIVE.swap(state, Ordering::SeqCst) != state
}

pub fn active() -> bool {
    match ACTIVE.load(Ordering::SeqCst) {
        // Before the watch has answered, at startup: ask now.
        UNKNOWN => {
            let active = tauri::async_runtime::block_on(has_owner()).unwrap_or(false);
            set_active(active);
            active
        }
        state => state == YES,
    }
}

async fn has_owner() -> zbus::Result<bool> {
    let connection = super::session_bus().await?;
    let reply = connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &(BUS,),
        )
        .await?;
    reply.body().deserialize()
}

pub fn status() -> Status {
    if active() {
        Status::Active
    } else if dir().is_some_and(|d| d.join("extension.js").is_file()) {
        Status::Installed
    } else {
        Status::Missing
    }
}

/// Writes the extension into the user's extensions folder and enables it.
/// GNOME on Wayland runs it from the next login.
pub fn install() -> Result<(), String> {
    let dir = dir().ok_or("no home folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, text) in FILES {
        std::fs::write(dir.join(name), text).map_err(|e| format!("cannot write {name}: {e}"))?;
    }
    let enabled = std::process::Command::new("gnome-extensions")
        .args(["enable", UUID])
        .output()
        .map_err(|e| format!("cannot run gnome-extensions: {e}"))?;
    if !enabled.status.success() {
        // An extension GNOME has not loaded yet cannot be enabled by name
        // on some versions; add it to the list GNOME reads at login.
        add_to_enabled()?;
    }
    Ok(())
}

fn add_to_enabled() -> Result<(), String> {
    let mut list = super::gsettings_list("org.gnome.shell", "enabled-extensions")?;
    if list.iter().any(|uuid| uuid == UUID) {
        return Ok(());
    }
    list.push(UUID.to_owned());
    super::gsettings_set_list("org.gnome.shell", "enabled-extensions", &list)
        .map_err(|e| format!("cannot enable the extension: {e}"))
}

async fn call<B>(method: &str, body: &B) -> zbus::Result<zbus::Message>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let connection = super::session_bus().await?;
    connection.call_method(Some(BUS), PATH, Some(BUS), method, body).await
}

/// Ctrl+V, typed by the shell.
pub fn paste() -> Result<(), String> {
    tauri::async_runtime::block_on(call("Paste", &())).map(drop).map_err(|e| e.to_string())
}

/// Ctrl+Z, typed by the shell.
pub fn undo() -> Result<(), String> {
    tauri::async_runtime::block_on(call("Undo", &())).map(drop).map_err(|e| e.to_string())
}

/// The focused window's process and app name.
pub fn focused_app() -> Option<(i32, String)> {
    tauri::async_runtime::block_on(async {
        let reply = call("FocusedApp", &()).await.ok()?;
        reply.body().deserialize::<(i32, String)>().ok()
    })
}

/// What the shell's pill has yet to be sent.
#[derive(Default)]
struct Pending {
    /// Pill views, every one in order: each is a state the user may need
    /// to see, and they must not arrive out of order.
    views: Vec<String>,
    /// Only the newest level and live text: older ones are stale by the
    /// time a slow call returns, and would hold up the views behind them.
    level: Option<f64>,
    partial: Option<(u64, String)>,
}

static PENDING: Mutex<Pending> = Mutex::new(Pending {
    views: Vec::new(),
    level: None,
    partial: None,
});

/// Queues a change and wakes the one task that sends them, in turn.
fn send(queue: impl FnOnce(&mut Pending)) {
    static WAKE: OnceLock<Arc<Notify>> = OnceLock::new();
    let wake = WAKE.get_or_init(|| {
        let wake = Arc::new(Notify::new());
        let waiting = wake.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                waiting.notified().await;
                let Pending { views, level, partial } = std::mem::take(&mut *lock(&PENDING));
                for json in views {
                    if let Err(error) = call("ShowPill", &(json,)).await {
                        tracing::warn!(%error, "cannot show the pill in GNOME Shell");
                    }
                }
                if let Some((token, text)) = partial {
                    let _ = call("SetPartial", &(token, text)).await;
                }
                if let Some(level) = level {
                    let _ = call("SetLevel", &(level,)).await;
                }
            }
        });
        wake
    });
    queue(&mut lock(&PENDING));
    // Stored if the task is busy sending: it goes round again.
    wake.notify_one();
}

/// Shows `view` in the shell's pill.
pub fn show_pill(view: &PillView) {
    if let Ok(json) = serde_json::to_string(view) {
        send(|pending| pending.views.push(json));
    }
}

pub fn level(level: f32) {
    send(|pending| pending.level = Some(f64::from(level)));
}

pub fn partial(token: u64, text: String) {
    send(|pending| pending.partial = Some((token, text)));
}

/// Acts on the pill's buttons, for as long as Viary runs.
/// Also follows whether the extension is running.
pub fn listen(app: &AppHandle) {
    let app = app.clone();
    let watching = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = actions(&app).await {
            tracing::warn!(%error, "cannot listen to the GNOME Shell pill");
        }
    });
    tauri::async_runtime::spawn(async move {
        if let Err(error) = watch_owner(&watching).await {
            tracing::warn!(%error, "cannot follow the GNOME Shell extension");
        }
    });
}

/// Keeps [`ACTIVE`] current: the extension's name gains an owner when the
/// shell turns it on, and loses it when the shell turns it off or restarts.
async fn watch_owner(app: &AppHandle) -> zbus::Result<()> {
    const DBUS: &str = "org.freedesktop.DBus";
    let connection = super::session_bus().await?;
    // Subscribed before asking, so no change falls in between.
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(DBUS)?
        .interface(DBUS)?
        .member("NameOwnerChanged")?
        .add_arg(BUS)?
        .build();
    let mut stream = MessageStream::for_match_rule(rule, &connection, None).await?;
    if set_active(has_owner().await?) {
        crate::ui::refresh(app);
    }
    while let Some(message) = stream.next().await {
        let Ok(message) = message else { continue };
        // Only the bus itself may say who owns a name; no client can own
        // `org.freedesktop.DBus`.
        if message.header().sender().is_none_or(|sender| sender.as_str() != DBUS) {
            continue;
        }
        let Ok((name, _old, new)) = message.body().deserialize::<(String, String, String)>() else {
            continue;
        };
        if name == BUS && set_active(!new.is_empty()) {
            crate::ui::refresh(app);
        }
    }
    Ok(())
}

async fn actions(app: &AppHandle) -> zbus::Result<()> {
    let connection = super::session_bus().await?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path(PATH)?
        .interface(BUS)?
        .member("PillAction")?
        .build();
    let mut stream = MessageStream::for_match_rule(rule, &connection, None).await?;
    while let Some(message) = stream.next().await {
        let Ok(message) = message else { continue };
        if !super::sent_by(&message, BUS).await {
            tracing::warn!("a PillAction signal not from Viary's extension; ignored");
            continue;
        }
        let Ok(action) = message.body().deserialize::<String>() else {
            continue;
        };
        // The same names the web pill sends: "undo", "useRaw", "stop"...
        match serde_json::from_value::<PillAction>(serde_json::Value::String(action)) {
            Ok(action) => app.state::<App>().send(Msg::Pill(action)),
            Err(error) => tracing::warn!(%error, "unknown pill action from GNOME Shell"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Run by `ci/gnome-wayland.sh`, inside a headless GNOME Shell with the
    /// extension installed: Viary finds it and it answers.
    #[test]
    #[ignore = "needs GNOME Shell running Viary's extension"]
    fn gnome_extension_answers() {
        assert_eq!(super::status(), super::Status::Active);
        // The status check connected, owning Viary's client name: the
        // extension answers this process as it answers Viary.
        tauri::async_runtime::block_on(super::call(
            "ShowPill",
            &(r#"{"kind":"hint","text":"CI"}"#,),
        ))
        .expect("ShowPill");
        tauri::async_runtime::block_on(super::call("SetLevel", &(0.1_f64,))).expect("SetLevel");
        assert!(super::focused_app().is_some(), "FocusedApp did not answer");
        super::paste().expect("Paste");
        super::undo().expect("Undo");
    }
}
