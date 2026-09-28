//! Microphone audio for a dictation, teed three ways.
//!
//! `speechkit::io::Microphone` listens into an `AsrSession`. Viary needs the
//! same audio for the engine, for the pill's waveform, and kept in memory so
//! a failed dictation can be retried (speechkit never keeps input audio).
//! So the microphone feeds a session on a small backend of our own, the
//! tap, whose stream hands every chunk to a channel. A pump thread reads
//! it, records it, measures it, and pushes it into the engine's session.
//!
//! Recording starts at once; the engine session may take a while to open
//! (a cloud handshake), so audio waits in a backlog until it is ready.

use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use speechkit::{
    AudioBuffer, SampleRate, SpeechError,
    asr::{
        AsrBackend, AsrCapabilities, AsrEngine, AsrSession, AsrSessionLimits, BackendEvent,
        PushError, RecognizerStream, SessionOptions,
    },
    io::{Listening, Microphone},
};

/// Dictations longer than this are cut: the most a recording keeps.
pub const MAX_RECORDING: Duration = Duration::from_secs(10 * 60);

struct Tap {
    caps: AsrCapabilities,
    chunks: Sender<Vec<f32>>,
}

struct TapStream {
    chunks: Option<Sender<Vec<f32>>>,
}

impl AsrBackend for Tap {
    fn name(&self) -> &str {
        "viary-tap"
    }

    fn capabilities(&self) -> &AsrCapabilities {
        &self.caps
    }

    fn open(&self, _: &SessionOptions) -> Result<Box<dyn RecognizerStream>, SpeechError> {
        Ok(Box::new(TapStream {
            chunks: Some(self.chunks.clone()),
        }))
    }
}

impl RecognizerStream for TapStream {
    fn accept(&mut self, samples: &[f32]) -> Result<Vec<BackendEvent>, SpeechError> {
        if let Some(chunks) = &self.chunks {
            // The pump is gone only when the dictation was dropped.
            let _ = chunks.send(samples.to_vec());
        }
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<Vec<BackendEvent>, SpeechError> {
        // Closing the channel tells the pump the audio is complete.
        self.chunks = None;
        Ok(Vec::new())
    }
}

/// The engine session, as it becomes known.
#[derive(Clone)]
pub enum Engine {
    Starting,
    Ready(Arc<AsrSession>),
    /// It could not start, with the error.
    Failed(Arc<SpeechError>),
}

/// Where the pump learns about the engine session.
#[derive(Clone)]
pub struct EngineSlot(Arc<(Mutex<Engine>, Condvar)>);

impl EngineSlot {
    pub fn new() -> Self {
        Self(Arc::new((Mutex::new(Engine::Starting), Condvar::new())))
    }

    pub fn set(&self, engine: Engine) {
        *lock(&self.0.0) = engine;
        self.0.1.notify_all();
    }

    pub fn get(&self) -> Engine {
        lock(&self.0.0).clone()
    }

    /// Waits until the session is no longer starting, or `deadline`.
    pub fn settled(&self, deadline: Instant) -> Engine {
        let (mutex, changed) = &*self.0;
        let mut engine = lock(mutex);
        while matches!(*engine, Engine::Starting) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            engine = changed
                .wait_timeout(engine, left)
                .map_or_else(|p| p.into_inner().0, |(g, _)| g);
        }
        engine.clone()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The root mean square of `samples`: the pill's input level.
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    #[expect(clippy::cast_precision_loss, reason = "a level, not an exact count")]
    let len = samples.len() as f32;
    (sum / len).sqrt()
}

/// Keeps the audio of one dictation, up to [`MAX_RECORDING`].
struct Recorder {
    sample_rate: SampleRate,
    cap: usize,
    samples: Vec<f32>,
}

impl Recorder {
    fn new(sample_rate: SampleRate) -> Self {
        Self {
            sample_rate,
            cap: usize::try_from(sample_rate.frames_in(MAX_RECORDING)).unwrap_or(usize::MAX),
            samples: Vec::new(),
        }
    }

    fn push(&mut self, chunk: &[f32]) {
        let room = self.cap.saturating_sub(self.samples.len());
        self.samples.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }

