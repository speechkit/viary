//! Viary: hold a key, speak, and the words are typed where the cursor is.

mod caps;
mod dictation;
mod dictionary;
mod engines;
mod history;
mod icons;
mod json_store;
mod mic_test;
mod keychain;
mod note_recorder;
mod notes;
mod pause;
mod platform;
mod polish;
mod reload;
mod settings;
mod subtitles;
mod transcriber;
mod transcripts;
#[cfg(not(target_os = "macos"))]
mod tray_menu;
mod recording;
mod ui;

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use speechkit::{
    audio::{DecodeLimits, read},
    sherpa::Provider as ExecutionProvider,
};
use tauri::{AppHandle, Emitter, Manager, State, tray::TrayIconBuilder};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::{
    dictation::{Msg, PillAction, PillView},
    engines::{EngineInfo, EngineStatus, Engines, ModelInspection},
    history::{Entry, History, Status},
    keychain::Provider,
    platform::{
        hotkey::{HotkeyEvent, HotkeyListener},
        permissions,
    },
    settings::{
        DashScopeSettings, DictionaryEntry, Hotkey, Language, LocalModel, OpenAiSettings,
        Settings, SettingsStore, Tone,
    },
};

/// Everything the commands and the dictation controller share.
pub struct App {
    settings: Mutex<Settings>,
    store: SettingsStore,
    engines: Arc<Engines>,
    history: History,
    notes: notes::Notes,
    recorder: note_recorder::Recorder,
    transcripts: transcripts::Transcripts,
    transcriber: transcriber::Transcriber,
    pill: Mutex<PillView>,
    dictation: OnceLock<Sender<Msg>>,
    hotkey: OnceLock<HotkeyListener>,
    pause: pause::Pause,
    /// The setup window is testing the talk key: report it, start nothing.
    key_test: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl App {
    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    fn change(&self, edit: impl FnOnce(&mut Settings)) -> Settings {
        let mut settings = lock(&self.settings);
        edit(&mut settings);
        self.store.save(&settings);
        settings.clone()
    }

    pub fn set_pill(&self, view: PillView) {
        *lock(&self.pill) = view;
    }

    /// Loads engine `id`; once it is ready it becomes the default for new
    /// dictations. A failed load keeps the previous engine.
    pub fn activate_engine(
        &self,
        app: &AppHandle,
        id: String,
        done: impl FnOnce(Result<EngineInfo, String>) + Send + 'static,
    ) {
        let handle = app.clone();
        self.engines.switch(id, self.settings(), move |outcome| {
            // The engine that loaded: a newer request may have replaced ours.
            if let Ok(info) = &outcome {
                handle
                    .state::<App>()
                    .change(|s| s.active_engine = Some(info.id.clone()));
            }
            ui::refresh(&handle);
            done(outcome);
        });
        ui::refresh(app);
    }

    /// Reloads the engine the user chose after a setting it depends on
    /// changed: the one loading, else the one whose last load failed (the
    /// change may be the fix), else the one in use.
    fn reload_if_active(&self, app: &AppHandle, matches: impl Fn(&str) -> bool) {
        let status = self.engines.status();
        let chosen = match status.loading {
            // Reloading the old engine now would supersede the new one.
            Some(loading) => Some(loading),
            None => status.failed.or(status.active.map(|info| info.id)),
        };
        if let Some(id) = chosen.filter(|id| matches(id)) {
            self.activate_engine(app, id, |_| {});
        }
    }

    /// Stops using engine `id`, which can no longer be built: it is no
    /// longer the default, and new dictations stop using it, unless another
    /// engine is already loading to replace it.
    fn forget_engine(&self, id: &str) {
        self.change(|s| {
            if s.active_engine.as_deref() == Some(id) {
                s.active_engine = None;
            }
        });
        let status = self.engines.status();
        let in_use = status.active.is_some_and(|info| info.id == id);
        match status.loading {
            Some(loading) if loading == id => self.engines.unload(),
            None if in_use => self.engines.unload(),
            _ => {}
        }
    }

    fn send(&self, msg: Msg) {
        if let Some(mailbox) = self.dictation.get() {
            let _ = mailbox.send(msg);
        }
    }
}

type CmdResult<T> = std::result::Result<T, String>;

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

// ---------------------------------------------------------------------------
// State for the web views

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Keys {
    open_ai: bool,
    dash_scope: bool,
    custom_polish: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    /// `macos`, `windows`, or `linux`.
    platform: &'static str,
    settings: Settings,
    speech_caps: caps::SpeechCaps,
    engine: EngineStatus,
    keys: Keys,
    permissions: permissions::Permissions,
    hotkey_active: bool,
    hotkey_name: String,
    pill: PillView,
    /// Dictation paused from the tray.
    paused: Option<pause::Paused>,
    /// The session, shortcut, and typing method on Linux; null elsewhere.
    desktop: serde_json::Value,
    /// The keyboard layout on Windows; null elsewhere.
    keyboard: Option<platform::Keyboard>,
    autostart: bool,
    /// Each local model's family, as the Voice engine screen describes it.
    families: Vec<(String, engines::FamilyInfo)>,
    punct_layout: Option<String>,
}

#[tauri::command]
fn get_state(app: AppHandle, state: State<'_, App>) -> Snapshot {
    let settings = state.settings();
    let families = settings
        .local_models
        .iter()
        .filter_map(|m| Some((m.id.clone(), engines::family_info(m.family.parse().ok()?))))
        .collect();
    let punct_layout = settings.punct_model.as_deref().and_then(engines::punct_kind);
    Snapshot {
        platform: platform::NAME,
        engine: state.engines.status(),
        keys: Keys {
            open_ai: keychain::has(Provider::OpenAi),
            dash_scope: keychain::has(Provider::DashScope),
            custom_polish: keychain::has(Provider::CustomPolish),
        },
        permissions: permissions::check(),
        hotkey_active: state.hotkey.get().is_some_and(HotkeyListener::is_active),
        hotkey_name: dictation::hotkey_name(settings.hotkey),
        pill: lock(&state.pill).clone(),
        paused: state.pause.get(),
        desktop: platform::desktop(),
        keyboard: platform::keyboard(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        families,
        punct_layout,
        speech_caps: caps::current(),
        settings,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Microphone {
    name: String,
    is_default: bool,
}

#[tauri::command]
fn list_microphones() -> CmdResult<Vec<Microphone>> {
    Ok(speechkit::io::Microphone::list()
        .map_err(|e| engines::describe(&e))?
        .into_iter()
        .map(|d| Microphone {
            name: d.name,
            is_default: d.is_default,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Engines

#[tauri::command]
fn inspect_model(path: PathBuf) -> CmdResult<ModelInspection> {
    engines::inspect(&path).map_err(|e| engines::describe(&e))
}

#[tauri::command]
fn add_local_model(
    app: AppHandle,
    state: State<'_, App>,
    path: PathBuf,
    family: String,
) -> CmdResult<String> {
    let inspection = engines::inspect(&path).map_err(|e| engines::describe(&e))?;
    if !inspection.families.iter().any(|f| f.id == family) {
        return Err(format!("this folder is not a {family} model"));
    }
    let mut id = history::now_ms().to_string();
    state.change(|s| {
        // Adding a folder again replaces its entry but keeps its id, so the
        // active engine still names it.
        if let Some(existing) = s.local_models.iter().position(|m| m.path == path) {
            id = s.local_models.remove(existing).id;
        }
        s.local_models.push(LocalModel {
            id: id.clone(),
            name: inspection.name,
            path,
            family,
            size_bytes: inspection.size_bytes,
        });
        if s.vad_model.is_none() {
            s.vad_model = inspection.vad_nearby;
        }
    });
    let engine_id = engines::local_id_of(&id);
    // The family may have changed: reload if this model is in use.
    state.reload_if_active(&app, |active| active == engine_id);
    ui::refresh(&app);
    Ok(engine_id)
}

#[tauri::command]
fn remove_local_model(app: AppHandle, state: State<'_, App>, id: String) {
    state.change(|s| s.local_models.retain(|m| m.id != id));
    state.forget_engine(&engines::local_id_of(&id));
    ui::refresh(&app);
}

#[tauri::command]
fn set_active_engine(app: AppHandle, state: State<'_, App>, id: String) {
    state.activate_engine(&app, id, |_| {});
}

#[tauri::command]
fn set_vad_model(app: AppHandle, state: State<'_, App>, path: Option<PathBuf>) -> CmdResult<()> {
    if let Some(path) = &path
        && !path.is_file()
    {
        return Err("choose the silero_vad.onnx file".into());
    }
    state.change(|s| s.vad_model = path);
    state.reload_if_active(&app, |id| id.starts_with("local:"));
    ui::refresh(&app);
    Ok(())
}

/// Checking a punctuation model reads through its folder, which can be
/// large, so this runs off the main thread. The check does not load the
/// model: see `engines::check_punct`.
#[tauri::command]
async fn set_punct_model(app: AppHandle, path: Option<PathBuf>) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(path) = &path {
            engines::check_punct(path).map_err(|e| engines::describe(&e))?;
        }
        let state = app.state::<App>();
        state.change(|s| s.punct_model = path);
        state.reload_if_active(&app, |id| id.starts_with("local:"));
        ui::refresh(&app);
        Ok(())
    })
    .await
    .map_err(err)?
}

#[tauri::command]
fn set_openai(app: AppHandle, state: State<'_, App>, openai: OpenAiSettings) {
    state.change(|s| s.openai = openai);
    state.reload_if_active(&app, |id| id == engines::OPENAI);
    ui::refresh(&app);
}

#[tauri::command]
fn set_dashscope(app: AppHandle, state: State<'_, App>, dashscope: DashScopeSettings) {
    state.change(|s| s.dashscope = dashscope);
    state.reload_if_active(&app, |id| id == engines::DASHSCOPE);
    ui::refresh(&app);
}

fn provider_engine(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::OpenAi => Some(engines::OPENAI),
        Provider::DashScope => Some(engines::DASHSCOPE),
        Provider::CustomPolish => None,
    }
}

#[tauri::command]
fn save_api_key(
    app: AppHandle,
    state: State<'_, App>,
    provider: Provider,
    key: String,
) -> CmdResult<()> {
    keychain::save(provider, &key)?;
    if let Some(engine) = provider_engine(provider) {
        state.reload_if_active(&app, |id| id == engine);
    }
    ui::refresh(&app);
    Ok(())
}

#[tauri::command]
fn delete_api_key(app: AppHandle, state: State<'_, App>, provider: Provider) -> CmdResult<()> {
    keychain::delete(provider)?;
    // The loaded engine holds its own copy of the key: stop using it.
    if let Some(engine) = provider_engine(provider) {
        state.forget_engine(engine);
    }
    ui::refresh(&app);
    Ok(())
}

// ---------------------------------------------------------------------------
// Preferences

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    /// `Some(None)` chooses the default device.
    #[serde(default, with = "serde_with_option")]
    microphone: Option<Option<String>>,
    language: Option<Language>,
    hotkey: Option<Hotkey>,
    keep_recordings_days: Option<u32>,
    provider: Option<String>,
    threads: Option<usize>,
}

/// Distinguishes a missing field from an explicit `null`.
mod serde_with_option {
    use serde::{Deserialize, Deserializer};

    pub(crate) fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

#[tauri::command]
fn set_preferences(app: AppHandle, state: State<'_, App>, prefs: Preferences) -> CmdResult<()> {
    if let Some(provider) = &prefs.provider {
        let parsed = provider
            .parse::<ExecutionProvider>()
            .map_err(|e| engines::describe(&e))?;
        if !engines::PROVIDERS.contains(&parsed) {
            return Err(format!("{provider} is not available on this computer"));
        }
    }
    if let Some(hotkey) = prefs.hotkey
        && !Hotkey::AVAILABLE.contains(&hotkey)
    {
        return Err(format!("{} is not a talk key on this computer", dictation::key_name(hotkey)));
    }
    let inference_changed = prefs.provider.is_some() || prefs.threads.is_some();
    let settings = state.change(|s| {
        if let Some(microphone) = prefs.microphone {
            s.microphone = microphone;
        }
        if let Some(language) = prefs.language {
            s.language = language;
        }
        if let Some(hotkey) = prefs.hotkey {
            s.hotkey = hotkey;
        }
        if let Some(days) = prefs.keep_recordings_days {
            s.keep_recordings_days = days;
        }
        if let Some(provider) = prefs.provider {
            s.provider = provider;
        }
        if let Some(threads) = prefs.threads {
            s.threads = threads.clamp(1, 16);
        }
    });
    if let Some(listener) = state.hotkey.get() {
        listener.set_hotkey(settings.hotkey);
    }
    if prefs.keep_recordings_days.is_some() {
        state
            .history
            .purge_recordings(settings.keep_recordings_days);
    }
    if inference_changed {
        state.reload_if_active(&app, |id| id.starts_with("local:"));
    }
    ui::refresh(&app);
    Ok(())
}

// ---------------------------------------------------------------------------
// Dictionary and polish

impl App {
    /// Reloads the engine after the dictionary changed, if it loads the
    /// words (as hotwords or a prompt) and they changed.
    fn dictionary_changed(&self, app: &AppHandle, before: &[DictionaryEntry]) {
        let after = self.settings().dictionary;
        let status = self.engines.status();
        let settled = status.loading.is_none() && status.failed.is_none();
        let uses = status.active.as_ref().map(|info| info.dictionary);
        // Hotwords take no priority; an engine still loading may be any kind.
        let priority = !(settled && uses == Some("hotwords"));
        if dictionary::load_inputs(before, priority) == dictionary::load_inputs(&after, priority) {
            return;
        }
        if settled && uses == Some("replacements") {
            return;
        }
        self.reload_if_active(app, |id| id.starts_with("local:") || id == engines::OPENAI);
    }
}

/// Adds a word, or replaces the one with its id, and returns its id.
#[tauri::command]
fn dictionary_save(app: AppHandle, state: State<'_, App>, entry: DictionaryEntry) -> CmdResult<String> {
    let mut entry = dictionary::tidy(entry)?;
    let before = state.settings().dictionary;
    let is_new = !before.iter().any(|e| e.id == entry.id);
    if is_new && before.len() >= dictionary::MAX_WORDS {
        return Err(format!("the dictionary holds up to {} words", dictionary::MAX_WORDS));
    }
    if let Some(same) = before
        .iter()
        .find(|e| e.id != entry.id && e.word.eq_ignore_ascii_case(&entry.word))
    {
        return Err(format!("“{}” is already in the dictionary", same.word));
    }
    if is_new {
        entry.id = history::now_ms().to_string();
    }
    let id = entry.id.clone();
    state.change(|s| match s.dictionary.iter_mut().find(|e| e.id == entry.id) {
        Some(existing) => *existing = entry,
        None => s.dictionary.push(entry),
    });
    state.dictionary_changed(&app, &before);
    ui::refresh(&app);
    Ok(id)
}

#[tauri::command]
fn dictionary_remove(app: AppHandle, state: State<'_, App>, id: String) {
    let before = state.settings().dictionary;
    state.change(|s| s.dictionary.retain(|e| e.id != id));
    state.dictionary_changed(&app, &before);
    ui::refresh(&app);
}

/// Changes the polish settings named in `patch`, keeping the rest as saved,
/// so quick changes, from this window or the popover, never undo each other.
#[tauri::command]
fn update_polish(
    app: AppHandle,
    state: State<'_, App>,
    patch: serde_json::Map<String, serde_json::Value>,
) -> CmdResult<()> {
    let mut outcome = Ok(());
    state.change(|s| match s.polish.patched(patch) {
        Ok(polish) => s.polish = polish,
        Err(error) => outcome = Err(error),
    });
    ui::refresh(&app);
    outcome
}

/// Sets the polish tone of `target`, or takes it off the list with `None`.
#[tauri::command]
fn set_app_tone(app: AppHandle, state: State<'_, App>, target: String, tone: Option<Tone>) {
    state.change(|s| s.polish.set_tone(&target, tone));
    ui::refresh(&app);
}

/// Tests a model draft without saving its settings or exposing a stored key.
#[tauri::command]
async fn test_polish_connection(
    app: AppHandle,
    patch: serde_json::Map<String, serde_json::Value>,
    key: Option<String>,
) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut settings = app.state::<App>().settings();
        settings.polish = settings.polish.patched(patch)?;
        polish::test_connection(&settings, key)
    })
    .await
    .map_err(err)?
}

/// Polishes `text` as if dictated into `app`, for the Polish page.
#[tauri::command]
async fn polish_preview(app: AppHandle, target: String, text: String) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = app.state::<App>().settings();
        let text = dictionary::apply(&settings.dictionary, &target, &text);
        polish::request(&settings, &target)?.run(&text, polish::TIMEOUT)
    })
    .await
    .map_err(err)?
}

