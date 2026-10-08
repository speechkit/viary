//! Recording a voice note.
//!
//! A note is recorded in parts. Pausing ends the running part, and a long
//! recording rolls over to a new part every ten minutes at a quiet moment
//! (at eleven, mid-speech if need be), because speechkit keeps a
//! listening's whole recording in memory at the microphone's rate. The next
//! part takes over on the same microphone, so nothing is lost between them. Each finished part's audio goes to disk at once, and
//! its segments are moved by the length of the audio before it, so passage
//! times match the saved WAV.
//!
//! The recorder lives in the backend, so ⌥⌘N works before the window is up,
//! and a recording keeps going with the window closed.

use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use rubato::{
    Async, FixedAsync, Resampler as _, SincInterpolationParameters, WindowFunction,
    audioadapter_buffers::direct::InterleavedSlice,
};
use serde::Serialize;
use speechkit::{
    AudioBuffer, SampleRate,
    asr::{AsrUpdate, Segment},
    io::{Capture, CaptureOptions},
};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    App, caps, dictionary,
    engines::LoadedEngine,
    history::{self, EngineLabel},
    notes::{self, Note, Passage, UNTITLED},
    recording::Recording,
};

/// A part rolls over after this long, at the next quiet moment...
const PART_LENGTH: Duration = Duration::from_secs(10 * 60);
/// ...or here, if nobody pauses.
const PART_FORCE: Duration = Duration::from_secs(11 * 60);
/// The most audio one part's listening holds.
const PART_MAX: Duration = Duration::from_secs(12 * 60);
/// How long a note may wait for the voice engine to load.
const ENGINE_WAIT: Duration = Duration::from_secs(120);
/// The audio the microphone holds meanwhile: a little more than the wait,
/// so an engine ready just at its end still finds the start of the note.
const ENGINE_WAIT_HISTORY: Duration = Duration::from_secs(130);
/// A note stops and saves itself here.
const MAX_NOTE: Duration = Duration::from_secs(3 * 60 * 60);
/// A part whose recognition ends sooner than this is not followed by
/// another automatically: it would only end again.
const RECOVER_AFTER: Duration = Duration::from_secs(5);
/// The audio a part's microphone keeps, so the next part can start on it
/// where the running one is.
const HANDOVER: Duration = Duration::from_secs(5);
/// Below this level (RMS) for `QUIET_FOR`, a part can roll over.
const QUIET_LEVEL: f32 = 0.01;
const QUIET_FOR: Duration = Duration::from_millis(400);
/// How long a part may take to finish once its input ends.
const FINISH_TIMEOUT: Duration = Duration::from_secs(90);
/// The rate a note's audio is saved at: what the engines hear, and plenty
/// for speech.
const NOTE_RATE: SampleRate = SampleRate::HZ_16000;
/// Bars in a saved note's waveform.
const PEAKS: usize = 120;
/// The live view shows the tail of the transcript.
const LIVE_LINES: usize = 40;
/// The app name notes use for the dictionary and polish.
const APP: &str = "Voice Notes";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Idle,
    Recording,
    Paused,
    Finishing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum StepState {
    Done,
    Now,
    Todo,
}

#[derive(Debug, Clone, Serialize)]
struct Step {
    label: &'static str,
    state: StepState,
}

/// What the window shows of the recorder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderState {
    phase: Phase,
    id: Option<String>,
    title: String,
    elapsed_ms: u64,
    running_since: Option<u64>,
    marks: Vec<u64>,
    steps: Vec<Step>,
    error: Option<String>,
    /// When `error` happened (epoch ms), so a window opened later can tell
    /// a fresh error from an old one.
    error_at: Option<u64>,
    saved: Option<String>,
    /// Recording, while the voice engine finishes loading.
    waiting_for_engine: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveLine {
    start_ms: u64,
    text: String,
}

#[derive(Default)]
struct Live {
    lines: Vec<LiveLine>,
    partial: String,
    /// Where the speech behind `partial` started, as speechkit reported.
    partial_start: Option<u64>,
}

/// The live transcript as the window shows it: its tail.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LivePayload {
    id: String,
    lines: Vec<LiveLine>,
    partial: String,
    partial_start_ms: Option<u64>,
}

impl Live {
    fn payload(&self, id: &str) -> LivePayload {
        let tail = self.lines.len().saturating_sub(LIVE_LINES);
        LivePayload {
            id: id.to_owned(),
            lines: self.lines[tail..].to_vec(),
            partial: self.partial.clone(),
            partial_start_ms: self.partial_start,
        }
    }
}

struct Part {
    index: usize,
    /// The microphone the part listens on; the next part takes over on it.
    capture: Capture,
    recording: Recording,
    started: Instant,
    started_at: u64,
    /// The latest level, as f32 bits, for finding a quiet moment.
    level: Arc<AtomicU32>,
    quiet_since: Option<Instant>,
}

/// A finished part: its segments, times from its own start, and its audio
/// on disk as 16-bit PCM.
struct PartOut {
    segments: Vec<Segment>,
    pcm: PathBuf,
    rate: u32,
    frames: u64,
    /// Recorded time before the part, by the clock.
    clock_ms: u64,
    error: Option<String>,
    /// The audio, while it could not be written to `pcm`: kept to try again
    /// when the note is saved.
    unsaved: Option<AudioBuffer>,
}

/// Where a part's speech goes in the note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Its audio starts here in the joined audio.
    Audio(u64),
    /// Its audio is not in the joined audio, which goes on from here: its
    /// text is kept, at this point, without a span of audio.
    Gap(u64),
}

impl Place {
    /// Where a time `into` the part goes.
    fn at(self, into: u64) -> u64 {
        match self {
            Self::Audio(offset) => offset + into,
            Self::Gap(at) => at,
        }
    }
}

/// The microphone running before the engine has loaded; the first part
/// listens from its start once it has.
struct Waiting {
    capture: Capture,
    started: Instant,
    started_at: u64,
}

