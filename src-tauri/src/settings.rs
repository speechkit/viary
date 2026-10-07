//! Persistent preferences, stored as JSON in the app's config directory.
//!
//! API keys are never stored here; they live in the Keychain
//! (see [`crate::keychain`]).

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// The key that starts a dictation while held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Hotkey {
    #[default]
    Fn,
    RightOption,
    RightCommand,
}

/// The spoken language passed to engines that accept an override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    #[default]
    Auto,
    En,
    Zh,
}

impl Language {
    /// The code for `AsrOptions::language`, or `None` for automatic.
    pub fn code(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::En => Some("en"),
            Self::Zh => Some("zh"),
        }
    }
}

/// A sherpa-onnx model folder the user added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    pub id: String,
    /// The folder name, shown as the engine's name.
    pub name: String,
    pub path: PathBuf,
    /// A `speechkit::sherpa::AsrFamily` name, such as `sense-voice`.
    pub family: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum OpenAiMode {
    /// One upload when the key is released: `POST /audio/transcriptions`.
    #[default]
    File,
    /// Streaming over the Realtime API, with live words.
    Realtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OpenAiSettings {
    pub base_url: String,
    /// Empty until the user names one: Viary never picks a model.
    pub model: String,
    pub mode: OpenAiMode,
}

impl Default for OpenAiSettings {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".into(),
            model: String::new(),
            mode: OpenAiMode::File,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DashScopeRegion {
    #[default]
    China,
    International,
}

impl DashScopeRegion {
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::China => "wss://dashscope.aliyuncs.com/api-ws/v1/inference/",
            Self::International => "wss://dashscope-intl.aliyuncs.com/api-ws/v1/inference/",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct DashScopeSettings {
    /// Empty until the user names one.
    pub model: String,
    pub region: DashScopeRegion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WordKind {
    #[default]
    Term,
    Person,
}

/// Prompt priority. Transducer hotwords use the backend's default boost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Boost {
    #[default]
    Normal,
    Strong,
}

/// A word the engine should get right: a name, a term, or a fix for a
/// mishearing. See [`crate::dictionary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DictionaryEntry {
    pub id: String,
    /// How to write it.
    pub word: String,
    /// What the engine writes instead, replaced after recognition.
    pub sounds_like: Vec<String>,
    pub kind: WordKind,
    pub boost: Boost,
    /// App names it applies in; empty for every app.
    pub apps: Vec<String>,
}

impl Default for DictionaryEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            word: String::new(),
            sounds_like: Vec::new(),
            kind: WordKind::Term,
            boost: Boost::Normal,
            apps: Vec::new(),
        }
    }
}

impl DictionaryEntry {
    /// Whether it applies in the app named `app`.
    pub fn applies_in(&self, app: &str) -> bool {
        self.apps.is_empty() || self.apps.iter().any(|a| a.eq_ignore_ascii_case(app))
    }
}

/// How polished text should read in an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Tone {
    Formal,
    Casual,
    #[default]
    AsSpoken,
    /// No polish at all, as for code editors and terminals.
    Literal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppTone {
    pub app: String,
    pub tone: Tone,
}

/// Where the polish model runs: an OpenAI-compatible chat server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PolishProvider {
    /// A server on this Mac, such as Ollama or LM Studio.
    #[default]
    Local,
    /// Any OpenAI-compatible server, with its own optional key.
    Custom,
    /// OpenAI, with the key and base URL of the OpenAI engine.
    OpenAi,
    /// DashScope's compatible mode, with its key and region.
    DashScope,
}

/// Polish & tone: a language model rewrites the transcript before it is
/// inserted. See [`crate::polish`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PolishSettings {
    pub enabled: bool,
    pub remove_fillers: bool,
    pub self_corrections: bool,
    pub format_lists: bool,
    pub app_tone: bool,
    pub translate: bool,
    /// The language to translate into, by its English name.
    pub translate_to: String,
    pub provider: PolishProvider,
    /// The local or custom server's OpenAI-compatible base URL.
    pub base_url: String,
    /// Empty until the user names one: Viary never picks a model.
    pub model: String,
    pub tones: Vec<AppTone>,
    /// The tone in apps without their own.
    pub default_tone: Tone,
}

