//! A microphone test for the setup window: the chosen microphone's level,
//! sent as `mic-level` events twenty times a second while the test runs,
//! or a `mic-error` event when the microphone cannot be opened. Nothing is
//! recorded or transcribed.

use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use speechkit::io::CaptureOptions;
use tauri::{AppHandle, Emitter, Manager};

use crate::{App, lock, recording::Recording};

/// The running test: its capture, and the flag that ends its thread.
struct Running {
    capture: speechkit::io::Capture,
    stop: Arc<AtomicBool>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// Starts (`on`) or ends the test. Returns at once: opening a microphone
/// can take a while (a Bluetooth headset), so a thread of its own does it,
/// one request after another, and only the latest of those waiting: a
/// start then an end, as a window loses the focus, ends it.
pub fn request(app: &AppHandle, on: bool) {
    static REQUESTS: OnceLock<mpsc::Sender<bool>> = OnceLock::new();
    let requests = REQUESTS.get_or_init(|| {
        let (sender, received) = mpsc::channel::<bool>();
        let app = app.clone();
        let spawned = std::thread::Builder::new()
            .name("viary-mic-test-requests".into())
            .spawn(move || {
                while let Ok(mut on) = received.recv() {
                    while let Ok(later) = received.try_recv() {
                        on = later;
                    }
                    if !on {
                        stop();
                    } else if let Err(error) = start(&app) {
                        let _ = app.emit("mic-error", error);
                    }
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "cannot start the microphone test");
        }
        sender
    });
    let _ = requests.send(on);
}

/// Starts the test on the microphone in the settings, ending any test
/// already running (the microphone may have changed).
fn start(app: &AppHandle) -> Result<(), String> {
    stop();
    let name = app.state::<App>().settings().microphone;
    let microphone = Recording::open_microphone(name.as_deref()).map_err(|e| crate::engines::describe(&e))?;
    let capture = microphone
        .capture(CaptureOptions::default())
        .map_err(|e| crate::engines::describe(&e))?;
    let flag = Arc::new(AtomicBool::new(false));
    let (meter, stopped, app) = (capture.clone(), flag.clone(), app.clone());
    std::thread::Builder::new()
        .name("viary-mic-test".into())
        .spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let _ = app.emit("mic-level", meter.level());
                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .map_err(|e| e.to_string())?;
    *lock(&RUNNING) = Some(Running { capture, stop: flag });
    Ok(())
}

/// Ends the test and releases the microphone.
fn stop() {
    if let Some(running) = lock(&RUNNING).take() {
        running.stop.store(true, Ordering::Release);
        running.capture.stop();
    }
}
