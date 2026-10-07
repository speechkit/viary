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
    asr::{AsrEngine, AsrOptions, PostProcessor, Transcript},
    cloud::{
        CloudRuntime, DashScopeAsr, DashScopeAsrConfig, OpenAiRealtime, OpenAiRealtimeConfig,
        OpenAiTranscription, OpenAiTranscriptionConfig,
    },
    sherpa::{
        AsrConfig, AsrFamily, Inference, Punctuation, PunctuationConfig, PunctuationFamily,
        SileroVadConfig,
    },
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

/// Detects model families from the files without loading native code.
pub fn inspect(dir: &Path) -> Result<ModelInspection, SpeechError> {
    let mut families = speechkit::sherpa::inspect(dir)?.asr;
    if families.is_empty() {
        return Err(SpeechError::InvalidModel(
            "this folder has no recognition model".into(),
        ));
    }
    // Prefer offline unless the folder itself names a streaming model.
    let named_streaming = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .is_some_and(|name| name.contains("streaming") || name.contains("online"));
    if families.contains(&AsrFamily::OfflineTransducer) && !named_streaming {
        families.sort_by_key(|family| *family != AsrFamily::OfflineTransducer);
    }
    // A single model whose tokens lack SenseVoice's language markers fits it
    // too, and speechkit lists it first, but the published SenseVoice models
    // have the markers: such a folder is Paraformer or FireRedASR CTC far more
    // often. The first family is the one the Voice engine screen preselects.
    if families.len() > 1 {
        families.sort_by_key(|family| *family == AsrFamily::SenseVoice);
    }
    let layout = if families.contains(&AsrFamily::OfflineTransducer) {
        "transducer"
    } else {
        "single-model"
    };
    Ok(ModelInspection {
        path: dir.to_path_buf(),
        name: display_name(dir),
        layout: layout.into(),
        families: families.into_iter().map(family_info).collect(),
        size_bytes: folder_size(dir, 3),
        vad_nearby: find_vad(dir),
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

/// The punctuation family of the model in `dir`, from its files.
///
/// Not `PunctuationConfig::validate`, which takes any folder with a
/// `model*.onnx`, a recognition model's included. Nothing may then load such
/// a folder as punctuation: sherpa-onnx exits the whole process for a model of
/// another kind, and aborts it for a corrupt one. `inspect` also reads the
/// other files, so it tells a recognition model apart.
fn punct_family(dir: &Path) -> Result<PunctuationFamily, SpeechError> {
    let found = speechkit::sherpa::inspect(dir)?;
    let message = if found.asr.is_empty() {
        "this folder has no punctuation model"
    } else {
        "this is a recognition model; choose a punctuation model folder"
    };
    found
        .punctuation
        .ok_or_else(|| SpeechError::InvalidModel(message.into()))
}

/// Checks that `dir` holds a punctuation model, from its files alone: see
/// [`punct_family`] for why it is not loaded.
pub fn check_punct(dir: &Path) -> Result<(), SpeechError> {
    punct_family(dir).map(drop)
}

/// The punctuation family described by its files.
pub fn punct_kind(dir: &Path) -> Option<String> {
    Some(
        match punct_family(dir).ok()? {
            PunctuationFamily::CnnBiLstm => "CNN-BiLSTM (English)",
            PunctuationFamily::CtTransformer => "CT-Transformer (Chinese and English)",
            _ => return None,
        }
        .into(),
    )
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
    /// Applied once to the complete text after the session, so the raw
    /// text stays available for "Use raw".
    pub punct: Option<Arc<Punctuation>>,
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
    pub fn session_options(&self, settings: &Settings, app: &str) -> AsrOptions {
        let mut options = AsrOptions::default();
        if let Some(code) = settings.language.code()
            && self.info.language_override
        {
            options = options.with_language(code);
        }
        if !self.session_words.is_empty() && self.engine.capabilities().accepts_hints {
            let hints = dictionary::session_hints(&settings.dictionary, app, &self.session_words, self.upper_case);
            if !hints.is_empty() {
                options = options.with_hints(hints);
            }
        }
        options
    }

    /// Punctuates the completed dictation, keeping the recognizer's text
    /// wherever the model fails. `dictionary` gives the words a model that
    /// punctuates the whole text should see.
    pub fn punctuate(&self, transcript: &Transcript, dictionary: impl Fn(&str) -> String) -> String {
        let Some(punct) = &self.punct else {
            return transcript.text();
        };
        if self.engine.capabilities().punctuated {
            let texts: Vec<_> = transcript.segments.iter().map(|s| s.text.as_str()).collect();
            punctuate_boundaries(&texts, punct.as_ref())
        } else {
            punctuate_text(&dictionary(&transcript.text()), punct.as_ref())
        }
    }
}

/// Punctuates unpunctuated text, keeping it if the model fails.
fn punctuate_text(text: &str, punct: &dyn PostProcessor) -> String {
    if text.trim().is_empty() {
        return text.to_owned();
    }
    match punct.process(text) {
        Ok(punctuated) if !punctuated.trim().is_empty() => punctuated,
        Ok(_) => {
            tracing::warn!("punctuation returned empty text; keeping the raw text");
            text.to_owned()
        }
        Err(error) => {
            tracing::warn!(%error, "punctuation failed; keeping the raw text");
            text.to_owned()
        }
    }
}

/// What the model put where one recognition segment meets the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boundary {
    Nothing,
    Comma(char),
    Stop(char),
}

/// Native recognizers end every segment as a sentence, so a pause in the
/// middle of a thought becomes "。". The model reads the whole dictation and
/// decides only what belongs at each boundary between segments: nothing, a
/// comma, or a sentence end, which keeps the recognizer's own mark ("？",
/// "！"). Everything inside a segment stays as recognized. A boundary the
/// model's output does not line up with keeps its native mark.
fn punctuate_boundaries(texts: &[&str], punct: &dyn PostProcessor) -> String {
    let texts: Vec<_> = texts.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
    let mut joined = String::new();
    if texts.len() < 2 {
        texts.iter().for_each(|t| push_joined(&mut joined, t));
        return joined;
    }
    // The model's input: every segment without its trailing mark (the last
    // keeps it) and without non-ASCII punctuation, which the model would
    // not echo back. `ends` counts the content characters before each
    // boundary.
    let last = texts.len() - 1;
    let mut input = String::new();
    let mut ends = Vec::with_capacity(last);
    for (i, text) in texts.iter().enumerate() {
        let body = if i == last { text } else { split_mark(text).0 };
        let kept: String = body.chars().filter(|&c| model_content(c) || c.is_whitespace()).collect();
        push_joined(&mut input, kept.trim());
        if i < last {
            ends.push(input.chars().filter(|&c| model_content(c)).count());
        }
    }
    let decided = match punct.process(&input) {
        Ok(output) => boundaries(&input, &output, &ends),
        Err(error) => {
            tracing::warn!(%error, "punctuation failed; keeping the raw text");
            vec![None; ends.len()]
        }
    };
    for (i, text) in texts.iter().enumerate() {
        let Some(Some(boundary)) = decided.get(i) else {
            push_joined(&mut joined, text);
            continue;
        };
        let (body, native) = split_mark(text);
        let ascii = body.ends_with(|c: char| c.is_ascii_alphanumeric())
            && texts[i + 1].starts_with(|c: char| c.is_ascii_alphanumeric());
        let mark = match *boundary {
            Boundary::Nothing => String::new(),
            Boundary::Comma(_) if ascii => ",".into(),
            Boundary::Comma(mark) => mark.into(),
            Boundary::Stop(_) if !native.is_empty() => native.into(),
            Boundary::Stop(mark) if ascii => if mark == '？' { "?" } else { "." }.into(),
            Boundary::Stop(mark) => mark.into(),
        };
        push_joined(&mut joined, &format!("{body}{mark}"));
    }
    joined
}

/// Reads the model's decision at each boundary, at `ends` content
/// characters. A decision counts only where the output's content matches
/// the input on both sides of it: the model sometimes drops words.
fn boundaries(input: &str, output: &str, ends: &[usize]) -> Vec<Option<Boundary>> {
    let expected: Vec<_> = input.chars().filter(|&c| model_content(c)).collect();
    let mut marks = vec![Boundary::Nothing; expected.len() + 1];
    let mut matched = 0;
    for c in output.chars().filter(|c| !c.is_whitespace()) {
        if model_content(c) {
            if !expected.get(matched).is_some_and(|e| e.eq_ignore_ascii_case(&c)) {
                break;
            }
            matched += 1;
        } else if matches!(c, '。' | '？' | '！') {
            marks[matched] = Boundary::Stop(c);
        } else if marks[matched] == Boundary::Nothing {
            marks[matched] = Boundary::Comma(if c == '、' { c } else { '，' });
        }
    }
    if ends.iter().any(|&end| end >= matched) {
        tracing::warn!("punctuation changed the text; keeping some native marks");
    }
    ends.iter()
        .map(|&end| (end < matched).then(|| marks[end]))
        .collect()
}

/// Characters the model echoes: ASCII and letters, but not CJK punctuation.
fn model_content(c: char) -> bool {
    !c.is_whitespace() && (c.is_ascii() || c.is_alphanumeric())
}

/// Splits a segment's trailing marks ("。", "?!", "...") from its text.
fn split_mark(text: &str) -> (&str, &str) {
    let body = text
        .trim_end_matches(|c| {
            matches!(c, '，' | '、' | '。' | '！' | '？' | '；' | '…' | ',' | '.' | '!' | '?' | ';')
        })
        .trim_end();
    (body, text[body.len()..].trim())
}

/// Appends `text` as `Transcript::text` joins segments: a space only
/// between ASCII words or after ASCII punctuation.
fn push_joined(out: &mut String, text: &str) {
    if out.ends_with(|c: char| c.is_ascii_alphanumeric() || ".!?;:,".contains(c))
        && text.starts_with(|c: char| c.is_ascii_alphanumeric())
    {
        out.push(' ');
    }
    out.push_str(text);
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
        .with_threads(settings.threads.clamp(1, 16));
    let dir = model.path.as_path();
    let config = if family == AsrFamily::StreamingTransducer {
        AsrConfig::streaming(dir)
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
        let config = AsrConfig::offline(dir, &vad).with_family(family);
        if family == AsrFamily::SenseVoice {
            // Keep brief thinking pauses in one utterance, so SenseVoice
            // does not punctuate each fragment as a complete sentence.
            config.with_vad(SileroVadConfig::new(vad).with_min_silence(Duration::from_secs(1)))
        } else {
            config
        }
    };
    let words = with_dictionary(config, family, dir, &settings.dictionary);
    let built = AsrEngine::new(words.config.with_inference(inference).load()?);
    let caps = built.capabilities().clone();
    // Punctuation is optional: an unloadable model (moved, deleted, broken)
    // leaves the engine usable with its raw text. Its files are checked first:
    // a folder that is no longer a punctuation model would end the process.
    let punct = match &settings.punct_model {
        Some(punct_dir) => {
            // Native marks are provisional when a Chinese/English
            // model is available. An English-only model cannot replace them.
            let load = || {
                let punct_family = punct_family(punct_dir)?;
                if caps.punctuated && punct_family != PunctuationFamily::CtTransformer {
                    return Ok(None);
                }
                PunctuationConfig::new(punct_dir)
                    .load()
                    .map(|p| Some(Arc::new(p)))
            };
            match load() {
                Ok(punct) => punct,
                Err(error) => {
                    tracing::warn!(
                        error = %describe(&error),
                        dir = %punct_dir.display(),
                        "cannot load the punctuation model; dictating without it"
                    );
                    None
                }
            }
        }
        None => None,
    };
    Ok(LoadedEngine {
        info: EngineInfo {
            id: id.to_owned(),
            name: model.name.trim_start_matches("sherpa-onnx-").to_owned(),
            kind: meta.label.into(),
            on_device: true,
            live: caps.reports_partials,
            punctuation: if punct.is_some() {
                "model"
            } else if caps.punctuated {
                "native"
            } else {
                "none"
            },
            language_override: caps.accepts_language,
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
    config: AsrConfig,
    use_: Use,
    session_words: Vec<String>,
    upper_case: bool,
}

/// Puts the dictionary into `config` as far as the family takes it:
/// hotwords for transducers, a prompt for Qwen3-ASR and FunASR-Nano. Words
/// the model cannot take, such as characters it does not know, are left
/// out rather than failing the load; replacements still apply them.
fn with_dictionary(
    config: AsrConfig,
    family: AsrFamily,
    dir: &Path,
    entries: &[DictionaryEntry],
) -> WithDictionary {
    let plain = |config: AsrConfig, use_| WithDictionary {
        config,
        use_,
        session_words: Vec::new(),
        upper_case: false,
    };
    let fits = |hotwords: &[String]| config.clone().with_hotwords(hotwords.to_vec()).validate().is_ok();
    match family {
        AsrFamily::StreamingTransducer | AsrFamily::OfflineTransducer => {
            let tokens = dictionary::Tokens::read(dir);
            if !tokens.take_hotwords(dir.join("bpe.vocab").is_file()) {
                tracing::info!("the model has no bpe.vocab for hotwords; the dictionary applies as replacements");
                return plain(config, Use::Replacements);
            }
            // Hotwords need what the model's tokens need.
            if let Err(error) = config.clone().with_hotwords(Vec::<String>::new()).validate() {
                tracing::info!(error = %describe(&error), "the model takes no hotwords; the dictionary applies as replacements");
                return plain(config, Use::Replacements);
            }
            let upper_case = tokens.upper_case;
            let words = dictionary::transducer_words(entries, upper_case);
            let every_app = accepted(words.every_app, &fits);
            let session_words: Vec<String> = accepted(words.app_only, &fits);
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
fn accepted(hotwords: Vec<String>, fits: &impl Fn(&[String]) -> bool) -> Vec<String> {
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
        live: caps.reports_partials,
        punctuation: if caps.punctuated {
            "native"
        } else {
            "none"
        },
        language_override: caps.accepts_language,
        dictionary: dictionary.as_str(),
    }
}

fn build_openai(settings: &Settings, runtime: &CloudRuntime) -> Result<LoadedEngine, SpeechError> {
    let openai = &settings.openai;
    let model = require_model(&openai.model, "OpenAI")?;
    let key = require_key(Provider::OpenAi, "OpenAI")?;
    let (built, kind, dictionary) = match openai.mode {
        OpenAiMode::File => {
            let mut config = OpenAiTranscriptionConfig::new(model.as_str())
                .with_endpoint(openai.base_url.trim())
                .with_api_key(key);
            if let Some(prompt) = dictionary::openai_prompt(&settings.dictionary) {
                config = config.with_prompt(prompt);
            }
            (AsrEngine::new(OpenAiTranscription::new(config, runtime.clone())?), "OpenAI", Use::Prompt)
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

    struct CheckedPunctuation {
        input: &'static str,
        output: Option<&'static str>,
    }

    impl PostProcessor for CheckedPunctuation {
        fn process(&self, text: &str) -> Result<String, SpeechError> {
            assert_eq!(text, self.input);
            self.output
                .map(str::to_owned)
                .ok_or_else(|| SpeechError::backend("test", false, "unavailable"))
        }
    }

    fn boundaries_of(texts: &[&str], input: &'static str, output: Option<&'static str>) -> String {
        punctuate_boundaries(texts, &CheckedPunctuation { input, output })
    }

    #[test]
    fn a_pause_is_not_a_sentence_end() {
        assert_eq!(
            boundaries_of(
                &["从而导致不该间断的地方。", "被加上了标点符号。"],
                "从而导致不该间断的地方被加上了标点符号",
                Some("从而导致不该间断的地方被加上了标点符号。"),
            ),
            "从而导致不该间断的地方被加上了标点符号。"
        );
        assert_eq!(
            boundaries_of(
                &["我今天去了超市。", "买了很多东西。", "然后回家做饭。"],
                "我今天去了超市买了很多东西然后回家做饭",
                Some("我今天去了超市，买了很多东西，然后回家做饭。"),
            ),
            "我今天去了超市，买了很多东西，然后回家做饭。"
        );
    }

    #[test]
    fn a_sentence_end_keeps_the_recognizers_mark() {
        assert_eq!(
            boundaries_of(
                &["太好了！", "我们明天见。"],
                "太好了我们明天见",
                Some("太好了。我们明天见。"),
            ),
            "太好了！我们明天见。"
        );
    }

    #[test]
    fn english_boundaries_get_ascii_marks() {
        assert_eq!(
            boundaries_of(
                &["Hello.", "how are you?", "I am fine", "thanks"],
                "Hello how are you I am fine thanks",
                Some("Hello，how are you？I am fine。thanks。"),
            ),
            "Hello, how are you? I am fine. thanks"
        );
        assert_eq!(
            boundaries_of(&["speech.", "kit is fast."], "speech kit is fast.", Some("speech kit is fast .。")),
            "speech kit is fast."
        );
    }

    #[test]
    fn text_inside_segments_is_kept_as_recognized() {
        assert_eq!(
            boundaries_of(
                &["版本 1.2.3，价格 3.14。", "网址 example.com。"],
                "版本 1.2.3价格 3.14网址 example.com",
                Some("版本1 . 2 . 3，价格3 . 14，网址example . com。"),
            ),
            "版本 1.2.3，价格 3.14，网址 example.com。"
        );
        assert_eq!(
            boundaries_of(
                &["“你好！”她说。", "明天见。"],
                "你好她说明天见",
                Some("你好，她说，明天见。"),
            ),
            "“你好！”她说，明天见。"
        );
    }

    #[test]
    fn a_boundary_the_model_does_not_line_up_with_keeps_its_mark() {
        // The model dropped words after the first boundary.
        assert_eq!(
            boundaries_of(
                &["我很好。", "谢谢你。", "再见。"],
                "我很好谢谢你再见",
                Some("我很好，谢谢。"),
            ),
            "我很好，谢谢你。再见。"
        );
        // A mark right before the output stops is not confirmed either.
        assert_eq!(
            boundaries_of(&["我很好。", "谢谢你。"], "我很好谢谢你", Some("我很好，")),
            "我很好。谢谢你。"
        );
        for output in [None, Some(""), Some("完全不同的内容。")] {
            assert_eq!(
                boundaries_of(&["我很好。", "谢谢你。"], "我很好谢谢你", output),
                "我很好。谢谢你。"
            );
        }
    }

    #[test]
    fn one_segment_needs_no_model() {
        for texts in [&[][..], &[" "], &["我很好。"], &["我很好。", " "]] {
            let joined = texts.iter().map(|t| t.trim()).collect::<String>();
            assert_eq!(boundaries_of(texts, "must not be called", None), joined);
        }
    }

    #[test]
    fn unpunctuated_text_takes_the_models_output_unless_it_fails() {
        let punct = |output| CheckedPunctuation { input: "speech kit is fast", output };
        assert_eq!(punctuate_text("speech kit is fast", &punct(Some("Speech kit is fast."))), "Speech kit is fast.");
        for output in [None, Some(" ")] {
            assert_eq!(punctuate_text("speech kit is fast", &punct(output)), "speech kit is fast");
        }
        assert_eq!(punctuate_text(" ", &punct(None)), " ");
    }

    #[test]
    fn hotwords_are_checked_by_halves() {
        let checks = std::cell::Cell::new(0);
        let fits = |hotwords: &[String]| {
            checks.set(checks.get() + 1);
            hotwords.iter().all(|h| !h.contains('7'))
        };
        let words: Vec<String> = (0..64).map(|i| format!("w{i}")).collect();
        let kept = accepted(words, &fits);
        assert_eq!(kept.len(), 58, "w7, w17, w27, w37, w47 and w57 are out");
        assert!(kept.iter().all(|h| !h.contains('7')));
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
        let found = inspect(&dir).unwrap();
        assert_eq!(ids(&found), ["sense-voice"]);
        assert!(found.families[0].needs_vad);
    }

    #[test]
    fn a_markerless_flat_model_asks_for_the_family() {
        let dir = dir_with("flat", &[("model.onnx", "x"), ("tokens.txt", "<unk> 0\n")]);
        let found = inspect(&dir).unwrap();
        // The first is preselected, and loading a Paraformer folder as
        // SenseVoice ends the process, so SenseVoice goes last.
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
        let found = inspect(&transducer("transducer", "m-streaming")).unwrap();
        assert_eq!(ids(&found), ["streaming-transducer", "transducer"]);
        assert!(found.families[0].streaming);
    }

    #[test]
    fn only_the_folder_name_says_streaming() {
        // "streaming" in a parent folder says nothing about this model.
        let found = inspect(&transducer("streaming-models", "zipformer-en")).unwrap();
        assert_eq!(ids(&found)[0], "transducer");
    }

    #[test]
    fn an_unknown_folder_names_the_problem() {
        let dir = dir_with("unknown", &[("readme.md", "hi")]);
        let error = inspect(&dir).unwrap_err();
        assert!(!error.to_string().is_empty(), "{error}");
    }

    #[test]
    fn punctuation_kind_comes_from_the_files() {
        let ct = dir_with("punct-ct", &[("model.onnx", "x")]);
        assert_eq!(punct_kind(&ct).unwrap(), "CT-Transformer (Chinese and English)");
        let bilstm = dir_with("punct-bilstm", &[("model.onnx", "x"), ("bpe.vocab", "x")]);
        assert_eq!(punct_kind(&bilstm).unwrap(), "CNN-BiLSTM (English)");
        assert!(punct_kind(&ct.join("missing")).is_none());
        assert!(check_punct(&ct).is_ok());
        assert!(check_punct(&bilstm).is_ok());
    }

    #[test]
    fn a_recognition_model_is_not_taken_for_a_punctuation_model() {
        // `PunctuationConfig::validate` accepts both folders, and loading one
        // as punctuation exits the process: sherpa-onnx finds no punctuation
        // metadata in it.
        for (name, tokens) in [
            ("sense-voice", "<unk> 0\n<|zh|> 1\n"),
            ("paraformer", "<unk> 0\n"),
        ] {
            let dir = dir_with(
                &format!("asr-as-punct-{name}"),
                &[("model.int8.onnx", "x"), ("tokens.txt", tokens)],
            );
            let error = check_punct(&dir).unwrap_err();
            assert!(error.to_string().contains("recognition model"), "{error}");
            assert!(punct_kind(&dir).is_none());
        }
        let empty = dir_with("no-model", &[("readme.md", "hi")]);
        assert!(check_punct(&empty).is_err());
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
        let found = inspect(&models().join(folder)).unwrap();
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
        let audio = speechkit::audio::read(wav, speechkit::audio::DecodeLimits::default()).unwrap();
        let outcome = loaded
            .engine
            .transcribe(
                &audio,
                speechkit::asr::AsrOptions::default(),
                Instant::now() + Duration::from_secs(120),
            )
            .unwrap();
        (outcome.text(), loaded.punctuate(&outcome, str::to_owned))
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
    fn sense_voice_keeps_a_short_pause_in_one_utterance_and_flushes_on_stop() {
        let mut settings = Settings::default();
        let id = add(&mut settings, SENSE_VOICE);
        let dir = &settings.local_models[0].path;
        let mut audio = speechkit::audio::read(
            dir.join("test_wavs/zh.wav"),
            speechkit::audio::DecodeLimits::default(),
        )
        .unwrap();
        // Insert a thinking pause into the same Chinese sentence for both
        // engines; add no trailing silence, so finish must flush the tail.
        let middle = audio.samples.len() / 2;
        let pause = audio.sample_rate.frames_in(Duration::from_millis(650)) as usize;
        audio
            .samples
            .splice(middle..middle, std::iter::repeat_n(0.0, pause));
        let recognize = |engine: &AsrEngine, audio: &speechkit::AudioBuffer| {
            engine
                .transcribe(
                    audio,
                    AsrOptions::default(),
                    Instant::now() + Duration::from_secs(120),
                )
                .unwrap()
        };
        let original = AsrEngine::new(
            AsrConfig::offline(dir, settings.vad_model.as_ref().unwrap())
                .with_family(AsrFamily::SenseVoice)
                .load()
                .unwrap(),
        );
        let before = recognize(&original, &audio);
        drop(original);
        let loaded = build(&id, &settings, &CloudRuntime::owned(1).unwrap()).unwrap();
        let after = recognize(&loaded.engine, &audio);
        eprintln!("500 ms: {} segments: {}", before.segments.len(), before.text());
        eprintln!("1000 ms: {} segments: {}", after.segments.len(), after.text());
        assert!(before.segments.len() > 1, "the old VAD must split the pause");
        assert_eq!(after.segments.len(), 1, "the brief pause must stay together");
        assert!(
            after.text().ends_with("下午5点。"),
            "the final words must survive stop: {}",
            after.text()
        );
        assert_eq!(after.text().matches('。').count(), 1, "{}", after.text());

        // A real sentence break still ends an utterance.
        let extra = audio.sample_rate.frames_in(Duration::from_millis(850)) as usize;
        audio
            .samples
            .splice(middle..middle, std::iter::repeat_n(0.0, extra));
        let long_pause = recognize(&loaded.engine, &audio);
        assert_eq!(long_pause.segments.len(), 2, "a 1.5 s pause must still split");
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
        let punctuation = PunctuationConfig::new(&dir).load().unwrap();
        let text = punctuation.process("HELLO WORLD HOW ARE YOU").unwrap();
        assert_ne!(text, "HELLO WORLD HOW ARE YOU", "{text}");
        assert_eq!(punct_kind(&dir).unwrap(), "CNN-BiLSTM (English)");
        assert!(check_punct(&models().join(ZIPFORMER)).is_err());
        // A recognition model passes `PunctuationConfig::validate`, and loading
        // it as punctuation exits the process.
        assert!(check_punct(&models().join(SENSE_VOICE)).is_err());

        // The application must restore dictionary spelling after punctuation
        // while keeping the original recognizer text for Use raw.
        let mut settings = Settings {
            punct_model: Some(dir),
            ..Settings::default()
        };
        settings.dictionary.push(DictionaryEntry {
            word: "speechkit".into(),
            sounds_like: vec!["speech kit".into()],
            ..Default::default()
        });
        let id = add(&mut settings, ZIPFORMER);
        let loaded = build(&id, &settings, &CloudRuntime::owned(1).unwrap()).unwrap();
        let transcript = speechkit::asr::Transcript::new(
            vec![speechkit::asr::Segment {
                utterance: speechkit::asr::UtteranceId(0),
                text: "speech kit is fast".into(),
                start: Duration::ZERO,
                end: Duration::from_secs(1),
            }],
            Duration::from_secs(1),
        );
        let raw = transcript.text();
        let text = crate::dictation::finish(&settings, "", Some(&loaded), transcript);
        let punctuated = punctuation.process("speechkit is fast").unwrap();
        let expected = dictionary::apply(&settings.dictionary, "", &punctuated);
        assert_eq!(text, expected);
        assert!(text.starts_with("speechkit"), "{text}");
        assert_eq!(raw, "speech kit is fast");

        // Pausing between words must not cause an extra punctuation call.
        let transcript = speechkit::asr::Transcript::new(
            ["speech kit", "is", "fast"]
                .into_iter()
                .enumerate()
                .map(|(i, text)| speechkit::asr::Segment {
                    utterance: speechkit::asr::UtteranceId(i as u64),
                    text: text.into(),
                    start: Duration::from_secs(i as u64 * 5),
                    end: Duration::from_secs(i as u64 * 5 + 1),
                })
                .collect(),
            Duration::from_secs(11),
        );
        assert_eq!(
            crate::dictation::finish(&settings, "", Some(&loaded), transcript),
            expected
        );
    }

    #[test]
    #[ignore = "needs SenseVoice, FunASR-Nano and Chinese/English CT-Transformer models"]
    fn native_engines_restore_punctuation_after_joining_the_dictation() {
        for folder in [SENSE_VOICE, "sherpa-onnx-funasr-nano-int8-2025-12-30"] {
            let mut settings = Settings {
                punct_model: Some(
                    models()
                        .join("sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8"),
                ),
                ..Settings::default()
            };
            let id = add(&mut settings, folder);
            let loaded = build(&id, &settings, &CloudRuntime::owned(1).unwrap()).unwrap();
            assert_eq!(loaded.info.punctuation, "model");
            let transcript = speechkit::asr::Transcript::new(
                ["从而导致不该间断的地方。", "被加上了标点符号。"]
                    .into_iter()
                    .enumerate()
                    .map(|(i, text)| speechkit::asr::Segment {
                        utterance: speechkit::asr::UtteranceId(i as u64),
                        text: text.into(),
                        start: Duration::from_secs(i as u64 * 8),
                        end: Duration::from_secs(i as u64 * 8 + 2),
                    })
                    .collect(),
                Duration::from_secs(10),
            );
            let text = crate::dictation::finish(&settings, "", Some(&loaded), transcript);
            assert_eq!(text, "从而导致不该间断的地方被加上了标点符号。");

            let transcript = speechkit::asr::Transcript::new(
                ["版本 1.2.3，价格 3.14！", "网址 example.com。"]
                    .into_iter()
                    .enumerate()
                    .map(|(i, text)| speechkit::asr::Segment {
                        utterance: speechkit::asr::UtteranceId(i as u64),
                        text: text.into(),
                        start: Duration::from_secs(i as u64 * 8),
                        end: Duration::from_secs(i as u64 * 8 + 2),
                    })
                    .collect(),
                Duration::from_secs(10),
            );
            let text = loaded.punctuate(&transcript, str::to_owned);
            eprintln!("numbers and domain: {text}");
            assert!(
                text.starts_with("版本 1.2.3，价格 3.14") && text.ends_with("网址 example.com。"),
                "{text}"
            );
        }
    }

    #[test]
    #[ignore = "needs sherpa-onnx models"]
    fn a_saved_punctuation_folder_that_is_a_recognition_model_is_skipped_not_loaded() {
        // A saved setting can name any folder. Loading this one as punctuation
        // would end the process, here the test run.
        let mut settings = Settings {
            punct_model: Some(models().join(SENSE_VOICE)),
            ..Settings::default()
        };
        let id = add(&mut settings, ZIPFORMER);
        let loaded = build(&id, &settings, &CloudRuntime::owned(1).unwrap()).unwrap();
        assert!(loaded.punct.is_none());
        assert_eq!(loaded.info.punctuation, "none");
    }
}
