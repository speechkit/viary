//! Voice engines: inspecting model folders, building engines, and swapping
//! them between dictations with [`EngineManager`].
//!
//! Nothing here names a model. Local engines come from folders the user
//! picks, and cloud engines use the model name the user typed.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use speechkit::{
    SpeechError,
    asr::{AsrEngine, SegmentPostProcessor, SessionOptions},
    cloud::{
        CloudRuntime, DashScopeAsr, DashScopeAsrConfig, OpenAiHttp, OpenAiHttpConfig,
        OpenAiRealtime, OpenAiRealtimeConfig,
    },
    sherpa::{AsrFamily, Hotword, Inference, SherpaAsr, SherpaAsrConfig, SherpaPunctuation},
};

use crate::{
    dictionary::{self, Use},
    keychain::{self, Provider},
    reload::EngineManager,
    settings::{DictionaryEntry, LocalModel, OpenAiMode, Settings},
};

/// The id of the OpenAI engine in [`Settings::active_engine`].
pub const OPENAI: &str = "openai";
/// The id of the DashScope engine.
pub const DASHSCOPE: &str = "dashscope";

/// The id of a local model.
pub fn local_id(model: &LocalModel) -> String {
    local_id_of(&model.id)
}

/// The engine id of the local model whose [`LocalModel::id`] is `id`.
pub fn local_id_of(id: &str) -> String {
    format!("local:{id}")
}

/// A readable message for an error, including its sources, which
/// `SpeechError::Backend`'s own message leaves out.
pub fn describe(error: &(dyn std::error::Error + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

// ---------------------------------------------------------------------------
// Families

/// How a family behaves, for the Voice engine screen and the pill.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyInfo {
    /// The family's name, as `AsrFamily` parses it.
    pub id: String,
    pub label: &'static str,
    pub description: &'static str,
    pub tags: Vec<&'static str>,
    /// Shows words in the pill while you speak.
    pub streaming: bool,
    /// Runs behind silero VAD, so it needs `silero_vad.onnx`.
    pub needs_vad: bool,
    /// Writes its own punctuation.
    pub native_punctuation: bool,
}

pub fn family_info(family: AsrFamily) -> FamilyInfo {
    let streaming = family == AsrFamily::StreamingTransducer;
    let (label, description, tags, native_punctuation): (_, _, Vec<_>, _) = match family {
        AsrFamily::StreamingTransducer => (
            "Streaming Zipformer",
            "Shows words while you speak. Commits a phrase at each pause.",
            vec!["Live preview", "Hotwords"],
            false,
        ),
        AsrFamily::OfflineTransducer => (
            "Offline transducer",
            "Transcribes each phrase after a pause.",
            vec!["Hotwords"],
            false,
        ),
        AsrFamily::SenseVoice => (
            "SenseVoice",
            "Fast and punctuated. Transcribes each phrase after a pause.",
            vec!["5 languages", "Punctuation"],
            true,
        ),
        AsrFamily::Paraformer => (
            "Paraformer",
            "Transcribes each phrase after a pause. Strong for Chinese.",
            vec!["中文 · EN"],
            false,
        ),
        AsrFamily::FireRedCtc | AsrFamily::FireRedAed => (
            "FireRedASR",
            "Transcribes each phrase after a pause.",
            vec!["中文 · EN"],
            false,
        ),
        AsrFamily::Qwen3Asr => (
            "Qwen3-ASR",
            "Most accurate. Needs several gigabytes of memory.",
            vec!["Best accuracy", "Many languages"],
            true,
        ),
        AsrFamily::FunAsrNano => (
            "FunASR-Nano",
            "LLM-based recognition. Needs several gigabytes of memory.",
            vec!["Many languages"],
            true,
        ),
        _ => ("Other", "A sherpa-onnx model.", vec![], false),
    };
    FamilyInfo {
        id: family.to_string(),
        label,
        description,
        tags,
        streaming,
        needs_vad: !streaming,
        native_punctuation,
    }
}

/// What a picked model folder turned out to be.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInspection {
    pub path: PathBuf,
    pub name: String,
    /// The kind of files found, for when the user must pick the family.
    pub layout: String,
    /// The families this folder can be, most likely first. More than one
    /// means the files cannot tell, and the user confirms.
    pub families: Vec<FamilyInfo>,
    pub size_bytes: u64,
    /// A `silero_vad.onnx` found next to the model, if any.
    pub vad_nearby: Option<PathBuf>,
}