impl Default for PolishSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            remove_fillers: true,
            self_corrections: true,
            format_lists: true,
            app_tone: true,
            translate: false,
            translate_to: "English".into(),
            provider: PolishProvider::Local,
            base_url: "http://localhost:11434/v1".into(),
            model: String::new(),
            tones: Vec::new(),
            default_tone: Tone::AsSpoken,
        }
    }
}

impl PolishSettings {
    /// The tone set for the app named `app`.
    pub fn tone_in(&self, app: &str) -> Tone {
        self.tones
            .iter()
            .find(|t| t.app.eq_ignore_ascii_case(app))
            .map_or(self.default_tone, |t| t.tone)
    }

    /// These settings with the fields in `patch` changed, as the web views
    /// send them: one change never carries stale copies of the others.
    /// Tones change one app at a time, with [`Self::set_tone`].
    ///
    /// # Errors
    ///
    /// When `patch` names an unknown field or holds a wrong value.
    pub fn patched(&self, patch: serde_json::Map<String, serde_json::Value>) -> Result<Self, String> {
        let serde_json::Value::Object(mut fields) =
            serde_json::to_value(self).map_err(|e| e.to_string())?
        else {
            return Err("polish settings are not an object".into());
        };
        for (key, value) in patch {
            if key == "tones" || !fields.contains_key(&key) {
                return Err(format!("unknown polish setting `{key}`"));
            }
            fields.insert(key, value);
        }
        let mut polish: Self =
            serde_json::from_value(serde_json::Value::Object(fields)).map_err(|e| e.to_string())?;
        polish.model = polish.model.trim().to_owned();
        polish.base_url = polish.base_url.trim().to_owned();
        Ok(polish)
    }

    /// Sets the tone of the app named `app`, adding it to the list, or
    /// takes it off the list with `None`.
    pub fn set_tone(&mut self, app: &str, tone: Option<Tone>) {
        let app = app.trim();
        if app.is_empty() {
            return;
        }
        let at = self.tones.iter().position(|t| t.app.eq_ignore_ascii_case(app));
        match (at, tone) {
            (Some(at), Some(tone)) => self.tones[at].tone = tone,
            (Some(at), None) => {
                self.tones.remove(at);
            }
            (None, Some(tone)) => self.tones.push(AppTone {
                app: app.to_owned(),
                tone,
            }),
            (None, None) => {}
        }
    }
}

/// Everything Viary remembers between launches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub local_models: Vec<LocalModel>,
    /// `local:<id>`, `openai`, or `dashscope`. Set once the engine loads.
    pub active_engine: Option<String>,
    /// `silero_vad.onnx`, needed by offline models.
    pub vad_model: Option<PathBuf>,
    /// A punctuation model folder for the completed dictation. CT-Transformer
    /// also replaces local engines' native punctuation using the full text.
    pub punct_model: Option<PathBuf>,
    /// `cpu` or `coreml`.
    pub provider: String,
    pub threads: usize,
    /// A name from `speechkit::io::Microphone::list`, or the default device.
    pub microphone: Option<String>,
    pub language: Language,
    pub hotkey: Hotkey,
    /// 0 keeps no recordings.
    pub keep_recordings_days: u32,
    pub openai: OpenAiSettings,
    pub dashscope: DashScopeSettings,
    pub dictionary: Vec<DictionaryEntry>,
    pub polish: PolishSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            local_models: Vec::new(),
            active_engine: None,
            vad_model: None,
            punct_model: None,
            provider: "cpu".into(),
            threads: 2,
            microphone: None,
            language: Language::Auto,
            hotkey: Hotkey::Fn,
            keep_recordings_days: 7,
            openai: OpenAiSettings::default(),
            dashscope: DashScopeSettings::default(),
            dictionary: Vec::new(),
            polish: PolishSettings::default(),
        }
    }
}

impl Settings {
    pub fn local(&self, id: &str) -> Option<&LocalModel> {
        self.local_models.iter().find(|m| m.id == id)
    }
}