struct Active {
    id: String,
    title: String,
    created_at: u64,
    /// None until the engine that was loading at the start is ready.
    engine: Option<Arc<LoadedEngine>>,
    waiting: Option<Waiting>,
    part: Option<Part>,
    parts: Vec<JoinHandle<PartOut>>,
    /// Recorded time before the running part, by the clock.
    elapsed_ms: u64,
    /// Each mark's part and time into it.
    marks: Vec<(usize, u64)>,
    /// Each mark as the clock showed it, for the window.
    shown_marks: Vec<u64>,
    live: Arc<Mutex<Live>>,
}

struct Finishing {
    id: String,
    title: String,
    elapsed_ms: u64,
    steps: Vec<Step>,
}

#[derive(Default)]
struct Inner {
    active: Option<Active>,
    /// Stopped notes still being saved, oldest first. A new note can be
    /// recorded while earlier ones finish.
    finishing: Vec<Finishing>,
    /// The last error, and when it happened.
    error: Option<(String, u64)>,
    saved: Option<String>,
}

/// An error that happened now.
fn failure(text: impl Into<String>) -> Option<(String, u64)> {
    Some((text.into(), history::now_ms()))
}

#[derive(Default)]
pub struct Recorder {
    inner: Mutex<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl Recorder {
    pub fn state(&self) -> RecorderState {
        let inner = lock(&self.inner);
        let (error, error_at) = inner.error.clone().unzip();
        let saved = inner.saved.clone();
        if let Some(a) = &inner.active {
            return RecorderState {
                phase: if a.part.is_some() || a.waiting.is_some() {
                    Phase::Recording
                } else {
                    Phase::Paused
                },
                id: Some(a.id.clone()),
                title: a.title.clone(),
                elapsed_ms: a.elapsed_ms,
                running_since: a
                    .part
                    .as_ref()
                    .map(|p| p.started_at)
                    .or(a.waiting.as_ref().map(|w| w.started_at)),
                marks: a.shown_marks.clone(),
                steps: Vec::new(),
                error,
                error_at,
                saved,
                waiting_for_engine: a.waiting.is_some(),
            };
        }
        if let Some(f) = inner.finishing.last() {
            return RecorderState {
                phase: Phase::Finishing,
                id: Some(f.id.clone()),
                title: f.title.clone(),
                elapsed_ms: f.elapsed_ms,
                running_since: None,
                marks: Vec::new(),
                steps: f.steps.clone(),
                error,
                error_at,
                saved,
                waiting_for_engine: false,
            };
        }
        RecorderState {
            phase: Phase::Idle,
            id: None,
            title: String::new(),
            elapsed_ms: 0,
            running_since: None,
            marks: Vec::new(),
            steps: Vec::new(),
            error,
            error_at,
            saved,
            waiting_for_engine: false,
        }
    }

    /// The live transcript of the note being recorded, for a window that
    /// opens mid-recording; `note-live` keeps it current from there.
    pub fn live(&self) -> Option<LivePayload> {
        let inner = lock(&self.inner);
        let active = inner.active.as_ref()?;
        let payload = lock(&active.live).payload(&active.id);
        Some(payload)
    }

    fn changed(&self, app: &AppHandle) {
        let _ = app.emit("note-recorder", self.state());
    }

    /// Starts a note, unless one is already being recorded.
    ///
    /// # Errors
    ///
    /// No voice engine, or the microphone cannot be opened.
    pub fn start(&self, app: &AppHandle) -> Result<(), String> {
        let state = app.state::<App>();
        let mut inner = lock(&self.inner);
        if inner.active.is_some() {
            return Ok(());
        }
        inner.error = None;
        inner.saved = None;
        // A loading engine is waited for: recording starts now.
        let engine = state.engines.current();
        if engine.is_none() && state.engines.status().loading.is_none() {
            let message = "Choose a voice engine first";
            inner.error = failure(message);
            drop(inner);
            self.changed(app);
            return Err(message.into());
        }
        let id = history::now_ms().to_string();
        let mut active = Active {
            id: id.clone(),
            title: UNTITLED.into(),
            created_at: history::now_ms(),
            engine,
            waiting: None,
            part: None,
            parts: Vec::new(),
            elapsed_ms: 0,
            marks: Vec::new(),
            shown_marks: Vec::new(),
            live: Arc::default(),
        };
        let started = if active.engine.is_some() {
            start_part(app, &mut active)
        } else {
            start_waiting(app, &mut active)
        };
        if let Err(error) = started {
            inner.error = failure(error.clone());
            drop(inner);
            self.changed(app);
            return Err(error);
        }
        inner.active = Some(active);
        drop(inner);
        self.changed(app);
        spawn_ticker(app.clone(), id);
        Ok(())
    }

    /// Pausing waits until the engine has loaded, like stopping.
    pub fn pause(&self, app: &AppHandle) {
        let mut inner = lock(&self.inner);
        if let Some(active) = inner.active.as_mut().filter(|a| a.waiting.is_none()) {
            end_part(app, active);
        }
        drop(inner);
        self.changed(app);
    }

    pub fn resume(&self, app: &AppHandle) {
        let mut inner = lock(&self.inner);
        if let Some(active) = &mut inner.active
            && active.part.is_none()
            && let Err(error) = start_part(app, active)
        {
            inner.error = failure(error);
        }
        drop(inner);
        self.changed(app);
    }

    pub fn mark(&self, app: &AppHandle) {
        let mut inner = lock(&self.inner);
        if let Some(active) = &mut inner.active {
            // While waiting for the engine, the first part has begun.
            let running = match (&active.part, &active.waiting) {
                (Some(part), _) => Some((part.index, part.started)),
                (None, Some(waiting)) => Some((0, waiting.started)),
                (None, None) => None,
            };
            if let Some((index, started)) = running {
                let into = ms(started.elapsed());
                active.marks.push((index, into));
                active.shown_marks.push(active.elapsed_ms + into);
            }
        }
        drop(inner);
        self.changed(app);
    }

    /// Throws the recording away.
    pub fn discard(&self, app: &AppHandle) {
        let active = lock(&self.inner).active.take();
        if let Some(mut active) = active {
            // Dropping a recording cancels it.
            active.part = None;
            let parts = std::mem::take(&mut active.parts);
            std::thread::spawn(move || {
                for part in parts {
                    if let Ok(out) = part.join() {
                        let _ = fs::remove_file(out.pcm);
                    }
                }
            });
        }
        self.changed(app);
    }

    /// Stops recording, and finishes and saves the note in the background.
    /// Stopping waits until the engine has loaded: speechkit can only hand
    /// over the audio held so far to a listening, and a listening needs the
    /// engine. Discarding works at any time.
    pub fn stop(&self, app: &AppHandle) {
        let mut inner = lock(&self.inner);
        if inner.active.as_ref().is_none_or(|a| a.waiting.is_some()) {
            return;
        }
        let Some(mut active) = inner.active.take() else {
            return;
        };
        end_part(app, &mut active);
        let settings = app.state::<App>().settings();
        let caps = caps::current();
        let mut labels = vec!["Saving the recording"];
        if caps.speakers {
            labels.push("Finding who spoke when");
        }
        if caps.word_timings {
            labels.push("Timing every word");
        }
        if settings.polish.enabled {
            labels.push("Writing a summary");
        }
        inner.finishing.push(Finishing {
            id: active.id.clone(),
            title: active.title.clone(),
            elapsed_ms: active.elapsed_ms,
            steps: labels
                .into_iter()
                .map(|label| Step {
                    label,
                    state: StepState::Todo,
                })
                .collect(),
        });
        drop(inner);
        self.changed(app);
        let app = app.clone();
        let _ = std::thread::Builder::new()
            .name("viary-note".into())
            .spawn(move || finish(&app, active));
    }

    /// Renames the note being recorded or finished. Returns false if it is
    /// neither.
    pub fn rename(&self, app: &AppHandle, id: &str, title: &str) -> bool {
        let mut inner = lock(&self.inner);
        let renamed = if let Some(active) = inner.active.as_mut().filter(|a| a.id == id) {
            active.title = title.into();
            true
        } else if let Some(finishing) = inner.finishing.iter_mut().find(|f| f.id == id) {
            finishing.title = title.into();
            true
        } else {
            false
        };
        drop(inner);
        if renamed {
            self.changed(app);
        }
        renamed
    }

    /// A part of note `id` could not be written to disk: pauses the note if
    /// it is still being recorded, and shows why. The part's audio is kept
    /// and written when the note is saved.
    fn storage_failed(&self, app: &AppHandle, id: &str, error: &std::io::Error) {
        let mut inner = lock(&self.inner);
        let Some(active) = inner.active.as_mut().filter(|a| a.id == id) else {
            return;
        };
        end_part(app, active);
        inner.error = failure(format!(
            "Part of the recording couldn't be saved to disk ({error}), so recording is paused; it is saved when you stop"
        ));
        drop(inner);
        self.changed(app);
    }

    fn step(&self, app: &AppHandle, id: &str, label: &str) {
        let mut inner = lock(&self.inner);
        if let Some(finishing) = inner.finishing.iter_mut().find(|f| f.id == id) {
            let mut reached = false;
            for step in &mut finishing.steps {
                step.state = if step.label == label {
                    reached = true;
                    StepState::Now
                } else if reached {
                    StepState::Todo
                } else {
                    StepState::Done
                };
            }
        }
        drop(inner);
        self.changed(app);
    }
}

/// Where a new part listens.
struct PartStart {
    capture: Capture,
    /// Capture time to listen from, or now.
    from: Option<Duration>,
    started: Instant,
    started_at: u64,
    /// Recorded time before the part, for its live lines.
    offset: u64,
    index: usize,
}

/// A part listening on `start.capture`.
fn open_part(app: &AppHandle, active: &Active, start: PartStart) -> Result<Part, String> {
    let settings = app.state::<App>().settings();
    let engine = active
        .engine
        .clone()
        .ok_or("The voice engine is still loading")?;
    let options = engine.session_options(&settings, APP);
    let offset = start.offset;
    // The listening's times are capture times; the part's start from here.
    let base = start.from.unwrap_or(Duration::ZERO);
    let level = Arc::new(AtomicU32::new(0));
    let (shared_level, live, id, emitter) = (
        level.clone(),
        active.live.clone(),
        active.id.clone(),
        app.clone(),
    );
    let on_update = move |now: f32, updates: &[AsrUpdate]| {
        shared_level.store(now.to_bits(), Ordering::Relaxed);
        let _ = emitter.emit("note-level", now);
        if updates.is_empty() {
            return;
        }
        let mut live = lock(&live);
        for update in updates {
            match update {
                AsrUpdate::Segment(segment) => {
                    let text = segment.text.trim();
                    if !text.is_empty() {
                        live.lines.push(LiveLine {
                            start_ms: offset + ms(segment.start.saturating_sub(base)),
                            text: text.into(),
                        });
                    }
                    live.partial.clear();
                    live.partial_start = None;
                }
                AsrUpdate::Partial(partial) => live.partial.clone_from(&partial.text),
                AsrUpdate::SpeechStarted { at } => {
                    live.partial_start = Some(offset + ms(at.saturating_sub(base)));
                }
                AsrUpdate::Closed(_) => live.partial.clear(),
                _ => {}
            }
        }
        let _ = emitter.emit("note-live", live.payload(&id));
    };
    let recording = Recording::listen_on(
        &start.capture,
        &engine.engine,
        options,
        PART_MAX,
        start.from,
        on_update,
    )
    .map_err(|error| {
        tracing::warn!(%error, "cannot start a voice note");
        "Cannot start recording".to_owned()
    })?;
    Ok(Part {
        index: start.index,
        capture: start.capture,
        recording,
        started: start.started,
        started_at: start.started_at,
        level,
        quiet_since: None,
    })
}

/// Starts the next part of `active`: on the microphone held while the
/// engine loaded, from its start, or on the microphone opened anew.
fn start_part(app: &AppHandle, active: &mut Active) -> Result<(), String> {
    if active.engine.is_none() {
        return Err("The voice engine is still loading".into());
    }
    let start = if let Some(waited) = active.waiting.take() {
        // The audio recorded while the engine loaded is transcribed first.
        PartStart {
            capture: waited.capture,
            from: Some(Duration::ZERO),
            started: waited.started,
            started_at: waited.started_at,
            offset: active.elapsed_ms,
            index: active.parts.len(),
        }
    } else {
        let settings = app.state::<App>().settings();
        let microphone = Recording::open_microphone(settings.microphone.as_deref())
            .map_err(|_| "Microphone unavailable".to_owned())?;
        let capture = microphone
            .capture(CaptureOptions::default().with_history(HANDOVER))
            .map_err(|error| {
                tracing::warn!(%error, "cannot start a voice note");
                "Cannot start recording".to_owned()
            })?;
        PartStart {
            capture,
            from: None,
            started: Instant::now(),
            started_at: history::now_ms(),
            offset: active.elapsed_ms,
            index: active.parts.len(),
        }
    };
    active.part = Some(open_part(app, active, start)?);
    Ok(())
}

/// After the running part stopped hearing: with its microphone `lost`, goes
/// on with the microphone opened anew (the chosen one, or the default);
/// otherwise (its recognition ended) hands over to a new part on the same
/// microphone, unless it ended within moments of starting, which would only
/// repeat. Returns the error to show; when nothing can go on, the note is
/// paused.
fn recover(app: &AppHandle, active: &mut Active, lost: bool, running: Duration) -> Option<(String, u64)> {
    if lost {
        tracing::warn!("the microphone of a voice note was lost");
        end_part(app, active);
        return match start_part(app, active) {
            Ok(()) => failure("The microphone was disconnected; recording goes on with the one available"),
            Err(_) => failure("The microphone was disconnected, so recording is paused"),
        };
    }
    tracing::warn!("a voice note's recognition ended by itself");
    if running < RECOVER_AFTER {
        end_part(app, active);
        return failure("Recognition stopped, so recording is paused");
    }
    roll_part(app, active).err().and_then(failure)
}

/// Rolls the running part over without a gap: the next part listens on the
/// same microphone from where it is now, then the running part stops there
/// (the two share at most a buffer of audio). If the next part cannot start
/// that way, it opens the microphone anew.
fn roll_part(app: &AppHandle, active: &mut Active) -> Result<(), String> {
    let Some(running) = active.part.take() else {
        return Ok(());
    };
    // Where the running part stops hearing: now, or where its recognition
    // already ended by itself (the capture still holds that audio).
    let handover = running
        .recording
        .end()
        .unwrap_or_else(|| running.capture.position());
    let next = open_part(
        app,
        active,
        PartStart {
            capture: running.capture.clone(),
            from: Some(handover),
            started: Instant::now(),
            started_at: history::now_ms(),
            offset: active.elapsed_ms + ms(running.started.elapsed()),
            index: running.index + 1,
        },
    );
    // The running part stops a little after the handover; what it heard
    // past it belongs to the next part.
    finish_part(app, active, running, Some(handover));
    match next {
        Ok(part) => {
            active.part = Some(part);
            Ok(())
        }
        Err(error) => {
            tracing::warn!(%error, "cannot hand a voice note over to its next part; reopening the microphone");
            start_part(app, active)
        }
    }
}

/// Starts the microphone before the engine has loaded, holding its audio.
fn start_waiting(app: &AppHandle, active: &mut Active) -> Result<(), String> {
    let settings = app.state::<App>().settings();
    let microphone = Recording::open_microphone(settings.microphone.as_deref())
        .map_err(|_| "Microphone unavailable".to_owned())?;
    let capture = microphone
        .capture(CaptureOptions::default().with_history(ENGINE_WAIT_HISTORY))
        .map_err(|error| {
            tracing::warn!(%error, "cannot start a voice note");
            "Cannot start recording".to_owned()
        })?;
    active.waiting = Some(Waiting {
        capture,
        started: Instant::now(),
        started_at: history::now_ms(),
    });
    Ok(())
}

/// Ends the running part; it finishes on a worker and writes its audio.
fn end_part(app: &AppHandle, active: &mut Active) {
    if let Some(part) = active.part.take() {
        finish_part(app, active, part, None);
    }
}

/// Stops `part`, which finishes on a worker and writes its audio. With
/// `until`, the part ends at that capture time, where the next one starts.
fn finish_part(app: &AppHandle, active: &mut Active, part: Part, until: Option<Duration>) {
    part.recording.stop();
    let clock_ms = active.elapsed_ms;
    active.elapsed_ms += ms(part.started.elapsed());
    let pcm = app
        .state::<App>()
        .notes
        .dir()
        .join(format!("{}.part{}.pcm", active.id, part.index));
    let recording = part.recording;
    let (app, id) = (app.clone(), active.id.clone());
    active.parts.push(std::thread::spawn(move || {
        let origin = recording.origin();
        let (result, mut audio) = recording.finish(FINISH_TIMEOUT);
        let (mut segments, error) = match result {
            Ok(transcript) => (transcript.segments, None),
            Err(failure) => {
                tracing::warn!(error = %failure.error, "part of a voice note failed");
                (failure.confirmed.segments, Some(failure.error.to_string()))
            }
        };
        within_part(&mut audio, &mut segments, origin, until);
        let audio = match at_note_rate(&audio) {
            Ok(Some(resampled)) => resampled,
            Ok(None) => audio,
            Err(error) => {
                // Kept at its own rate; it joins the WAV only if that matches.
                tracing::error!(%error, "cannot resample part of a voice note");
                audio
            }
        };
        let rate = audio.sample_rate.hz();
        let (frames, unsaved) = match write_pcm(&pcm, &audio) {
            Ok(frames) => (frames, None),
            Err(error) => {
                tracing::error!(%error, "cannot save part of a voice note");
                // A partial file would not match its frame count.
                let _ = fs::remove_file(&pcm);
                // The disk is likely full: stop adding to it, and say so.
                app.state::<App>().recorder.storage_failed(&app, &id, &error);
                (0, Some(audio))
            }
        };
        PartOut {
            segments,
            pcm,
            rate,
            frames,
            clock_ms,
            error,
            unsaved,
        }
    }));
}

/// Makes a part's segment times run from its own start (`origin`, in capture
/// time), and with `until` cuts the part there: its audio after it, and the
/// segments that start after it, belong to the next part.
fn within_part(audio: &mut AudioBuffer, segments: &mut Vec<Segment>, origin: Duration, until: Option<Duration>) {
    if let Some(until) = until {
        let keep = audio.sample_rate.frames_in(until.saturating_sub(origin));
        audio
            .samples
            .truncate(usize::try_from(keep).unwrap_or(usize::MAX));
        segments.retain(|s| s.start < until);
    }
    for segment in segments.iter_mut() {
        segment.start = segment.start.saturating_sub(origin);
        segment.end = segment.end.saturating_sub(origin);
    }
}

/// Rolls long recordings over to a new part, and stops at `MAX_NOTE`.
fn spawn_ticker(app: AppHandle, id: String) {
    let _ = std::thread::Builder::new()
        .name("viary-note-ticker".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(200));
                let recorder = &app.state::<App>().recorder;
                let mut inner = lock(&recorder.inner);
                let Some(active) = inner.active.as_mut().filter(|a| a.id == id) else {
                    return;
                };
                if let Some(waiting) = &active.waiting {
                    let engines = &app.state::<App>().engines;
                    let _ = app.emit("note-level", waiting.capture.level());
                    if let Some(engine) = engines.current() {
                        active.engine = Some(engine);
                        if let Err(error) = start_part(&app, active) {
                            // Paused; the user can resume.
                            inner.error = failure(error);
                        }
                    } else if engines.status().loading.is_none()
                        || waiting.started.elapsed() >= ENGINE_WAIT
                    {
                        // Nothing to transcribe with: drop the audio.
                        inner.active = None;
                        inner.error =
                            failure("The voice engine did not load, so the note was not kept");
                    } else {
                        continue;
                    }
                    drop(inner);
                    recorder.changed(&app);
                    continue;
                }
                let Some(part) = &mut active.part else {
                    continue;
                };
                let running = part.started.elapsed();
                if Duration::from_millis(active.elapsed_ms) + running >= MAX_NOTE {
                    drop(inner);
                    recorder.stop(&app);
                    return;
                }
                // A part that hears nothing more must not keep the clock
                // running until the next rollover.
                let lost = part.capture.device_lost();
                if lost || part.recording.ended() {
                    inner.error = recover(&app, active, lost, running);
                    drop(inner);
                    recorder.changed(&app);
                    continue;
                }
                if f32::from_bits(part.level.load(Ordering::Relaxed)) < QUIET_LEVEL {
                    part.quiet_since.get_or_insert_with(Instant::now);
                } else {
                    part.quiet_since = None;
                }
                if rolls_over(running, part.quiet_since.map(|at| at.elapsed())) {
                    if let Err(error) = roll_part(&app, active) {
                        // Paused; the user can resume.
                        inner.error = failure(error);
                    }
                    drop(inner);
                    recorder.changed(&app);
                }
            }
        });
}

