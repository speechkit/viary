//! Viary: hold a key, speak, and the words are typed where the cursor is.

mod dictation;
mod dictionary;
mod engines;
mod history;
mod icons;
mod keychain;
#[cfg(target_os = "macos")]
mod macos;
mod polish;
mod reload;
mod settings;
mod tap;
mod ui;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, mpsc::Sender},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use speechkit::{audio::read_wav_pcm16, sherpa::ExecutionProvider};
use tauri::{AppHandle, Manager, State, tray::TrayIconBuilder};

use crate::{
    dictation::{Msg, PillAction, PillView},
    engines::{EngineInfo, EngineStatus, Engines, ModelInspection},
    history::{Entry, History, Status},
    keychain::Provider,
    macos::{hotkey::HotkeyListener, permissions},
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
    pill: Mutex<PillView>,
    dictation: OnceLock<Sender<Msg>>,
    hotkey: OnceLock<HotkeyListener>,
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    settings: Settings,
    engine: EngineStatus,
    keys: Keys,
    permissions: permissions::Permissions,
    hotkey_active: bool,
    hotkey_name: &'static str,
    pill: PillView,
    /// Each local model's family, as the Voice engine screen describes it.
    families: Vec<(String, engines::FamilyInfo)>,
    punct_layout: Option<String>,
}

#[tauri::command]
fn get_state(state: State<'_, App>) -> Snapshot {
    let settings = state.settings();
    let families = settings
        .local_models
        .iter()
        .filter_map(|m| Some((m.id.clone(), engines::family_info(m.family.parse().ok()?))))
        .collect();
    let punct_layout = settings.punct_model.as_deref().and_then(engines::punct_kind);
    Snapshot {
        engine: state.engines.status(),
        keys: Keys {
            open_ai: keychain::has(Provider::OpenAi),
            dash_scope: keychain::has(Provider::DashScope),
        },
        permissions: permissions::check(),
        hotkey_active: state.hotkey.get().is_some_and(HotkeyListener::is_active),
        hotkey_name: dictation::key_name(settings.hotkey),
        pill: lock(&state.pill).clone(),
        families,
        punct_layout,
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
    Ok(speechkit::io::input_devices()
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
fn inspect_model(state: State<'_, App>, path: PathBuf) -> CmdResult<ModelInspection> {
    let vad = state.settings().vad_model;
    engines::inspect(&path, vad.as_deref()).map_err(|e| engines::describe(&e))
}

#[tauri::command]
fn add_local_model(
    app: AppHandle,
    state: State<'_, App>,
    path: PathBuf,
    family: String,
) -> CmdResult<String> {
    let vad = state.settings().vad_model;
    let inspection = engines::inspect(&path, vad.as_deref()).map_err(|e| engines::describe(&e))?;
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

/// Checking a punctuation model means loading it, so this runs off the
/// main thread.
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

fn provider_engine(provider: Provider) -> &'static str {
    match provider {
        Provider::OpenAi => engines::OPENAI,
        Provider::DashScope => engines::DASHSCOPE,
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
    state.reload_if_active(&app, |id| id == provider_engine(provider));
    ui::refresh(&app);
    Ok(())
}

#[tauri::command]
fn delete_api_key(app: AppHandle, state: State<'_, App>, provider: Provider) -> CmdResult<()> {
    keychain::delete(provider)?;
    // The loaded engine holds its own copy of the key: stop using it.
    state.forget_engine(provider_engine(provider));
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
        // CUDA needs an NVIDIA GPU; on a Mac only the CPU and CoreML run.
        if cfg!(target_os = "macos") && parsed == ExecutionProvider::Cuda {
            return Err(format!("{provider} is not available on this Mac"));
        }
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
        if dictionary::load_inputs(before) == dictionary::load_inputs(&after) {
            return;
        }
        let status = self.engines.status();
        let settled = status.loading.is_none() && status.failed.is_none();
        if settled && status.active.is_some_and(|info| info.dictionary == "replacements") {
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
        let audio = read_wav_pcm16(&path).map_err(|e| engines::describe(&e))?;
        let settings = state.settings();
        let options = loaded.session_options(&settings, &entry.app);
        let outcome = loaded
            .engine
            .transcribe(&audio, options, Instant::now() + Duration::from_secs(300))
            .map_err(|failure| engines::describe(&failure.error))?;
        let segments = dictation::texts(&outcome.transcript);
        let raw = engines::join(&segments);
        let mut text = dictation::finish(&settings, &entry.app, Some(&loaded), &segments);
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
    macos::pasteboard::set_text(&text);
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

#[tauri::command]
fn open_main(app: AppHandle, page: String) {
    ui::open_main(&app, &page);
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
        pill: Mutex::new(PillView::Idle),
        dictation: OnceLock::new(),
        hotkey: OnceLock::new(),
    });
    let state = app.state::<App>();

    // Windows: the pill is always there; the main window opens on first run.
    ui::build_pill(&handle)?;
    ui::build_popover(&handle)?;
    ui::build_main(&handle, first_run)?;

    TrayIconBuilder::with_id(ui::TRAY_ID)
        .icon(ui::tray_icon())
        .icon_as_template(true)
        .tooltip("Viary")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button_state: tauri::tray::MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                ui::toggle_popover(tray.app_handle(), rect);
            }
        })
        .build(app)?;

    let mailbox = dictation::spawn(handle.clone());
    let _ = state.dictation.set(mailbox.clone());
    let listener = HotkeyListener::spawn(settings.hotkey, move |event| {
        let _ = mailbox.send(Msg::Hotkey(event));
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

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,viary_lib=debug".into()),
        )
        .init();
    let built = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
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
            history_list,
            history_delete,
            history_retranscribe,
            copy_text,
            pill_action,
            fit_pill,
            request_permission,
            open_main,
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
