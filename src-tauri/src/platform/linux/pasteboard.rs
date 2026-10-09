//! The clipboard, through X11: on Wayland, XWayland's, which GNOME keeps
//! in step with the Wayland one. X11 has no change count, so a write is
//! known by its text.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::Mutex,
};

use arboard::{Clipboard, ImageData, SetExtLinux};

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

/// What the user had copied, in the richest form Viary can put back.
enum Content {
    /// Files copied in Files (Nautilus).
    Files(Vec<PathBuf>),
    /// An image, such as a screenshot.
    Image(ImageData<'static>),
    /// Formatted text, with its plain-text form.
    Html { html: String, text: Option<String> },
    Text(String),
}

/// What the user had copied, if anything Viary can read.
pub struct Saved(Option<Content>);

fn mark(text: &str) -> isize {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish() as isize
}

/// Files and images first: an app copying an image also offers HTML
/// pointing at it, and Files offers the paths as text.
fn read(c: &mut Clipboard) -> Option<Content> {
    if let Ok(files) = c.get().file_list()
        && !files.is_empty()
    {
        return Some(Content::Files(files));
    }
    if let Ok(image) = c.get().image() {
        return Some(Content::Image(image));
    }
    if let Ok(html) = c.get().html() {
        return Some(Content::Html {
            html,
            text: c.get_text().ok(),
        });
    }
    c.get_text().ok().map(Content::Text)
}

pub fn save() -> Saved {
    Saved(with(read).flatten())
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
    let Some(content) = saved.0 else {
        return;
    };
    with(|c| {
        // The user's copy is already in any clipboard history.
        let set = c.set().exclude_from_history();
        let restored = match content {
            Content::Files(files) => set.file_list(files.as_slice()),
            Content::Image(image) => set.image(image),
            Content::Html { html, text } => set.html(html.as_str(), text.as_deref()),
            Content::Text(text) => set.text(text),
        };
        if let Err(error) = restored {
            tracing::warn!(%error, "cannot put the clipboard back");
        }
    });
}