// ---------------------------------------------------------------------------
// History

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryItem {
    #[serde(flatten)]
    entry: Entry,
    words: usize,
    /// The recording's absolute path, while it is kept.
    recording_path: Option<PathBuf>,
}

#[tauri::command]
fn history_list(state: State<'_, App>) -> Vec<HistoryItem> {
    state
        .history
        .list()
        .into_iter()
        .map(|entry| HistoryItem {
            words: entry.words(),
            recording_path: state.history.recording_path(&entry),
            entry,
        })
        .collect()
}

#[tauri::command]
fn history_delete(app: AppHandle, state: State<'_, App>, id: String) {
    state.history.delete(&id);
    ui::refresh(&app);
}

/// Transcribes a kept recording again with the current engine.
#[tauri::command]
async fn history_retranscribe(app: AppHandle, id: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<App>();
        let entry = state.history.get(&id).ok_or("this dictation was deleted")?;
        let path = state
            .history
            .recording_path(&entry)
            .ok_or("the recording is no longer kept")?;
        let loaded = state
            .engines
            .current()
            .ok_or("choose a voice engine first")?;
        let audio = read(&path, DecodeLimits::new(recording::MAX_RECORDING))
            .map_err(|e| engines::describe(&e))?;
        let settings = state.settings();
        let options = loaded.session_options(&settings, &entry.app);
        let outcome = loaded
            .engine
            .transcribe(&audio, options, Instant::now() + Duration::from_secs(300))
            .map_err(|failure| engines::describe(&failure.error))?;
        let raw = outcome.text();
        let mut text = dictation::finish(&settings, &entry.app, Some(&loaded), outcome);
        if polish::applies(&settings, &entry.app) {
            match polish::request(&settings, &entry.app).and_then(|r| r.run(&text, polish::TIMEOUT)) {
                Ok(polished) => text = polished,
                Err(error) => tracing::warn!(%error, "polish failed; keeping the text as recognized"),
            }
        }
        state.history.update(&id, |entry| {
            entry.punctuated = (text != raw).then(|| text.clone());
            entry.text = text;
            entry.raw = raw;
            entry.error = None;
            if entry.status == Status::Failed {
                entry.status = Status::Transcribed;
            }
            entry.engine = history::EngineLabel {
                name: loaded.info.name.clone(),
                kind: loaded.info.kind.clone(),
                on_device: loaded.info.on_device,
            };
        });
        ui::refresh(&app);
        Ok(())
    })
    .await
    .map_err(err)?
}

