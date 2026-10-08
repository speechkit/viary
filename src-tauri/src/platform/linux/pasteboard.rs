//! The clipboard, through X11: on Wayland, XWayland's, which GNOME keeps
//! in step with the Wayland one. X11 has no change count, so a write is
//! known by its text.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Mutex,
};

use arboard::{Clipboard, SetExtLinux};

use crate::lock;

/// On X11 the owner serves the clipboard: it lives as long as Viary.
static CLIPBOARD: Mutex<Option<Clipboard>> = Mutex::new(None);

fn with<T>(use_it: impl FnOnce(&mut Clipboard) -> T) -> Option<T> {
    let mut held = lock(&CLIPBOARD);
    if held.is_none() {
        match Clipboard::new() {
            Ok(clipboard) => *held = Some(clipboard),
            Err(error) => {
                tracing::warn!(%error, "cannot open the clipboard");
                return None;
            }
        }
    }
    held.as_mut().map(use_it)
}

/// The text the user had copied, if any.
pub struct Saved(Option<String>);

fn mark(text: &str) -> isize {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish() as isize
}

pub fn save() -> Saved {
    Saved(with(|c| c.get_text().ok()).flatten())
}

fn write(text: &str, transient: bool) -> isize {
    with(|c| {
        let set = c.set();
        let set = if transient { set.exclude_from_history() } else { set };
        if let Err(error) = set.text(text) {
            tracing::warn!(%error, "cannot put the text on the clipboard");
        }
    });
    mark(text)
}

pub fn set_text(text: &str) -> isize {
    write(text, false)
}

/// Like [`set_text`], marked for clipboard managers to leave out.
pub fn set_transient_text(text: &str) -> isize {
    write(text, true)
}

/// Puts `saved` back, unless something else was copied since `ours`.
pub fn restore(saved: Saved, ours: isize) {
    let current = with(|c| c.get_text().ok()).flatten();
    if current.as_deref().map(mark) != Some(ours) {
        return;
    }
    if let Some(text) = saved.0 {
        write(&text, true);
    }
}
