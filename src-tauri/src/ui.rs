//! Viary's windows and menu bar icon: the pill overlay, the menu bar
//! popover, and the main window.

use std::sync::{
    Mutex,
    atomic::{AtomicU8, Ordering},
};

use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, image::Image,
};

use crate::{App, dictation::PillView, icons};

pub const TRAY_ID: &str = "viary";

/// The pill window before the pill measures itself, and the most it grows
/// to. Click-through except while it shows buttons.
const PILL_SIZE: (f64, f64) = (820.0, 120.0);
/// Room around the pill for its shadow and entry animation: each side,
/// above, and below.
const PILL_MARGIN: (f64, f64, f64) = (40.0, 40.0, 16.0);
const POPOVER_SIZE: (f64, f64) = (360.0, 560.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Listening,
    Working,
    Failed,
    /// Idle, with dictation paused from the tray.
    Paused,
}

impl From<u8> for TrayState {
    fn from(code: u8) -> Self {
        [Self::Idle, Self::Listening, Self::Working, Self::Failed, Self::Paused]
            .into_iter()
            .find(|state| *state as u8 == code)
            .unwrap_or(Self::Idle)
    }
}

/// Viary lives in the menu bar, out of the Dock and ⌘Tab. While the main
/// window is open it becomes a regular app, so ⌘Tab can switch back to it.
pub fn in_app_switcher(app: &AppHandle, shown: bool) {
    #[cfg(target_os = "macos")]
    {
        let policy = if shown {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        };
        if let Err(error) = app.set_activation_policy(policy) {
            tracing::warn!(%error, "cannot change the activation policy");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, shown);
}

/// Shows the Viary icon in the Dock and ⌘Tab. A bundled app gets it from
/// `icon.icns`; `tauri dev` runs a bare binary, which macOS would show as a
/// generic executable. Call on the main thread.
#[cfg(target_os = "macos")]
pub fn set_app_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../icons/icon.png"));
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        // SAFETY: on the main thread, with a valid image.
        unsafe {
            NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image))
        };
    }
}

/// Tells every window to reload what it shows from `get_state`.
pub fn refresh(app: &AppHandle) {
    let _ = app.emit("state-changed", ());
    // GNOME shows the menu without asking first: keep it current.
    #[cfg(target_os = "linux")]
    crate::tray_menu::refresh(app);
}

pub fn show_pill(app: &AppHandle, view: &PillView) {
    app.state::<App>().set_pill(view.clone());
    // Wayland has no pill window: results come as notifications.
    #[cfg(target_os = "linux")]
    if crate::platform::is_wayland() {
        crate::platform::notify::pill(view);
    }
    if let Some(pill) = app.get_webview_window("pill") {
        let _ = pill.set_ignore_cursor_events(!view.interactive());
    }
    let _ = app.emit("pill-state", view);
}

/// Opens the main window on `page` (`engine`, `history`, ...).
pub fn open_main(app: &AppHandle, page: &str) {
    if let Some(popover) = app.get_webview_window("popover") {
        let _ = popover.hide();
    }
    if let Some(main) = app.get_webview_window("main") {
        in_app_switcher(app, true);
        let _ = main.show();
        let _ = main.unminimize();
        let _ = main.set_focus();
        let _ = main.emit("navigate", page);
    }
}

/// Whether the bar the icon sits in is dark: the menu bar in Dark Mode, a
/// Windows taskbar in the dark theme, or GNOME's top bar, which always is.
fn dark_tray() -> bool {
    #[cfg(target_os = "macos")]
    return std::process::Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "Dark");
    #[cfg(target_os = "windows")]
    return crate::platform::dark_taskbar();
    #[cfg(target_os = "linux")]
    return true;
}

/// The icon's color: macOS tints a template itself, so only an icon with a
/// colored dot needs to know.
fn tray_ink(template: bool) -> [u8; 3] {
    if !template && dark_tray() { [0xFF; 3] } else { [0; 3] }
}

/// Whether the plain icon is a template image macOS colors for the menu
/// bar. Elsewhere Viary colors it.
const TEMPLATE: bool = cfg!(target_os = "macos");

/// The state the dictation last reported, and the one the icon shows.
static DICTATION: AtomicU8 = AtomicU8::new(TrayState::Idle as u8);
static SHOWN: AtomicU8 = AtomicU8::new(u8::MAX);

/// The tray icon for the dictation's `state`: the plain mark when idle,
/// with a colored dot while listening (red), working (blue), or failed
/// (amber); grayed and struck through while paused.
pub fn set_tray(app: &AppHandle, state: TrayState) {
    DICTATION.store(state as u8, Ordering::SeqCst);
    redraw_tray(app);
}

