//! Dictation capture through speechkit's recording and background opening API.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use speechkit::{
    AudioBuffer, SampleRate, SpeechError,
    asr::{AsrEngine, AsrOptions, AsrResult, AsrUpdate, LiveTranscript},
    io::{Capture, CaptureOptions, ListenOptions, Listening, Microphone},
};

/// The most audio kept for History and Retry.
pub const MAX_RECORDING: Duration = Duration::from_secs(10 * 60);
/// How long `finish` waits, once the session is over, for the listening to
/// read the last of the audio. By then it only records.
const DRAIN: Duration = Duration::from_secs(5);

/// A listening with its audio retained, and one observer for the pill.
pub struct Recording {
    listening: Arc<Listening>,
    sample_rate: SampleRate,
    /// Set at key-up, when the observer has nothing left to show.
    stopped: Arc<AtomicBool>,
}

impl Recording {
    /// Opens the chosen microphone, falling back to the default if it is gone.
    pub fn open_microphone(name: Option<&str>) -> Result<Microphone, SpeechError> {
        match name {
            Some(name) => Microphone::open(name).or_else(|error| {
                tracing::warn!(%error, "the chosen microphone is unavailable; using the default");
                Microphone::open_default()
            }),
            None => Microphone::open_default(),
        }
    }

    /// Starts at once; speechkit retains audio while the backend connects.
    /// `on_update` gets the level and, when it changed, the live text.
    pub fn start(
        microphone: &Microphone,
        engine: &AsrEngine,
        options: AsrOptions,
        on_update: impl Fn(f32, Option<String>) + Send + 'static,
    ) -> Result<Self, SpeechError> {
        let mut live = LiveTranscript::new();
        Self::start_with(
            microphone,
            engine,
            options,
            MAX_RECORDING,
            move |level, updates| {
                for update in updates {
                    live.apply(update);
                }
                on_update(level, (!updates.is_empty()).then(|| live.text()));
            },
        )
    }

    /// Like `start`, for up to `max_length` of audio, with the updates
    /// received since the last call instead of the joined text.
    pub fn start_with(
        microphone: &Microphone,
        engine: &AsrEngine,
        options: AsrOptions,
        max_length: Duration,
        on_update: impl FnMut(f32, &[AsrUpdate]) + Send + 'static,
    ) -> Result<Self, SpeechError> {
        let capture = microphone.capture(CaptureOptions::default())?;
        Self::listen_on(&capture, engine, options, max_length, None, on_update)
    }

    /// Like `start_with`, on a capture already running, from capture time
    /// `start` (which the capture must still hold), or from now.
    pub fn listen_on(
        capture: &Capture,
        engine: &AsrEngine,
        options: AsrOptions,
        max_length: Duration,
        start: Option<Duration>,
        mut on_update: impl FnMut(f32, &[AsrUpdate]) + Send + 'static,
    ) -> Result<Self, SpeechError> {
        // The recording grows only as the engine takes audio, and audio the
        // listening could not hold is gone from it too. Hold as much as the
        // recording does, so an engine slower than real time, or one still
        // connecting, does not cost the audio after the default 30 s.
        let listen = start
            .map_or_else(ListenOptions::default, ListenOptions::starting_at)
            .with_recording(max_length)
            .with_max_backlog(max_length);
        let listening =
            Arc::new(capture.listen(engine, options.with_max_length(max_length), listen)?);
        let observer = Arc::downgrade(&listening);
        let mut updates = listening.updates();
        let stopped = Arc::new(AtomicBool::new(false));
        let quiet = stopped.clone();
        std::thread::Builder::new()
            .name("viary-observer".into())
            .spawn(move || {
                let mut batch = Vec::new();
                while let Some(listening) = observer.upgrade() {
                    // The listening ends only once its session does, which
                    // can be long after key-up; the pill is done by then.
                    if quiet.load(Ordering::Acquire) {
                        break;
                    }
                    batch.clear();
                    while let Ok(update) = updates.try_recv() {
                        batch.push(update);
                    }
                    on_update(listening.level(), &batch);
                    if listening.end().is_some() {
                        break;
                    }
                    // A failed session closes its updates, but the microphone
                    // keeps recording and metering until key-up.
                    drop(listening);
                    std::thread::sleep(Duration::from_millis(50));
                }
            })
            .map_err(|error| SpeechError::backend("viary", true, error))?;
        Ok(Self {
            listening,
            sample_rate: capture.sample_rate(),
            stopped,
        })
    }

