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

/// How often a timed pause wakes: to end on time after the computer slept
/// (sleeping threads do not count suspended time), and to keep the
/// tooltip's "for 12 more minutes" current.
const TICK: Duration = Duration::from_secs(30);

/// The time now, in milliseconds since the epoch.
pub fn now_ms() -> u64 {
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
        // The generation changes with the state, under its lock, so a timer
        // left from an earlier pause cannot see its own generation with
        // this pause in effect.
        let generation = {
            let mut state = lock(&self.state);
            *state = Some(Paused { until });
            self.generation.fetch_add(1, Ordering::SeqCst) + 1
        };
        if let Some(until) = until {
            let app = app.clone();
            std::thread::spawn(move || {
                let pause = &app.state::<App>().pause;
                loop {
                    let left = until.saturating_sub(now_ms());
                    std::thread::sleep(Duration::from_millis(left).min(TICK));
                    if pause.generation.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    if now_ms() >= until {
                        pause.end(&app, generation);
                        return;
                    }
                    // The time left, in the tooltip and the Linux menu.
                    ui::refresh(&app);
                }
            });
        }
        changed(app);
    }

    pub fn resume(&self, app: &AppHandle) {
        {
            let mut state = lock(&self.state);
            self.generation.fetch_add(1, Ordering::SeqCst);
            *state = None;
        }
        changed(app);
    }

    /// Ends the pause `generation` started, unless another has since.
    fn end(&self, app: &AppHandle, generation: u64) {
        {
            let mut state = lock(&self.state);
            if self.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            *state = None;
        }
        changed(app);
    }
}

fn changed(app: &AppHandle) {
    ui::refresh(app);
}
