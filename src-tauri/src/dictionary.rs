//! The dictionary: names, jargon and spellings the engine should get right.
//!
//! Each engine takes the words as far as it can. Transducers favor them as
//! hotwords (decoding bias), Qwen3-ASR, FunASR-Nano and OpenAI's file mode
//! read them in a prompt, and every engine then gets the "When I say"
//! replacements after recognition, which also fix each word's spelling.

use std::path::Path;

use speechkit::sherpa::Hotword;

use crate::settings::{Boost, DictionaryEntry};

/// The most words: speechkit takes at most 256 hotwords, engine and session
/// combined.
pub const MAX_WORDS: usize = 256;
/// The longest word or phrase, as speechkit allows for a hotword.
const MAX_CHARS: usize = 64;
/// The boost of a Strong word; Normal ones take speechkit's 2.0.
const STRONG_BOOST: f32 = 3.5;
/// Prompts for Qwen3-ASR and FunASR-Nano hold at most 64 characters.
const MAX_MODEL_PROMPT: usize = 64;
/// OpenAI reads only the last 224 tokens of a prompt; the glossary stays
/// under that, by a rough estimate, with room to spare.
const MAX_OPENAI_TOKENS: usize = 180;

/// How an engine uses the dictionary, as the Dictionary page explains it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    Hotwords,
    Prompt,
    Replacements,
}

impl Use {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hotwords => "hotwords",
            Self::Prompt => "prompt",
            Self::Replacements => "replacements",
        }
    }
}

fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn check_phrase(phrase: &str, what: &str) -> Result<(), String> {
    if phrase.chars().count() > MAX_CHARS {
        return Err(format!("{what} can be at most {MAX_CHARS} characters"));
    }
    if phrase.chars().any(char::is_control) {
        return Err(format!("{what} cannot hold control characters"));
    }
    Ok(())
}

/// `entry` tidied for saving: spaces collapsed, empty and repeated phrases
/// and apps dropped.
///
/// # Errors
///
/// A message for the user when the word is empty or too long.
pub fn tidy(entry: DictionaryEntry) -> Result<DictionaryEntry, String> {
    let word = clean(&entry.word);
    if word.is_empty() {
        return Err("enter the word to write".into());
    }
    check_phrase(&word, "a word")?;
    let mut sounds_like: Vec<String> = Vec::new();
    for phrase in entry.sounds_like.iter().map(|p| clean(p)) {
        check_phrase(&phrase, "a “When I say” phrase")?;
        if !phrase.is_empty()
            && phrase != word
            && !sounds_like.iter().any(|p| p.eq_ignore_ascii_case(&phrase))
        {
            sounds_like.push(phrase);
        }
    }
    let mut apps: Vec<String> = Vec::new();
    for app in entry.apps.iter().map(|a| clean(a)) {
        if !app.is_empty() && !apps.iter().any(|a| a.eq_ignore_ascii_case(&app)) {
            apps.push(app);
        }
    }
    Ok(DictionaryEntry {
        word,
        sounds_like,
        apps,
        ..entry
    })
}

/// What an engine load depends on: a change to anything else, such as a
/// "When I say" phrase, needs no reload.
pub fn load_inputs(entries: &[DictionaryEntry]) -> Vec<(String, Boost, bool)> {
    entries
        .iter()
        .map(|e| (e.word.clone(), e.boost, e.apps.is_empty()))
        .collect()
}

// ---------------------------------------------------------------------------
// Hotwords and prompts

/// Whether sherpa-onnx's hotword syntax can hold `phrase`.
fn hotword_syntax_ok(phrase: &str) -> bool {
    !phrase.is_empty()
        && phrase.chars().count() <= MAX_CHARS
        && !phrase
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | ':' | ',' | '#' | '@'))
}

/// What a transducer's `tokens.txt` says about hotwords.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tokens {
    /// It holds CJK characters, which hotwords can be spelled with alone.
    pub cjk: bool,
    /// Its Latin pieces are all upper case, as for the English streaming
    /// Zipformer, so hotwords must be upper case too.
    pub upper_case: bool,
}