    fn finish(self) -> AudioBuffer {
        AudioBuffer::new(self.sample_rate, self.samples)
    }
}

/// What the pump reports while recording.
pub enum PumpEvent {
    /// The RMS level of the latest chunk.
    Level(f32),
    /// The engine session stopped taking audio. Recording goes on, so the
    /// audio can still be retried.
    EngineStopped(String),
}

/// A running recording.
pub struct Recording {
    /// The microphone feeding the tap session.
    listening: Option<Listening>,
    sample_rate: SampleRate,
    pump: Option<JoinHandle<Recorder>>,
    /// Set when the recording is dropped without `stop`.
    cancelled: Arc<AtomicBool>,
}

impl Recording {
    /// Opens the microphone called `name`, falling back to the default
    /// one if it is gone.
    ///
    /// # Errors
    ///
    /// Any error opening the device.
    pub fn open_microphone(name: Option<&str>) -> Result<Microphone, SpeechError> {
        match name {
            Some(name) => Microphone::open(name).or_else(|error| {
                tracing::warn!(%error, "the chosen microphone is unavailable; using the default");
                Microphone::open_default()
            }),
            None => Microphone::open_default(),
        }
    }

    /// Starts recording from `microphone`, feeding the engine session in
    /// `slot` once it is ready. Start that session at
    /// `microphone.sample_rate()`.
    ///
    /// # Errors
    ///
    /// Any error starting the device.
    pub fn start(
        microphone: &Microphone,
        slot: EngineSlot,
        on_event: impl Fn(PumpEvent) + Send + 'static,
    ) -> Result<Self, SpeechError> {
        let sample_rate = microphone.sample_rate();
        let (chunks, received) = mpsc::channel();
        let tap = AsrEngine::new(Tap {
            caps: AsrCapabilities::new(sample_rate),
            chunks,
        })
        .with_limits(AsrSessionLimits::default().with_max_session(MAX_RECORDING));
        let cancelled = Arc::new(AtomicBool::new(false));
        let pump_cancelled = cancelled.clone();
        let pump = std::thread::Builder::new()
            .name("viary-pump".into())
            .spawn(move || pump(&received, sample_rate, &slot, &pump_cancelled, &on_event))
            .map_err(|e| SpeechError::backend("viary", true, e))?;
        // The tap engine, and with it the channel's first sender, goes at
        // the end of this call: the stream's sender is the last, so the
        // pump ends when the tap session does.
        let listening = microphone.listen(&tap, SessionOptions::default())?;
        Ok(Self {
            listening: Some(listening),
            sample_rate,
            pump: Some(pump),
            cancelled,
        })
    }

    /// Stops the device and returns everything recorded, once the pump has
    /// handed all of it to the engine session. The session's input is left
    /// open; the caller finishes it.
    pub fn stop(mut self) -> AudioBuffer {
        if let Some(listening) = self.listening.take() {
            if listening.device_lost() {
                tracing::warn!("microphone lost during the dictation");
            }
            let _ = listening.finish(Instant::now() + Duration::from_secs(5));
        }
        self.pump
            .take()
            .and_then(|pump| pump.join().ok())
            .map_or_else(
                || AudioBuffer::new(self.sample_rate, Vec::new()),
                Recorder::finish,
            )
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        // After `stop` the pump is done; otherwise the dictation was dropped.
        if self.pump.is_some() {
            self.cancelled.store(true, Ordering::SeqCst);
        }
        // Dropping `Listening` stops the microphone and cancels the tap.
        self.listening.take();
    }
}

/// How long the pump waits for a slow engine session after the key is
/// released, before giving up on it. The audio is kept either way.
const LATE_START: Duration = Duration::from_secs(35);
/// How long one chunk may wait for room in a busy engine session. Past
/// this the engine is stuck, and the dictation fails with its audio kept.
const PUSH_PATIENCE: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(10)
};

/// Pushes the backlog into the engine session, once it is ready.
fn feed(
    backlog: &mut Vec<Vec<f32>>,
    slot: &EngineSlot,
    open: &mut bool,
    on_event: &dyn Fn(PumpEvent),
) {
    let session = match slot.get() {
        Engine::Starting => return,
        Engine::Failed(_) => {
            backlog.clear();
            return;
        }
        Engine::Ready(session) => session,
    };
    for chunk in backlog.drain(..) {
        if !*open {
            continue;
        }
        if let Err(error) = session.push_until(chunk, Instant::now() + PUSH_PATIENCE) {
            *open = false;
            on_event(PumpEvent::EngineStopped(error.to_string()));
            // A session that ended reports its own error when finished.
            // One that is only stuck would finish "successfully" with the
            // audio cut short, so fail the dictation instead.
            if matches!(error, PushError::Full(_)) {
                slot.set(Engine::Failed(Arc::new(SpeechError::DeadlineExceeded)));
            }
        }
    }
}

