//! The general pasteboard: put text on it for ⌘V, then put back whatever
//! the user had copied, every type of every item.

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSString};

/// A copy of the pasteboard's contents.
pub struct Saved {
    items: Vec<Vec<(Retained<NSString>, Retained<NSData>)>>,
}

// NSString and NSData are immutable here, so the copy may move between
// threads.
unsafe impl Send for Saved {}

pub fn save() -> Saved {
    let board = NSPasteboard::generalPasteboard();
    let items = board
        .pasteboardItems()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    item.types()
                        .iter()
                        .filter_map(|kind| item.dataForType(&kind).map(|data| (kind, data)))
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default();
    Saved { items }
}

/// Tells clipboard managers to leave a write out of their history
/// (<http://nspasteboard.org>).
const TRANSIENT: &str = "org.nspasteboard.TransientType";

fn write_text(text: &str, transient: bool) -> isize {
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    // SAFETY: NSPasteboardTypeString is a constant AppKit exports.
    let kind = unsafe { NSPasteboardTypeString };
    board.setString_forType(&NSString::from_str(text), kind);
    if transient {
        board.setData_forType(Some(&NSData::new()), &NSString::from_str(TRANSIENT));
    }
    board.changeCount()
}

/// Replaces the contents with `text`, for the user to paste, and returns
/// the change count that identifies this write.
pub fn set_text(text: &str) -> isize {
    write_text(text, false)
}

/// Like [`set_text`], for text on the clipboard only long enough for ⌘V:
/// clipboard managers do not record it.
pub fn set_transient_text(text: &str) -> isize {
    write_text(text, true)
}

/// Puts `saved` back, unless something else was copied since `ours`. The
/// restore is marked transient too: managers already recorded the user's
/// copy when it was made.
pub fn restore(saved: Saved, ours: isize) {
    let board = NSPasteboard::generalPasteboard();
    if board.changeCount() != ours {
        return;
    }
    board.clearContents();
    if saved.items.is_empty() {
        return;
    }
    let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = saved
        .items
        .into_iter()
        .map(|types| {
            let item = NSPasteboardItem::new();
            for (kind, data) in &types {
                item.setData_forType(data, kind);
            }
            item.setData_forType(&NSData::new(), &NSString::from_str(TRANSIENT));
            ProtocolObject::from_retained(item)
        })
        .collect();
    board.writeObjects(&NSArray::from_retained_slice(&items));
}