impl Tokens {
    pub fn read(dir: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(dir.join("tokens.txt")) else {
            return Self::default();
        };
        let pieces = text
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .filter(|piece| !piece.starts_with('<'));
        let (mut cjk, mut upper, mut lower) = (false, false, false);
        for c in pieces.flat_map(str::chars) {
            cjk |= is_cjk(c);
            upper |= c.is_ascii_uppercase();
            lower |= c.is_ascii_lowercase();
        }
        Self {
            cjk,
            upper_case: upper && !lower,
        }
    }

    /// Whether hotwords can match what the model decodes. Without
    /// `bpe.vocab` speechkit spells hotwords character by character, which
    /// suits Chinese models; for a Latin BPE model the phrases pass its
    /// checks but never match, and only slow decoding down.
    pub fn take_hotwords(self, has_bpe_vocab: bool) -> bool {
        has_bpe_vocab || self.cjk
    }
}

/// A transducer's view of the words: every word as it should be spelled for
/// the model, split into the ones for every app, which load with the engine
/// with their boost, and the app-only ones, which sessions add as hints.
pub struct TransducerWords {
    pub every_app: Vec<Hotword>,
    pub app_only: Vec<String>,
}

pub fn transducer_words(entries: &[DictionaryEntry], upper: bool) -> TransducerWords {
    let spell = |word: &str| {
        if upper {
            word.to_uppercase()
        } else {
            word.to_owned()
        }
    };
    let mut words = TransducerWords {
        every_app: Vec::new(),
        app_only: Vec::new(),
    };
    for entry in entries.iter().filter(|e| hotword_syntax_ok(&e.word)) {
        let word = spell(&entry.word);
        if entry.apps.is_empty() {
            let hotword = Hotword::new(word);
            words.every_app.push(match entry.boost {
                Boost::Strong => hotword.with_boost(STRONG_BOOST),
                Boost::Normal => hotword,
            });
        } else if !words.app_only.contains(&word) {
            words.app_only.push(word);
        }
    }
    words
}

/// The session hints for a dictation in `app`: the app-only words that
/// apply there, spelled as the engine accepted them at load.
pub fn session_hints(
    entries: &[DictionaryEntry],
    app: &str,
    accepted: &[String],
    upper: bool,
) -> Vec<String> {
    entries
        .iter()
        .filter(|e| !e.apps.is_empty() && e.applies_in(app))
        .map(|e| {
            if upper {
                e.word.to_uppercase()
            } else {
                e.word.clone()
            }
        })
        .filter(|word| accepted.contains(word))
        .collect()
}

/// Words for every app, Strong ones first.
fn prompt_words(entries: &[DictionaryEntry]) -> impl Iterator<Item = &str> {
    let every_app = entries.iter().filter(|e| e.apps.is_empty());
    every_app
        .clone()
        .filter(|e| e.boost == Boost::Strong)
        .chain(every_app.filter(|e| e.boost == Boost::Normal))
        .map(|e| e.word.as_str())
}

/// The prompt words for Qwen3-ASR or FunASR-Nano: as many as fit in the
/// 64 characters speechkit allows, joined with commas. FunASR-Nano also
/// takes no `;`, `；` or `，`.
pub fn model_prompt(entries: &[DictionaryEntry], funasr: bool) -> Vec<Hotword> {
    let mut words = Vec::new();
    let mut total = 0;
    for word in prompt_words(entries) {
        let rejected = word.contains(',') || (funasr && word.contains([';', '；', '，']));
        let len = word.chars().count() + usize::from(!words.is_empty());
        if rejected || total + len > MAX_MODEL_PROMPT {
            continue;
        }
        total += len;
        words.push(Hotword::new(word));
    }
    words
}

/// A generous estimate of the tokens `text` takes: a CJK character may be
/// two, and other text about one per three bytes.
fn estimated_tokens(text: &str) -> usize {
    let cjk = text.chars().filter(|c| is_cjk(*c)).count();
    let other: usize = text.chars().filter(|c| !is_cjk(*c)).map(char::len_utf8).sum();
    cjk * 2 + other.div_ceil(3)
}

