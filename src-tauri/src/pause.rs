//! Pausing dictation from the tray: the talk key does nothing until the
//! time runs out or the user resumes. Voice Notes and Transcripts go on.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{App, lock, ui};

/// A pause in effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paused {
    /// When it ends, in milliseconds since the epoch; `None` until resumed.
    pub until: Option<u64>,
}

#[derive(Default)]
pub struct Pause {
    state: Mutex<Option<Paused>>,
    /// Counts pauses, so a timer ends only the pause that started it.
    generation: AtomicU64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Pause {
    /// The pause in effect, if any.
    pub fn get(&self) -> Option<Paused> {
        let mut state = lock(&self.state);
        if state.is_some_and(|p| p.until.is_some_and(|until| until <= now_ms())) {
            *state = None;
        }
        *state
    }

    /// Pauses for `length`, or until [`resume`](Self::resume) when `None`.
    pub fn start(&self, app: &AppHandle, length: Option<Duration>) {
        let until = length.map(|length| {
            now_ms().saturating_add(u64::try_from(length.as_millis()).unwrap_or(u64::MAX))
        });
        *lock(&self.state) = Some(Paused { until });
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(length) = length {
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(length);
                let pause = &app.state::<App>().pause;
                if pause.generation.load(Ordering::SeqCst) == generation {
                    pause.resume(&app);
                }
            });
        }
        changed(app);
    }

    pub fn resume(&self, app: &AppHandle) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *lock(&self.state) = None;
        changed(app);
    }
}

fn changed(app: &AppHandle) {
    ui::redraw_tray(app);
    ui::refresh(app);
}
