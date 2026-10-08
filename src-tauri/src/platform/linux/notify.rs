//! Results as GNOME notifications, where Wayland keeps the pill from
//! floating over other windows: "Inserted" with Undo and Use raw, "Copied
//! to clipboard", a failure with Retry. Each replaces the last one, so they
//! do not pile up in the message tray.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU32, Ordering},
};

use ashpd::zbus::{self, MatchRule, MessageStream, message::Type, zvariant::Value};
use futures_util::StreamExt;
use tauri::{AppHandle, Manager};

use crate::{
    App,
    dictation::{Msg, PillAction, PillView},
    ui,
};

const BUS: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// The notification on screen, which the next one replaces.
static SHOWN: AtomicU32 = AtomicU32::new(0);

/// Shows `view` as a notification, if it is one the user needs to see.
pub fn pill(view: &PillView) {
    let (summary, body, actions): (String, String, Vec<(&str, &str)>) = match view {
        PillView::Inserted { label, can_raw } => {
            let mut actions = vec![("undo", "Undo")];
            if *can_raw {
                actions.push(("useRaw", "Use raw"));
            }
            (label.clone(), String::new(), actions)
        }
        PillView::Copied { label, hint } => {
            let actions = if super::permissions::can_type() {
                vec![]
            } else {
                vec![("allow", "Allow typing…")]
            };
            (label.clone(), hint.clone(), actions)
        }
        PillView::Failed { message, retryable, .. } => {
            let actions = if *retryable { vec![("retry", "Retry")] } else { vec![] };
            (message.clone(), "The audio is kept in History.".into(), actions)
        }
        PillView::Hint { text } => (text.clone(), String::new(), vec![]),
        _ => return,
    };
    let actions: Vec<(String, String)> = actions.into_iter().map(|(i, l)| (i.into(), l.into())).collect();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = show(&summary, &body, &actions).await {
            tracing::warn!(%error, "cannot show a notification");
        }
    });
}

async fn show(summary: &str, body: &str, actions: &[(String, String)]) -> zbus::Result<()> {
    let connection = super::session_bus().await?;
    let actions: Vec<&str> = actions.iter().flat_map(|(id, label)| [id.as_str(), label.as_str()]).collect();
    let hints: HashMap<&str, Value<'_>> = HashMap::from([("desktop-entry", Value::from("viary"))]);
    let reply = connection
        .call_method(
            Some(BUS),
            PATH,
            Some(BUS),
            "Notify",
            &(
                "Viary",
                SHOWN.load(Ordering::SeqCst),
                "audio-input-microphone-symbolic",
                summary,
                body,
                actions,
                hints,
                5000_i32,
            ),
        )
        .await?;
    let id: u32 = reply.body().deserialize()?;
    SHOWN.store(id, Ordering::SeqCst);
    Ok(())
}

/// Acts on the buttons of Viary's notifications, for as long as it runs.
pub fn listen(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = actions(&app).await {
            tracing::warn!(%error, "cannot listen to notification buttons");
        }
    });
}

async fn actions(app: &AppHandle) -> zbus::Result<()> {
    let connection = super::session_bus().await?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(BUS)?
        .member("ActionInvoked")?
        .build();
    let mut stream = MessageStream::for_match_rule(rule, &connection, None).await?;
    while let Some(message) = stream.next().await {
        let Ok(message) = message else { continue };
        let Ok((id, action)) = message.body().deserialize::<(u32, String)>() else {
            continue;
        };
        if id != SHOWN.load(Ordering::SeqCst) {
            continue;
        }
        let state = app.state::<App>();
        match action.as_str() {
            "undo" => state.send(Msg::Pill(PillAction::Undo)),
            "useRaw" => state.send(Msg::Pill(PillAction::UseRaw)),
            "retry" => state.send(Msg::Pill(PillAction::Retry)),
            "allow" => ui::open_main(app, "settings"),
            _ => {}
        }
    }
    Ok(())
}