/// A stand-in VAD file for inspecting a folder before the user has chosen
/// silero_vad.onnx. `SherpaAsrConfig::validate` needs a VAD file for an
/// offline family and checks only that it exists and is not empty; it
/// never loads it, and nothing is built from this configuration.
fn vad_stand_in() -> Result<PathBuf, SpeechError> {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    if let Some(path) = PATH.get() {
        return Ok(path.clone());
    }
    let path = std::env::temp_dir().join("viary-inspect-vad-stand-in");
    std::fs::write(&path, b"stand-in")
        .map_err(|e| SpeechError::backend("viary", true, e))?;
    Ok(PATH.get_or_init(|| path).clone())
}

/// Detects a model folder's families without loading it. `vad` is the
/// silero VAD to check offline families against, if one is chosen.
///
/// # Errors
///
/// `InvalidModel` or `InvalidInput` naming what is missing or
/// unrecognized.
pub fn inspect(dir: &Path, vad: Option<&Path>) -> Result<ModelInspection, SpeechError> {
    let vad_nearby = find_vad(dir);
    let probe = match vad.map(Path::to_path_buf).or_else(|| vad_nearby.clone()) {
        Some(vad) => vad,
        None => vad_stand_in()?,
    };
    let offline = |family: Option<AsrFamily>| {
        let config = SherpaAsrConfig::offline(dir, &probe);
        match family {
            Some(family) => config.with_family(family),
            None => config,
        }
        .validate()
    };
    let streaming = SherpaAsrConfig::streaming(dir).validate().is_ok();
    let (layout, families) = match offline(None) {
        Ok(AsrFamily::OfflineTransducer) => {
            // Streaming and offline transducers share one layout;
            // sherpa-onnx archives say which in the folder's name.
            let named_streaming = dir
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .is_some_and(|name| name.contains("streaming") || name.contains("online"));
            let mut families = vec![AsrFamily::OfflineTransducer];
            if streaming {
                if named_streaming {
                    families.insert(0, AsrFamily::StreamingTransducer);
                } else {
                    families.push(AsrFamily::StreamingTransducer);
                }
            }
            ("transducer", families)
        }
        Ok(family) => ("single-model", vec![family]),
        // One model and tokens.txt without SenseVoice's language markers:
        // Paraformer or FireRedASR CTC, and the markers' absence proves
        // nothing about SenseVoice, so the user picks.
        Err(error @ SpeechError::InvalidInput(_)) => {
            let candidates: Vec<AsrFamily> = [
                AsrFamily::Paraformer,
                AsrFamily::FireRedCtc,
                AsrFamily::SenseVoice,
            ]
            .into_iter()
            .filter(|family| offline(Some(*family)).is_ok())
            .collect();
            if candidates.is_empty() {
                return Err(error);
            }
            ("single-model", candidates)
        }
        Err(_) if streaming => ("streaming transducer", vec![AsrFamily::StreamingTransducer]),
        Err(error) => return Err(error),
    };
    Ok(ModelInspection {
        path: dir.to_path_buf(),
        name: display_name(dir),
        layout: layout.into(),
        families: families.into_iter().map(family_info).collect(),
        size_bytes: folder_size(dir, 3),
        vad_nearby,
    })
}

/// A model folder's name without the `sherpa-onnx-` every archive starts with.
fn display_name(dir: &Path) -> String {
    let name = dir.file_name().map_or_else(
        || dir.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    name.strip_prefix("sherpa-onnx-")
        .map_or(name.clone(), str::to_owned)
}

fn folder_size(dir: &Path, depth: usize) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() && depth > 0 => folder_size(&entry.path(), depth - 1),
            Ok(meta) if meta.is_file() => meta.len(),
            _ => 0,
        })
        .sum()
}

/// `silero_vad.onnx` in the model folder or beside it, where the sherpa-onnx
/// archives are usually unpacked.
fn find_vad(dir: &Path) -> Option<PathBuf> {
    [Some(dir), dir.parent()]
        .into_iter()
        .flatten()
        .map(|d| d.join("silero_vad.onnx"))
        .find(|p| p.is_file())
}

/// Checks a punctuation model folder by loading it: speechkit has no
/// lighter check. Slow; call off the main thread.
///
/// # Errors
///
/// `InvalidModel` if it is not one.
pub fn check_punct(dir: &Path) -> Result<(), SpeechError> {
    SherpaPunctuation::load(dir).map(drop)
}

/// What kind of punctuation model `dir` holds, from its files: the
/// CNN-BiLSTM model comes with `bpe.vocab`.
pub fn punct_kind(dir: &Path) -> Option<String> {
    if !dir.is_dir() {
        return None;
    }
    Some(if dir.join("bpe.vocab").is_file() {
        "CNN-BiLSTM (English)".into()
    } else {
        "CT-Transformer (Chinese and English)".into()
    })
}