#[tauri::command]
fn copy_text(text: String) {
    platform::pasteboard::set_text(&text);
}

// ---------------------------------------------------------------------------
// Pill, windows, and permissions

#[tauri::command]
fn pill_action(state: State<'_, App>, action: PillAction) {
    state.send(Msg::Pill(action));
}

#[tauri::command]
fn fit_pill(app: AppHandle, width: f64, height: f64) {
    ui::fit_pill(&app, width, height);
}

#[tauri::command]
fn request_permission(app: AppHandle, kind: String) {
    permissions::request(&kind);
    permissions::open_settings(&kind);
    ui::refresh(&app);
}

/// Pauses dictation for `minutes`, or until resumed when `None`.
#[tauri::command]
fn pause_dictation(app: AppHandle, state: State<'_, App>, minutes: Option<u64>) {
    state
        .pause
        .start(&app, minutes.map(|m| Duration::from_secs(m.saturating_mul(60))));
}

#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> CmdResult<()> {
    let autostart = app.autolaunch();
    if on { autostart.enable() } else { autostart.disable() }.map_err(err)?;
    ui::refresh(&app);
    Ok(())
}

/// Asks GNOME for the talk shortcut (Linux).
#[tauri::command]
async fn bind_shortcut(app: AppHandle) -> CmdResult<()> {
    let bound = tauri::async_runtime::spawn_blocking(platform::bind_shortcut)
        .await
        .map_err(err)?;
    ui::refresh(&app);
    bound
}