/// Loads and saves [`Settings`] at a fixed path.
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("settings.json"),
        }
    }

    /// The saved settings, or the defaults if there are none or they are
    /// unreadable. An unreadable file is set aside first, so the next save
    /// does not overwrite the user's only copy.
    pub fn load(&self) -> Settings {
        match fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                let backup = self.path.with_extension("json.bak");
                match fs::copy(&self.path, &backup) {
                    Ok(_) => tracing::warn!(%error, backup = %backup.display(), "settings unreadable; using defaults"),
                    Err(copy) => tracing::warn!(%error, %copy, "settings unreadable and not backed up; using defaults"),
                }
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self, settings: &Settings) {
        let result = self
            .path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| {
                let text = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
                // Write then rename, so a crash never leaves half a file.
                let tmp = self.path.with_extension("json.tmp");
                fs::write(&tmp, text)?;
                fs::rename(&tmp, &self.path)
            });
        if let Err(error) = result {
            tracing::error!(%error, "cannot save settings");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"hotkey":"rightOption"}"#).unwrap();
        assert_eq!(settings.hotkey, Hotkey::RightOption);
        assert_eq!(settings.openai.base_url, "https://api.openai.com/v1");
        assert!(
            settings.openai.model.is_empty(),
            "no model is picked for the user"
        );
    }

    #[test]
    fn polish_patches_change_only_their_fields() {
        let saved = PolishSettings {
            enabled: true,
            remove_fillers: false,
            ..PolishSettings::default()
        };
        let patch = serde_json::json!({ "formatLists": false, "model": " qwen3:8b " });
        let polish = saved.patched(patch.as_object().unwrap().clone()).unwrap();
        assert!(polish.enabled && !polish.remove_fillers && !polish.format_lists);
        assert_eq!(polish.model, "qwen3:8b");
        let unknown = serde_json::json!({ "tones": [] });
        assert!(saved.patched(unknown.as_object().unwrap().clone()).is_err());
        let wrong = serde_json::json!({ "enabled": "yes" });
        assert!(saved.patched(wrong.as_object().unwrap().clone()).is_err());
    }

    #[test]
    fn custom_polish_settings_round_trip_without_credentials() {
        let old: PolishSettings = serde_json::from_str(r#"{"model":"local-model"}"#).unwrap();
        assert_eq!(old.provider, PolishProvider::Local);
        let patch = serde_json::json!({
            "provider": "custom", "baseUrl": " https://gateway.example/v1/ ", "model": " custom-model "
        });
        let custom = old.patched(patch.as_object().unwrap().clone()).unwrap();
        assert_eq!(custom.provider, PolishProvider::Custom);
        assert_eq!(custom.base_url, "https://gateway.example/v1/");
        let encoded = serde_json::to_value(&custom).unwrap();
        assert!(encoded.get("apiKey").is_none());
        assert_eq!(
            serde_json::from_value::<PolishSettings>(encoded).unwrap(),
            custom
        );
    }

    #[test]
    fn app_tones_change_one_app_at_a_time() {
        let mut polish = PolishSettings::default();
        polish.set_tone(" Mail ", Some(Tone::Formal));
        polish.set_tone("Slack", Some(Tone::Casual));
        polish.set_tone("mail", Some(Tone::Casual));
        assert_eq!(polish.tone_in("Mail"), Tone::Casual);
        assert_eq!(polish.tones.len(), 2);
        polish.set_tone("MAIL", None);
        assert_eq!(polish.tones.len(), 1);
        assert_eq!(polish.tone_in("Mail"), Tone::AsSpoken);
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("viary-settings-{}", std::process::id()));
        let store = SettingsStore::new(&dir);
        let settings = Settings {
            language: Language::Zh,
            ..Settings::default()
        };
        store.save(&settings);
        assert_eq!(store.load(), settings);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_settings_are_backed_up() {
        let dir = std::env::temp_dir().join(format!("viary-settings-bad-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), r#"{"hotkey":"leftShift"}"#).unwrap();
        let store = SettingsStore::new(&dir);
        assert_eq!(store.load(), Settings::default());
        store.save(&Settings::default());
        let backup = fs::read_to_string(dir.join("settings.json.bak")).unwrap();
        assert!(backup.contains("leftShift"), "{backup}");
        let _ = fs::remove_dir_all(dir);
    }
}
