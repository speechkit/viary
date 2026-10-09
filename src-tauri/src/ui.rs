//! Viary's windows and menu bar icon: the pill overlay, the menu bar
//! popover, and the main window.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
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
    Transcribing,
    Polishing,
    Failed,
    /// Idle, with dictation paused from the tray.
    Paused,
    /// Idle, with the voice engine still loading, as after login.
    Loading,
    /// Idle, but something is missing: the microphone, the talk key, a
    /// permission, or a voice engine.
    Attention,
}

impl From<u8> for TrayState {
    fn from(code: u8) -> Self {
        [
            Self::Idle,
            Self::Listening,
            Self::Transcribing,
            Self::Polishing,
            Self::Failed,
            Self::Paused,
            Self::Loading,
            Self::Attention,
        ]
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
    // An engine loading, a permission granted: the icon may change too,
    // and on Linux the menu, which GNOME shows without asking first. On a
    // thread of its own, so no caller's lock is held while they read the
    // state, and once for a burst of changes.
    if !TRAY_PENDING.swap(true, Ordering::SeqCst) {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            // Cleared before reading, so a change from here on queues
            // another update.
            TRAY_PENDING.store(false, Ordering::SeqCst);
            redraw_tray(&app);
            #[cfg(target_os = "linux")]
            crate::tray_menu::refresh(&app);
        });
    }
}

/// Whether a tray update is queued by [`refresh`].
static TRAY_PENDING: AtomicBool = AtomicBool::new(false);

pub fn show_pill(app: &AppHandle, view: &PillView) {
    app.state::<App>().set_pill(view.clone());
    // Wayland has no pill window: the shell draws it when Viary's
    // extension runs; otherwise results come as notifications.
    #[cfg(target_os = "linux")]
    if crate::platform::shell_pill() {
        crate::platform::extension::show_pill(view);
    } else if crate::platform::is_wayland() {
        crate::platform::notify::pill(view);
    }
    if let Some(pill) = app.get_webview_window("pill") {
        let _ = pill.set_ignore_cursor_events(!view.interactive());
    }
    let _ = app.emit("pill-state", view);
}

/// The microphone level while listening, for the pill's bars.
pub fn pill_level(app: &AppHandle, level: f32) {
    let _ = app.emit_to("pill", "pill-level", level);
    #[cfg(target_os = "linux")]
    if crate::platform::shell_pill() {
        crate::platform::extension::level(level);
    }
}