/// Installs Viary's GNOME Shell extension (Linux).
#[tauri::command]
async fn install_extension(app: AppHandle) -> CmdResult<()> {
    let installed = tauri::async_runtime::spawn_blocking(platform::install_extension)
        .await
        .map_err(err)?;
    ui::refresh(&app);
    installed
}

/// Chooses how Viary types on Wayland: `extension`, `portal`, or
/// `clipboard` (Linux).
#[tauri::command]
async fn set_typing(app: AppHandle, method: String) -> CmdResult<()> {
    let set = tauri::async_runtime::spawn_blocking(move || platform::set_typing(&method))
        .await
        .map_err(err)?;
    ui::refresh(&app);
    set
}

#[tauri::command]
fn set_key_test(state: State<'_, App>, on: bool) {
    state.key_test.store(on, Ordering::SeqCst);
}

/// Setup ran to its end: it does not open again. Async, so it runs off the
/// main thread: building the tip's web view from a synchronous command
/// deadlocks on Windows (WebView2).
#[tauri::command]
async fn finish_setup(app: AppHandle) -> CmdResult<()> {
    let state = app.state::<App>();
    ui::end_setup_tests(&app);
    let before = state.settings();
    state.change(|s| {
        s.setup_done = true;
        s.tray_tip_shown = true;
    });
    if let Some(setup) = app.get_webview_window("setup") {
        let _ = setup.close();
    }
    // Windows hides a new tray icon under ^: say where Viary went, once.
    if cfg!(target_os = "windows")
        && !before.tray_tip_shown
        && let Err(error) = ui::show_tray_tip(&app)
    {
        tracing::warn!(%error, "cannot show the tray tip");
    }
    ui::refresh(&app);
    Ok(())
}

