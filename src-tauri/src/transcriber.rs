//! The Transcripts queue: files are transcribed one at a time on a worker
//! thread, so dictation and voice notes keep going meanwhile.
//!
//! Each file is decoded whole (speechkit 0.5 has no streaming decoder, so
//! files are capped at three hours), then pushed through a session in
//! chunks: the share pushed is the progress, since the session takes audio
//! only as fast as it recognizes it.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use speechkit::{
    asr::AsrSession,
    audio::{self, DecodeLimits},
};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    App, caps, dictionary,
    engines::{LoadedEngine, describe},
    history::{self, EngineLabel},
    notes::{Passage, Speaker},
    settings::Settings,
    transcripts::{self, Transcript},
};

/// The longest file accepted, as agreed while speechkit decodes whole files.
const MAX_FILE: Duration = Duration::from_secs(3 * 60 * 60);
/// Audio pushed at a time.
const CHUNK: Duration = Duration::from_millis(500);
/// How long one chunk may wait for room in the session.
const PUSH_TIMEOUT: Duration = Duration::from_secs(120);
/// How long the dictation engine may take to finish loading.
const ENGINE_WAIT: Duration = Duration::from_secs(120);
/// The app name files use for the dictionary.
const APP: &str = "Transcripts";
/// How often a blocking step looks for a cancel.
const CANCEL_POLL: Duration = Duration::from_millis(200);
const CANCELLED: &str = "Cancelled";

/// Held by a blocking step while it runs, so a step abandoned on cancel
/// finishes before the next one starts: cancelled decodes and engine
/// loads never pile up in memory.
static HEAVY: Mutex<()> = Mutex::new(());

/// Numbers jobs, so no two share an id.
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum JobState {
    Waiting,
    Running {
        stage: String,
        progress: Option<f32>,
        seconds_left: Option<u64>,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub duration_ms: Option<u64>,
    pub state: JobState,
}

#[derive(Default)]
pub struct Transcriber {
    jobs: Mutex<Vec<Job>>,
    wake: Condvar,
    /// Set to cancel the running job. Locked after `jobs`, never before.
    cancel: Mutex<Option<(String, Arc<AtomicBool>)>>,
    /// The running job's recognition, so a cancel also wakes a push that
    /// waits for room in it. Locked after `cancel`, never before.
    session: Mutex<Option<Arc<AsrSession>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The files to transcribe in `paths`: files speechkit decodes, and those
/// in folders, two levels down.
pub fn expand(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| Some(e.ok()?.path())).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                if depth > 0 {
                    walk(&path, depth - 1, out);
                }
            } else if decodable(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            walk(&path, 2, &mut out);
        } else if decodable(&path) {
            out.push(path);
        }
    }
    out
}

