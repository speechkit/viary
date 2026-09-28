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
    /// The code for `SessionOptions::language`, or `None` for automatic.
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

/// Everything Viary remembers between launches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub local_models: Vec<LocalModel>,
    /// `local:<id>`, `openai`, or `dashscope`. Set once the engine loads.
    pub active_engine: Option<String>,
    /// `silero_vad.onnx`, needed by offline models.
    pub vad_model: Option<PathBuf>,
    /// A punctuation model folder, for engines without native punctuation.
    pub punct_model: Option<PathBuf>,
    /// `cpu` or `coreml`.
    pub provider: String,
    pub threads: usize,
    /// A name from `speechkit::io::input_devices`, or the default device.
    pub microphone: Option<String>,
    pub language: Language,
    pub hotkey: Hotkey,
    /// 0 keeps no recordings.
    pub keep_recordings_days: u32,
    pub openai: OpenAiSettings,
    pub dashscope: DashScopeSettings,
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
