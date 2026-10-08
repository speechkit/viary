//! The dictation state machine: hold the key to record, release to
//! transcribe and paste, then Undo or Use raw; on failure keep the audio
//! and offer Retry or another engine.
//!
//! One thread owns the state and handles [`Msg`]s one at a time. Slow work
//! (finishing recognition and polishing text) runs on worker threads
//! that report back with a message carrying the dictation's token, so a
//! late report for an abandoned dictation is ignored.

use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

use serde::Serialize;

use speechkit::{
    AudioBuffer, SpeechError,
    asr::Transcript,
};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    App, dictionary,
    engines::{self, DASHSCOPE, EngineInfo, LoadedEngine, OPENAI, describe},
    history::{self, EngineLabel, Entry, Status},
    keychain::{self, Provider},
    platform::{
        self,
        apps::{self, TargetApp},
        focus::{self, Focus},
        hotkey::HotkeyEvent,
        keys, pasteboard, permissions,
    },
    polish,
    settings::{Hotkey, Settings},
    recording::Recording,
    ui::{self, TrayState},
};

/// A key press shorter than this is a tap, not a dictation.
const MIN_HOLD: Duration = Duration::from_millis(350);
/// A second tap pressed this soon after the first one is released starts
/// hands-free dictation.
const DOUBLE_TAP: Duration = Duration::from_millis(400);
/// A hands-free dictation stops by itself after this long.
const HANDS_FREE_FOR: Duration = Duration::from_secs(5 * 60);
/// How long recognition may take to finish after the input stops, at least.
/// A longer recording gets as long as it ran, for an engine that fell behind.
const FINISH_FOR: Duration = Duration::from_secs(90);
/// How long "Inserted" stays up with its Undo button.
const INSERTED_FOR: Duration = Duration::from_secs(3);
const HINT_FOR: Duration = Duration::from_millis(2200);
const COPIED_FOR: Duration = Duration::from_millis(3200);

/// What the pill shows. Sent to the web views as `pill-state`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PillView {
    Idle,
    #[serde(rename_all = "camelCase")]
    Listening {
        /// The dictation, to match `pill-partial` events.
        token: u64,
        /// Milliseconds since the epoch, for the clock.
        started_at: u64,
        /// The app, and "Cloud" for cloud engines.
        context: String,
        /// Whether words appear while speaking.
        live: bool,
    },
    /// Listening after a double tap, until the next tap, Stop, or the limit.
    #[serde(rename_all = "camelCase")]
    HandsFree {
        /// The dictation, as in `Listening`.
        token: u64,
        /// Milliseconds since the epoch, for the clock.
        started_at: u64,
        /// The longest it runs, in milliseconds.
        limit_ms: u64,
        /// The app and "Cloud", as while holding the key.
        context: String,
        /// Whether words appear while speaking.
        live: bool,
    },
    Transcribing {
        label: String,
    },
    /// The polish model is rewriting the text; Skip inserts it as
    /// recognized.
    Polishing,
    #[serde(rename_all = "camelCase")]
    Inserted {
        label: String,
        can_raw: bool,
    },
    Copied {
        label: String,
        hint: String,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        message: String,
        detail: String,
        retryable: bool,
        /// "Use on-device" or "Use OpenAI", when another engine is set up.
        alternative: Option<String>,
    },
    Hint {
        text: String,
    },
}

impl PillView {
    /// Whether the pill has buttons, and so must take clicks.
    pub fn interactive(&self) -> bool {
        matches!(
            self,
            Self::HandsFree { .. } | Self::Polishing | Self::Inserted { .. } | Self::Failed { .. }
        )
    }

    fn tray(&self) -> TrayState {
        match self {
            Self::Listening { .. } | Self::HandsFree { .. } => TrayState::Listening,
            Self::Transcribing { .. } | Self::Polishing => TrayState::Working,
            Self::Failed { .. } => TrayState::Failed,
            _ => TrayState::Idle,
        }
    }
}

/// A button in the pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PillAction {
    Undo,
    UseRaw,
    Retry,
    SwitchEngine,
    Dismiss,
    /// Stop waiting for the polish model and insert the text as recognized.
    SkipPolish,
    /// End a hands-free dictation and insert it.
    Stop,
}

pub enum Msg {
    /// A hotkey event, and when the listener saw it.
    Hotkey(HotkeyEvent, Instant),
    Pill(PillAction),
    /// A single short tap of `token`'s dictation: unless a second tap
    /// follows in time, say how to dictate.
    Tapped(u64),
    /// The session of `token`'s dictation closed by itself.
    Closed(u64),
    /// `token`'s hands-free dictation reached its limit.
    HandsFreeLimit(u64),
    /// Recognition for `token` ended.
    Finished(u64, Transcribed),
    /// The polish model rewrote the text of `token`, or gave up.
    Polished(u64, Polished),
    /// The switch to engine `id` asked for by the Failed pill ended.
    Switched(u64, String, Result<EngineInfo, String>),
    /// A timed state ends: hide the pill if `token` is still current.
    Expire(u64),
    /// Start listening hands-free, as a double tap does: from the tray menu.
    #[cfg_attr(target_os = "macos", expect(dead_code, reason = "macOS has no tray menu"))]
    StartHandsFree,
    /// Type the newest dictation into the app in front again.
    #[cfg_attr(target_os = "macos", expect(dead_code, reason = "macOS has no tray menu"))]
    PasteLast,
}

/// A recognition result.
pub struct Transcribed {
    result: Result<Transcript, SpeechError>,
    audio: Arc<AudioBuffer>,
    engine: Option<Arc<LoadedEngine>>,
}