    /// Whether the listening has ended by itself (its session failed or cut
    /// itself off, or the device was lost): it records nothing more.
    pub fn ended(&self) -> bool {
        self.end().is_some()
    }

    /// The capture time where the listening ended, once it has.
    pub fn end(&self) -> Option<Duration> {
        self.listening.end()
    }

    /// The capture time of the recording's first sample. Its segments' times
    /// are capture times too.
    pub fn origin(&self) -> Duration {
        self.listening.origin()
    }

    /// Ends the input immediately in the key-up handler.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.listening.stop();
    }

    /// Waits on a worker thread, keeping the recording even on failure.
    pub fn finish(self, timeout: Duration) -> (AsrResult, AudioBuffer) {
        self.stopped.store(true, Ordering::Release);
        let result = self.listening.finish(timeout);
        // A session that ran out of time fails before the listening has read
        // the audio it still held. From then on the listening only records,
        // so it ends within moments: wait for that, or the recording is cut
        // short. After a session that finished, it has ended already.
        let until = Instant::now() + DRAIN;
        while self.listening.end().is_none() {
            if Instant::now() >= until {
                tracing::warn!(
                    "the listening has not read all the audio; the recording may be cut short"
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if self.listening.device_lost() {
            tracing::warn!("microphone lost during the dictation");
        }
        let audio = self.listening.recording().map_or_else(
            || AudioBuffer::new(self.sample_rate, Vec::new()),
            |recording| {
                if recording.truncated {
                    tracing::warn!("the dictation recording is truncated");
                }
                recording.audio
            },
        );
        (result, audio)
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.listening.cancel();
    }
}

#[cfg(test)]
mod tests {
    // These drive `Microphone::fake`, which speechkit hides from its docs and
    // leaves out of semver: expect to revisit them when speechkit is upgraded.

    use std::sync::{Mutex, atomic::AtomicUsize, mpsc};

    use speechkit::{
        asr::{
            AsrBackend, AsrCapabilities, AsrEvent, AsrEvents, AsrStream, Partial, Segment,
            UtteranceId,
        },
        io::FakeMicrophone,
    };

    use super::*;

    const RATE: SampleRate = SampleRate::HZ_16000;
    const TIMEOUT: Duration = Duration::from_secs(5);

    struct Counter {
        caps: AsrCapabilities,
        gate: Option<Mutex<mpsc::Receiver<()>>>,
        fail: bool,
    }

    impl AsrBackend for Counter {
        fn name(&self) -> &str {
            "counter"
        }

        fn capabilities(&self) -> &AsrCapabilities {
            &self.caps
        }

        fn open(
            &self,
            _: &AsrOptions,
            events: AsrEvents,
        ) -> Result<Box<dyn AsrStream>, SpeechError> {
            if let Some(gate) = &self.gate {
                gate.lock().unwrap().recv_timeout(TIMEOUT).unwrap();
            }
            if self.fail {
                return Err(SpeechError::backend("counter", true, "connection failed"));
            }
            Ok(Box::new(CounterStream { events, samples: 0 }))
        }
    }

    struct CounterStream {
        events: AsrEvents,
        samples: usize,
    }

    impl AsrStream for CounterStream {
        fn accept(&mut self, samples: &[f32]) -> Result<(), SpeechError> {
            self.samples += samples.len();
            let _ = self.events.send(AsrEvent::Partial(Partial {
                utterance: UtteranceId(0),
                text: self.samples.to_string(),
            }));
            Ok(())
        }

        fn finish(&mut self) -> Result<(), SpeechError> {
            let _ = self.events.send(AsrEvent::Segment(Segment {
                utterance: UtteranceId(0),
                text: self.samples.to_string(),
                start: Duration::ZERO,
                end: RATE.duration_of(self.samples as u64),
            }));
            Ok(())
        }
    }

    fn engine(gate: Option<mpsc::Receiver<()>>, fail: bool) -> AsrEngine {
        let mut caps = AsrCapabilities::new(RATE);
        caps.reports_partials = true;
        AsrEngine::new(Counter {
            caps,
            gate: gate.map(Mutex::new),
            fail,
        })
    }

    fn wait_recorded(recording: &Recording, samples: usize) {
        let deadline = Instant::now() + TIMEOUT;
        while recording.listening.recording().unwrap().audio.samples.len() < samples {
            assert!(
                Instant::now() < deadline,
                "capture did not record the audio"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_session_opened_after_key_up_gets_all_recorded_audio() {
        let (microphone, input) = Microphone::fake(RATE);
        let (release, gate) = mpsc::channel();
        let engine = engine(Some(gate), false);
        let recording =
            Recording::start(&microphone, &engine, AsrOptions::default(), |_, _| {}).unwrap();
        let samples = vec![0.25; 1600];
        input.push(&samples);
        wait_recorded(&recording, samples.len());
        // The backend is still opening. Neither start nor stop waited for it.
        recording.stop();
        release.send(()).unwrap();
        let (result, audio) = recording.finish(TIMEOUT);
        assert_eq!(result.unwrap().text(), "1600");
        assert_eq!(audio.samples, samples);
    }

    #[test]
    fn a_failed_connection_keeps_audio_spoken_after_failure() {
        let (microphone, input) = Microphone::fake(RATE);
        let engine = engine(None, true);
        let (levels, received) = mpsc::channel();
        let recording = Recording::start(
            &microphone,
            &engine,
            AsrOptions::default(),
            move |level, _| {
                let _ = levels.send(level);
            },
        )
        .unwrap();
        let mut updates = recording.listening.updates();
        assert!(matches!(
            updates.recv(TIMEOUT).unwrap(),
            speechkit::asr::AsrUpdate::Closed(Err(_))
        ));
        let samples = vec![0.25; 1600];
        input.push(&samples);
        wait_recorded(&recording, samples.len());
        // Closed recognition must leave the shared observer metering.
        let deadline = Instant::now() + TIMEOUT;
        while received
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
            == 0.0
        {}
        let (result, audio) = recording.finish(TIMEOUT);
        assert!(result.unwrap_err().error.retryable());
        assert_eq!(audio.samples, samples);
    }

    #[test]
    fn one_observer_reports_live_text_and_input_level() {
        let (microphone, input) = Microphone::fake(RATE);
        let engine = engine(None, false);
        let (updates, received) = mpsc::channel();
        let recording = Recording::start(
            &microphone,
            &engine,
            AsrOptions::default(),
            move |level, text| {
                let _ = updates.send((level, text));
            },
        )
        .unwrap();
        input.push(&vec![0.25; 1600]);
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let (level, text) = received
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if text.as_deref() == Some("1600") {
                assert!(level > 0.0);
                break;
            }
        }
        let (result, audio) = recording.finish(TIMEOUT);
        assert_eq!(result.unwrap().text(), "1600");
        assert_eq!(audio.samples.len(), 1600);
    }

    #[test]
    fn dropping_a_short_dictation_cancels_the_session() {
        let (microphone, _) = Microphone::fake(RATE);
        let engine = engine(None, false);
        let recording =
            Recording::start(&microphone, &engine, AsrOptions::default(), |_, _| {}).unwrap();
        let mut updates = recording.listening.updates();
        drop(recording);
        assert!(matches!(
            updates.recv(TIMEOUT).unwrap(),
            speechkit::asr::AsrUpdate::Closed(Err(_))
        ));
    }

    #[test]
    fn recorded_audio_can_be_retried_on_another_engine() {
        let (microphone, input) = Microphone::fake(RATE);
        let failed = engine(None, true);
        let recording =
            Recording::start(&microphone, &failed, AsrOptions::default(), |_, _| {}).unwrap();
        input.push(&vec![0.25; 1600]);
        wait_recorded(&recording, 1600);
        let (result, audio) = recording.finish(TIMEOUT);
        assert!(result.is_err());
        let backup = engine(None, false);
        let transcript = backup
            .transcribe(&audio, AsrOptions::default(), TIMEOUT)
            .unwrap();
        assert_eq!(transcript.text(), "1600");
    }

    /// Speaks for `seconds` at twenty times real time (each 100 ms of audio is
    /// pushed 5 ms after the last), so a test can outlast the 30 s speechkit
    /// holds by default in a couple of seconds. Returns the samples spoken.
    fn speak(input: &FakeMicrophone, seconds: usize) -> usize {
        let chunk = vec![0.25; 1600];
        for _ in 0..seconds * 10 {
            input.push(&chunk);
            std::thread::sleep(Duration::from_millis(5));
        }
        seconds * 10 * chunk.len()
    }

    #[test]
    fn a_session_that_runs_out_of_time_keeps_the_audio_it_never_got() {
        let (microphone, input) = Microphone::fake(RATE);
        let (release, gate) = mpsc::channel();
        let engine = engine(Some(gate), false);
        let recording =
            Recording::start(&microphone, &engine, AsrOptions::default(), |_, _| {}).unwrap();
        // The backend never finishes opening, so at the deadline the listening
        // still holds audio the session was not given: more than the session's
        // 2 s input queue.
        let spoken = speak(&input, 4);
        recording.stop();
        let (result, audio) = recording.finish(Duration::from_millis(200));
        assert!(matches!(
            result.unwrap_err().error,
            SpeechError::DeadlineExceeded
        ));
        assert_eq!(audio.samples.len(), spoken);
        release.send(()).unwrap();
    }

    #[test]
    fn a_stalled_engine_does_not_cost_the_audio_after_the_default_backlog() {
        let (microphone, input) = Microphone::fake(RATE);
        let (release, gate) = mpsc::channel();
        let engine = engine(Some(gate), false);
        let recording =
            Recording::start(&microphone, &engine, AsrOptions::default(), |_, _| {}).unwrap();
        // 35 s with the engine stuck: past the 30 s a listening holds by
        // default, after which it would fail with `Capacity` and drop what it
        // had not read, leaving only the first seconds for Retry.
        let spoken = speak(&input, 35);
        recording.stop();
        let (result, audio) = recording.finish(Duration::from_millis(200));
        assert!(matches!(
            result.unwrap_err().error,
            SpeechError::DeadlineExceeded
        ));
        assert_eq!(audio.samples.len(), spoken);
        release.send(()).unwrap();
    }

    #[test]
    fn a_listening_handed_over_on_the_same_capture_leaves_no_gap() {
        // As a voice note rolls over: the next listening starts at the
        // capture's position, then the running one stops, and is cut there.
        let (microphone, input) = Microphone::fake(RATE);
        let engine = engine(None, false);
        let capture = microphone
            .capture(CaptureOptions::default().with_history(Duration::from_secs(5)))
            .unwrap();
        let listen = |from| {
            Recording::listen_on(&capture, &engine, AsrOptions::default(), MAX_RECORDING, from, |_, _| {})
                .unwrap()
        };
        let first = listen(None);
        let mut spoken = speak(&input, 2);
        let handover = capture.position();
        let second = listen(Some(handover));
        first.stop();
        spoken += speak(&input, 2);
        std::thread::sleep(Duration::from_millis(200));
        second.stop();

        let origin = first.origin();
        let (_, mut a) = first.finish(TIMEOUT);
        let keep = RATE.frames_in(handover - origin);
        a.samples.truncate(usize::try_from(keep).unwrap());
        // The second listening's times are capture times, from the handover.
        assert_eq!(second.origin(), handover);
        let (result, b) = second.finish(TIMEOUT);
        // Cut at the handover, every sample is in exactly one part.
        assert_eq!(a.samples.len() + b.samples.len(), spoken);
        assert!(result.unwrap().segments.iter().all(|s| s.end > handover));
    }

    #[test]
    fn a_lost_microphone_ends_the_listening_at_once() {
        // What a voice note's ticker looks for to go on without it.
        let (microphone, input) = Microphone::fake(RATE);
        let engine = engine(None, false);
        let capture = microphone.capture(CaptureOptions::default()).unwrap();
        let recording =
            Recording::listen_on(&capture, &engine, AsrOptions::default(), MAX_RECORDING, None, |_, _| {})
                .unwrap();
        speak(&input, 1);
        assert!(!recording.ended() && !capture.device_lost());
        input.lose();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !recording.ended() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(capture.device_lost());
        assert!(recording.ended());
    }

    #[test]
    fn the_observer_goes_quiet_at_key_up_while_the_session_still_works() {
        let (microphone, _) = Microphone::fake(RATE);
        let (release, gate) = mpsc::channel();
        let engine = engine(Some(gate), false);
        let reports = Arc::new(AtomicUsize::new(0));
        let counted = reports.clone();
        let recording =
            Recording::start(&microphone, &engine, AsrOptions::default(), move |_, _| {
                counted.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();
        recording.stop();
        // The session is still opening, so the listening has not ended, and
        // an observer that waited for that would go on reporting.
        std::thread::sleep(Duration::from_millis(200));
        let settled = reports.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(reports.load(Ordering::SeqCst), settled);
        release.send(()).unwrap();
        assert!(recording.finish(TIMEOUT).0.is_ok());
    }
}