/// Whether a part that has run `running`, quiet for `quiet_for` (None while
/// there is sound), rolls over now: at a quiet moment past `PART_LENGTH`,
/// or past `PART_FORCE` even mid-speech, well before its listening runs out
/// of room at `PART_MAX`.
fn rolls_over(running: Duration, quiet_for: Option<Duration>) -> bool {
    running >= PART_FORCE
        || (running >= PART_LENGTH && quiet_for.is_some_and(|quiet| quiet >= QUIET_FOR))
}

/// Joins the parts into `wav` (see `write_wav`). The parts are the only
/// copy of the audio until the WAV is whole, so they are removed only once
/// it is; if it cannot be written they stay, and the error says where.
fn save_audio(wav: &Path, parts: &[PartOut]) -> Result<(u32, u64, Vec<f32>), String> {
    match write_wav(wav, parts) {
        Ok(saved) => {
            for part in parts {
                let _ = fs::remove_file(&part.pcm);
            }
            Ok(saved)
        }
        Err(error) => {
            tracing::error!(%error, "cannot save a voice note's audio");
            let _ = fs::remove_file(wav);
            Err(format!(
                "The recording could not be saved ({error}); its parts are kept, and Viary tries again when it next starts"
            ))
        }
    }
}

/// Writes the parts whose audio could not be written while recording. Returns
/// the error to show if one still cannot be: its audio is then lost.
fn write_unsaved(parts: &mut [PartOut]) -> Option<String> {
    let mut lost = None;
    for part in parts.iter_mut() {
        let Some(audio) = part.unsaved.take() else {
            continue;
        };
        match write_pcm(&part.pcm, &audio) {
            Ok(frames) => part.frames = frames,
            Err(error) => {
                tracing::error!(%error, "cannot save part of a voice note");
                let _ = fs::remove_file(&part.pcm);
                lost = Some(format!("Part of the recording could not be saved ({error}), so its audio is missing"));
            }
        }
    }
    lost
}

