//! The clipboard: put text on it for Ctrl+V, then put back whatever the
//! user had copied, every format that holds plain memory.

use std::time::Duration;

use clipboard_win::{formats, raw};

/// A copy of the clipboard's contents; `None` when the clipboard could not
/// be read, so there is nothing to put back.
pub struct Saved {
    formats: Option<Vec<(u32, Vec<u8>)>>,
}

/// Formats Windows makes from others, or that hold GDI handles rather than
/// memory: they cannot be copied as bytes, and come back by themselves.
const SKIPPED: &[u32] = &[
    formats::CF_BITMAP,
    formats::CF_METAFILEPICT,
    formats::CF_PALETTE,
    formats::CF_ENHMETAFILE,
    formats::CF_OWNERDISPLAY,
    formats::CF_DSPBITMAP,
    formats::CF_DSPMETAFILEPICT,
    formats::CF_DSPENHMETAFILE,
    formats::CF_TEXT,
    formats::CF_OEMTEXT,
    formats::CF_LOCALE,
    formats::CF_DIBV5,
];

/// Opens the clipboard, waiting a moment if another app holds it.
fn open() -> Option<Clipboard> {
    for _ in 0..20 {
        if raw::open().is_ok() {
            return Some(Clipboard);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    tracing::warn!("the clipboard is busy");
    None
}

/// Closes the clipboard when dropped.
struct Clipboard;

impl Drop for Clipboard {
    fn drop(&mut self) {
        let _ = raw::close();
    }
}

pub fn save() -> Saved {
    let Some(_open) = open() else {
        return Saved { formats: None };
    };
    let formats = raw::EnumFormats::new()
        .filter(|format| !SKIPPED.contains(format))
        .filter_map(|format| {
            let mut bytes = Vec::new();
            raw::get_vec(format, &mut bytes).ok()?;
            Some((format, bytes))
        })
        .collect();
    Saved {
        formats: Some(formats),
    }
}

/// Marks a write as one clipboard history and cloud sync leave out, and
/// that clipboard managers skip.
fn mark_transient() {
    for (name, value) in [
        ("ExcludeClipboardContentFromMonitorProcessing", &[0_u8; 0][..]),
        ("CanIncludeInClipboardHistory", &0_u32.to_le_bytes()[..]),
        ("CanUploadToCloudClipboard", &0_u32.to_le_bytes()[..]),
    ] {
        if let Some(format) = raw::register_format(name) {
            let _ = raw::set_without_clear(format.get(), value);
        }
    }
}

fn write_text(text: &str, transient: bool) -> isize {
    let Some(_open) = open() else {
        return 0;
    };
    let _ = raw::empty();
    if let Err(error) = raw::set_string(text) {
        tracing::warn!(%error, "cannot put the text on the clipboard");
    }
    if transient {
        mark_transient();
    }
    raw::seq_num().map_or(0, |n| n.get() as isize)
}

/// Replaces the contents with `text`, for the user to paste, and returns
/// the sequence number that identifies this write.
pub fn set_text(text: &str) -> isize {
    write_text(text, false)
}

/// Like [`set_text`], for text on the clipboard only long enough for
/// Ctrl+V: clipboard history and managers do not record it.
pub fn set_transient_text(text: &str) -> isize {
    write_text(text, true)
}

/// Puts `saved` back, unless something else was copied since `ours`. A
/// clipboard that could not be saved is left with the dictated text rather
/// than emptied.
pub fn restore(saved: Saved, ours: isize) {
    let Some(formats) = saved.formats else {
        return;
    };
    if raw::seq_num().map_or(0, |n| n.get() as isize) != ours {
        return;
    }
    let Some(_open) = open() else {
        return;
    };
    let _ = raw::empty();
    for (format, bytes) in &formats {
        if let Err(error) = raw::set_without_clear(*format, bytes) {
            tracing::debug!(format, %error, "cannot restore a clipboard format");
        }
    }
    // Windows history already has the user's copy from when it was made.
    mark_transient();
}