/// A file speechkit decodes in this build, by its extension.
pub fn decodable(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| audio::EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

impl Transcriber {
    pub fn jobs(&self) -> Vec<Job> {
        lock(&self.jobs).clone()
    }

    fn changed(&self, app: &AppHandle) {
        let _ = app.emit("transcript-jobs", self.jobs());
    }

    /// Queues `paths`, skipping files already queued. Returns how many.
    pub fn add(&self, app: &AppHandle, paths: Vec<PathBuf>) -> usize {
        let mut jobs = lock(&self.jobs);
        let mut added = 0;
        for path in paths {
            if jobs.iter().any(|j| j.path == path) {
                continue;
            }
            jobs.push(Job {
                id: format!("job-{}", NEXT_JOB.fetch_add(1, Ordering::Relaxed)),
                name: path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                path,
                duration_ms: None,
                state: JobState::Waiting,
            });
            added += 1;
        }
        drop(jobs);
        self.wake.notify_all();
        self.changed(app);
        added
    }

    pub fn retry(&self, app: &AppHandle, id: &str) {
        if let Some(job) = lock(&self.jobs).iter_mut().find(|j| j.id == id) {
            job.state = JobState::Waiting;
        }
        self.wake.notify_all();
        self.changed(app);
    }

    /// Takes a job off the queue, stopping it if it is running.
    pub fn cancel(&self, app: &AppHandle, id: &str) {
        // Under the jobs lock, so a job is either still queued here or has
        // been handed to the worker with its flag registered.
        let mut jobs = lock(&self.jobs);
        self.stop_running(id);
        jobs.retain(|j| j.id != id);
        drop(jobs);
        self.changed(app);
    }

    /// Stops job `id` if it is the one running: flags it, and cancels its
    /// recognition, which also ends a push waiting for room.
    fn stop_running(&self, id: &str) {
        if let Some((running, flag)) = &*lock(&self.cancel)
            && running == id
        {
            flag.store(true, Ordering::Release);
            if let Some(session) = &*lock(&self.session) {
                session.cancel();
            }
        }
    }

    fn set(&self, app: &AppHandle, id: &str, change: impl FnOnce(&mut Job)) {
        if let Some(job) = lock(&self.jobs).iter_mut().find(|j| j.id == id) {
            change(job);
        }
        self.changed(app);
    }

    fn stage(
        &self,
        app: &AppHandle,
        id: &str,
        stage: &str,
        progress: Option<f32>,
        seconds_left: Option<u64>,
    ) {
        self.set(app, id, |job| {
            job.state = JobState::Running {
                stage: stage.into(),
                progress,
                seconds_left,
            };
        });
    }

    /// The next waiting job, waiting for one, marked as started and with
    /// the flag that cancels it. `idle` runs while the queue has nothing to
    /// do.
    fn next(&self, mut idle: impl FnMut()) -> (Job, Arc<AtomicBool>) {
        let mut jobs = lock(&self.jobs);
        loop {
            if let Some(job) = jobs
                .iter_mut()
                .find(|j| matches!(j.state, JobState::Waiting))
            {
                job.state = JobState::Running {
                    stage: "Starting".into(),
                    progress: None,
                    seconds_left: None,
                };
                let flag = Arc::new(AtomicBool::new(false));
                *lock(&self.cancel) = Some((job.id.clone(), flag.clone()));
                return (job.clone(), flag);
            }
            idle();
            jobs = self
                .wake
                .wait(jobs)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

/// Runs the queue on its own thread for as long as Viary does.
pub fn spawn(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("viary-transcribe".into())
        .spawn(move || {
            let transcriber = &app.state::<App>().transcriber;
            // An engine other than the dictation one, kept while the queue
            // has work, then let go to free its memory.
            let mut separate: Option<(String, Arc<LoadedEngine>)> = None;
            loop {
                let (job, flag) = transcriber.next(|| separate = None);
                transcriber.changed(&app);
                let result = run(&app, &job, &flag, &mut separate);
                *lock(&transcriber.cancel) = None;
                if flag.load(Ordering::Acquire) {
                    continue;
                }
                match result {
                    Ok(transcript) => {
                        lock(&transcriber.jobs).retain(|j| j.id != job.id);
                        transcriber.changed(&app);
                        let id = transcript.id.clone();
                        app.state::<App>().transcripts.add(transcript);
                        let _ = app.emit("transcripts-changed", ());
                        let _ = app.emit("transcript-done", id);
                    }
                    Err(error) => {
                        tracing::warn!(%error, path = %job.path.display(), "cannot transcribe a file");
                        transcriber.set(&app, &job.id, |j| j.state = JobState::Failed { error });
                    }
                }
            }
        });
}

/// The engine for files: the dictation engine (waiting while it loads),
/// or the one chosen for files.
/// Runs `work` on its own thread, giving up once `cancelled` is set: the
/// work then ends in the background and its result is dropped, so the
/// queue goes on at once. The next step waits for it (`HEAVY`), still
/// watching for a cancel.
fn unless_cancelled<T: Send + 'static>(
    cancelled: &AtomicBool,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (done, result) = mpsc::channel();
    std::thread::Builder::new()
        .name("viary-transcribe-step".into())
        .spawn(move || {
            let _heavy = lock(&HEAVY);
            let _ = done.send(work());
        })
        .map_err(|e| e.to_string())?;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(CANCELLED.into());
        }
        match result.recv_timeout(CANCEL_POLL) {
            Ok(value) => return Ok(value),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err("The transcription stopped unexpectedly".into());
            }
        }
    }
}

/// The running job's session, registered for `Transcriber::cancel` while
/// this lives.
struct Registered<'a>(&'a Mutex<Option<Arc<AsrSession>>>);

impl<'a> Registered<'a> {
    fn new(slot: &'a Mutex<Option<Arc<AsrSession>>>, session: &Arc<AsrSession>) -> Self {
        *lock(slot) = Some(session.clone());
        Self(slot)
    }
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        *lock(self.0) = None;
    }
}