/// Starts or ends the setup window's microphone test (`mic-level` events).
#[tauri::command]
fn mic_test(app: AppHandle, on: bool) -> CmdResult<()> {
    if on { mic_test::start(&app) } else {
        mic_test::stop();
        Ok(())
    }
}

/// Closes the tray tip; `show_me` opens the taskbar settings first.
#[tauri::command]
fn close_tray_tip(app: AppHandle, show_me: bool) {
    if show_me {
        permissions::open_settings("taskbar");
    }
    if let Some(tip) = app.get_webview_window("tip") {
        let _ = tip.close();
    }
}

#[tauri::command]
fn resume_dictation(app: AppHandle, state: State<'_, App>) {
    state.pause.resume(&app);
}

#[tauri::command]
fn open_main(app: AppHandle, page: String) {
    ui::open_main(&app, &page);
}

/// From anywhere: Voice Notes, recording. ⌥⌘N on macOS (Win+Alt+N on
/// Windows, Ctrl+Alt+N on Linux).
#[cfg(target_os = "macos")]
const NEW_NOTE_SHORTCUT: (Modifiers, Code) = (Modifiers::ALT.union(Modifiers::SUPER), Code::KeyN);
/// Not Ctrl+Alt+N: Windows reports AltGr as Ctrl+Alt, so a global
/// Ctrl+Alt+N would take AltGr+N (ń on Polish) from every app.
#[cfg(target_os = "windows")]
const NEW_NOTE_SHORTCUT: (Modifiers, Code) = (Modifiers::SUPER.union(Modifiers::ALT), Code::KeyN);
/// X11 and GNOME keep AltGr apart from Ctrl+Alt.
#[cfg(target_os = "linux")]
const NEW_NOTE_SHORTCUT: (Modifiers, Code) = (Modifiers::CONTROL.union(Modifiers::ALT), Code::KeyN);

/// Opens Voice Notes and starts recording, unless a note already is.
fn new_voice_note_now(app: &AppHandle) -> Result<(), String> {
    ui::open_main(app, "notes:new");
    app.state::<App>().recorder.start(app)
}

#[tauri::command]
fn new_voice_note(app: AppHandle) -> CmdResult<()> {
    new_voice_note_now(&app)
}

// ---------------------------------------------------------------------------
// Voice notes

#[tauri::command]
fn notes_list(state: State<'_, App>) -> Vec<notes::Listed> {
    state.notes.list()
}

#[tauri::command]
fn note_recorder_state(state: State<'_, App>) -> note_recorder::RecorderState {
    state.recorder.state()
}

#[tauri::command]
fn note_live(state: State<'_, App>) -> Option<note_recorder::LivePayload> {
    state.recorder.live()
}

#[tauri::command]
fn note_pause(app: AppHandle, state: State<'_, App>) {
    state.recorder.pause(&app);
}

#[tauri::command]
fn note_resume(app: AppHandle, state: State<'_, App>) {
    state.recorder.resume(&app);
}

#[tauri::command]
fn note_mark(app: AppHandle, state: State<'_, App>) {
    state.recorder.mark(&app);
}

#[tauri::command]
fn note_discard(app: AppHandle, state: State<'_, App>) {
    state.recorder.discard(&app);
}

#[tauri::command]
fn note_stop(app: AppHandle, state: State<'_, App>) {
    state.recorder.stop(&app);
}

fn notes_changed(app: &AppHandle) {
    let _ = app.emit("notes-changed", ());
}

#[tauri::command]
fn note_rename(app: AppHandle, state: State<'_, App>, id: String, title: String) {
    let title = title.trim();
    if title.is_empty() || state.recorder.rename(&app, &id, title) {
        return;
    }
    state.notes.update(&id, |note| note.title = title.into());
    notes_changed(&app);
}

#[tauri::command]
fn note_set_action(app: AppHandle, state: State<'_, App>, id: String, index: usize, done: bool) {
    state.notes.update(&id, |note| {
        if let Some(action) = note.actions.get_mut(index) {
            action.done = done;
        }
    });
    notes_changed(&app);
}

/// Sets a note's marks, as the note view edits them.
#[tauri::command]
fn note_set_marks(app: AppHandle, state: State<'_, App>, id: String, mut marks: Vec<u64>) {
    marks.sort_unstable();
    marks.dedup();
    state.notes.update(&id, |note| {
        marks.retain(|&m| m <= note.duration_ms);
        note.marks = marks;
    });
    notes_changed(&app);
}

#[tauri::command]
fn note_delete(app: AppHandle, state: State<'_, App>, id: String) {
    state.notes.delete(&id);
    notes_changed(&app);
}