/// OpenAI's prompt: a short glossary, which the model treats as preceding
/// text and so spells its words as given. The words that fit are chosen
/// Strong first, and listed Strong last: if OpenAI cuts the prompt, it
/// cuts from the start.
pub fn openai_prompt(entries: &[DictionaryEntry]) -> Option<String> {
    const INTRO: &str = "Glossary: ";
    let mut total = estimated_tokens(INTRO) + 1;
    let mut words: Vec<&str> = Vec::new();
    for word in prompt_words(entries) {
        // The word and its ", ".
        let len = estimated_tokens(word) + 1;
        if total + len > MAX_OPENAI_TOKENS {
            continue;
        }
        total += len;
        words.push(word);
    }
    if words.is_empty() {
        return None;
    }
    words.reverse();
    Some(format!("{INTRO}{}.", words.join(", ")))
}

/// Words that apply in `app`, for the polish model to keep as spelled.
pub fn spellings<'a>(entries: &'a [DictionaryEntry], app: &str) -> Vec<&'a str> {
    entries
        .iter()
        .filter(|e| e.applies_in(app))
        .map(|e| e.word.as_str())
        .collect()
}

// ---------------------------------------------------------------------------
// Replacements

/// CJK text has no spaces, so a CJK phrase matches anywhere; a Latin one
/// only as whole words.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() && !is_cjk(c)
}

pub(crate) fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