/// The polish model's rewrite, or why there is none.
pub type Polished = Result<String, String>;

/// A dictation waiting for the polish model, with the text as recognized
/// to insert if it fails or the user skips it.
struct Polishing {
    job: Job,
    text: String,
    raw: String,
    info: Option<EngineInfo>,
}

/// One dictation, from key release until its text lands or it fails.
struct Job {
    target: TargetApp,
    started_ms: u64,
    /// The History entry, once there is one (after a failure).
    history_id: Option<String>,
    /// The recording, once stopped.
    audio: Option<Arc<AudioBuffer>>,
    /// The engine that last failed to transcribe it.
    failed_engine: Option<String>,
    /// Every engine that failed it, or failed to load for it, so the
    /// alternative offered is none of them.
    tried: Vec<String>,
}

struct Listening {
    recording: Recording,
    engine: Arc<LoadedEngine>,
    /// When the key was first pressed: what the limit and deadline count from.
    started: Instant,
    started_ms: u64,
    /// When the key was last pressed: a double tap's second press.
    pressed: Instant,
    target: TargetApp,
    /// Pressed soon after a tap: released quickly, it goes hands-free.
    second_tap: bool,
    /// Started by a double tap: the next press ends it, not key-up.
    hands_free: bool,
    /// The session closed by itself, as a failed one does.
    closed: bool,
}

/// The last insertion, for Undo and Use raw.
struct Inserted {
    target: TargetApp,
    raw: String,
    history_id: String,
}

enum Phase {
    Idle,
    Listening(Box<Listening>),
    /// Released after a tap at the instant given: the recording goes on in
    /// case a second tap follows, which continues it instead of starting
    /// another session.
    Tapped(Box<Listening>, Instant),
    Busy(Job),
    /// Waiting for the polish model.
    Polishing(Box<Polishing>),
    Inserted(Inserted),
    /// Failed with the audio kept. `alternative` is another engine's id.
    Failed {
        job: Job,
        alternative: Option<String>,
    },
    /// A hint or "copied": nothing to act on.
    Notice,
}

/// Starts the controller thread and returns its mailbox.
pub fn spawn(app: AppHandle) -> Sender<Msg> {
    let (sender, receiver) = mpsc::channel();
    let mailbox = sender.clone();
    let spawned = std::thread::Builder::new()
        .name("viary-dictation".into())
        .spawn(move || {
            Controller {
                app,
                mailbox,
                phase: Phase::Idle,
                token: 0,
            }
            .run(&receiver);
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "cannot start the dictation controller");
    }
    sender
}

struct Controller {
    app: AppHandle,
    mailbox: Sender<Msg>,
    phase: Phase,
    /// Numbers each dictation and timed state.
    token: u64,
}

