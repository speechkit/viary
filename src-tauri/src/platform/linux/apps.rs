//! The app the user is dictating into. Wayland does not tell other apps
//! which window is in front.

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
    None
}

pub fn activate(_target: &TargetApp) -> bool {
    true
}

/// Whether the system keeps Viary's keystrokes from `target`: never here.
pub fn is_protected(_target: &TargetApp) -> bool {
    false
}
