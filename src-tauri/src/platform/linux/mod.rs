//! Linux (GNOME): not written yet. Every item here keeps the shape the rest
//! of Viary expects, so the app builds and its windows, engines, Voice
//! Notes, and Transcripts work; dictation leaves text on no clipboard and
//! the talk shortcut never fires.

pub mod apps;
pub mod focus;
pub mod hotkey;
pub mod keys;
pub mod pasteboard;
pub mod permissions;

/// How the user pastes text left on the clipboard.
pub const PASTE_HINT: &str = "Ctrl+V to paste";
/// The key that undoes an insertion.
pub const UNDO_KEY: &str = "Ctrl+Z";
