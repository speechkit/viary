//! Results as GNOME notifications, where Wayland keeps the pill from
//! floating over other windows (LinuxIndicator artboard, 02 and 03):
//!
//! - "Inserted 13 words", with what went in, and Undo and Use raw;
//! - "Copied to clipboard", and when Viary may not type, why, with "Allow
//!   typing…", which opens setup at the typing step;
//! - a failure, with Retry; a hint, transient.
//!
//! Each replaces the last, so they do not pile up. Undo, Use raw, and
//! Retry act only while the dictation still offers them; when it moves on,
//! their notification closes, so no button there silently does nothing.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
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
/// Whether its buttons act on the current dictation.
static ACTIONABLE: AtomicBool = AtomicBool::new(false);

/// A notification to show.
#[derive(Debug, PartialEq, Eq)]
struct Note {
    summary: String,
    body: String,
    actions: Vec<(&'static str, &'static str)>,
    /// Gone from the message tray once seen: hints.
    transient: bool,
    /// Its buttons act on the dictation shown, and expire with it.
    actionable: bool,
}

/// Escapes text for a notification body, which GNOME reads as markup.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// What `view` looks like as a notification; `None` for views the user
/// need not be told about (while listening, GNOME shows its own
/// microphone icon in the top bar).
fn note(view: &PillView, can_type: bool) -> Option<Note> {
    let note = match view {
        PillView::Inserted { label, can_raw, text } => {
            let mut actions = vec![("undo", "Undo")];
            if *can_raw {
                actions.push(("useRaw", "Use raw"));
            }
            Note {
                summary: if label.starts_with("Inserted") {
                    label.clone()
                } else {
                    format!("Inserted {label}")
                },
                body: escape(text),
                actions,
                transient: false,
                actionable: true,
            }
        }
        PillView::Copied { label, .. } if !can_type => Note {
            summary: label.clone(),
            body: "Viary isn’t allowed to type into windows. Press Ctrl+V to paste.".into(),
            actions: vec![("allow", "Allow typing…"), ("dismiss", "Dismiss")],
            transient: false,
            actionable: false,
        },
        PillView::Copied { label, hint } => Note {
            summary: label.clone(),
            body: escape(hint),
            actions: vec![],
            transient: false,
            actionable: false,
        },
        PillView::Failed { message, retryable, .. } => Note {
            summary: message.clone(),
            body: "The audio is kept in History.".into(),
            actions: if *retryable { vec![("retry", "Retry")] } else { vec![] },
            transient: false,
            actionable: *retryable,
        },
        PillView::Hint { text } => Note {
            summary: text.clone(),
            body: String::new(),
            actions: vec![],
            transient: true,
            actionable: false,
        },
        _ => return None,
    };
    Some(note)
}

/// Shows `view` as a notification, or closes the last one when its buttons
/// no longer act: a new dictation started, or the last one settled.
pub fn pill(view: &PillView) {
    let note = note(view, super::permissions::can_type());
    let was_actionable =
        ACTIONABLE.swap(note.as_ref().is_some_and(|n| n.actionable), Ordering::SeqCst);
    if note.is_none() && !was_actionable {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result = match note {
            Some(note) => show(&note).await,
            None => close_shown().await,
        };
        if let Err(error) = result {
            tracing::warn!(%error, "cannot update the notification");
        }
    });
}

async fn show(note: &Note) -> zbus::Result<()> {
    let connection = super::session_bus().await?;
    let actions: Vec<&str> = note.actions.iter().flat_map(|(id, label)| [*id, *label]).collect();
    let mut hints: HashMap<&str, Value<'_>> =
        HashMap::from([("desktop-entry", Value::from("viary"))]);
    if note.transient {
        hints.insert("transient", Value::from(true));
    }
    let reply = connection
        .call_method(
            Some(BUS),
            PATH,
            Some(BUS),
            "Notify",
            &(
                "Viary",
                SHOWN.load(Ordering::SeqCst),
                "viary",
                note.summary.as_str(),
                note.body.as_str(),
                actions,
                hints,
                -1_i32,
            ),
        )
        .await?;
    let id: u32 = reply.body().deserialize()?;
    SHOWN.store(id, Ordering::SeqCst);
    Ok(())
}

async fn close_shown() -> zbus::Result<()> {
    let id = SHOWN.load(Ordering::SeqCst);
    if id == 0 {
        return Ok(());
    }
    super::session_bus()
        .await?
        .call_method(Some(BUS), PATH, Some(BUS), "CloseNotification", &(id,))
        .await
        .map(drop)
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
        .path(PATH)?
        .interface(BUS)?
        .member("ActionInvoked")?
        .build();
    let mut stream = MessageStream::for_match_rule(rule, &connection, None).await?;
    while let Some(message) = stream.next().await {
        let Ok(message) = message else { continue };
        if !super::sent_by(&message, BUS).await {
            continue;
        }
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
            "allow" => {
                if let Err(error) = ui::open_setup(app, Some("typing")) {
                    tracing::warn!(%error, "cannot open setup");
                }
            }
            // "dismiss": GNOME closes a notification when a button is used.
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_insertion_shows_what_went_in_and_its_buttons() {
        let view = PillView::Inserted {
            label: "13 words".into(),
            can_raw: true,
            text: "Fish & <chips>".into(),
        };
        let note = note(&view, true).unwrap();
        assert_eq!(note.summary, "Inserted 13 words");
        assert_eq!(note.body, "Fish &amp; &lt;chips&gt;");
        assert_eq!(note.actions, vec![("undo", "Undo"), ("useRaw", "Use raw")]);
        assert!(note.actionable);
    }

    #[test]
    fn copied_without_typing_says_why_and_offers_to_allow_it() {
        let view = PillView::Copied {
            label: "Copied to clipboard".into(),
            hint: "Ctrl+V to paste".into(),
        };
        let blocked = note(&view, false).unwrap();
        assert!(blocked.body.starts_with("Viary isn’t allowed to type"));
        assert_eq!(blocked.actions[0], ("allow", "Allow typing…"));
        assert!(note(&view, true).unwrap().actions.is_empty());
    }

    #[test]
    fn idle_is_not_a_notification() {
        assert!(note(&PillView::Idle, true).is_none());
    }
}