/// Draws the icon again, after the dictation or a pause changed.
pub fn redraw_tray(app: &AppHandle) {
    let state = TrayState::from(DICTATION.load(Ordering::SeqCst));
    let paused = app.state::<App>().pause.get();
    let shown = if state == TrayState::Idle && paused.is_some() {
        TrayState::Paused
    } else {
        state
    };
    if SHOWN.swap(shown as u8, Ordering::SeqCst) == shown as u8 && shown != TrayState::Paused {
        return;
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let badge = match shown {
        TrayState::Idle | TrayState::Paused => None,
        TrayState::Listening => Some([0xE0, 0x45, 0x2B]),
        TrayState::Working => Some([0x2F, 0x46, 0xC8]),
        TrayState::Failed => Some([0xD0, 0x8A, 0x00]),
    };
    let paused_ink = shown == TrayState::Paused;
    // A template image follows the menu bar's color but loses the dot's.
    let template = TEMPLATE && badge.is_none();
    let ink = if paused_ink { [0x8A; 3] } else { tray_ink(template) };
    let size = 44;
    let _ = tray.set_icon(Some(Image::new_owned(
        icons::tray(size, ink, badge, paused_ink),
        size,
        size,
    )));
    let _ = tray.set_icon_as_template(template && !paused_ink);
    let key = crate::dictation::key_name(app.state::<App>().settings().hotkey);
    let tip = match shown {
        TrayState::Idle => format!("Viary: hold {key}"),
        TrayState::Listening => "Viary: listening".into(),
        TrayState::Working => "Viary: working".into(),
        TrayState::Failed => "Viary: needs attention".into(),
        TrayState::Paused => match paused.and_then(|p| p.until) {
            Some(_) => "Viary: paused for a while".into(),
            None => "Viary: paused".into(),
        },
    };
    let _ = tray.set_tooltip(Some(tip));
}

pub fn tray_icon() -> Image<'static> {
    Image::new_owned(icons::tray(44, tray_ink(TEMPLATE), None, false), 44, 44)
}

pub fn build_main(app: &AppHandle, visible: bool) -> tauri::Result<WebviewWindow> {
    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Viary")
        .inner_size(1180.0, 780.0)
        .min_inner_size(960.0, 640.0)
        .theme(Some(tauri::Theme::Light))
        .visible(visible);
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    let window = builder.build()?;
    let handle = window.clone();
    window.on_window_event(move |event| {
        // Closing hides: Viary keeps running in the menu bar.
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = handle.hide();
            in_app_switcher(handle.app_handle(), false);
        }
    });
    Ok(window)
}

/// The setup window: its title bar is drawn in the page, as the design has
/// it. Hidden, not closed, by its close button: setup runs again next time.
pub fn build_setup(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, "setup", WebviewUrl::default())
        .title("Set up Viary")
        .inner_size(1040.0, 700.0)
        .resizable(false)
        .maximizable(false)
        .decorations(false)
        .center()
        .theme(Some(tauri::Theme::Light))
        .build()
}