impl Controller {
    fn state(&self) -> tauri::State<'_, App> {
        self.app.state::<App>()
    }

    fn settings(&self) -> Settings {
        self.state().settings()
    }

    fn run(mut self, receiver: &Receiver<Msg>) {
        while let Ok(msg) = receiver.recv() {
            match msg {
                Msg::Hotkey(HotkeyEvent::Down, at) => self.key_down(at),
                Msg::Hotkey(HotkeyEvent::Up, at) => self.key_up(at),
                Msg::Hotkey(HotkeyEvent::OtherKey, at) => self.other_key(at),
                Msg::Pill(action) => self.pill(action),
                Msg::Tapped(token) if token == self.token => {
                    // No second tap: drop the recording and say how to dictate.
                    if matches!(self.phase, Phase::Tapped(..)) {
                        let key = key_name(self.settings().hotkey);
                        self.hint(format!("Hold {key} to dictate, or double-tap it"));
                    }
                }
                Msg::Closed(token) if token == self.token => self.closed(),
                Msg::HandsFreeLimit(token) if token == self.token => {
                    if matches!(&self.phase, Phase::Listening(l) if l.hands_free) {
                        self.stop_listening();
                    }
                }
                Msg::Finished(token, done) if token == self.token => self.finished(done),
                Msg::Polished(token, polished) if token == self.token => self.polished(polished),
                Msg::Switched(token, id, outcome) if token == self.token => {
                    self.switched(id, outcome)
                }
                Msg::StartHandsFree => self.start_hands_free(),
                Msg::PasteLast => self.paste_last(),
                Msg::Expire(token) if token == self.token => {
                    if matches!(self.phase, Phase::Inserted(_) | Phase::Notice) {
                        self.phase = Phase::Idle;
                        self.show(PillView::Idle);
                    }
                }
                _ => {}
            }
        }
    }

    fn next_token(&mut self) -> u64 {
        self.token += 1;
        self.token
    }

    fn show(&self, view: PillView) {
        ui::set_tray(&self.app, view.tray());
        ui::show_pill(&self.app, &view);
    }

    /// Sends `msg` to the controller after `after`.
    fn send_after(&self, after: Duration, msg: Msg) {
        let mailbox = self.mailbox.clone();
        std::thread::spawn(move || {
            std::thread::sleep(after);
            let _ = mailbox.send(msg);
        });
    }

    /// Sends `Expire` for the current token after `after`.
    fn expire_after(&self, after: Duration) {
        self.send_after(after, Msg::Expire(self.token));
    }

    fn hint(&mut self, text: impl Into<String>) {
        self.next_token();
        self.phase = Phase::Notice;
        self.show(PillView::Hint { text: text.into() });
        self.expire_after(HINT_FOR);
    }

    // -----------------------------------------------------------------
    // Recording

    fn key_down(&mut self, at: Instant) {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            // Toggle-to-talk: the press after a double tap ends the
            // dictation; its key-up then finds nothing to do.
            Phase::Listening(listening) if listening.hands_free => {
                self.phase = Phase::Listening(listening);
                self.stop_listening();
                return;
            }
            // A second tap goes on with the first one's recording.
            Phase::Tapped(mut listening, released) if is_second_tap(released, at) => {
                listening.pressed = at;
                listening.second_tap = true;
                self.show(PillView::Listening {
                    token: self.token,
                    started_at: listening.started_ms,
                    context: context(&listening.target, &listening.engine.info),
                    live: listening.engine.info.live,
                });
                self.phase = Phase::Listening(listening);
                return;
            }
            // Too late for a double tap: a new dictation instead.
            Phase::Tapped(..) => {}
            phase @ (Phase::Listening(_) | Phase::Busy(_) | Phase::Polishing(_)) => {
                self.phase = phase;
                return;
            }
            phase => self.phase = phase,
        }
        if self.state().pause.get().is_some() {
            self.hint("Dictation is paused");
            return;
        }
        let token = self.next_token();
        let settings = self.settings();
        let Some(engine) = self.state().engines.current() else {
            // The saved engine may still be loading, as after login.
            if let Some(loading) = self.state().engines.status().loading {
                self.hint(format!("Loading {}…", engine_name(&settings, &loading)));
                return;
            }
            self.hint("Choose a voice engine first");
            ui::open_main(&self.app, "engine");
            return;
        };
        let target = apps::frontmost().unwrap_or_default();
        let microphone = match Recording::open_microphone(settings.microphone.as_deref()) {
            Ok(microphone) => microphone,
            Err(error) => {
                tracing::warn!(error = %describe(&error), "cannot open the microphone");
                self.hint("Microphone unavailable");
                return;
            }
        };
        let options = engine.session_options(&settings, &target.name);
        let (app, mailbox) = (self.app.clone(), self.mailbox.clone());
        let recording = match Recording::start(
            &microphone,
            &engine.engine,
            options,
            move |level, text, closed| {
                let _ = app.emit_to("pill", "pill-level", level);
                if let Some(text) = text {
                    let _ = app.emit_to("pill", "pill-partial", (token, text));
                }
                if closed {
                    let _ = mailbox.send(Msg::Closed(token));
                }
            },
        ) {
            Ok(recording) => recording,
            Err(error) => {
                tracing::warn!(error = %describe(&error), "cannot start recording");
                // speechkit names the backend of a device that would not start.
                let device = matches!(
                    &error,
                    SpeechError::Backend { backend, .. } if backend == "microphone"
                );
                self.hint(if device {
                    "Microphone unavailable"
                } else {
                    "Cannot start dictation"
                });
                return;
            }
        };
        let started_ms = history::now_ms();
        self.show(PillView::Listening {
            token,
            started_at: started_ms,
            context: context(&target, &engine.info),
            live: engine.info.live,
        });
        self.phase = Phase::Listening(Box::new(Listening {
            recording,
            engine,
            started: at,
            started_ms,
            pressed: at,
            target,
            second_tap: false,
            hands_free: false,
            closed: false,
        }));
    }

    fn other_key(&mut self, at: Instant) {
        // fn+arrow and similar shortcuts: drop a dictation that just began.
        if let Phase::Listening(listening) = &self.phase
            && at.saturating_duration_since(listening.pressed) < Duration::from_millis(600)
        {
            self.next_token();
            self.phase = Phase::Idle;
            self.show(PillView::Idle);
        }
    }

    fn key_up(&mut self, at: Instant) {
        let Phase::Listening(listening) = &self.phase else {
            return;
        };
        if listening.hands_free {
            return;
        }
        match release(at.saturating_duration_since(listening.pressed), listening.second_tap) {
            Release::Insert => self.stop_listening(),
            Release::HandsFree => self.hands_free(),
            Release::Tap => {
                // Maybe the first of two: keep listening, out of sight, for
                // a moment before the hint.
                let Phase::Listening(listening) = std::mem::replace(&mut self.phase, Phase::Idle)
                else {
                    return;
                };
                self.show(PillView::Idle);
                self.phase = Phase::Tapped(listening, at);
                self.send_after(DOUBLE_TAP, Msg::Tapped(self.token));
            }
        }
    }

    /// Starts a hands-free dictation without the key.
    fn start_hands_free(&mut self) {
        if matches!(self.phase, Phase::Listening(_) | Phase::Tapped(..)) {
            return;
        }
        self.key_down(Instant::now());
        if matches!(&self.phase, Phase::Listening(listening) if !listening.hands_free) {
            self.hands_free();
        }
    }

    fn paste_last(&mut self) {
        let last = self.state().history.list().into_iter().find(|e| !e.text.is_empty());
        let Some(entry) = last else {
            self.hint("Nothing dictated yet");
            return;
        };
        let target = apps::frontmost().unwrap_or_default();
        if let Delivery::Copied { label, .. } = deliver(&self.app, &entry.text, &target) {
            self.hint(label);
        }
    }

    /// Keeps listening after a double tap until the next press, Stop, the
    /// session closing, or the limit.
    fn hands_free(&mut self) {
        let Phase::Listening(listening) = &mut self.phase else {
            return;
        };
        if listening.closed {
            self.stop_listening();
            return;
        }
        listening.hands_free = true;
        let left = HANDS_FREE_FOR.saturating_sub(listening.started.elapsed());
        let view = PillView::HandsFree {
            token: self.token,
            started_at: listening.started_ms,
            limit_ms: u64::try_from(HANDS_FREE_FOR.as_millis()).unwrap_or(u64::MAX),
            context: context(&listening.target, &listening.engine.info),
            live: listening.engine.info.live,
        };
        self.show(view);
        self.send_after(left, Msg::HandsFreeLimit(self.token));
    }

    /// The session ended by itself. Hands-free has no key-up to wait for,
    /// so it finishes now; holding the key, key-up still does.
    fn closed(&mut self) {
        let listening = match &mut self.phase {
            Phase::Listening(listening) | Phase::Tapped(listening, _) => listening,
            _ => return,
        };
        listening.closed = true;
        if listening.hands_free {
            self.stop_listening();
        }
    }

    /// Stops the input and finishes recognition on a worker thread.
    fn stop_listening(&mut self) {
        let Phase::Listening(listening) = std::mem::replace(&mut self.phase, Phase::Idle) else {
            return;
        };
        let Listening {
            recording,
            engine,
            started,
            started_ms,
            target,
            ..
        } = *listening;
        let deadline = finish_deadline(started.elapsed());
        recording.stop();
        let token = self.token;
        self.show(PillView::Transcribing {
            label: transcribing_label(&engine.info),
        });
        self.phase = Phase::Busy(Job {
            target,
            started_ms,
            history_id: None,
            audio: None,
            failed_engine: None,
            tried: Vec::new(),
        });
        let mailbox = self.mailbox.clone();
        std::thread::spawn(move || {
            let (outcome, audio) = recording.finish(deadline);
            let audio = Arc::new(audio);
            let result = outcome.map_err(|failure| failure.error);
            let engine = Some(engine);
            let _ = mailbox.send(Msg::Finished(
                token,
                Transcribed {
                    result,
                    audio,
                    engine,
                },
            ));
        });
    }

    // -----------------------------------------------------------------
    // Results

    fn finished(&mut self, done: Transcribed) {
        let Phase::Busy(mut job) = std::mem::replace(&mut self.phase, Phase::Idle) else {
            return;
        };
        job.audio = Some(done.audio.clone());
        let info = done.engine.as_ref().map(|e| e.info.clone());
        match done.result {
            Ok(transcript) => {
                let raw = transcript.text();
                if raw.trim().is_empty() {
                    if job.history_id.is_some() {
                        // A retry that heard nothing: keep the failed entry.
                        self.phase = Phase::Idle;
                    }
                    self.hint("No speech detected");
                    return;
                }
                let settings = self.settings();
                let text = finish(&settings, &job.target.name, done.engine.as_deref(), transcript);
                if polish::applies(&settings, &job.target.name) {
                    self.polish(job, &settings, text, raw, info);
                } else {
                    self.insert(job, text, raw, info.as_ref(), None);
                }
            }
            Err(error) => self.fail(job, &error, info.as_ref()),
        }
    }

    /// Sends `text` through the polish model, then inserts it. The pill
    /// says so meanwhile, with Skip; a model that fails or is not set up
    /// leaves the text as recognized.
    fn polish(&mut self, job: Job, settings: &Settings, text: String, raw: String, info: Option<EngineInfo>) {
        let request = match polish::request(settings, &job.target.name) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(%error, "polish is on but cannot run");
                self.insert(job, text, raw, info.as_ref(), Some("not polished"));
                return;
            }
        };
        let token = self.token;
        self.show(PillView::Polishing);
        let (mailbox, spoken) = (self.mailbox.clone(), text.clone());
        self.phase = Phase::Polishing(Box::new(Polishing { job, text, raw, info }));
        std::thread::spawn(move || {
            let polished = request.run(&spoken, polish::TIMEOUT);
            let _ = mailbox.send(Msg::Polished(token, polished));
        });
    }

    fn polished(&mut self, polished: Polished) {
        let Phase::Polishing(waiting) = std::mem::replace(&mut self.phase, Phase::Idle) else {
            return;
        };
        let Polishing { job, text, raw, info } = *waiting;
        match polished {
            Ok(polished) => self.insert(job, polished, raw, info.as_ref(), None),
            Err(error) => {
                tracing::warn!(%error, "polish failed; inserting the text as recognized");
                self.insert(job, text, raw, info.as_ref(), Some("not polished"));
            }
        }
    }

    fn insert(&mut self, job: Job, text: String, raw: String, info: Option<&EngineInfo>, note: Option<&str>) {
        let delivery = deliver(&self.app, &text, &job.target);
        let status = match delivery {
            Delivery::Pasted => Status::Inserted,
            Delivery::Copied { .. } => Status::Copied,
        };
        let punctuated = (text != raw).then(|| text.clone());
        let history_id = self.record(&job, info, |entry| {
            entry.text.clone_from(&text);
            entry.raw.clone_from(&raw);
            entry.punctuated = punctuated;
            entry.status = status;
            entry.error = None;
        });
        self.next_token();
        match delivery {
            Delivery::Pasted => {
                let words = history::count_words(&text);
                let mut label = format!("{words} word{}", if words == 1 { "" } else { "s" });
                if job.failed_engine.is_some()
                    && let Some(info) = info
                {
                    label.push_str(if info.on_device {
                        " · on-device"
                    } else {
                        " · cloud"
                    });
                }
                if let Some(note) = note {
                    label.push_str(" · ");
                    label.push_str(note);
                }
                self.show(PillView::Inserted {
                    label,
                    can_raw: raw != text,
                });
                self.phase = Phase::Inserted(Inserted {
                    target: job.target,
                    raw,
                    history_id,
                });
                self.expire_after(INSERTED_FOR);
            }
            Delivery::Copied { label, hint } => {
                self.show(PillView::Copied { label, hint });
                self.phase = Phase::Notice;
                self.expire_after(COPIED_FOR);
            }
        }
        ui::refresh(&self.app);
    }

    fn fail(&mut self, mut job: Job, error: &SpeechError, info: Option<&EngineInfo>) {
        let detail = describe(error);
        tracing::warn!(error = %detail, "dictation failed; audio kept");
        let (message, retryable) = failure_message(error, info);
        let history_id = self.record(&job, info, |entry| {
            entry.status = Status::Failed;
            entry.error = Some(detail.clone());
        });
        job.history_id = Some(history_id);
        // Without `info` (a switch that did not load), the last engine that
        // failed to transcribe still decides what to offer.
        if let Some(info) = info {
            job.failed_engine = Some(info.id.clone());
            job.tried.push(info.id.clone());
        }
        let alternative = alternative(&self.settings(), job.failed_engine.as_deref(), &job.tried);
        self.next_token();
        self.show(PillView::Failed {
            message,
            detail,
            retryable,
            alternative: alternative.as_ref().map(|(_, label)| label.clone()),
        });
        self.phase = Phase::Failed {
            job,
            alternative: alternative.map(|(id, _)| id),
        };
        ui::refresh(&self.app);
    }

    /// Adds the History entry for `job`, or updates the one a failure
    /// created, and returns its id. Without `info`, an existing entry keeps
    /// its engine.
    fn record(
        &self,
        job: &Job,
        info: Option<&EngineInfo>,
        fill: impl FnOnce(&mut Entry),
    ) -> String {
        let state = self.state();
        let history = &state.history;
        let label = info.map(|info| EngineLabel {
            name: info.name.clone(),
            kind: info.kind.clone(),
            on_device: info.on_device,
        });
        let mut fill = Some(fill);
        if let Some(id) = &job.history_id
            && history
                .update(id, |entry| {
                    if let Some(label) = &label {
                        entry.engine = label.clone();
                    }
                    if let Some(fill) = fill.take() {
                        fill(entry);
                    }
                })
                .is_some()
        {
            return id.clone();
        }
        let id = history::now_ms().to_string();
        let duration_ms = job.audio.as_ref().map_or(0, |audio| {
            u64::try_from(audio.duration().as_millis()).unwrap_or(u64::MAX)
        });
        let recording = match &job.audio {
            Some(audio) if self.settings().keep_recordings_days > 0 => {
                history.save_recording(&id, audio)
            }
            _ => None,
        };
        let mut entry = Entry {
            id: id.clone(),
            created_at: job.started_ms,
            app: job.target.name.clone(),
            duration_ms,
            text: String::new(),
            raw: String::new(),
            punctuated: None,
            engine: label.unwrap_or_else(|| EngineLabel {
                name: "No engine".into(),
                kind: String::new(),
                on_device: true,
            }),
            status: Status::Inserted,
            error: None,
            recording,
        };
        if let Some(fill) = fill {
            fill(&mut entry);
        }
        history.add(entry);
        id
    }

    // -----------------------------------------------------------------
    // Pill buttons

    fn pill(&mut self, action: PillAction) {
        match (action, std::mem::replace(&mut self.phase, Phase::Idle)) {
            (PillAction::Undo | PillAction::UseRaw, Phase::Inserted(inserted)) => {
                if !refocus(&inserted.target) {
                    // ⌘Z would undo something in whichever app is in front.
                    self.hint(format!(
                        "{} · undo there with {}",
                        not_returned(&inserted.target),
                        platform::UNDO_KEY
                    ));
                } else if action == PillAction::Undo {
                    self.undo(&inserted);
                } else {
                    self.use_raw(inserted);
                }
            }
            (PillAction::Retry, Phase::Failed { job, .. }) => self.retry(job),
            (
                PillAction::SwitchEngine,
                Phase::Failed {
                    job,
                    alternative: Some(id),
                },
            ) => {
                let token = self.next_token();
                let name = engine_name(&self.settings(), &id);
                self.show(PillView::Transcribing {
                    label: format!("Switching to {name}"),
                });
                self.phase = Phase::Busy(job);
                let (mailbox, target) = (self.mailbox.clone(), id.clone());
                self.state().activate_engine(&self.app, id, move |outcome| {
                    let _ = mailbox.send(Msg::Switched(token, target, outcome));
                });
            }
            (PillAction::SkipPolish, Phase::Polishing(waiting)) => {
                // Inserting moves to a new token: a late reply is dropped.
                let Polishing { job, text, raw, info } = *waiting;
                self.insert(job, text, raw, info.as_ref(), Some("not polished"));
            }
            (PillAction::Stop, phase @ Phase::Listening(_)) => {
                let hands_free = matches!(&phase, Phase::Listening(l) if l.hands_free);
                self.phase = phase;
                if hands_free {
                    self.stop_listening();
                }
            }
            (PillAction::Dismiss, Phase::Failed { .. }) => {
                self.next_token();
                self.show(PillView::Idle);
            }
            (_, phase) => self.phase = phase,
        }
    }

    /// Takes the insertion back with ⌘Z; `inserted.target` is in front.
    fn undo(&mut self, inserted: &Inserted) {
        self.next_token();
        self.show(PillView::Idle);
        if let Err(error) = keys::undo(&self.app) {
            tracing::warn!(%error, "cannot undo");
        }
        self.state()
            .history
            .update(&inserted.history_id, |e| e.status = Status::Undone);
        ui::refresh(&self.app);
    }

    /// Replaces the insertion with the raw text; `inserted.target` is in
    /// front.
    fn use_raw(&mut self, inserted: Inserted) {
        if let Err(error) = keys::undo(&self.app) {
            tracing::warn!(%error, "cannot undo before inserting the raw text");
        }
        std::thread::sleep(Duration::from_millis(80));
        let delivery = deliver(&self.app, &inserted.raw, &inserted.target);
        self.state().history.update(&inserted.history_id, |e| {
            e.text.clone_from(&inserted.raw);
            if matches!(delivery, Delivery::Copied { .. }) {
                e.status = Status::Copied;
            }
        });
        self.next_token();
        match delivery {
            Delivery::Pasted => {
                self.show(PillView::Inserted {
                    label: "Inserted as spoken".into(),
                    can_raw: false,
                });
                self.phase = Phase::Inserted(inserted);
                self.expire_after(INSERTED_FOR);
            }
            Delivery::Copied { label, hint } => {
                self.show(PillView::Copied { label, hint });
                self.phase = Phase::Notice;
                self.expire_after(COPIED_FOR);
            }
        }
        ui::refresh(&self.app);
    }

    fn switched(&mut self, id: String, outcome: Result<EngineInfo, String>) {
        let Phase::Busy(mut job) = std::mem::replace(&mut self.phase, Phase::Idle) else {
            return;
        };
        match outcome {
            Ok(_) => self.retry(job),
            Err(error) => {
                job.tried.push(id);
                let failed = SpeechError::InvalidModel(error);
                self.fail(job, &failed, None);
            }
        }
    }

    /// Transcribes the kept audio again with the current engine.
    fn retry(&mut self, job: Job) {
        let token = self.next_token();
        let (Some(audio), Some(engine)) = (job.audio.clone(), self.state().engines.current())
        else {
            self.hint("Nothing to retry");
            return;
        };
        self.show(PillView::Transcribing {
            label: format!("Retrying with {}", engine.info.name),
        });
        let options = engine.session_options(&self.settings(), &job.target.name);
        self.phase = Phase::Busy(job);
        let mailbox = self.mailbox.clone();
        std::thread::spawn(move || {
            let result = engine
                .engine
                .transcribe(&audio, options, Instant::now() + Duration::from_secs(120))
                .map_err(|failure| failure.error);
            let engine = Some(engine);
            let _ = mailbox.send(Msg::Finished(
                token,
                Transcribed {
                    result,
                    audio,
                    engine,
                },
            ));
        });
    }
}