/// The parts with audio on disk, as a note keeps them until they are joined.
fn pending(parts: &[PartOut]) -> Vec<notes::PendingPart> {
    parts
        .iter()
        .filter(|p| p.frames > 0 && p.pcm.exists())
        .filter_map(|p| {
            Some(notes::PendingPart {
                file: p.pcm.file_name()?.to_string_lossy().into_owned(),
                rate: p.rate,
            })
        })
        .collect()
}

/// Joins the recordings left in parts by an earlier failed save, in the
/// background. A part that is gone is left out; one that cannot be joined
/// yet stays for the next start.
pub fn retry_pending(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("viary-note-audio".into())
        .spawn(move || {
            let notes = &app.state::<App>().notes;
            for note in notes.with_pending_audio() {
                let parts: Vec<PartOut> = note
                    .pending_audio
                    .iter()
                    .filter_map(|p| {
                        let pcm = notes.dir().join(&p.file);
                        let frames = fs::metadata(&pcm).ok()?.len() / 2;
                        Some(PartOut {
                            segments: Vec::new(),
                            pcm,
                            rate: p.rate,
                            frames,
                            clock_ms: 0,
                            error: None,
                            unsaved: None,
                        })
                    })
                    .collect();
                let audio_name = format!("{}.wav", note.id);
                let joined = save_audio(&notes.dir().join(&audio_name), &parts);
                if let Err(error) = &joined {
                    tracing::warn!(%error, "the recording of a voice note is still in parts");
                    continue;
                }
                notes.update(&note.id, |n| {
                    if let Ok((rate, frames, peaks)) = joined
                        && rate > 0
                    {
                        n.audio = Some(audio_name);
                        n.duration_ms = frames * 1000 / u64::from(rate);
                        n.peaks = peaks;
                    }
                    n.pending_audio.clear();
                });
                let _ = app.emit("notes-changed", ());
            }
        });
}