// ---------------------------------------------------------------------------
// Loaded engines

/// What the menu bar, the pill, and History say about an engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub id: String,
    /// The folder name, or the provider and model.
    pub name: String,
    /// The family or provider, such as `SenseVoice` or `OpenAI Realtime`.
    pub kind: String,
    pub on_device: bool,
    /// Reports words while you speak.
    pub live: bool,
    /// `native`, `model` (a punctuation model runs after), or `none`.
    pub punctuation: &'static str,
    pub language_override: bool,
    /// How it uses the dictionary: `hotwords`, `prompt`, or `replacements`.
    pub dictionary: &'static str,
}

/// An engine ready for dictation.
pub struct LoadedEngine {
    pub engine: AsrEngine,
    /// Applied to each committed segment after the session, so the raw
    /// text stays available for "Use raw".
    pub punct: Option<Arc<SherpaPunctuation>>,
    pub info: EngineInfo,
    /// App-only dictionary words the model accepted as hotwords, spelled
    /// for it, for sessions in those apps to add as hints.
    pub session_words: Vec<String>,
    /// Whether the model spells Latin hotwords in upper case.
    pub upper_case: bool,
}

impl LoadedEngine {
    /// Session options for a dictation in `app`: the chosen language, when
    /// the engine takes one, and the app's own dictionary words.
    pub fn session_options(&self, settings: &Settings, app: &str) -> SessionOptions {
        let mut options = SessionOptions::default();
        if let Some(code) = settings.language.code()
            && self.info.language_override
        {
            options = options.with_language(code);
        }
        if !self.session_words.is_empty() && self.engine.capabilities().session_hints {
            let hints = dictionary::session_hints(&settings.dictionary, app, &self.session_words, self.upper_case);
            if !hints.is_empty() {
                options = options.with_hints(hints);
            }
        }
        options
    }

    /// The text to insert: each segment punctuated if a model is loaded.
    pub fn finish_text(&self, segments: &[String]) -> String {
        let parts: Vec<String> = segments
            .iter()
            .map(|text| match &self.punct {
                Some(punct) if !text.trim().is_empty() => {
                    punct.process(text).unwrap_or_else(|error| {
                        tracing::warn!(%error, "punctuation failed; keeping the raw text");
                        text.clone()
                    })
                }
                _ => text.clone(),
            })
            .collect();
        join(&parts)
    }
}

/// Joins segment texts the way `Transcript::text` does: spaces only
/// between Latin text.
pub fn join(parts: &[String]) -> String {
    let segments = parts
        .iter()
        .enumerate()
        .map(|(i, text)| speechkit::asr::Segment {
            utterance: speechkit::asr::UtteranceId(i as u64),
            text: text.clone(),
            start: Duration::ZERO,
            end: Duration::ZERO,
        })
        .collect();
    speechkit::asr::Transcript::new(segments).text()
}

fn build(
    id: &str,
    settings: &Settings,
    runtime: &CloudRuntime,
) -> Result<LoadedEngine, SpeechError> {
    if id == OPENAI {
        return build_openai(settings, runtime);
    }
    if id == DASHSCOPE {
        return build_dashscope(settings, runtime);
    }
    let model = id
        .strip_prefix("local:")
        .and_then(|local| settings.local(local))
        .ok_or_else(|| SpeechError::InvalidInput("this engine was removed".into()))?;
    build_local(id, model, settings)
}

