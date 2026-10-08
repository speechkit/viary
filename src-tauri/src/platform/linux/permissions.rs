//! What the desktop lets Viary do: not checked yet.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    pub accessibility: bool,
    pub input_monitoring: bool,
}

pub fn check() -> Permissions {
    Permissions {
        accessibility: false,
        input_monitoring: false,
    }
}

pub fn can_type() -> bool {
    false
}

pub const ALLOW_TYPING: &str = "Typing into apps is not available yet";

pub fn request(_kind: &str) {}

pub fn open_settings(_kind: &str) {}