/// Saves a stopped note: its audio, passages, and summary.
fn finish(app: &AppHandle, mut active: Active) {
    let state = app.state::<App>();
    let recorder = &state.recorder;
    let settings = state.settings();
    let Some(engine) = active.engine.clone() else {
        // Stop waits for the engine, so every stopped note has one.
        tracing::error!("a voice note stopped before its engine loaded");
        lock(&recorder.inner)
            .finishing
            .retain(|f| f.id != active.id);
        recorder.changed(app);
        return;
    };
    recorder.step(app, &active.id, "Saving the recording");

    let mut parts: Vec<PartOut> = active
        .parts
        .drain(..)
        .filter_map(|part| part.join().ok())
        .collect();
    let unsaved_error = write_unsaved(&mut parts);
    let dir = state.notes.dir().to_owned();
    let audio_name = format!("{}.wav", active.id);
    let (saved, save_error) = match save_audio(&dir.join(&audio_name), &parts) {
        Ok(saved) => (saved, None),
        Err(error) => ((0, 0, Vec::new()), Some(error)),
    };
    let (rate, frames, peaks) = saved;
    let duration_ms = if rate > 0 {
        frames * 1000 / u64::from(rate)
    } else {
        active.elapsed_ms
    };

    // Each part's times start at its own beginning; move them by the audio
    // before it.
    let places = part_places(&parts);
    let words = &settings.dictionary;
    let mut passages: Vec<Passage> = parts
        .iter()
        .zip(places.iter().copied())
        .flat_map(|(part, place)| {
            part.segments.iter().filter_map(move |segment| {
                let text = dictionary::apply(words, APP, segment.text.trim());
                (!text.trim().is_empty()).then(|| Passage {
                    start_ms: place.at(ms(segment.start)),
                    end_ms: place.at(ms(segment.end)),
                    text,
                    speaker: None,
                    words: None,
                    original: None,
                })
            })
        })
        .collect();
    let marks: Vec<u64> = active
        .marks
        .iter()
        .filter_map(|(part, into)| Some(places.get(*part)?.at(*into)))
        // The clock and the audio can differ a little at the very end.
        .map(|mark| mark.min(duration_ms))
        .collect();
    let failed = parts.iter().filter_map(|p| p.error.clone()).next();

    // Dev only: speakers and word times from the scripted fixture.
    let mut speakers = Vec::new();
    if let Some((mock, who)) = caps::mock_passages(duration_ms) {
        recorder.step(app, &active.id, "Finding who spoke when");
        passages = mock;
        speakers = who;
        recorder.step(app, &active.id, "Timing every word");
    }

    let info = &engine.info;
    let mut note = Note {
        id: active.id.clone(),
        title: active.title.clone(),
        created_at: active.created_at,
        duration_ms,
        engine: EngineLabel {
            name: if caps::current().mock {
                format!("{} + fixture", info.name)
            } else {
                info.name.clone()
            },
            kind: info.kind.clone(),
            on_device: info.on_device,
        },
        passages,
        speakers,
        marks,
        summary: None,
        actions: Vec::new(),
        summary_error: None,
        peaks,
        audio: (frames > 0).then_some(audio_name),
        // Kept for another try when the WAV could not be written.
        pending_audio: if save_error.is_some() {
            pending(&parts)
        } else {
            Vec::new()
        },
    };

    let mut summary_title = None;
    if settings.polish.enabled && !note.passages.is_empty() {
        recorder.step(app, &active.id, "Writing a summary");
        match notes::summarize(&settings, &note) {
            Ok(summary) => {
                note.summary = Some(summary.summary);
                note.actions = summary.actions;
                summary_title = summary.title;
            }
            Err(error) => {
                tracing::warn!(%error, "no summary for a voice note");
                note.summary_error = Some(error);
            }
        }
    }

    let mut inner = lock(&recorder.inner);
    // The user may have renamed it while it finished.
    if let Some(at) = inner.finishing.iter().position(|f| f.id == note.id) {
        note.title = inner.finishing.remove(at).title;
    }
    if note.title == UNTITLED
        && let Some(title) = summary_title.or_else(|| notes::first_words(&note.passages))
    {
        note.title = title;
    }
    inner.saved = Some(note.id.clone());
    inner.error = unsaved_error
        .or(save_error)
        .or_else(|| {
            failed.map(|error| format!("Part of the recording could not be transcribed: {error}"))
        })
        .and_then(failure);
    drop(inner);
    state.notes.add(note);
    let _ = app.emit("notes-changed", ());
    recorder.changed(app);
}