fn build_local(
    id: &str,
    model: &LocalModel,
    settings: &Settings,
) -> Result<LoadedEngine, SpeechError> {
    let family: AsrFamily = model.family.parse()?;
    let meta = family_info(family);
    let inference = Inference::default()
        .with_provider(settings.provider.parse()?)
        .with_num_threads(settings.threads.clamp(1, 16));
    let dir = model.path.as_path();
    let config = if family == AsrFamily::StreamingTransducer {
        SherpaAsrConfig::streaming(dir)
    } else {
        // The chosen VAD, or one next to the model if that one is gone.
        let vad = settings
            .vad_model
            .clone()
            .filter(|path| path.is_file())
            .or_else(|| find_vad(dir))
            .ok_or_else(|| {
                SpeechError::InvalidModel(format!(
                    "{} transcribes phrase by phrase and needs the silero VAD model; \
                     choose silero_vad.onnx under Voice activity detection",
                    meta.label
                ))
            })?;
        SherpaAsrConfig::offline(dir, vad).with_family(family)
    };
    let words = with_dictionary(config, family, dir, &settings.dictionary);
    let built = AsrEngine::new(SherpaAsr::load(&words.config.with_inference(inference))?);
    let caps = built.capabilities().clone();
    // Punctuation is optional: an unloadable model (moved, deleted, broken)
    // leaves the engine usable with its raw text.
    let punct = match (&settings.punct_model, caps.native_punctuation) {
        (Some(punct_dir), false) => match SherpaPunctuation::load(punct_dir) {
            Ok(punct) => Some(Arc::new(punct)),
            Err(error) => {
                tracing::warn!(
                    error = %describe(&error),
                    dir = %punct_dir.display(),
                    "cannot load the punctuation model; dictating without it"
                );
                None
            }
        },
        _ => None,
    };
    Ok(LoadedEngine {
        info: EngineInfo {
            id: id.to_owned(),
            name: model.name.trim_start_matches("sherpa-onnx-").to_owned(),
            kind: meta.label.into(),
            on_device: true,
            live: caps.partial_results,
            punctuation: if caps.native_punctuation {
                "native"
            } else if punct.is_some() {
                "model"
            } else {
                "none"
            },
            language_override: caps.language_override,
            dictionary: words.use_.as_str(),
        },
        engine: built,
        punct,
        session_words: words.session_words,
        upper_case: words.upper_case,
    })
}

/// A model's configuration with the dictionary in it, and what came of it.
struct WithDictionary {
    config: SherpaAsrConfig,
    use_: Use,
    session_words: Vec<String>,
    upper_case: bool,
}

/// Puts the dictionary into `config` as far as the family takes it:
/// hotwords for transducers, a prompt for Qwen3-ASR and FunASR-Nano. Words
/// the model cannot take, such as characters it does not know, are left
/// out rather than failing the load; replacements still apply them.
fn with_dictionary(
    config: SherpaAsrConfig,
    family: AsrFamily,
    dir: &Path,
    entries: &[DictionaryEntry],
) -> WithDictionary {
    let plain = |config: SherpaAsrConfig, use_| WithDictionary {
        config,
        use_,
        session_words: Vec::new(),
        upper_case: false,
    };
    let fits = |hotwords: &[Hotword]| config.clone().with_hotwords(hotwords.to_vec()).validate().is_ok();
    match family {
        AsrFamily::StreamingTransducer | AsrFamily::OfflineTransducer => {
            let tokens = dictionary::Tokens::read(dir);
            if !tokens.take_hotwords(dir.join("bpe.vocab").is_file()) {
                tracing::info!("the model has no bpe.vocab for hotwords; the dictionary applies as replacements");
                return plain(config, Use::Replacements);
            }
            // Hotwords need what the model's tokens need.
            if let Err(error) = config.clone().with_hotwords(Vec::<Hotword>::new()).validate() {
                tracing::info!(error = %describe(&error), "the model takes no hotwords; the dictionary applies as replacements");
                return plain(config, Use::Replacements);
            }
            let upper_case = tokens.upper_case;
            let words = dictionary::transducer_words(entries, upper_case);
            let every_app = accepted(words.every_app, &fits);
            let session_words: Vec<String> = accepted(words.app_only.into_iter().map(Hotword::new).collect(), &fits)
                .into_iter()
                .map(|h| h.text)
                .collect();
            let skipped = entries.len().saturating_sub(every_app.len() + session_words.len());
            if skipped > 0 {
                tracing::info!(skipped, "dictionary words the model cannot take as hotwords");
            }
            // Without words, keep greedy decoding: hotwords make decoding
            // 2 to 4 times slower.
            if every_app.is_empty() && session_words.is_empty() {
                return plain(config, Use::Hotwords);
            }
            WithDictionary {
                config: config.with_hotwords(every_app),
                use_: Use::Hotwords,
                session_words,
                upper_case,
            }
        }
        AsrFamily::Qwen3Asr | AsrFamily::FunAsrNano => {
            let prompt = dictionary::model_prompt(entries, family == AsrFamily::FunAsrNano);
            if prompt.is_empty() || !fits(&prompt) {
                return plain(config, Use::Prompt);
            }
            plain(config.with_hotwords(prompt), Use::Prompt)
        }
        _ => plain(config, Use::Replacements),
    }
}