// ---------------------------------------------------------------------------

/// How the text reached the user.
enum Delivery {
    Pasted,
    /// Left on the clipboard, with why.
    Copied {
        label: String,
        hint: String,
    },
}

/// Brings `target` back to the front, unless it is Viary. Returns whether
/// keystrokes will now reach it.
fn refocus(target: &TargetApp) -> bool {
    target.is_self() || apps::activate(target)
}

/// The app's name, or "The app" when unknown.
fn app_name(target: &TargetApp) -> &str {
    if target.name.is_empty() { "The app" } else { &target.name }
}

/// "Could not return to Notes", for when `target` did not come back.
fn not_returned(target: &TargetApp) -> String {
    if target.name.is_empty() {
        "Could not return to the app".into()
    } else {
        format!("Could not return to {}", target.name)
    }
}

/// Pastes `text` into `target`'s focused field with ⌘V (Ctrl+V elsewhere),
/// then restores the
/// clipboard. Without a text field, without permission to type into other
/// apps, or when `target` cannot be brought back to the front, the text
/// stays on the clipboard instead: a paste would land in another app.
fn deliver(app: &AppHandle, text: &str, target: &TargetApp) -> Delivery {
    if !permissions::can_type() {
        pasteboard::set_text(text);
        return Delivery::Copied {
            label: "Copied to clipboard".into(),
            hint: permissions::ALLOW_TYPING.into(),
        };
    }
    if !refocus(target) {
        pasteboard::set_text(text);
        return Delivery::Copied {
            label: format!("{} · copied to clipboard", not_returned(target)),
            hint: platform::PASTE_HINT.into(),
        };
    }
    if apps::is_protected(target) {
        pasteboard::set_text(text);
        return Delivery::Copied {
            label: format!("{} runs as administrator · copied to clipboard", app_name(target)),
            hint: platform::PASTE_HINT.into(),
        };
    }
    if focus::focused(target.pid) == Focus::NotEditable {
        pasteboard::set_text(text);
        return Delivery::Copied {
            label: "No text field focused · copied to clipboard".into(),
            hint: platform::PASTE_HINT.into(),
        };
    }
    let saved = pasteboard::save();
    let ours = pasteboard::set_transient_text(text);
    std::thread::sleep(Duration::from_millis(30));
    if let Err(error) = keys::paste(app) {
        tracing::warn!(%error, "cannot paste");
        // Left for the user to paste, so clipboard managers should see it.
        pasteboard::set_text(text);
        return Delivery::Copied {
            label: "Copied to clipboard".into(),
            hint: platform::PASTE_HINT.into(),
        };
    }
    // Let the app read the pasteboard before putting the user's back.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(600));
        pasteboard::restore(saved, ours);
    });
    Delivery::Pasted
}