#[tauri::command]
async fn note_summarize(app: AppHandle, id: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<App>();
        let note = state.notes.get(&id).ok_or("the note is gone")?;
        let result = notes::summarize(&state.settings(), &note);
        state.notes.update(&id, |note| match result {
            Ok(summary) => {
                note.summary = Some(summary.summary);
                note.actions = summary.actions;
                note.summary_error = None;
            }
            Err(error) => note.summary_error = Some(error),
        });
        notes_changed(&app);
        Ok(())
    })
    .await
    .map_err(err)?
}

/// The note as Markdown or text; the summary only while polish is on.
fn note_document(state: &App, id: &str, format: notes::Format) -> CmdResult<String> {
    let note = state.notes.get(id).ok_or("the note is gone")?;
    Ok(notes::render(
        &note,
        format,
        state.settings().polish.enabled,
    ))
}

#[tauri::command]
fn note_text(state: State<'_, App>, id: String, format: notes::Format) -> CmdResult<String> {
    note_document(&state, &id, format)
}

#[tauri::command]
fn note_export(
    state: State<'_, App>,
    id: String,
    format: notes::Format,
    path: PathBuf,
) -> CmdResult<()> {
    let text = note_document(&state, &id, format)?;
    std::fs::write(&path, text).map_err(err)
}

const NOTHING_TO_TRANSCRIBE: &str = "None of these files can be transcribed";

/// Files and folders to transcribe, from the file dialog, the window, or
/// the menu bar icon: queues them and opens Transcripts.
fn transcribe(app: &AppHandle, paths: Vec<PathBuf>) -> Result<(), String> {
    let files = transcriber::expand(paths);
    if files.is_empty() {
        // Transcripts says so, however the files came (the menu bar icon
        // has no other way to answer).
        ui::open_main(app, "transcripts:nothing");
        return Err(NOTHING_TO_TRANSCRIBE.into());
    }
    app.state::<App>().transcriber.add(app, files);
    ui::open_main(app, "transcripts:added");
    Ok(())
}

/// Looking through folders can take a while, so it is off the main thread.
#[tauri::command]
async fn transcribe_files(app: AppHandle, paths: Vec<PathBuf>) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || transcribe(&app, paths))
        .await
        .map_err(err)?
}

// ---------------------------------------------------------------------------
// Transcripts

#[tauri::command]
fn transcript_jobs(state: State<'_, App>) -> Vec<transcriber::Job> {
    state.transcriber.jobs()
}

#[tauri::command]
fn transcript_job_retry(app: AppHandle, state: State<'_, App>, id: String) {
    state.transcriber.retry(&app, &id);
}

#[tauri::command]
fn transcript_job_cancel(app: AppHandle, state: State<'_, App>, id: String) {
    state.transcriber.cancel(&app, &id);
}

#[tauri::command]
fn transcripts_list(state: State<'_, App>) -> Vec<transcripts::Summary> {
    state.transcripts.list()
}

#[tauri::command]
fn transcript_get(
    app: AppHandle,
    state: State<'_, App>,
    id: String,
) -> CmdResult<transcripts::Doc> {
    let transcript = state.transcripts.get(&id).ok_or("the transcript is gone")?;
    // The original stays where the user keeps it; let the player read it.
    if let Err(error) = app.asset_protocol_scope().allow_file(&transcript.source) {
        tracing::warn!(%error, "cannot play the original file");
    }
    Ok(transcript.doc())
}

#[tauri::command]
fn transcripts_search(state: State<'_, App>, query: String) -> Vec<transcripts::Hit> {
    state.transcripts.search(&query)
}

/// Changes a transcript, then the files saved next to its original.
fn change_transcript<T>(
    app: &AppHandle,
    state: &App,
    id: &str,
    change: impl FnOnce(&mut transcripts::Transcript) -> T,
) -> CmdResult<T> {
    let (out, changed) = state
        .transcripts
        .update(id, change)
        .ok_or("the transcript is gone")?;
    state
        .transcripts
        .rewrite_saved(&changed.id, || state.settings().transcripts.speaker_names, false);
    let _ = app.emit("transcripts-changed", ());
    Ok(out)
}

#[tauri::command]
fn transcript_edit_passage(
    app: AppHandle,
    state: State<'_, App>,
    id: String,
    index: usize,
    text: String,
) -> CmdResult<()> {
    change_transcript(&app, &state, &id, |t| t.edit(index, text.trim()))
}

#[tauri::command]
fn transcript_rename_speaker(
    app: AppHandle,
    state: State<'_, App>,
    id: String,
    index: usize,
    name: String,
) -> CmdResult<()> {
    change_transcript(&app, &state, &id, |t| {
        if let Some(speaker) = t.speakers.get_mut(index) {
            speaker.name = name.trim().to_owned();
        }
    })
}

#[tauri::command]
fn transcript_replace_all(
    app: AppHandle,
    state: State<'_, App>,
    id: String,
    find: String,
    replace: String,
) -> CmdResult<usize> {
    change_transcript(&app, &state, &id, |t| t.replace_all(&find, &replace))
}

#[tauri::command]
fn transcript_delete(app: AppHandle, state: State<'_, App>, id: String) {
    state.transcripts.delete(&id);
    let _ = app.emit("transcripts-changed", ());
}

#[tauri::command]
fn transcript_cues(
    state: State<'_, App>,
    id: String,
    speaker_names: bool,
) -> CmdResult<Vec<subtitles::Cue>> {
    let t = state.transcripts.get(&id).ok_or("the transcript is gone")?;
    Ok(subtitles::cues(&t.passages, &t.speakers, speaker_names))
}