/// The `hotwords` that `fits` accepts: all at once when it takes them, else
/// by halves, so a few bad words cost a few checks, not one per word.
fn accepted(hotwords: Vec<Hotword>, fits: &impl Fn(&[Hotword]) -> bool) -> Vec<Hotword> {
    if hotwords.is_empty() || fits(&hotwords) {
        return hotwords;
    }
    if hotwords.len() == 1 {
        return Vec::new();
    }
    let mut first = hotwords;
    let second = first.split_off(first.len() / 2);
    let mut kept = accepted(first, fits);
    kept.extend(accepted(second, fits));
    kept
}

fn require_model(model: &str, provider: &str) -> Result<String, SpeechError> {
    let model = model.trim();
    if model.is_empty() {
        return Err(SpeechError::InvalidInput(format!(
            "enter the {provider} model to use"
        )));
    }
    Ok(model.to_owned())
}

fn require_key(provider: Provider, name: &str) -> Result<Arc<speechkit::Secret>, SpeechError> {
    keychain::load(provider)
        .ok_or_else(|| SpeechError::InvalidInput(format!("add your {name} API key first")))
}

fn cloud_info(id: &str, name: String, kind: &str, built: &AsrEngine, dictionary: Use) -> EngineInfo {
    let caps = built.capabilities();
    EngineInfo {
        id: id.into(),
        name,
        kind: kind.into(),
        on_device: false,
        live: caps.partial_results,
        punctuation: if caps.native_punctuation {
            "native"
        } else {
            "none"
        },
        language_override: caps.language_override,
        dictionary: dictionary.as_str(),
    }
}

fn build_openai(settings: &Settings, runtime: &CloudRuntime) -> Result<LoadedEngine, SpeechError> {
    let openai = &settings.openai;
    let model = require_model(&openai.model, "OpenAI")?;
    let key = require_key(Provider::OpenAi, "OpenAI")?;
    let (built, kind, dictionary) = match openai.mode {
        OpenAiMode::File => {
            let mut config = OpenAiHttpConfig::new(openai.base_url.trim(), model.as_str()).with_api_key(key);
            if let Some(prompt) = dictionary::openai_prompt(&settings.dictionary) {
                config = config.with_prompt(prompt);
            }
            (AsrEngine::new(OpenAiHttp::new(config, runtime.clone())?), "OpenAI", Use::Prompt)
        }
        OpenAiMode::Realtime => (
            AsrEngine::new(OpenAiRealtime::new(
                OpenAiRealtimeConfig::new(model.as_str(), key),
                runtime.clone(),
            )?),
            "OpenAI Realtime",
            Use::Replacements,
        ),
    };
    Ok(LoadedEngine {
        info: cloud_info(OPENAI, model, kind, &built, dictionary),
        engine: built,
        punct: None,
        session_words: Vec::new(),
        upper_case: false,
    })
}

fn build_dashscope(
    settings: &Settings,
    runtime: &CloudRuntime,
) -> Result<LoadedEngine, SpeechError> {
    let dashscope = &settings.dashscope;
    let model = require_model(&dashscope.model, "DashScope")?;
    let key = require_key(Provider::DashScope, "DashScope")?;
    let built = AsrEngine::new(DashScopeAsr::new(
        DashScopeAsrConfig::new(model.as_str(), key).with_endpoint(dashscope.region.endpoint()),
        runtime.clone(),
    )?);
    Ok(LoadedEngine {
        info: cloud_info(DASHSCOPE, model, "DashScope", &built, Use::Replacements),
        engine: built,
        punct: None,
        session_words: Vec::new(),
        upper_case: false,
    })
}

// ---------------------------------------------------------------------------
// Switching

/// The engine the next dictation uses, or none before the first load.
pub type Current = Option<Arc<LoadedEngine>>;

/// What the Voice engine screen shows about loading.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub active: Option<EngineInfo>,
    /// The engine being loaded.
    pub loading: Option<String>,
    /// The engine whose last load failed, and why.
    pub failed: Option<String>,
    pub error: Option<String>,
}

/// Holds the active engine and replaces it in the background.
pub struct Engines {
    manager: EngineManager<Current>,
    status: Mutex<EngineStatus>,
    runtime: OnceLock<CloudRuntime>,
    newest: AtomicU64,
}

impl Engines {
    pub fn new() -> Self {
        Self {
            // A short debounce: a click on an engine card should load at
            // once, but a burst of setting changes still collapses.
            manager: EngineManager::with_debounce(None, Duration::from_millis(150)),
            status: Mutex::new(EngineStatus::default()),
            runtime: OnceLock::new(),
            newest: AtomicU64::new(0),
        }
    }

    /// The engine for a new dictation. Dictations already running keep
    /// the engine they started with.
    pub fn current(&self) -> Current {
        self.manager.current()
    }