/// The rate of the joined audio: the first part with audio sets it.
fn wav_rate(parts: &[PartOut]) -> Option<u32> {
    parts.iter().find(|p| p.frames > 0).map(|p| p.rate)
}

/// Whether `part` is in the joined audio at `rate`. A part recorded at
/// another rate (the microphone changed) would play at the wrong speed,
/// and one whose audio was not saved has nothing to add.
fn in_wav(part: &PartOut, rate: u32) -> bool {
    part.rate == rate && part.frames > 0
}

/// Where each part goes: where it starts in the joined audio, the length
/// of the audio before it measured in frames, not by the clock. A part
/// left out of the audio is a gap where the audio before it ends. With no
/// audio at all, parts go by the clock.
fn part_places(parts: &[PartOut]) -> Vec<Place> {
    let Some(rate) = wav_rate(parts) else {
        return parts.iter().map(|p| Place::Audio(p.clock_ms)).collect();
    };
    let mut at = 0;
    parts
        .iter()
        .map(|part| {
            if !in_wav(part, rate) {
                return Place::Gap(at);
            }
            let offset = at;
            at += part.frames * 1000 / u64::from(rate);
            Place::Audio(offset)
        })
        .collect()
}

/// `audio` at `NOTE_RATE`, or None when it already is. Every part is saved
/// at one rate, so parts recorded on different microphones (pausing and
/// switching, say) join into one WAV.
fn at_note_rate(audio: &AudioBuffer) -> Result<Option<AudioBuffer>, String> {
    let from = audio.sample_rate;
    if from == NOTE_RATE || audio.samples.is_empty() {
        return Ok(None);
    }
    let error = |e: &dyn std::fmt::Display| e.to_string();
    // As speechkit resamples what the engines hear.
    let parameters = SincInterpolationParameters::new(256, WindowFunction::BlackmanHarris2)
        .f_cutoff(0.95)
        .oversampling_factor(256);
    let mut resampler = Async::<f32>::new_sinc(
        f64::from(NOTE_RATE.hz()) / f64::from(from.hz()),
        1.0,
        &parameters,
        1024,
        1,
        FixedAsync::Input,
    )
    .map_err(|e| error(&e))?;
    let frames = audio.samples.len();
    let input = InterleavedSlice::new(&audio.samples[..], 1, frames).map_err(|e| error(&e))?;
    let mut samples = vec![0.0; resampler.process_all_needed_output_len(frames)];
    let room = samples.len();
    let mut output = InterleavedSlice::new_mut(&mut samples[..], 1, room).map_err(|e| error(&e))?;
    let (_, produced) = resampler
        .process_all_into_buffer(&input, &mut output, frames, None)
        .map_err(|e| error(&e))?;
    samples.truncate(produced);
    Ok(Some(AudioBuffer::new(NOTE_RATE, samples)))
}