#[tauri::command]
fn transcript_export(
    state: State<'_, App>,
    id: String,
    format: settings::OutputFormat,
    path: PathBuf,
) -> CmdResult<()> {
    let t = state.transcripts.get(&id).ok_or("the transcript is gone")?;
    transcripts::export(
        &t,
        format,
        &path,
        state.settings().transcripts.speaker_names,
    )
    .map_err(err)
}

/// Changes the settings for new files named in `patch`. Turning speaker
/// names on or off updates the subtitles already saved.
#[tauri::command]
fn update_transcript_settings(
    app: AppHandle,
    state: State<'_, App>,
    patch: serde_json::Map<String, serde_json::Value>,
) -> CmdResult<()> {
    let before = state.settings().transcripts.speaker_names;
    let mut outcome = Ok(());
    let settings = state.change(|s| match s.transcripts.patched(patch) {
        Ok(transcripts) => s.transcripts = transcripts,
        Err(error) => outcome = Err(error),
    });
    let names = settings.transcripts.speaker_names;
    if names != before {
        // Only subtitles of transcripts with speakers change; rewrite them
        // off the main thread, each as it is then, with the names setting
        // as it is then.
        let with_speakers: Vec<String> = state
            .transcripts
            .all()
            .into_iter()
            .filter(|t| !t.speakers.is_empty())
            .map(|t| t.id)
            .collect();
        let app = app.clone();
        std::thread::spawn(move || {
            let state = app.state::<App>();
            for id in &with_speakers {
                state
                    .transcripts
                    .rewrite_saved(id, || state.settings().transcripts.speaker_names, true);
            }
        });
    }
    ui::refresh(&app);
    outcome
}

#[tauri::command]
fn audio_extensions() -> Vec<&'static str> {
    speechkit::audio::EXTENSIONS.to_vec()
}

#[tauri::command]
fn hide_popover(app: AppHandle) {
    if let Some(popover) = app.get_webview_window("popover") {
        let _ = popover.hide();
    }
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

// ---------------------------------------------------------------------------

fn setup(app: &mut tauri::App) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();

    let config_dir = app.path().app_config_dir()?;
    let data_dir = app.path().app_data_dir()?;
    let store = SettingsStore::new(&config_dir);
    let settings = store.load();
    let history = History::open(&data_dir);
    history.purge_recordings(settings.keep_recordings_days);
    let first_run = settings.active_engine.is_none();
    #[cfg(target_os = "macos")]
    ui::set_app_icon();
    // In ⌘Tab only while the main window is open; see `ui::in_app_switcher`.
    #[cfg(target_os = "macos")]
    app.set_activation_policy(if first_run {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    });
    app.manage(App {
        settings: Mutex::new(settings.clone()),
        store,
        engines: Arc::new(Engines::new()),
        history,
        notes: notes::Notes::open(&data_dir),
        recorder: note_recorder::Recorder::default(),
        transcripts: transcripts::Transcripts::open(&data_dir),
        transcriber: transcriber::Transcriber::default(),
        pill: Mutex::new(PillView::Idle),
        dictation: OnceLock::new(),
        hotkey: OnceLock::new(),
        pause: pause::Pause::default(),
        key_test: AtomicBool::new(false),
    });
    let state = app.state::<App>();

    // Windows: the pill is always there; the main window opens on first run.
    ui::build_pill(&handle)?;
    ui::build_popover(&handle)?;
    // Windows and Linux walk a first-time user through setup in a window
    // of its own.
    let setting_up = !cfg!(target_os = "macos") && !settings.setup_done;
    ui::build_main(&handle, first_run && !setting_up)?;
    if setting_up {
        ui::build_setup(&handle, None)?;
    }

    let tray = TrayIconBuilder::with_id(ui::TRAY_ID)
        .icon(ui::tray_icon())
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Viary")
        // Linux shows the menu on any click; Windows on a right click.
        .show_menu_on_left_click(cfg!(target_os = "linux"))
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
            match event {
                // macOS: any click opens the popover. Windows: a left click
                // opens it; the right click shows the menu.
                TrayIconEvent::Click {
                    button,
                    button_state: MouseButtonState::Up,
                    rect,
                    ..
                } if cfg!(target_os = "macos") || button == MouseButton::Left => {
                    ui::toggle_popover(tray.app_handle(), rect);
                }
                #[cfg(not(target_os = "macos"))]
                TrayIconEvent::Enter { .. } => tray_menu::refresh(tray.app_handle()),
                _ => {}
            }
        });
    #[cfg(not(target_os = "macos"))]
    let tray = tray
        .menu(&tray_menu::build(&handle)?)
        .on_menu_event(tray_menu::on_event);
    tray.build(app)?;
    #[cfg(target_os = "macos")]
    if let Some(tray) = app.tray_by_id(ui::TRAY_ID) {
        let dropped = handle.clone();
        let _ = tray.with_inner_tray_icon(move |inner| {
            if let Some(item) = inner.ns_status_item() {
                platform::tray_drop::install(&item, move |paths| {
                    // Called on the main thread; folders are looked through
                    // on another.
                    let dropped = dropped.clone();
                    std::thread::spawn(move || {
                        if let Err(error) = transcribe(&dropped, paths) {
                            tracing::warn!(%error, "nothing to transcribe in the dropped files");
                        }
                    });
                });
            }
        });
    }
    let (modifiers, key) = NEW_NOTE_SHORTCUT;
    if let Err(error) = app
        .global_shortcut()
        .register(Shortcut::new(Some(modifiers), key))
    {
        // Another app holds ⌥⌘N; the menu bar item still works.
        tracing::warn!(%error, "cannot register the new voice note shortcut");
    }

    transcriber::spawn(handle.clone());
    note_recorder::retry_pending(handle.clone());
    let mailbox = dictation::spawn(handle.clone());
    let _ = state.dictation.set(mailbox.clone());
    #[cfg(target_os = "linux")]
    {
        platform::init(&handle, &config_dir);
        platform::notify::listen(&handle);
        platform::extension::listen(&handle);
    }
    let keys = handle.clone();
    let listener = HotkeyListener::spawn(settings.hotkey, move |event| {
        let at = Instant::now();
        match event {
            HotkeyEvent::Down => keys.emit("hotkey", "down"),
            HotkeyEvent::Up => keys.emit("hotkey", "up"),
            HotkeyEvent::OtherKey => Ok(()),
            // A press with no release: the test sees both at once.
            #[cfg(target_os = "linux")]
            HotkeyEvent::Toggle => keys.emit("hotkey", "down").and_then(|()| keys.emit("hotkey", "up")),
        }
        .ok();
        if keys.state::<App>().key_test.load(Ordering::SeqCst) {
            return;
        }
        #[cfg(target_os = "linux")]
        if event == HotkeyEvent::Toggle {
            let _ = mailbox.send(Msg::Toggle);
            return;
        }
        let _ = mailbox.send(Msg::Hotkey(event, at));
    });
    let _ = state.hotkey.set(listener);

    // Viary runs for weeks in the menu bar: keep the retention period
    // while it does, not only at launch.
    let purger = handle.clone();
    let _ = std::thread::Builder::new()
        .name("viary-purge".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(60 * 60));
                let state = purger.state::<App>();
                if state
                    .history
                    .purge_recordings(state.settings().keep_recordings_days)
                {
                    ui::refresh(&purger);
                }
            }
        });

    if let Some(id) = settings.active_engine {
        state.activate_engine(&handle, id, |outcome| {
            if let Err(error) = outcome {
                tracing::warn!(%error, "the saved engine did not load");
            }
        });
    }
    Ok(())
}