fn engine(
    app: &AppHandle,
    job: &Job,
    settings: &Settings,
    separate: &mut Option<(String, Arc<LoadedEngine>)>,
    cancelled: &AtomicBool,
) -> Result<Arc<LoadedEngine>, String> {
    let state = app.state::<App>();
    let current = state.engines.current();
    let wanted = settings.transcripts.engine.clone();
    match wanted {
        Some(id) if current.as_ref().is_none_or(|c| c.info.id != id) => {
            if let Some((loaded, engine)) = separate
                && *loaded == id
            {
                return Ok(engine.clone());
            }
            state
                .transcriber
                .stage(app, &job.id, "Loading the engine", None, None);
            *separate = None;
            let (engines, wanted, settings) = (state.engines.clone(), id.clone(), settings.clone());
            let engine = Arc::new(
                unless_cancelled(cancelled, move || {
                    engines.build_separate(&wanted, &settings)
                })?
                .map_err(|e| format!("The engine for files did not load: {}", describe(&e)))?,
            );
            *separate = Some((id, engine.clone()));
            Ok(engine)
        }
        _ => {
            let started = Instant::now();
            loop {
                if cancelled.load(Ordering::Acquire) {
                    return Err(CANCELLED.into());
                }
                if let Some(engine) = state.engines.current() {
                    return Ok(engine);
                }
                if state.engines.status().loading.is_none() || started.elapsed() > ENGINE_WAIT {
                    return Err("Choose a voice engine first".into());
                }
                state
                    .transcriber
                    .stage(app, &job.id, "Waiting for the voice engine", None, None);
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

fn run(
    app: &AppHandle,
    job: &Job,
    cancelled: &AtomicBool,
    separate: &mut Option<(String, Arc<LoadedEngine>)>,
) -> Result<Transcript, String> {
    let state = app.state::<App>();
    let transcriber = &state.transcriber;
    let mut settings = state.settings();
    let engine = engine(app, job, &settings, separate, cancelled)?;

    transcriber.stage(app, &job.id, "Reading the file", None, None);
    let path = job.path.clone();
    let audio = unless_cancelled(cancelled, move || {
        audio::read(&path, DecodeLimits::new(MAX_FILE))
    })?
    .map_err(|e| format!("This file can't be read: {}", describe(&e)))?;
    let total = audio.samples.len();
    let duration_ms = u64::try_from(audio.duration().as_millis()).unwrap_or(u64::MAX);
    transcriber.set(app, &job.id, |j| j.duration_ms = Some(duration_ms));
    if total == 0 {
        return Err("This file has no audio".into());
    }

    // The language chosen for files, for engines that take one.
    settings.language = settings.transcripts.language;
    let options = engine.session_options(&settings, APP);
    // Starting waits for a free slot on the engine (dictation may hold it).
    let (asr, rate) = (engine.engine.clone(), audio.sample_rate);
    let session = Arc::new(
        unless_cancelled(cancelled, move || asr.start(rate, options, Duration::from_secs(60)))?
            .map_err(|e| format!("Recognition did not start: {}", describe(&e)))?,
    );
    // Registered before the first look at `cancelled`, so a cancel either
    // is seen there or reaches the session; let go however this returns.
    let _registered = Registered::new(&transcriber.session, &session);
    let chunk = usize::try_from(audio.sample_rate.frames_in(CHUNK))
        .unwrap_or(8_000)
        .max(1);
    let started = Instant::now();
    let mut pushed = 0;
    let mut last_report = Instant::now() - Duration::from_secs(1);
    for piece in audio.samples.chunks(chunk) {
        if cancelled.load(Ordering::Acquire) {
            session.cancel();
            return Err(CANCELLED.into());
        }
        session.push(piece, PUSH_TIMEOUT).map_err(|e| {
            format!(
                "Recognition stopped: {}",
                describe(&speechkit::SpeechError::from(e))
            )
        })?;
        pushed += piece.len();
        if last_report.elapsed() >= Duration::from_millis(500) {
            last_report = Instant::now();
            #[allow(clippy::cast_precision_loss)]
            let progress = pushed as f32 / total as f32;
            let elapsed = started.elapsed().as_secs_f32();
            // An estimate once there is enough to go on.
            let left = (progress > 0.02 && elapsed > 3.0).then(|| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let secs = (elapsed / progress * (1.0 - progress)) as u64;
                secs
            });
            transcriber.stage(app, &job.id, "Transcribing", Some(progress), left);
        }
    }
    drop(audio);
    transcriber.stage(app, &job.id, "Finishing", Some(1.0), None);
    // Allow the rest of the recognition as long as the file itself,
    // looking for a cancel meanwhile.
    let deadline = Instant::now() + Duration::from_millis(duration_ms) + Duration::from_secs(120);
    session.close_input();
    let result = loop {
        if cancelled.load(Ordering::Acquire) {
            session.cancel();
            return Err(CANCELLED.into());
        }
        if let Some(result) = session.wait(CANCEL_POLL) {
            break result;
        }
        if Instant::now() >= deadline {
            break session.finish(Duration::ZERO);
        }
    };
    let segments = match result {
        Ok(transcript) => transcript.segments,
        Err(failure) => return Err(format!("Recognition failed: {}", describe(&failure.error))),
    };

    let words = &settings.dictionary;
    let mut passages: Vec<Passage> = segments
        .iter()
        .filter_map(|segment| {
            let text = dictionary::apply(words, APP, segment.text.trim());
            (!text.trim().is_empty()).then(|| Passage {
                start_ms: u64::try_from(segment.start.as_millis()).unwrap_or(u64::MAX),
                end_ms: u64::try_from(segment.end.as_millis()).unwrap_or(u64::MAX),
                text,
                speaker: None,
                words: None,
                original: None,
            })
        })
        .collect();

    // Dev only: speakers and word times from the scripted fixture. Speaker
    // separation would not know names, so they are numbered.
    let mut speakers: Vec<Speaker> = Vec::new();
    if let Some((mock, who)) = caps::mock_passages(duration_ms) {
        transcriber.stage(app, &job.id, "Finding speakers", Some(1.0), None);
        passages = mock;
        speakers = (1..=who.len())
            .map(|i| Speaker {
                name: format!("Speaker {i}"),
            })
            .collect();
    }

    let info = &engine.info;
    let mut transcript = Transcript {
        id: history::now_ms().to_string(),
        name: job
            .path
            .file_stem()
            .map_or_else(|| job.name.clone(), |s| s.to_string_lossy().into_owned()),
        source: job.path.clone(),
        created_at: history::now_ms(),
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
        saved: Vec::new(),
    };
    if !settings.transcripts.formats.is_empty() {
        transcriber.stage(app, &job.id, "Saving subtitles", Some(1.0), None);
        transcript.saved = transcripts::save_next_to_source(
            &transcript,
            &settings.transcripts.formats,
            settings.transcripts.speaker_names,
        );
    }
    Ok(transcript)
}

#[cfg(test)]
mod tests {
    use speechkit::{
        SampleRate, SpeechError,
        asr::{AsrBackend, AsrCapabilities, AsrEngine, AsrEvents, AsrOptions, AsrStream},
    };

    use super::*;

    /// A recognizer that takes no audio until `gate` opens.
    struct Stalled {
        caps: AsrCapabilities,
        gate: Arc<Mutex<mpsc::Receiver<()>>>,
    }

    impl AsrBackend for Stalled {
        fn name(&self) -> &str {
            "stalled"
        }

        fn capabilities(&self) -> &AsrCapabilities {
            &self.caps
        }

        fn open(&self, _: &AsrOptions, _: AsrEvents) -> Result<Box<dyn AsrStream>, SpeechError> {
            Ok(Box::new(StalledStream(self.gate.clone())))
        }
    }

    struct StalledStream(Arc<Mutex<mpsc::Receiver<()>>>);

    impl AsrStream for StalledStream {
        fn accept(&mut self, _: &[f32]) -> Result<(), SpeechError> {
            let _ = lock(&self.0).recv_timeout(Duration::from_secs(10));
            Ok(())
        }

        fn finish(&mut self) -> Result<(), SpeechError> {
            Ok(())
        }
    }

    #[test]
    fn a_cancel_ends_a_push_waiting_for_room() {
        let rate = SampleRate::HZ_16000;
        let (_open, gate) = mpsc::channel();
        let engine = AsrEngine::new(Stalled {
            caps: AsrCapabilities::new(rate),
            gate: Arc::new(Mutex::new(gate)),
        });
        let session = Arc::new(
            engine
                .start(rate, AsrOptions::default(), Duration::from_secs(5))
                .unwrap(),
        );
        let transcriber = Transcriber::default();
        *lock(&transcriber.cancel) = Some(("job-1".into(), Arc::new(AtomicBool::new(false))));
        let _registered = Registered::new(&transcriber.session, &session);

        // Push until the queue is full and a push waits for room.
        let pusher = session.clone();
        let (pushed, returned) = mpsc::channel();
        std::thread::spawn(move || {
            let chunk = vec![0.0; 8_000];
            while pusher.push(&chunk[..], PUSH_TIMEOUT).is_ok() {}
            let _ = pushed.send(());
        });
        assert!(returned.recv_timeout(Duration::from_millis(500)).is_err(), "the push waits");
        let started = Instant::now();
        transcriber.stop_running("job-1");
        returned.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_cancel_stops_waiting_for_a_blocking_step() {
        let cancelled = AtomicBool::new(false);
        assert_eq!(unless_cancelled(&cancelled, || 7), Ok(7));
        cancelled.store(true, Ordering::Release);
        let started = Instant::now();
        let slow = unless_cancelled(&cancelled, || std::thread::sleep(Duration::from_secs(2)));
        assert_eq!(slow, Err(CANCELLED.to_owned()));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn the_next_step_waits_for_an_abandoned_one() {
        let (holding, held) = mpsc::channel();
        let (finished, abandoned_done) = mpsc::channel();
        let cancelled = AtomicBool::new(true);
        let _ = unless_cancelled(&cancelled, move || {
            // Runs with `HEAVY` held.
            let _ = holding.send(());
            std::thread::sleep(Duration::from_millis(100));
            let _ = finished.send(());
        });
        held.recv().unwrap();
        cancelled.store(false, Ordering::Release);
        assert_eq!(
            unless_cancelled(&cancelled, move || abandoned_done.try_recv().is_ok()),
            Ok(true)
        );
    }
}