/// The live text of dictation `token`, for the pill.
pub fn pill_partial(app: &AppHandle, token: u64, text: String) {
    #[cfg(target_os = "linux")]
    if crate::platform::shell_pill() {
        crate::platform::extension::partial(token, text.clone());
    }
    let _ = app.emit_to("pill", "pill-partial", (token, text));
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

/// Whether the plain icon is a template image macOS colors for the menu
/// bar. Elsewhere Viary colors it.
const TEMPLATE: bool = cfg!(target_os = "macos");

/// The state the dictation last reported.
static DICTATION: AtomicU8 = AtomicU8::new(TrayState::Idle as u8);
/// The state and tooltip the icon shows, so it is drawn only on a change.
static SHOWN: Mutex<Option<(TrayState, String)>> = Mutex::new(None);

/// What the icon shows while the dictation is `dictation`: the dictation
/// first, then a pause, the engine loading, and anything missing.
fn resolve(dictation: TrayState, paused: bool, loading: bool, attention: bool) -> TrayState {
    match dictation {
        TrayState::Idle if paused => TrayState::Paused,
        TrayState::Idle if loading => TrayState::Loading,
        TrayState::Idle if attention => TrayState::Attention,
        state => state,
    }
}

/// How a state is drawn: the mark's color, its dot, and whether it is
/// struck through. On a dark bar (GNOME's, a dark taskbar or menu bar) the
/// dots are lighter, as the design draws them.
struct Look {
    ink: [u8; 3],
    badge: Option<[u8; 3]>,
    struck: bool,
    /// A template image macOS colors for the menu bar itself; it loses the
    /// dot's color and the gray, so only the plain mark is one.
    template: bool,
}

fn look(state: TrayState, dark: bool) -> Look {
    let pick = |light: [u8; 3], on_dark: [u8; 3]| Some(if dark { on_dark } else { light });
    let gray = if dark { [0x80; 3] } else { [0x8A; 3] };
    let (badge, dim, struck) = match state {
        TrayState::Idle => (None, false, false),
        TrayState::Listening => (pick([0xE0, 0x45, 0x2B], [0xFF, 0x5A, 0x36]), false, false),
        TrayState::Transcribing | TrayState::Polishing => {
            (pick([0x2F, 0x46, 0xC8], [0x7E, 0xA7, 0xFF]), false, false)
        }
        TrayState::Failed | TrayState::Attention => {
            (pick([0xD0, 0x8A, 0x00], [0xF5, 0xC2, 0x11]), false, false)
        }
        TrayState::Loading => (Some(gray), true, false),
        TrayState::Paused => (None, true, true),
    };
    let template = TEMPLATE && badge.is_none() && !dim;
    let ink = if dim {
        gray
    } else if !template && dark {
        [0xFF; 3]
    } else {
        [0; 3]
    };
    Look { ink, badge, struck, template }
}

/// "for 12 more minutes", "for 1 hour": what is left of a pause.
fn paused_for(left_ms: u64) -> String {
    let minutes = left_ms.div_ceil(60_000).max(1);
    match minutes {
        1 => "for 1 more minute".into(),
        60 => "for 1 hour".into(),
        m if m > 60 && m % 60 == 0 => format!("for {} hours", m / 60),
        m => format!("for {m} more minutes"),
    }
}

/// What Viary is doing, as the tooltip says it after "Viary: ": "hold
/// Right Alt", "listening", "loading SenseVoice", "microphone blocked".
fn status_text(app: &AppHandle, state: TrayState, attention: Option<&str>) -> String {
    let viary = app.state::<App>();
    match state {
        TrayState::Idle => format!("hold {}", crate::dictation::key_name(viary.settings().hotkey)),
        TrayState::Listening => "listening".into(),
        TrayState::Transcribing => "transcribing".into(),
        TrayState::Polishing => "polishing".into(),
        TrayState::Failed => "dictation failed · audio kept".into(),
        TrayState::Attention => attention.unwrap_or("needs attention").into(),
        TrayState::Loading => match viary.engines.status().loading {
            Some(id) => format!("loading {}", crate::dictation::engine_name(&viary.settings(), &id)),
            None => "loading".into(),
        },
        TrayState::Paused => match viary.pause.get().and_then(|p| p.until) {
            Some(until) => {
                format!("paused {}", paused_for(until.saturating_sub(crate::pause::now_ms())))
            }
            None => "paused until you resume".into(),
        },
    }
}

/// What is missing for dictation, if anything.
fn attention(app: &AppHandle) -> Option<&'static str> {
    let viary = app.state::<App>();
    let hotkey = viary.hotkey.get().is_some_and(crate::platform::hotkey::HotkeyListener::is_active);
    if let Some(missing) = crate::platform::permissions::missing(hotkey) {
        return Some(missing);
    }
    let engines = viary.engines.status();
    (engines.active.is_none() && engines.loading.is_none()).then_some("choose a voice engine")
}

/// The state the icon shows now, and its tooltip after "Viary: ".
pub fn tray_status(app: &AppHandle) -> (TrayState, String) {
    let viary = app.state::<App>();
    let missing = attention(app);
    let state = resolve(
        TrayState::from(DICTATION.load(Ordering::SeqCst)),
        viary.pause.get().is_some(),
        viary.engines.status().loading.is_some(),
        missing.is_some(),
    );
    (state, status_text(app, state, missing))
}

/// The tray icon for the dictation's `state`.
pub fn set_tray(app: &AppHandle, state: TrayState) {
    DICTATION.store(state as u8, Ordering::SeqCst);
    redraw_tray(app);
}

/// Draws the icon and its tooltip again if what they show changed.
pub fn redraw_tray(app: &AppHandle) {
    // Before the icon exists there is nothing to draw, and nothing to
    // remember as drawn.
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let (state, text) = tray_status(app);
    {
        let mut shown = crate::lock(&SHOWN);
        if shown.as_ref().is_some_and(|(s, t)| *s == state && *t == text) {
            return;
        }
        *shown = Some((state, text.clone()));
    }
    let Look { ink, badge, struck, template } = look(state, dark_tray());
    let size = 44;
    let _ = tray.set_icon(Some(Image::new_owned(
        icons::tray(size, ink, badge, struck),
        size,
        size,
    )));
    let _ = tray.set_icon_as_template(template);
    let _ = tray.set_tooltip(Some(format!("Viary: {text}")));
}