pub fn build_pill(app: &AppHandle) -> tauri::Result<Option<WebviewWindow>> {
    // Wayland lets no app place a window or keep it above the others.
    #[cfg(target_os = "linux")]
    if crate::platform::is_wayland() {
        return Ok(None);
    }
    let (width, height) = PILL_SIZE;
    let window = WebviewWindowBuilder::new(app, "pill", WebviewUrl::default())
        .title("Viary dictation")
        .inner_size(width, height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        // Never the key window: a click on Stop or Undo must leave the
        // keyboard with the field being dictated into, Viary's own too.
        .focusable(false)
        .accept_first_mouse(true)
        .visible(false)
        .build()?;
    place_pill(&window, PILL_SIZE);
    let _ = window.set_ignore_cursor_events(true);
    #[cfg(target_os = "macos")]
    float_over_everything(&window);
    // Shown once and never hidden: re-showing a window can activate the
    // app and take focus from the field being dictated into.
    let _ = window.show();
    Ok(Some(window))
}

/// Fits the pill window around a pill of `width` x `height` (logical
/// pixels, as the pill measures itself), so that while it takes clicks it
/// covers nothing but the pill.
pub fn fit_pill(app: &AppHandle, width: f64, height: f64) {
    let Some(window) = app.get_webview_window("pill") else {
        return;
    };
    let (side, above, below) = PILL_MARGIN;
    let size = (
        (width + 2.0 * side).clamp(1.0, PILL_SIZE.0),
        (height + above + below).clamp(1.0, PILL_SIZE.1),
    );
    let _ = window.set_size(LogicalSize::new(size.0, size.1));
    place_pill(&window, size);
}

/// Bottom center of the screen with the menu bar, for a window of `size`:
/// over the Dock on macOS, above the taskbar or dock elsewhere.
fn place_pill(window: &WebviewWindow, (width, height): (f64, f64)) {
    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    #[cfg(target_os = "macos")]
    let (size, origin) = (monitor.size(), monitor.position());
    #[cfg(not(target_os = "macos"))]
    let (size, origin) = (&monitor.work_area().size, &monitor.work_area().position);
    let size = size.to_logical::<f64>(scale);
    let origin = origin.to_logical::<f64>(scale);
    let _ = window.set_position(LogicalPosition::new(
        origin.x + (size.width - width) / 2.0,
        origin.y + size.height - height - 24.0,
    ));
}

/// Above full-screen apps and on every Space, like the system's own HUDs.
#[cfg(target_os = "macos")]
fn float_over_everything(window: &WebviewWindow) {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
    let Ok(pointer) = window.ns_window() else {
        return;
    };
    // Raw pointers are not Send; the address crosses to the main thread.
    let pointer = pointer as usize;
    let _ = window.run_on_main_thread(move || {
        // SAFETY: Tauri hands out its live NSWindow, used on the main thread.
        let ns_window = unsafe { &*(pointer as *const NSWindow) };
        ns_window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        // NSStatusWindowLevel.
        ns_window.setLevel(25);
    });
}

pub fn build_popover(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let (width, height) = POPOVER_SIZE;
    let window = WebviewWindowBuilder::new(app, "popover", WebviewUrl::default())
        .title("Viary")
        .inner_size(width, height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .theme(Some(tauri::Theme::Light))
        .visible(false)
        .build()?;
    let handle = window.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Focused(false) => {
            let _ = handle.hide();
        }
        // The popover fits its height to its content; above a taskbar,
        // its bottom stays put.
        tauri::WindowEvent::Resized(_) => place_popover(&handle),
        _ => {}
    });
    Ok(window)
}

/// The tray icon the popover last opened from, in logical pixels:
/// `(x, y, width, height)`.
static POPOVER_ANCHOR: Mutex<Option<(f64, f64, f64, f64)>> = Mutex::new(None);

/// Below the icon in a menu bar or top bar; above it in a taskbar at the
/// bottom of the screen. Centered on the icon, and kept on its monitor.
fn place_popover(popover: &WebviewWindow) {
    let Some((x, y, width, height)) = *crate::lock(&POPOVER_ANCHOR) else {
        return;
    };
    let scale = popover.scale_factor().unwrap_or(1.0);
    let size = popover
        .inner_size()
        .map_or(POPOVER_SIZE.into(), |s| s.to_logical::<f64>(scale));
    let screen = popover
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let area = m.work_area();
            let at = area.position.to_logical::<f64>(scale);
            let size = area.size.to_logical::<f64>(scale);
            (at.x, at.y, size.width, size.height)
        });
    let left = x + width / 2.0 - size.width / 2.0;
    let below = y + height + 4.0;
    let (left, top) = match screen {
        Some((sx, sy, sw, sh)) => {
            let left = left.clamp(sx + 8.0, (sx + sw - size.width - 8.0).max(sx + 8.0));
            // An icon in the lower half sits in a taskbar at the bottom.
            let top = if y > sy + sh / 2.0 {
                (y - size.height - 4.0).max(sy)
            } else {
                below
            };
            (left, top)
        }
        None => (left.max(8.0), below),
    };
    let _ = popover.set_position(LogicalPosition::new(left, top));
}

/// Shows the popover under the menu bar icon, or hides it.
pub fn toggle_popover(app: &AppHandle, icon: tauri::Rect) {
    let Some(popover) = app.get_webview_window("popover") else {
        return;
    };
    if popover.is_visible().unwrap_or(false) {
        let _ = popover.hide();
        return;
    }
    let scale = popover.scale_factor().unwrap_or(1.0);
    let at = icon.position.to_logical::<f64>(scale);
    let size = icon.size.to_logical::<f64>(scale);
    *crate::lock(&POPOVER_ANCHOR) = Some((at.x, at.y, size.width, size.height));
    // The popover keeps the height it fitted to its content.
    place_popover(&popover);
    refresh(app);
    let _ = popover.show();
    let _ = popover.set_focus();
}