/// Punctuates the completed dictation, then applies dictionary replacements
/// to the joined text, so they can span recognition boundaries. The caller
/// keeps the original transcript.
pub fn finish(
    settings: &Settings,
    app: &str,
    engine: Option<&LoadedEngine>,
    transcript: Transcript,
) -> String {
    let fix = |text: &str| dictionary::apply(&settings.dictionary, app, text);
    let text = match engine {
        Some(engine) => engine.punctuate(&transcript, fix),
        None => transcript.text(),
    };
    fix(&text)
}

/// What letting go of the key does.
#[derive(Debug, PartialEq, Eq)]
enum Release {
    /// It was held: stop and insert.
    Insert,
    /// A double tap's second tap: keep listening.
    HandsFree,
    /// A single tap, maybe the first of two.
    Tap,
}

/// What a key-up does after a press `held` long.
fn release(held: Duration, second_tap: bool) -> Release {
    if held >= MIN_HOLD {
        Release::Insert
    } else if second_tap {
        Release::HandsFree
    } else {
        Release::Tap
    }
}

/// Whether a press at `pressed` follows a tap released at `released`
/// closely enough to make a double tap.
fn is_second_tap(released: Instant, pressed: Instant) -> bool {
    pressed.saturating_duration_since(released) < DOUBLE_TAP
}