fn pump(
    chunks: &Receiver<Vec<f32>>,
    sample_rate: SampleRate,
    slot: &EngineSlot,
    cancelled: &AtomicBool,
    on_event: &dyn Fn(PumpEvent),
) -> Recorder {
    let mut recorder = Recorder::new(sample_rate);
    let mut backlog: Vec<Vec<f32>> = Vec::new();
    let mut open = true;
    while let Ok(chunk) = chunks.recv() {
        // The tap session already rejected invalid samples.
        recorder.push(&chunk);
        on_event(PumpEvent::Level(rms(&chunk)));
        if cancelled.load(Ordering::SeqCst) {
            continue;
        }
        backlog.push(chunk);
        feed(&mut backlog, slot, &mut open, on_event);
    }
    // A dropped dictation sends nothing more, and does not wait for the
    // engine: its audio must not reach a session that opens later.
    if cancelled.load(Ordering::SeqCst) {
        if let Engine::Ready(session) = slot.get() {
            session.cancel();
        }
        return recorder;
    }
    if !backlog.is_empty() {
        slot.settled(Instant::now() + LATE_START);
        feed(&mut backlog, slot, &mut open, on_event);
    }
    recorder
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use speechkit::asr::UtteranceId;

    use super::*;

    /// Counts the samples it is fed, and reports the count as its text.
    struct Counter(AsrCapabilities, Arc<AtomicUsize>);
    struct CounterStream(Arc<AtomicUsize>);

    impl AsrBackend for Counter {
        fn name(&self) -> &str {
            "counter"
        }
        fn capabilities(&self) -> &AsrCapabilities {
            &self.0
        }
        fn open(&self, _: &SessionOptions) -> Result<Box<dyn RecognizerStream>, SpeechError> {
            Ok(Box::new(CounterStream(self.1.clone())))
        }
    }

    impl RecognizerStream for CounterStream {
        fn accept(&mut self, samples: &[f32]) -> Result<Vec<BackendEvent>, SpeechError> {
            self.0.fetch_add(samples.len(), Ordering::SeqCst);
            Ok(Vec::new())
        }
        fn finish(&mut self) -> Result<Vec<BackendEvent>, SpeechError> {
            Ok(vec![BackendEvent::Segment(speechkit::asr::Segment {
                utterance: UtteranceId(0),
                text: self.0.load(Ordering::SeqCst).to_string(),
                start: Duration::ZERO,
                end: Duration::ZERO,
            })])
        }
    }

    const RATE: SampleRate = SampleRate::HZ_16000;

    fn counter() -> (AsrEngine, Arc<AtomicUsize>) {
        let seen = Arc::new(AtomicUsize::new(0));
        let engine = AsrEngine::new(Counter(AsrCapabilities::new(RATE), seen.clone()));
        (engine, seen)
    }

    fn start(engine: &AsrEngine) -> Arc<AsrSession> {
        Arc::new(engine.start(RATE, SessionOptions::default()).unwrap())
    }

    fn text(session: &AsrSession) -> String {
        let result = session.finish(Instant::now() + Duration::from_secs(5));
        result.unwrap().transcript.text()
    }

    /// Feeds ten 100 ms chunks through the pump, making the session ready
    /// after `ready_after` of them.
    fn run(slot: &EngineSlot, ready: Option<(usize, Engine)>) -> AudioBuffer {
        let (chunks, received) = mpsc::channel();
        let pump_slot = slot.clone();
        let levels = Arc::new(AtomicUsize::new(0));
        let counted = levels.clone();
        let pump = std::thread::spawn(move || {
            pump(&received, RATE, &pump_slot, &AtomicBool::new(false), &move |event| {
                if let PumpEvent::Level(_) = event {
                    counted.fetch_add(1, Ordering::SeqCst);
                }
            })
        });
        for i in 0..10 {
            if let Some((at, engine)) = &ready
                && *at == i
            {
                slot.set(engine.clone());
            }
            chunks.send(vec![0.1_f32; 1600]).unwrap();
        }
        drop(chunks);
        let audio = pump.join().unwrap().finish();
        assert_eq!(levels.load(Ordering::SeqCst), 10, "one level per chunk");
        audio
    }

    #[test]
    fn audio_before_the_session_opens_is_sent_once_it_does() {
        let (engine, seen) = counter();
        let session = start(&engine);
        let slot = EngineSlot::new();
        let audio = run(&slot, Some((4, Engine::Ready(session.clone()))));
        assert_eq!(audio.samples.len(), 16_000, "everything is recorded");
        assert_eq!(text(&session), "16000");
        assert_eq!(
            seen.load(Ordering::SeqCst),
            16_000,
            "and everything reached the engine"
        );
    }

    #[test]
    fn a_session_that_opens_after_release_still_gets_the_audio() {
        let (engine, _) = counter();
        let session = start(&engine);
        let slot = EngineSlot::new();
        let late = slot.clone();
        let ready = session.clone();
        let opener = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            late.set(Engine::Ready(ready));
        });
        let audio = run(&slot, None);
        opener.join().unwrap();
        assert_eq!(audio.samples.len(), 16_000);
        assert_eq!(text(&session), "16000");
    }

    #[test]
    fn a_session_that_fails_to_open_keeps_the_recording() {
        let slot = EngineSlot::new();
        let failed = Engine::Failed(Arc::new(SpeechError::backend(
            "openai-realtime",
            true,
            "reset",
        )));
        let audio = run(&slot, Some((2, failed)));
        assert_eq!(audio.samples.len(), 16_000, "the audio is kept for Retry");
    }

    /// Never takes audio: its input queue fills and stays full.
    struct Stuck(AsrCapabilities);
    struct StuckStream;

    impl AsrBackend for Stuck {
        fn name(&self) -> &str {
            "stuck"
        }
        fn capabilities(&self) -> &AsrCapabilities {
            &self.0
        }
        fn open(&self, _: &SessionOptions) -> Result<Box<dyn RecognizerStream>, SpeechError> {
            Ok(Box::new(StuckStream))
        }
    }

    impl RecognizerStream for StuckStream {
        fn accept(&mut self, _: &[f32]) -> Result<Vec<BackendEvent>, SpeechError> {
            std::thread::sleep(Duration::from_secs(60));
            Ok(Vec::new())
        }
        fn finish(&mut self) -> Result<Vec<BackendEvent>, SpeechError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn a_stuck_engine_fails_the_dictation_instead_of_cutting_it_short() {
        let engine = AsrEngine::new(Stuck(AsrCapabilities::new(RATE))).with_limits(
            AsrSessionLimits::default()
                .with_max_chunk(Duration::from_millis(100))
                .with_input_queue(Duration::from_millis(200)),
        );
        let slot = EngineSlot::new();
        slot.set(Engine::Ready(start(&engine)));
        let (chunks, received) = mpsc::channel();
        for _ in 0..10 {
            chunks.send(vec![0.1_f32; 1600]).unwrap();
        }
        drop(chunks);
        let audio = pump(&received, RATE, &slot, &AtomicBool::new(false), &|_| {});
        assert_eq!(audio.finish().samples.len(), 16_000, "the audio is kept");
        assert!(
            matches!(slot.get(), Engine::Failed(_)),
            "the dictation fails, so Retry is offered"
        );
    }

    #[test]
    fn a_dropped_dictation_sends_nothing_to_a_late_session() {
        let (engine, seen) = counter();
        let session = start(&engine);
        let slot = EngineSlot::new();
        let (chunks, received) = mpsc::channel();
        for _ in 0..3 {
            chunks.send(vec![0.1_f32; 1600]).unwrap();
        }
        drop(chunks);
        slot.set(Engine::Ready(session.clone()));
        let started = Instant::now();
        pump(&received, RATE, &slot, &AtomicBool::new(true), &|_| {});
        assert!(started.elapsed() < Duration::from_secs(1), "no wait for the engine");
        assert_eq!(seen.load(Ordering::SeqCst), 0);
        assert!(session.result().is_some(), "the orphaned session is cancelled");
    }

    #[test]
    fn levels_and_the_recording_cap() {
        assert!(rms(&[]).abs() < f32::EPSILON);
        assert!((rms(&[0.5, -0.5]) - 0.5).abs() < 1e-6);
        let mut recorder = Recorder::new(RATE);
        recorder.cap = 3;
        recorder.push(&[0.1; 2]);
        recorder.push(&[0.2; 2]);
        assert_eq!(recorder.finish().samples, [0.1, 0.1, 0.2]);
    }
}
