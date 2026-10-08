//! A microphone test for the setup window: the chosen microphone's level,
//! sent as `mic-level` events twenty times a second while the test runs.
//! Nothing is recorded or transcribed.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
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

/// Starts the test on the microphone in the settings, ending any test
/// already running (the microphone may have changed).
pub fn start(app: &AppHandle) -> Result<(), String> {
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
pub fn stop() {
    if let Some(running) = lock(&RUNNING).take() {
        running.stop.store(true, Ordering::Release);
        running.capture.stop();
    }
}