/// How long to wait for recognition after `recorded` of input.
fn finish_deadline(recorded: Duration) -> Duration {
    FINISH_FOR.max(recorded)
}

fn context(target: &TargetApp, info: &EngineInfo) -> String {
    let mut parts = Vec::new();
    if !target.name.is_empty() && !target.is_self() {
        parts.push(target.name.clone());
    }
    if !info.on_device {
        parts.push("Cloud".into());
    }
    parts.join(" · ")
}

fn transcribing_label(info: &EngineInfo) -> String {
    if info.on_device {
        "Transcribing".into()
    } else {
        format!("Transcribing · {}", info.kind)
    }
}

/// The pill's message for a failure, and whether Retry may help.
fn failure_message(error: &SpeechError, info: Option<&EngineInfo>) -> (String, bool) {
    let cloud = info.is_some_and(|i| !i.on_device);
    // speechkit classifies backend failures and capacity. Viary also offers
    // Retry after a timeout because it retains the complete recording.
    let retryable = error.retryable() || matches!(error, SpeechError::DeadlineExceeded);
    let message = match error {
        SpeechError::Backend { .. } if retryable && cloud => "Connection lost · audio kept",
        SpeechError::Backend { .. } if retryable => "Engine stopped · audio kept",
        SpeechError::DeadlineExceeded if cloud => "No response · audio kept",
        SpeechError::DeadlineExceeded => "Timed out · audio kept",
        SpeechError::Capacity => "Engine busy · audio kept",
        SpeechError::InvalidModel(_) => "Engine unavailable · audio kept",
        SpeechError::InvalidInput(_) | SpeechError::Unsupported(_) => {
            "Engine not ready · audio kept"
        }
        _ => "Recognition failed · audio kept",
    };
    (message.into(), retryable)
}

