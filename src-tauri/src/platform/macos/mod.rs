//! The parts of Viary that talk to macOS directly: the hold-to-talk key,
//! typing into other apps, privacy permissions, and files dropped on the
//! menu bar icon.

pub mod apps;
pub mod focus;
pub mod hotkey;
pub mod keys;
pub mod pasteboard;
pub mod permissions;
pub mod tray_drop;
