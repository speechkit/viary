//! Viary's GNOME Shell extension (`gnome-extension/` in the repository):
//! installing it, and asking it to paste, name the focused app, and show
//! the pill. Its pill buttons come back as a `PillAction` signal.
//!
//! GNOME on Wayland loads a newly installed extension only at the next
//! login, so "installed" and "active" are told apart.

use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use ashpd::zbus::{self, MatchRule, MessageStream, message::Type};
use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Manager};

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

/// Whether the extension answers, asked at most every few seconds: the
/// pill and every paste ask.
static ACTIVE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

pub fn active() -> bool {
    let mut cached = lock(&ACTIVE);
    if let Some((at, active)) = *cached
        && at.elapsed() < Duration::from_secs(3)
    {
        return active;
    }
    let active = tauri::async_runtime::block_on(has_owner()).unwrap_or(false);
    *cached = Some((Instant::now(), active));
    active
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
    *lock(&ACTIVE) = None;
    Ok(())
}

fn add_to_enabled() -> Result<(), String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.shell", "enabled-extensions"])
        .output()
        .map_err(|e| e.to_string())?;
    let current = String::from_utf8_lossy(&out.stdout);
    if current.contains(UUID) {
        return Ok(());
    }
    let mut list: Vec<String> = current
        .trim()
        .trim_start_matches("@as")
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect();
    list.push(format!("'{UUID}'"));
    let set = std::process::Command::new("gsettings")
        .args(["set", "org.gnome.shell", "enabled-extensions", &format!("[{}]", list.join(", "))])
        .status()
        .map_err(|e| e.to_string())?;
    if set.success() { Ok(()) } else { Err("cannot enable the extension".into()) }
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

/// Shows `view` in the shell's pill.
pub fn show_pill(view: &PillView) {
    let Ok(json) = serde_json::to_string(view) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        if let Err(error) = call("ShowPill", &(json,)).await {
            tracing::warn!(%error, "cannot show the pill in GNOME Shell");
        }
    });
}

pub fn level(level: f32) {
    tauri::async_runtime::spawn(async move {
        let _ = call("SetLevel", &(f64::from(level),)).await;
    });
}

pub fn partial(token: u64, text: String) {
    tauri::async_runtime::spawn(async move {
        let _ = call("SetPartial", &(token, text)).await;
    });
}

/// Acts on the pill's buttons, for as long as Viary runs.
pub fn listen(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = actions(&app).await {
            tracing::warn!(%error, "cannot listen to the GNOME Shell pill");
        }
    });
}

async fn actions(app: &AppHandle) -> zbus::Result<()> {
    let connection = super::session_bus().await?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(BUS)?
        .member("PillAction")?
        .build();
    let mut stream = MessageStream::for_match_rule(rule, &connection, None).await?;
    while let Some(message) = stream.next().await {
        let Ok(message) = message else { continue };
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