pub fn cloud_ready(settings: &Settings, id: &str) -> bool {
    match id {
        OPENAI => !settings.openai.model.trim().is_empty() && keychain::has(Provider::OpenAi),
        DASHSCOPE => {
            !settings.dashscope.model.trim().is_empty() && keychain::has(Provider::DashScope)
        }
        _ => false,
    }
}

/// Another engine that is set up and not in `tried`, preferring on-device
/// after a cloud failure and the cloud after an on-device one:
/// `(id, button label)`.
fn alternative(
    settings: &Settings,
    failed: Option<&str>,
    tried: &[String],
) -> Option<(String, String)> {
    let local: Vec<String> = settings
        .local_models
        .iter()
        .map(engines::local_id)
        .collect();
    let cloud: Vec<String> = [OPENAI, DASHSCOPE]
        .into_iter()
        .filter(|id| cloud_ready(settings, id))
        .map(String::from)
        .collect();
    let failed_is_local = failed.is_some_and(|id| id.starts_with("local:"));
    let order: Vec<String> = if failed_is_local {
        cloud.into_iter().chain(local).collect()
    } else {
        local.into_iter().chain(cloud).collect()
    };
    let id = order
        .into_iter()
        .find(|id| Some(id.as_str()) != failed && !tried.contains(id))?;
    let label = match id.as_str() {
        OPENAI => "Use OpenAI".into(),
        DASHSCOPE => "Use DashScope".into(),
        _ => "Use on-device".into(),
    };
    Some((id, label))
}

pub fn engine_name(settings: &Settings, id: &str) -> String {
    match id {
        OPENAI => "OpenAI".into(),
        DASHSCOPE => "DashScope".into(),
        _ => id
            .strip_prefix("local:")
            .and_then(|local| settings.local(local))
            .map_or_else(|| "on-device".into(), |m| m.name.clone()),
    }
}

