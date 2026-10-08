//! The app the user is dictating into.

use std::time::{Duration, Instant};

use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};

/// The frontmost app when a dictation started.
#[derive(Debug, Clone, Default)]
pub struct TargetApp {
    pub pid: i32,
    pub name: String,
}

impl TargetApp {
    pub fn is_self(&self) -> bool {
        self.pid == std::process::id().cast_signed()
    }
}

pub fn frontmost() -> Option<TargetApp> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(TargetApp {
        pid: app.processIdentifier(),
        name: app
            .localizedName()
            .map(|n| n.to_string())
            .unwrap_or_default(),
    })
}

/// Brings `target` back to the front, for example after a click on the
/// pill activated Viary, and waits until it is. Returns whether it is.
pub fn activate(target: &TargetApp) -> bool {
    if frontmost().is_some_and(|app| app.pid == target.pid) {
        return true;
    }
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(target.pid)
    else {
        return false;
    };
    #[expect(deprecated, reason = "the only option that works before macOS 14")]
    let options = NSApplicationActivationOptions::ActivateIgnoringOtherApps;
    app.activateWithOptions(options);
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        if frontmost().is_some_and(|app| app.pid == target.pid) {
            // Give the app's key window a moment to take focus again.
            std::thread::sleep(Duration::from_millis(60));
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    false
}

/// Whether the system keeps Viary's keystrokes from `target`: never here.
pub fn is_protected(_target: &TargetApp) -> bool {
    false
}