    pub fn status(&self) -> EngineStatus {
        let mut status = lock(&self.status).clone();
        status.active = self.current().map(|loaded| loaded.info.clone());
        status
    }

    fn runtime(&self) -> Result<CloudRuntime, SpeechError> {
        if let Some(runtime) = self.runtime.get() {
            return Ok(runtime.clone());
        }
        let runtime = CloudRuntime::owned(2)?;
        Ok(self.runtime.get_or_init(|| runtime).clone())
    }

    /// Loads engine `id` with `settings` in the background, and calls
    /// `done` once it is decided. A request superseded by a newer one is
    /// decided by that one, so `done` then reports whichever engine is
    /// current, which may not be `id`. A failed load keeps the previous
    /// engine.
    pub fn switch(
        self: &Arc<Self>,
        id: String,
        settings: Settings,
        done: impl FnOnce(Result<EngineInfo, String>) + Send + 'static,
    ) {
        {
            let mut status = lock(&self.status);
            status.loading = Some(id.clone());
            status.failed = None;
            status.error = None;
        }
        let runtime = match self.runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                self.settle(&id, Err(describe(&error)), done);
                return;
            }
        };
        let target = id.clone();
        let generation = self.manager.request_reload(move || {
            let started = Instant::now();
            let loaded = build(&target, &settings, &runtime)?;
            tracing::info!(engine = %target, elapsed = ?started.elapsed(), "engine loaded");
            Ok(Some(Arc::new(loaded)))
        });
        self.newest.fetch_max(generation.0, Ordering::SeqCst);
        let engines = self.clone();
        std::thread::spawn(move || {
            // Large models take a while; the wait only bounds a stuck load.
            let outcome = engines
                .manager
                .wait(generation, Instant::now() + Duration::from_secs(600));
            let outcome = match outcome {
                Ok(()) => engines
                    .current()
                    .map(|loaded| loaded.info.clone())
                    .ok_or_else(|| "no engine".to_owned()),
                Err(error) => Err(describe(&error)),
            };
            if engines.newest.load(Ordering::SeqCst) == generation.0 {
                engines.settle(&id, outcome, done);
            } else {
                // A newer request owns the status, but whoever asked for
                // this one is still waiting.
                done(outcome);
            }
        });
    }

    /// Drops the current engine, and any load in flight, so new
    /// dictations have none.
    pub fn unload(&self) {
        {
            let mut status = lock(&self.status);
            status.loading = None;
            status.failed = None;
            status.error = None;
        }
        let generation = self.manager.request_reload(|| Ok(None));
        self.newest.fetch_max(generation.0, Ordering::SeqCst);
    }

    fn settle(
        &self,
        id: &str,
        outcome: Result<EngineInfo, String>,
        done: impl FnOnce(Result<EngineInfo, String>),
    ) {
        {
            let mut status = lock(&self.status);
            status.loading = None;
            if let Err(error) = &outcome {
                status.failed = Some(id.to_owned());
                status.error = Some(error.clone());
            }
        }
        done(outcome);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotwords_are_checked_by_halves() {
        let checks = std::cell::Cell::new(0);
        let fits = |hotwords: &[Hotword]| {
            checks.set(checks.get() + 1);
            hotwords.iter().all(|h| !h.text.contains('7'))
        };
        let words: Vec<Hotword> = (0..64).map(|i| Hotword::new(format!("w{i}"))).collect();
        let kept = accepted(words, &fits);
        assert_eq!(kept.len(), 58, "w7, w17, w27, w37, w47 and w57 are out");
        assert!(kept.iter().all(|h| !h.text.contains('7')));
        assert!(checks.get() < 64, "{} checks", checks.get());
    }

    fn dir_with(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("viary-model-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (file, content) in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        dir
    }

    fn ids(found: &ModelInspection) -> Vec<&str> {
        found.families.iter().map(|f| f.id.as_str()).collect()
    }

    #[test]
    fn sense_voice_is_detected_from_its_tokens() {
        let dir = dir_with(
            "sense-voice",
            &[
                ("model.int8.onnx", "x"),
                ("tokens.txt", "<unk> 0\n<|zh|> 1\n"),
            ],
        );
        let found = inspect(&dir, None).unwrap();
        assert_eq!(ids(&found), ["sense-voice"]);
        assert!(found.families[0].needs_vad);
    }

    #[test]
    fn a_markerless_flat_model_asks_for_the_family() {
        let dir = dir_with("flat", &[("model.onnx", "x"), ("tokens.txt", "<unk> 0\n")]);
        let found = inspect(&dir, None).unwrap();
        assert_eq!(ids(&found), ["paraformer", "firered-ctc", "sense-voice"]);
    }

    fn transducer(root: &str, folder: &str) -> PathBuf {
        let files = ["encoder.onnx", "decoder.onnx", "joiner.onnx"]
            .map(|file| (format!("{folder}/{file}"), "x"));
        let mut all: Vec<(&str, &str)> = files.iter().map(|(f, c)| (f.as_str(), *c)).collect();
        let tokens = format!("{folder}/tokens.txt");
        all.push((&tokens, "a 0"));
        dir_with(root, &all).join(folder)
    }

    #[test]
    fn transducers_default_by_folder_name() {
        let found = inspect(&transducer("transducer", "m-streaming"), None).unwrap();
        assert_eq!(ids(&found), ["streaming-transducer", "transducer"]);
        assert!(found.families[0].streaming);
    }

    #[test]
    fn only_the_folder_name_says_streaming() {
        // "streaming" in a parent folder says nothing about this model.
        let found = inspect(&transducer("streaming-models", "zipformer-en"), None).unwrap();
        assert_eq!(ids(&found)[0], "transducer");
    }


    #[test]
    fn an_unknown_folder_names_the_problem() {
        let dir = dir_with("unknown", &[("readme.md", "hi")]);
        let error = inspect(&dir, None).unwrap_err();
        assert!(!error.to_string().is_empty(), "{error}");
    }

    #[test]
    fn punctuation_kind_comes_from_the_files() {
        let ct = dir_with("punct-ct", &[("model.onnx", "x")]);
        assert_eq!(punct_kind(&ct).unwrap(), "CT-Transformer (Chinese and English)");
        let bilstm = dir_with("punct-bilstm", &[("model.onnx", "x"), ("bpe.vocab", "x")]);
        assert_eq!(punct_kind(&bilstm).unwrap(), "CNN-BiLSTM (English)");
        assert!(punct_kind(&ct.join("missing")).is_none());
    }

    // Real models, from `~/.cache/speechkit/models` (or SPEECHKIT_MODELS).
    // Run with `cargo test -- --ignored`.
    fn models() -> PathBuf {
        std::env::var_os("SPEECHKIT_MODELS").map_or_else(
            || PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/speechkit/models"),
            PathBuf::from,
        )
    }

    const ZIPFORMER: &str = "sherpa-onnx-streaming-zipformer-en-2023-06-26";
    const SENSE_VOICE: &str = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17";

    /// Adds `folder` the way the Voice engine screen does: detect, then
    /// take the most likely family.
    fn add(settings: &mut Settings, folder: &str) -> String {
        let found = inspect(&models().join(folder), settings.vad_model.as_deref()).unwrap();
        let model = LocalModel {
            id: folder.into(),
            name: found.name,
            path: found.path,
            family: found.families[0].id.clone(),
            size_bytes: found.size_bytes,
        };
        if settings.vad_model.is_none() {
            settings.vad_model = found.vad_nearby;
        }
        let id = local_id(&model);
        settings.local_models.push(model);
        id
    }

    fn transcribe(loaded: &LoadedEngine, wav: &Path) -> (String, String) {
        let audio = speechkit::audio::read_wav_pcm16(wav).unwrap();
        let outcome = loaded
            .engine
            .transcribe(
                &audio,
                speechkit::asr::SessionOptions::default(),
                Instant::now() + Duration::from_secs(120),
            )
            .unwrap();
        let segments: Vec<String> = outcome
            .transcript
            .segments
            .iter()
            .map(|s| s.text.clone())
            .collect();
        (join(&segments), loaded.finish_text(&segments))
    }

    #[test]
    #[ignore = "needs sherpa-onnx models"]
    fn streaming_zipformer_is_detected_and_transcribes_live() {
        let mut settings = Settings::default();
        let id = add(&mut settings, ZIPFORMER);
        assert_eq!(settings.local_models[0].family, "streaming-transducer");
        let runtime = CloudRuntime::owned(1).unwrap();
        let loaded = build(&id, &settings, &runtime).unwrap();
        assert!(
            loaded.info.live,
            "a streaming model shows words in the pill"
        );
        assert_eq!(loaded.info.punctuation, "none");
        let (raw, text) = transcribe(&loaded, &models().join(ZIPFORMER).join("test_wavs/0.wav"));
        assert!(raw.to_lowercase().contains("yellow"), "{raw}");
        assert_eq!(raw, text, "no punctuation model configured");
    }

    #[test]
    #[ignore = "needs sherpa-onnx models"]
    fn sense_voice_runs_behind_the_vad_it_finds_nearby() {
        let mut settings = Settings::default();
        let id = add(&mut settings, SENSE_VOICE);
        assert_eq!(settings.local_models[0].family, "sense-voice");
        assert!(
            settings.vad_model.is_some(),
            "silero_vad.onnx sits next to the model folder"
        );
        let runtime = CloudRuntime::owned(1).unwrap();
        let loaded = build(&id, &settings, &runtime).unwrap();
        assert!(!loaded.info.live);
        assert_eq!(loaded.info.punctuation, "native");
        assert!(loaded.info.language_override);
        let (_, text) = transcribe(
            &loaded,
            &models().join(SENSE_VOICE).join("test_wavs/zh.wav"),
        );
        assert!(
            text.contains('。') || text.contains('，'),
            "SenseVoice punctuates: {text}"
        );
    }

    #[test]
    #[ignore = "needs sherpa-onnx models"]
    fn an_offline_model_without_vad_says_what_is_missing() {
        let mut settings = Settings::default();
        let id = add(&mut settings, SENSE_VOICE);
        settings.vad_model = Some(PathBuf::from("/nonexistent/silero_vad.onnx"));
        // The nearby VAD is found again, so hide the model somewhere else.
        settings.local_models[0].path = PathBuf::from("/nonexistent/model");
        let runtime = CloudRuntime::owned(1).unwrap();
        assert!(build(&id, &settings, &runtime).is_err());
    }

    #[test]
    #[ignore = "needs sherpa-onnx models"]
    fn switching_engines_keeps_the_old_one_until_the_new_one_loads() {
        let mut settings = Settings::default();
        let streaming = add(&mut settings, ZIPFORMER);
        let offline = add(&mut settings, SENSE_VOICE);
        let engines = Arc::new(Engines::new());
        let wait = |id: &str, settings: &Settings| {
            let (sender, received) = std::sync::mpsc::channel();
            engines.switch(id.into(), settings.clone(), move |outcome| {
                let _ = sender.send(outcome);
            });
            received.recv_timeout(Duration::from_secs(120)).unwrap()
        };
        assert_eq!(wait(&streaming, &settings).unwrap().id, streaming);
        // A session started now keeps the streaming engine across the switch.
        let before = engines.current().unwrap();
        assert_eq!(wait(&offline, &settings).unwrap().id, offline);
        assert_eq!(before.info.id, streaming);
        assert_eq!(engines.current().unwrap().info.id, offline);
        // A failed load keeps the engine in use and reports why.
        let error = wait(OPENAI, &settings).unwrap_err();
        assert!(error.contains("OpenAI model"), "{error}");
        assert_eq!(engines.current().unwrap().info.id, offline);
        let status = engines.status();
        assert_eq!(status.failed.as_deref(), Some(OPENAI));
        assert_eq!(status.active.unwrap().id, offline);
    }

    #[test]
    #[ignore = "needs sherpa-onnx models"]
    fn a_model_without_hotword_support_still_loads_with_a_dictionary() {
        let mut settings = Settings::default();
        let id = add(&mut settings, ZIPFORMER);
        settings.dictionary.push(DictionaryEntry {
            id: "1".into(),
            word: "yellow".into(),
            ..DictionaryEntry::default()
        });
        let runtime = CloudRuntime::owned(1).unwrap();
        let loaded = build(&id, &settings, &runtime).unwrap();
        // This archive has no bpe.vocab, which BPE hotwords need.
        assert_eq!(loaded.info.dictionary, "replacements");
        assert!(loaded.session_words.is_empty());
        let (raw, _) = transcribe(&loaded, &models().join(ZIPFORMER).join("test_wavs/0.wav"));
        assert!(raw.to_lowercase().contains("yellow"), "{raw}");
    }

    #[test]
    #[ignore = "needs sherpa-onnx models"]
    fn a_punctuation_model_loads_and_is_described() {
        let dir = models().join("sherpa-onnx-online-punct-en-2024-08-06");
        check_punct(&dir).unwrap();
        assert_eq!(punct_kind(&dir).unwrap(), "CNN-BiLSTM (English)");
        assert!(check_punct(&models().join(ZIPFORMER)).is_err());
    }

    #[test]
    fn joining_keeps_cjk_tight() {
        assert_eq!(join(&["你好，".into(), "世界".into()]), "你好，世界");
        assert_eq!(join(&["Hello.".into(), "World".into()]), "Hello. World");
    }
}
