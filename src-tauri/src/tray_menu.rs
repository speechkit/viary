//! The tray icon's menu on Windows (right click) and Linux (any click: GNOME
//! draws AppIndicator menus itself). macOS has the popover instead.
//!
//! The menu is rebuilt when the pointer reaches the icon on Windows, and
//! with every state change on Linux, which reports no pointer, so its
//! checks follow settings changed anywhere.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use tauri::{
    AppHandle, Manager, Wry,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};

use crate::{
    App, dictation,
    engines::{DASHSCOPE, OPENAI},
    lock,
    settings::Language,
    ui,
};

/// The microphones as last listed, by position in the Microphone menu, and
/// when: listing them takes a moment, so a recent list is reused.
static MICROPHONES: Mutex<(Vec<String>, Option<Instant>)> = Mutex::new((Vec::new(), None));
const LIST_EVERY: Duration = Duration::from_secs(5);

const LANGUAGES: [(Language, &str, &str); 3] = [
    (Language::Auto, "lang:auto", "Auto"),
    (Language::En, "lang:en", "English"),
    (Language::Zh, "lang:zh", "中文"),
];

pub fn refresh(app: &AppHandle) {
    match build(app) {
        Ok(menu) => {
            if let Some(tray) = app.tray_by_id(ui::TRAY_ID) {
                let _ = tray.set_menu(Some(menu));
            }
        }
        Err(error) => tracing::warn!(%error, "cannot build the tray menu"),
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<App>();
    let settings = state.settings();
    let key = dictation::key_name(settings.hotkey);
    let separator = || PredefinedMenuItem::separator(app);
    let menu = Menu::new(app)?;

    if cfg!(target_os = "linux") {
        // GNOME shows no tooltip: the menu says what state Viary is in.
        let status = match state.engines.status().active {
            Some(info) if info.on_device => "Ready · on-device".to_owned(),
            Some(info) => format!("Ready · {}", info.kind),
            None => "No voice engine".to_owned(),
        };
        menu.append(&MenuItem::with_id(app, "status", status, false, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(app, "hint", format!("Hold {key} to talk"), false, None::<&str>)?)?;
        menu.append(&separator()?)?;
        let polish = &settings.polish;
        menu.append(&CheckMenuItem::with_id(app, "polish", "Polish transcripts", true, polish.enabled, None::<&str>)?)?;
        menu.append(&CheckMenuItem::with_id(app, "translate", format!("Translate to {}", polish.translate_to), true, polish.translate, None::<&str>)?)?;
        menu.append(&separator()?)?;
    } else {
        menu.append(&MenuItem::with_id(app, "hands-free", format!("Start hands-free\t{key} ×2"), true, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(app, "paste-last", "Paste last dictation", true, None::<&str>)?)?;
        menu.append(&separator()?)?;
    }

    let language = Submenu::with_id(app, "language", "Language", true)?;
    for (value, id, label) in LANGUAGES {
        language.append(&CheckMenuItem::with_id(app, id, label, true, settings.language == value, None::<&str>)?)?;
    }
    menu.append(&language)?;

    let microphone = Submenu::with_id(app, "microphone", "Microphone", true)?;
    microphone.append(&CheckMenuItem::with_id(app, "mic:default", "Default", true, settings.microphone.is_none(), None::<&str>)?)?;
    for (i, name) in microphones().iter().enumerate() {
        let on = settings.microphone.as_deref() == Some(name.as_str());
        microphone.append(&CheckMenuItem::with_id(app, format!("mic:{i}"), name, true, on, None::<&str>)?)?;
    }
    menu.append(&microphone)?;

    let engine = Submenu::with_id(app, "engine", "Voice engine", true)?;
    let active = state.engines.status().active.map(|info| info.id);
    let mut ids: Vec<String> = settings.local_models.iter().map(|m| format!("local:{}", m.id)).collect();
    ids.extend(
        [OPENAI, DASHSCOPE]
            .into_iter()
            .filter(|id| dictation::cloud_ready(&settings, id))
            .map(str::to_owned),
    );
    for id in &ids {
        let name = dictation::engine_name(&settings, id);
        let on = active.as_deref() == Some(id.as_str());
        engine.append(&CheckMenuItem::with_id(app, format!("engine:{id}"), name, true, on, None::<&str>)?)?;
    }
    if !ids.is_empty() {
        engine.append(&separator()?)?;
    }
    engine.append(&MenuItem::with_id(app, "open:engine", "Voice engine settings…", true, None::<&str>)?)?;
    menu.append(&engine)?;
    menu.append(&separator()?)?;

    if cfg!(target_os = "linux") {
        menu.append(&MenuItem::with_id(app, "paste-last", "Paste last dictation", true, None::<&str>)?)?;
    }
    if state.pause.get().is_some() {
        menu.append(&MenuItem::with_id(app, "resume", "Resume dictation", true, None::<&str>)?)?;
    } else {
        let pause = Submenu::with_id(app, "pause", if cfg!(target_os = "linux") { "Pause" } else { "Pause dictation" }, true)?;
        pause.append(&MenuItem::with_id(app, "pause:15", "For 15 minutes", true, None::<&str>)?)?;
        pause.append(&MenuItem::with_id(app, "pause:60", "For 1 hour", true, None::<&str>)?)?;
        pause.append(&MenuItem::with_id(app, "pause:until", "Until I resume", true, None::<&str>)?)?;
        menu.append(&pause)?;
    }

    if cfg!(target_os = "linux") {
        menu.append(&separator()?)?;
        menu.append(&MenuItem::with_id(app, "open:home", "Open Viary", true, None::<&str>)?)?;
    } else {
        menu.append(&MenuItem::with_id(app, "open:history", "History", true, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(app, "open:settings", "Settings", true, None::<&str>)?)?;
        menu.append(&separator()?)?;
    }
    menu.append(&MenuItem::with_id(app, "quit", if cfg!(target_os = "linux") { "Quit" } else { "Quit Viary" }, true, None::<&str>)?)?;
    Ok(menu)
}

/// The microphones, listed again when the last list is a few seconds old.
fn microphones() -> Vec<String> {
    let mut cached = lock(&MICROPHONES);
    if cached.1.is_none_or(|at| at.elapsed() > LIST_EVERY) {
        cached.0 = speechkit::io::Microphone::list()
            .map(|list| list.into_iter().map(|m| m.name).collect())
            .unwrap_or_default();
        cached.1 = Some(Instant::now());
    }
    cached.0.clone()
}

/// Acts on a menu item.
pub fn on_event(app: &AppHandle, event: MenuEvent) {
    let state = app.state::<App>();
    let id = event.id().as_ref();
    match id {
        "hands-free" => state.send(dictation::Msg::StartHandsFree),
        "paste-last" => state.send(dictation::Msg::PasteLast),
        "polish" => toggle_polish(app, "enabled", !state.settings().polish.enabled),
        "translate" => toggle_polish(app, "translate", !state.settings().polish.translate),
        "pause:15" => state.pause.start(app, Some(Duration::from_secs(15 * 60))),
        "pause:60" => state.pause.start(app, Some(Duration::from_secs(60 * 60))),
        "pause:until" => state.pause.start(app, None),
        "resume" => state.pause.resume(app),
        "quit" => app.exit(0),
        _ => {
            if let Some(page) = id.strip_prefix("open:") {
                ui::open_main(app, page);
            } else if let Some(engine) = id.strip_prefix("engine:") {
                state.activate_engine(app, engine.to_owned(), |_| {});
            } else if let Some(language) = LANGUAGES.iter().find(|(_, item, _)| *item == id) {
                state.change(|s| s.language = language.0);
                ui::refresh(app);
            } else if let Some(microphone) = id.strip_prefix("mic:") {
                let name = microphone.parse::<usize>().ok().and_then(|i| lock(&MICROPHONES).0.get(i).cloned());
                state.change(|s| s.microphone = name);
                ui::refresh(app);
            }
        }
    }
    refresh(app);
}

fn toggle_polish(app: &AppHandle, field: &str, on: bool) {
    let state = app.state::<App>();
    let mut patch = serde_json::Map::new();
    patch.insert(field.to_owned(), on.into());
    state.change(|s| {
        if let Ok(polish) = s.polish.patched(patch) {
            s.polish = polish;
        }
    });
    ui::refresh(app);
}