pub fn tray_icon() -> Image<'static> {
    // A template's color is macOS's to choose: no need to ask the theme.
    let ink = look(TrayState::Idle, !TEMPLATE && dark_tray()).ink;
    Image::new_owned(icons::tray(44, ink, None, false), 44, 44)
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
/// `step` (`typing`, ...) opens it at that step.
pub fn build_setup(app: &AppHandle, step: Option<&str>) -> tauri::Result<WebviewWindow> {
    let url = match step {
        Some(step) => WebviewUrl::App(format!("index.html?step={step}").into()),
        None => WebviewUrl::default(),
    };
    let window = WebviewWindowBuilder::new(app, "setup", url)
        .title("Set up Viary")
        .inner_size(1040.0, 700.0)
        .resizable(false)
        .maximizable(false)
        .decorations(false)
        .center()
        .theme(Some(tauri::Theme::Light))
        .build()?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        // Closed some other way (Alt+F4): its page cannot end its tests.
        if let tauri::WindowEvent::Destroyed = event {
            end_setup_tests(&handle);
        }
    });
    Ok(window)
}

/// Ends the setup window's talk key and microphone tests, so the key
/// starts dictations again and the microphone is let go.
pub fn end_setup_tests(app: &AppHandle) {
    app.state::<App>().key_test.store(false, Ordering::SeqCst);
    crate::mic_test::stop();
}

/// The tip window: the card, and room around it for its shadow.
const TIP_SIZE: (f64, f64) = (388.0, 200.0);

/// Shows the first-launch tip above the tray, bottom right of the screen,
/// where Windows notifications appear. It closes from its own buttons.
pub fn show_tray_tip(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window("tip").is_some() {
        return Ok(());
    }
    let (width, height) = TIP_SIZE;
    let window = WebviewWindowBuilder::new(app, "tip", WebviewUrl::default())
        .title("Viary")
        .inner_size(width, height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        // Clicks without taking focus from the app being typed in.
        .focusable(false)
        .visible(false)
        .build()?;
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let at = area.position.to_logical::<f64>(scale);
        let size = area.size.to_logical::<f64>(scale);
        let _ = window.set_position(LogicalPosition::new(
            at.x + size.width - width,
            at.y + size.height - height,
        ));
    }
    window.show()
}

/// Opens setup at `step`, as "Allow typing…" in a notification does: the
/// window it left off in, or a new one.
#[cfg(target_os = "linux")]
pub fn open_setup(app: &AppHandle, step: Option<&str>) -> tauri::Result<()> {
    match app.get_webview_window("setup") {
        Some(setup) => {
            if let Some(step) = step {
                setup.emit("setup-step", step)?;
            }
            setup.show()?;
            setup.set_focus()
        }
        None => build_setup(app, step).map(drop),
    }
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

#[cfg(test)]
mod tray_tests {
    use super::*;

    #[test]
    fn a_dictation_shows_over_a_pause_and_a_pause_over_loading() {
        assert_eq!(resolve(TrayState::Listening, true, true, true), TrayState::Listening);
        assert_eq!(resolve(TrayState::Idle, true, true, true), TrayState::Paused);
        assert_eq!(resolve(TrayState::Idle, false, true, true), TrayState::Loading);
        assert_eq!(resolve(TrayState::Idle, false, false, true), TrayState::Attention);
        assert_eq!(resolve(TrayState::Idle, false, false, false), TrayState::Idle);
    }

    #[test]
    fn dark_bars_get_the_lighter_dots() {
        assert_eq!(look(TrayState::Listening, true).badge, Some([0xFF, 0x5A, 0x36]));
        assert_eq!(look(TrayState::Listening, false).badge, Some([0xE0, 0x45, 0x2B]));
        assert_eq!(look(TrayState::Attention, true).badge, Some([0xF5, 0xC2, 0x11]));
        let paused = look(TrayState::Paused, true);
        assert!(paused.struck && paused.badge.is_none() && !paused.template);
        assert_eq!(paused.ink, [0x80; 3]);
        assert_eq!(look(TrayState::Loading, false).ink, [0x8A; 3]);
    }

    #[test]
    fn a_pause_says_what_is_left() {
        assert_eq!(paused_for(60 * 60_000), "for 1 hour");
        assert_eq!(paused_for(12 * 60_000 - 5_000), "for 12 more minutes");
        assert_eq!(paused_for(10_000), "for 1 more minute");
        assert_eq!(paused_for(120 * 60_000), "for 2 hours");
    }
}