/// Writes `audio` as 16-bit little-endian PCM; returns the frames written.
fn write_pcm(path: &Path, audio: &AudioBuffer) -> std::io::Result<u64> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut out = BufWriter::new(File::create(path)?);
    for &sample in &audio.samples {
        // Rounded and clamped to the 16-bit range.
        #[allow(clippy::cast_possible_truncation)]
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.write_all(&value.to_le_bytes())?;
    }
    out.flush()?;
    Ok(audio.samples.len() as u64)
}

/// Joins the parts' PCM into one WAV at the first part's rate, and measures
/// its loudness for the waveform; parts not `in_wav` are left out. Returns
/// the rate, frames, and peaks.
fn write_wav(path: &Path, parts: &[PartOut]) -> std::io::Result<(u32, u64, Vec<f32>)> {
    let Some(rate) = wav_rate(parts) else {
        return Ok((0, 0, Vec::new()));
    };
    let kept: Vec<&PartOut> = parts.iter().filter(|p| in_wav(p, rate)).collect();
    if parts.iter().any(|p| p.rate != rate && p.frames > 0) {
        tracing::warn!("the microphone changed during a voice note; part of its audio is left out");
    }
    let frames: u64 = kept.iter().map(|p| p.frames).sum();
    let bytes = u32::try_from(frames * 2)
        .map_err(|_| std::io::Error::other("recording too long for a WAV"))?;
    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + bytes).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?; // PCM
    out.write_all(&1u16.to_le_bytes())?; // mono
    out.write_all(&rate.to_le_bytes())?;
    out.write_all(&(rate * 2).to_le_bytes())?;
    out.write_all(&2u16.to_le_bytes())?;
    out.write_all(&16u16.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&bytes.to_le_bytes())?;

    let mut peaks = vec![0f32; PEAKS];
    let mut frame = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    for part in kept {
        let mut input = BufReader::new(File::open(&part.pcm)?);
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            for pair in buf[..n].as_chunks::<2>().0 {
                let value = f32::from(i16::from_le_bytes(*pair)).abs() / 32768.0;
                #[allow(clippy::cast_possible_truncation)]
                let bin = ((frame * PEAKS as u64) / frames.max(1)) as usize;
                let peak = &mut peaks[bin.min(PEAKS - 1)];
                *peak = peak.max(value);
                frame += 1;
            }
        }
    }
    out.flush()?;
    let loudest = peaks.iter().copied().fold(0f32, f32::max);
    if loudest > 0.0 {
        peaks.iter_mut().for_each(|p| *p /= loudest);
    }
    Ok((rate, frames, peaks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(dir: &Path, name: &str, samples: Vec<f32>) -> PartOut {
        let pcm = dir.join(name);
        let audio = AudioBuffer::new(SampleRate::HZ_16000, samples);
        let frames = write_pcm(&pcm, &audio).unwrap();
        PartOut {
            segments: Vec::new(),
            pcm,
            rate: 16_000,
            frames,
            clock_ms: 0,
            error: None,
            unsaved: None,
        }
    }

    #[test]
    fn parts_join_into_one_wav_with_offsets_from_their_audio() {
        let dir = std::env::temp_dir().join(format!("viary-notes-{}", std::process::id()));
        let parts = vec![
            part(&dir, "a.pcm", vec![0.5; 16_000 * 2]),
            part(&dir, "b.pcm", vec![-1.0; 8_000]),
        ];
        assert_eq!(
            part_places(&parts),
            vec![Place::Audio(0), Place::Audio(2_000)]
        );

        let wav = dir.join("note.wav");
        let (rate, frames, peaks) = write_wav(&wav, &parts).unwrap();
        assert_eq!((rate, frames), (16_000, 40_000));
        assert_eq!(fs::metadata(&wav).unwrap().len(), 44 + 40_000 * 2);
        // The loudest bin is the second part's, at full scale.
        assert_eq!(peaks.len(), PEAKS);
        assert!((peaks[PEAKS - 1] - 1.0).abs() < 1e-3);
        assert!((peaks[0] - 0.5).abs() < 1e-2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parts_left_out_of_the_wav_become_gaps() {
        let dir = std::env::temp_dir().join(format!("viary-notes-rate-{}", std::process::id()));
        let mut other = part(&dir, "b.pcm", vec![0.1; 48_000]);
        other.rate = 48_000;
        let mut unsaved = part(&dir, "c.pcm", Vec::new());
        unsaved.pcm = dir.join("missing.pcm");
        let parts = vec![
            part(&dir, "a.pcm", vec![0.5; 16_000]),
            other,
            unsaved,
            part(&dir, "d.pcm", vec![0.5; 8_000]),
        ];
        assert_eq!(
            part_places(&parts),
            vec![
                Place::Audio(0),
                Place::Gap(1_000),
                Place::Gap(1_000),
                Place::Audio(1_000)
            ]
        );
        assert_eq!(Place::Gap(1_000).at(5_000), 1_000);
        let (rate, frames, _) = write_wav(&dir.join("note.wav"), &parts).unwrap();
        assert_eq!((rate, frames), (16_000, 24_000));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_handed_over_part_is_cut_where_the_next_one_starts() {
        let secs = Duration::from_secs;
        let segment = |start: u64, end: u64| Segment {
            utterance: speechkit::asr::UtteranceId(0),
            text: "x".into(),
            start: secs(start),
            end: secs(end),
        };
        // A part from capture time 600 s to about 662 s, handed over at 660 s.
        let mut audio = AudioBuffer::new(NOTE_RATE, vec![0.1; 16_000 * 62]);
        let mut segments = vec![segment(601, 605), segment(658, 661), segment(660, 662)];
        within_part(&mut audio, &mut segments, secs(600), Some(secs(660)));
        assert_eq!(audio.samples.len(), 16_000 * 60);
        // Times from the part's start; speech that began after the handover
        // is the next part's.
        let times: Vec<_> = segments.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(times, vec![(secs(1), secs(5)), (secs(58), secs(61))]);
    }

    #[test]
    fn parts_roll_over_by_force_even_while_loud() {
        let min = |m: u64| Duration::from_secs(m * 60);
        // Continuous speech or noise: never quiet.
        assert!(!rolls_over(min(10), None));
        assert!(rolls_over(min(11), None));
        // A quiet moment past ten minutes.
        assert!(rolls_over(min(10), Some(QUIET_FOR)));
        assert!(!rolls_over(min(10), Some(QUIET_FOR / 2)));
        assert!(!rolls_over(min(9), Some(QUIET_FOR)));
    }

    #[test]
    fn parts_from_another_microphone_rate_join_the_wav() {
        let dir = std::env::temp_dir().join(format!("viary-notes-48k-{}", std::process::id()));
        let loud = AudioBuffer::new(SampleRate::new(48_000).unwrap(), vec![0.5; 48_000]);
        let resampled = at_note_rate(&loud).unwrap().unwrap();
        assert_eq!(resampled.sample_rate, NOTE_RATE);
        assert!(
            resampled.samples.len().abs_diff(16_000) <= 2,
            "{}",
            resampled.samples.len()
        );
        assert!(
            at_note_rate(&AudioBuffer::new(NOTE_RATE, vec![0.1; 10]))
                .unwrap()
                .is_none()
        );

        let pcm = dir.join("b.pcm");
        let frames = write_pcm(&pcm, &resampled).unwrap();
        let parts = vec![
            part(&dir, "a.pcm", vec![0.5; 16_000]),
            PartOut {
                segments: Vec::new(),
                pcm,
                rate: 16_000,
                frames,
                clock_ms: 1_000,
                error: None,
                unsaved: None,
            },
        ];
        let (rate, frames, _) = write_wav(&dir.join("note.wav"), &parts).unwrap();
        assert_eq!(rate, 16_000);
        assert!(
            frames.abs_diff(32_000) <= 2,
            "both seconds are kept: {frames}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn audio_that_could_not_be_written_is_written_when_the_note_is_saved() {
        let dir = std::env::temp_dir().join(format!("viary-notes-unsaved-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("blocked"), "").unwrap();
        let unsaved = |pcm: PathBuf| PartOut {
            segments: Vec::new(),
            pcm,
            rate: 16_000,
            frames: 0,
            clock_ms: 0,
            error: None,
            unsaved: Some(AudioBuffer::new(NOTE_RATE, vec![0.5; 1_600])),
        };
        // Still cannot be written: the error says the audio is missing.
        let mut parts = vec![unsaved(dir.join("blocked").join("a.pcm"))];
        assert!(write_unsaved(&mut parts).unwrap().contains("could not be saved"));
        assert_eq!(parts[0].frames, 0);
        // Can be now: it joins the note like any other part.
        let mut parts = vec![unsaved(dir.join("a.pcm"))];
        assert_eq!(write_unsaved(&mut parts), None);
        assert_eq!(parts[0].frames, 1_600);
        assert!(parts[0].unsaved.is_none());
        assert_eq!(part_places(&parts), vec![Place::Audio(0)]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parts_stay_when_the_wav_cannot_be_written() {
        let dir = std::env::temp_dir().join(format!("viary-notes-keep-{}", std::process::id()));
        let parts = vec![part(&dir, "a.pcm", vec![0.5; 1_600])];
        // A WAV whose folder is a file: it cannot be created.
        fs::write(dir.join("blocked"), "").unwrap();
        let error = save_audio(&dir.join("blocked").join("note.wav"), &parts).unwrap_err();
        assert!(error.contains("parts are kept"), "{error}");
        assert!(parts[0].pcm.exists());
        assert_eq!(
            pending(&parts),
            vec![notes::PendingPart {
                file: "a.pcm".into(),
                rate: 16_000
            }]
        );
        // Once it can be written, the parts go.
        save_audio(&dir.join("note.wav"), &parts).unwrap();
        assert!(!parts[0].pcm.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_recorded_writes_no_wav() {
        let dir = std::env::temp_dir().join(format!("viary-notes-empty-{}", std::process::id()));
        let mut later = part(&dir, "b.pcm", Vec::new());
        later.clock_ms = 4_000;
        let parts = vec![part(&dir, "a.pcm", Vec::new()), later];
        let wav = dir.join("note.wav");
        assert_eq!(write_wav(&wav, &parts).unwrap(), (0, 0, Vec::new()));
        assert!(!wav.exists());
        // Without audio, the parts' text goes by the clock.
        assert_eq!(
            part_places(&parts),
            vec![Place::Audio(0), Place::Audio(4_000)]
        );
        let _ = fs::remove_dir_all(dir);
    }
}