/// Viary was started again while running: `viary --toggle` from GNOME's
/// custom shortcut acts as the talk shortcut; anything else shows Viary.
#[cfg(not(target_os = "macos"))]
fn another_launch(app: &AppHandle, args: &[String]) {
    #[cfg(target_os = "linux")]
    if args.iter().any(|a| a == "--toggle") {
        platform::hotkey::toggle();
        return;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = args;
    match app.get_webview_window("setup") {
        Some(setup) => {
            let _ = setup.show();
            let _ = setup.unminimize();
            let _ = setup.set_focus();
        }
        None => ui::open_main(app, "home"),
    }
}

/// `viary --toggle`: see `platform::hotkey`.
#[cfg(target_os = "linux")]
pub fn send_toggle() -> bool {
    platform::hotkey::send_toggle()
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,viary_lib=debug".into()),
        )
        .init();
    let builder = tauri::Builder::default();
    // Windows and Linux start a second Viary as readily as the first (at
    // sign-in, then from the Start menu): it would add a second keyboard
    // hook and tray icon. The second hands its arguments over and quits.
    // Registered first, as the plugin asks.
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        another_launch(app, &args);
    }));
    let built = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    let (modifiers, key) = NEW_NOTE_SHORTCUT;
                    if event.state() == ShortcutState::Pressed
                        && shortcut.matches(modifiers, key)
                        && let Err(error) = new_voice_note_now(app)
                    {
                        tracing::warn!(%error, "cannot start a voice note");
                    }
                })
                .build(),
        )
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            pause_dictation,
            resume_dictation,
            set_autostart,
            set_key_test,
            bind_shortcut,
            set_typing,
            install_extension,
            finish_setup,
            close_tray_tip,
            mic_test,
            get_state,
            list_microphones,
            inspect_model,
            add_local_model,
            remove_local_model,
            set_active_engine,
            set_vad_model,
            set_punct_model,
            set_openai,
            set_dashscope,
            save_api_key,
            delete_api_key,
            set_preferences,
            dictionary_save,
            dictionary_remove,
            update_polish,
            set_app_tone,
            polish_preview,
            test_polish_connection,
            history_list,
            history_delete,
            history_retranscribe,
            copy_text,
            pill_action,
            fit_pill,
            request_permission,
            open_main,
            new_voice_note,
            notes_list,
            note_recorder_state,
            note_live,
            note_pause,
            note_resume,
            note_mark,
            note_discard,
            note_stop,
            note_rename,
            note_set_action,
            note_set_marks,
            note_delete,
            note_summarize,
            note_text,
            note_export,
            transcribe_files,
            transcript_jobs,
            transcript_job_retry,
            transcript_job_cancel,
            transcripts_list,
            transcript_get,
            transcripts_search,
            transcript_edit_passage,
            transcript_rename_speaker,
            transcript_replace_all,
            transcript_delete,
            transcript_cues,
            transcript_export,
            update_transcript_settings,
            audio_extensions,
            hide_popover,
            quit,
        ])
        .build(tauri::generate_context!());
    match built {
        Ok(app) => app.run(|_, event| {
            // Keep running in the menu bar when every window is closed.
            if let tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } = event
            {
                api.prevent_exit();
            }
        }),
        Err(error) => tracing::error!(%error, "Viary could not start"),
    }
}