fn same_letter(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Whether `pattern` matches `text` at `at`, ignoring case, as whole words.
fn matches_at(text: &[char], at: usize, pattern: &[char]) -> bool {
    let end = at + pattern.len();
    if end > text.len()
        || !text[at..end]
            .iter()
            .zip(pattern)
            .all(|(a, b)| same_letter(*a, *b))
    {
        return false;
    }
    let starts_word = pattern.first().is_some_and(|c| is_word_char(*c));
    let ends_word = pattern.last().is_some_and(|c| is_word_char(*c));
    let before_ok = !starts_word || at == 0 || !is_word_char(text[at - 1]);
    let after_ok = !ends_word || end == text.len() || !is_word_char(text[end]);
    before_ok && after_ok
}

/// Applies the words that apply in `app` to `text`: each "When I say"
/// phrase becomes its word, and each word gets its own spelling, ignoring
/// case ("SPEECHKIT" and "Speechkit" both become "speechkit").
pub fn apply(entries: &[DictionaryEntry], app: &str, text: &str) -> String {
    let mut rules: Vec<(Vec<char>, &str)> = entries
        .iter()
        .filter(|e| e.applies_in(app) && !e.word.is_empty())
        .flat_map(|e| {
            e.sounds_like
                .iter()
                .map(String::as_str)
                .chain([e.word.as_str()])
                .filter(|p| !p.is_empty())
                .map(|p| (p.chars().collect::<Vec<_>>(), e.word.as_str()))
        })
        .collect();
    if rules.is_empty() {
        return text.to_owned();
    }
    // Longer phrases first, so "cargo clippie" wins over "cargo".
    rules.sort_by_key(|rule| std::cmp::Reverse(rule.0.len()));
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match rules
            .iter()
            .find(|(pattern, _)| matches_at(&chars, i, pattern))
        {
            Some((pattern, word)) => {
                out.push_str(word);
                i += pattern.len();
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::WordKind;

    fn entry(word: &str, say: &[&str], apps: &[&str]) -> DictionaryEntry {
        DictionaryEntry {
            id: word.into(),
            word: word.into(),
            sounds_like: say.iter().map(|s| (*s).into()).collect(),
            kind: WordKind::Term,
            boost: Boost::Normal,
            apps: apps.iter().map(|s| (*s).into()).collect(),
        }
    }

    #[test]
    fn replaces_what_the_engine_hears_as_whole_words() {
        let words = [
            entry("Mei Lin", &["may lin"], &[]),
            entry("cargo clippy", &["cargo clippie"], &[]),
        ];
        assert_eq!(
            apply(&words, "Mail", "Hi May Lin, run cargo clippie."),
            "Hi Mei Lin, run cargo clippy."
        );
        // Not inside other words.
        assert_eq!(apply(&words, "Mail", "dismay lingers"), "dismay lingers");
    }

    #[test]
    fn fixes_the_spelling_of_the_word_itself() {
        let words = [entry("speechkit", &[], &[])];
        assert_eq!(apply(&words, "", "SPEECHKIT IS FAST"), "speechkit IS FAST");
    }

    #[test]
    fn cjk_phrases_match_without_spaces() {
        let words = [entry("通义千问", &["通义前文"], &[])];
        assert_eq!(
            apply(&words, "", "我用通义前文写代码"),
            "我用通义千问写代码"
        );
    }

    #[test]
    fn app_words_apply_only_in_their_apps() {
        let words = [entry("sherpa-onnx", &["sherpa onyx"], &["Slack"])];
        assert_eq!(apply(&words, "slack", "try sherpa onyx"), "try sherpa-onnx");
        assert_eq!(apply(&words, "Mail", "try sherpa onyx"), "try sherpa onyx");
    }

    #[test]
    fn tidy_drops_empties_and_repeats() {
        let tidied = tidy(entry(
            " Mei  Lin ",
            &["may lin", " ", "May Lin", "Mei Lin"],
            &["Mail", "mail"],
        ))
        .unwrap();
        assert_eq!(tidied.word, "Mei Lin");
        assert_eq!(tidied.sounds_like, ["may lin"]);
        assert_eq!(tidied.apps, ["Mail"]);
        assert!(tidy(entry("  ", &[], &[])).is_err());
        assert!(tidy(entry(&"x".repeat(65), &[], &[])).is_err());
    }

    #[test]
    fn transducer_words_split_by_app_and_skip_reserved_syntax() {
        let mut strong = entry("speechkit", &[], &[]);
        strong.boost = Boost::Strong;
        let words = [
            strong,
            entry("a/b", &[], &[]),
            entry("Zipformer", &[], &["Slack"]),
        ];
        let split = transducer_words(&words, true);
        assert_eq!(split.every_app.len(), 1);
        assert_eq!(split.every_app[0].text, "SPEECHKIT");
        assert_eq!(split.every_app[0].boost, Some(STRONG_BOOST));
        assert_eq!(split.app_only, ["ZIPFORMER"]);
        let accepted = split.app_only.clone();
        assert_eq!(
            session_hints(&words, "Slack", &accepted, true),
            ["ZIPFORMER"]
        );
        assert!(session_hints(&words, "Mail", &accepted, true).is_empty());
    }

    #[test]
    fn tokens_say_whether_hotwords_can_match() {
        let dir = std::env::temp_dir().join(format!("viary-tokens-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tokens.txt"), "<blk> 0\n\u{2581}THE 1\nS 2\n").unwrap();
        let latin = Tokens::read(&dir);
        assert_eq!(
            latin,
            Tokens {
                cjk: false,
                upper_case: true
            }
        );
        assert!(
            !latin.take_hotwords(false),
            "a Latin BPE model needs bpe.vocab"
        );
        assert!(latin.take_hotwords(true));
        std::fs::write(dir.join("tokens.txt"), "<blk> 0\n语 1\nab 2\n").unwrap();
        let chinese = Tokens::read(&dir);
        assert!(chinese.cjk && !chinese.upper_case);
        assert!(chinese.take_hotwords(false));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn model_prompts_fit_in_64_characters() {
        let words: Vec<_> = (0..20)
            .map(|i| entry(&format!("word{i:02}"), &[], &[]))
            .collect();
        let prompt = model_prompt(&words, false);
        let joined: Vec<_> = prompt.iter().map(|h| h.text.as_str()).collect();
        assert!(joined.join(",").chars().count() <= MAX_MODEL_PROMPT);
        assert_eq!(prompt.len(), 9, "6 + 8 × 7 = 62 characters");
        assert!(model_prompt(&[entry("a；b", &[], &[])], true).is_empty());
    }

    #[test]
    fn openai_prompt_lists_words_for_every_app() {
        let words = [
            entry("Mei Lin", &[], &[]),
            entry("Zipformer", &[], &["Slack"]),
        ];
        assert_eq!(openai_prompt(&words).as_deref(), Some("Glossary: Mei Lin."));
        assert!(openai_prompt(&[]).is_none());
    }

    #[test]
    fn openai_prompt_keeps_strong_words_and_lists_them_last() {
        let mut words: Vec<_> = (0..200)
            .map(|i| entry(&format!("通义千问{i:03}"), &[], &[]))
            .collect();
        words[150].boost = Boost::Strong;
        let prompt = openai_prompt(&words).unwrap();
        assert!(estimated_tokens(&prompt) <= MAX_OPENAI_TOKENS);
        assert!(prompt.ends_with("通义千问150."), "{prompt}");
    }
}