pub fn key_name(hotkey: Hotkey) -> &'static str {
    match hotkey {
        Hotkey::Fn => "fn",
        Hotkey::RightOption => "right ⌥",
        Hotkey::RightCommand => "right ⌘",
        Hotkey::RightAlt => "Right Alt",
        Hotkey::CtrlWin => "Ctrl + Win",
        Hotkey::Shortcut => "your talk shortcut",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::LocalModel;

    fn transcript(parts: &[&str]) -> Transcript {
        use speechkit::asr::{Segment, UtteranceId};
        Transcript::new(
            parts
                .iter()
                .enumerate()
                .map(|(i, text)| Segment {
                    utterance: UtteranceId(i as u64),
                    text: (*text).into(),
                    start: Duration::from_secs(i as u64),
                    end: Duration::from_secs(i as u64 + 1),
                })
                .collect(),
            Duration::from_secs(parts.len() as u64),
        )
    }

    #[test]
    fn a_held_key_inserts_whether_or_not_it_followed_a_tap() {
        assert_eq!(release(MIN_HOLD, false), Release::Insert);
        assert_eq!(release(Duration::from_secs(4), true), Release::Insert);
    }

    #[test]
    fn a_quick_second_tap_goes_hands_free_and_a_first_one_waits() {
        let quick = MIN_HOLD - Duration::from_millis(1);
        assert_eq!(release(quick, true), Release::HandsFree);
        assert_eq!(release(quick, false), Release::Tap);
    }

    #[test]
    fn a_double_tap_counts_from_release_to_the_next_press() {
        let released = Instant::now();
        assert!(is_second_tap(released, released + DOUBLE_TAP - Duration::from_millis(1)));
        assert!(!is_second_tap(released, released + DOUBLE_TAP));
        // An event stamped before the release, as by a lagging listener.
        assert!(is_second_tap(released + Duration::from_millis(5), released));
    }

    #[test]
    fn a_long_recording_gets_as_long_to_finish_as_it_ran() {
        assert_eq!(finish_deadline(Duration::from_secs(3)), FINISH_FOR);
        assert_eq!(finish_deadline(HANDS_FREE_FOR), HANDS_FREE_FOR);
    }

    #[test]
    fn processing_keeps_raw_text_and_applies_the_target_apps_dictionary() {
        let mut settings = Settings::default();
        settings.dictionary.push(crate::settings::DictionaryEntry {
            word: "Viary".into(),
            sounds_like: vec!["hello".into()],
            apps: vec!["Slack".into()],
            ..Default::default()
        });
        let transcript = transcript(&["HELLO", "world."]);
        assert_eq!(
            finish(&settings, "Slack", None, transcript.clone()),
            "Viary world."
        );
        assert_eq!(
            finish(&settings, "Mail", None, transcript.clone()),
            "HELLO world."
        );
        assert_eq!(transcript.text(), "HELLO world.");
    }

    #[test]
    fn a_cjk_mishearing_needs_an_alias_and_preserves_the_raw_transcript() {
        let mut settings = Settings::default();
        settings.dictionary.push(crate::settings::DictionaryEntry {
            word: "通义千问".into(),
            ..Default::default()
        });
        let transcript = transcript(&["通易千问。"]);
        assert_eq!(
            finish(&settings, "ChatGPT", None, transcript.clone()),
            "通易千问。"
        );
        settings.dictionary[0].sounds_like.push("通易千问".into());
        assert_eq!(
            finish(&settings, "ChatGPT", None, transcript.clone()),
            "通义千问。"
        );
        assert_eq!(transcript.text(), "通易千问。");
    }

    #[test]
    fn processing_joins_mixed_language_segments_and_skips_empty_text() {
        let transcript = transcript(&["你好，", "世界", "", "hello", "world", " "]);
        assert_eq!(
            finish(&Settings::default(), "", None, transcript),
            "你好，世界hello world"
        );
    }

    #[test]
    fn dictionary_replacements_can_span_recognition_boundaries() {
        let mut settings = Settings::default();
        settings.dictionary.push(crate::settings::DictionaryEntry {
            word: "SpeechKit".into(),
            sounds_like: vec!["speech kit".into()],
            ..Default::default()
        });
        let transcript = transcript(&["speech", "kit is fast"]);
        assert_eq!(
            finish(&settings, "", None, transcript.clone()),
            "SpeechKit is fast"
        );
        assert_eq!(transcript.text(), "speech kit is fast");
    }

    fn with_local() -> Settings {
        let mut settings = Settings::default();
        settings.local_models.push(LocalModel {
            id: "a".into(),
            name: "sense-voice".into(),
            path: "/m".into(),
            family: "sense-voice".into(),
            size_bytes: 0,
        });
        settings
    }

    #[test]
    fn a_cloud_failure_offers_on_device() {
        let (id, label) = alternative(&with_local(), Some(OPENAI), &[]).unwrap();
        assert_eq!(id, "local:a");
        assert_eq!(label, "Use on-device");
    }

    #[test]
    fn no_alternative_when_nothing_else_is_set_up() {
        assert!(alternative(&with_local(), Some("local:a"), &[]).is_none());
    }

    #[test]
    fn an_engine_that_failed_to_load_is_not_offered_again() {
        let tried = [OPENAI.to_owned(), "local:a".to_owned()];
        assert!(alternative(&with_local(), Some(OPENAI), &tried).is_none());
    }

    #[test]
    fn network_failures_are_retryable() {
        let info = EngineInfo {
            id: OPENAI.into(),
            name: "m".into(),
            kind: "OpenAI".into(),
            on_device: false,
            live: false,
            punctuation: "native",
            language_override: true,
            dictionary: "prompt",
        };
        let (message, retry) = failure_message(
            &SpeechError::backend("openai-http", true, "reset"),
            Some(&info),
        );
        assert_eq!(message, "Connection lost · audio kept");
        assert!(retry);
        let (_, retry) = failure_message(&SpeechError::InvalidInput("no key".into()), Some(&info));
        assert!(!retry);
    }

    #[test]
    fn retry_policy_keeps_timeouts_retryable_and_rejects_permanent_failures() {
        for (error, expected) in [
            (SpeechError::DeadlineExceeded, true),
            (SpeechError::Capacity, true),
            (SpeechError::backend("local", false, "broken"), false),
            (SpeechError::InvalidModel("missing".into()), false),
            (SpeechError::Unsupported("language".into()), false),
            (SpeechError::Cancelled, false),
        ] {
            assert_eq!(failure_message(&error, None).1, expected, "{error}");
        }
        assert_eq!(
            failure_message(&SpeechError::DeadlineExceeded, None).0,
            "Timed out · audio kept"
        );
    }
}
