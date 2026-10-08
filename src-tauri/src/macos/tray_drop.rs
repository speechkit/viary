//! Files dropped on the menu bar icon go to Transcripts.
//!
//! tray-icon covers the status button with its own view for clicks, so the
//! drop target is not a view: the status item's window takes file drags
//! and forwards them to its delegate, this object. While a drag that holds
//! something to transcribe is over the icon, the button is highlighted.

use std::{path::PathBuf, sync::OnceLock};

use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject},
};
use objc2_app_kit::{
    NSDragOperation, NSDraggingDestination, NSDraggingInfo, NSPasteboardTypeFileURL,
    NSStatusBarButton, NSStatusItem, NSWindowDelegate,
};
use objc2_foundation::{NSArray, NSURL};

type OnDrop = Box<dyn Fn(Vec<PathBuf>) + Send + Sync>;

static ON_DROP: OnceLock<OnDrop> = OnceLock::new();

pub struct Ivars {
    button: Retained<NSStatusBarButton>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ViaryTrayDrop"]
    #[ivars = Ivars]
    struct TrayDrop;

    unsafe impl NSObjectProtocol for TrayDrop {}

    unsafe impl NSWindowDelegate for TrayDrop {}

    unsafe impl NSDraggingDestination for TrayDrop {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            let accept = dropped_paths(sender).iter().any(|p| transcribable(p));
            self.ivars().button.highlight(accept);
            if accept {
                NSDragOperation::Copy
            } else {
                NSDragOperation::None
            }
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            self.ivars().button.highlight(false);
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> Bool {
            self.ivars().button.highlight(false);
            let paths: Vec<PathBuf> = dropped_paths(sender)
                .into_iter()
                .filter(|p| transcribable(p))
                .collect();
            if paths.is_empty() {
                return Bool::NO;
            }
            if let Some(on_drop) = ON_DROP.get() {
                on_drop(paths);
            }
            Bool::YES
        }
    }
);

/// The local paths of the files and folders a drag holds.
fn dropped_paths(sender: &ProtocolObject<dyn NSDraggingInfo>) -> Vec<PathBuf> {
    let pasteboard = sender.draggingPasteboard();
    let Some(items) = pasteboard.pasteboardItems() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            // SAFETY: an AppKit constant.
            let url = item.stringForType(unsafe { NSPasteboardTypeFileURL })?;
            let url = NSURL::URLWithString(&url)?;
            Some(PathBuf::from(url.path()?.to_string()))
        })
        .collect()
}

/// A folder (its files are looked through later), or a file speechkit
/// decodes in this build.
fn transcribable(path: &std::path::Path) -> bool {
    path.is_dir() || crate::transcriber::decodable(path)
}

/// Makes `item`'s icon a drop target that calls `on_drop` with what was
/// dropped. Call once, on the main thread.
pub fn install(item: &NSStatusItem, on_drop: impl Fn(Vec<PathBuf>) + Send + Sync + 'static) {
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("the menu bar drop target must be installed on the main thread");
        return;
    };
    if ON_DROP.set(Box::new(on_drop)).is_err() {
        return;
    }
    let Some(button) = item.button(mtm) else {
        return;
    };
    let Some(window) = button.window() else {
        tracing::warn!("the menu bar icon has no window yet; dropping files on it is off");
        return;
    };
    let this = mtm.alloc::<TrayDrop>().set_ivars(Ivars { button });
    // SAFETY: NSObject's designated initializer.
    let delegate: Retained<TrayDrop> = unsafe { msg_send![super(this), init] };
    // SAFETY: an AppKit constant.
    window.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The window holds its delegate weakly; this one lives as long as Viary.
    std::mem::forget(delegate);
}
