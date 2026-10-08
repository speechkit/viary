//! macOS: the hold-to-talk key, typing into other apps, privacy
//! permissions, and files dropped on the menu bar icon.

pub mod apps;
pub mod focus;
pub mod hotkey;
pub mod keys;
pub mod pasteboard;
pub mod permissions;
pub mod tray_drop;

/// How the user pastes text left on the clipboard.
pub const PASTE_HINT: &str = "⌘V to paste";
/// The key that undoes an insertion.
pub const UNDO_KEY: &str = "⌘Z";
