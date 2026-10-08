//! The Transcripts library: what was said in files the user added, apart
//! from dictation History. A transcript links to its original file; the
//! subtitles and documents it saves go next to that file.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use crate::{
    history::EngineLabel,
    json_store,
    notes::{Passage, Speaker},
    settings::OutputFormat,
    subtitles,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub id: String,
    /// The original's name without its extension.
    pub name: String,
    pub source: PathBuf,
    pub created_at: u64,
    pub duration_ms: u64,
    pub engine: EngineLabel,
    pub passages: Vec<Passage>,
    pub speakers: Vec<Speaker>,
    /// Files Viary saved next to the original, kept up to date with edits.
    pub saved: Vec<PathBuf>,
}

/// A transcript in the library list, without its passages.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    id: String,
    name: String,
    source: PathBuf,
    source_exists: bool,
    created_at: u64,
    duration_ms: u64,
    engine: EngineLabel,
    speakers: Vec<Speaker>,
    /// File names, as the list shows them.
    saved: Vec<String>,
}

/// A transcript with its passages, for the editor.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    #[serde(flatten)]
    summary: Summary,
    passages: Vec<Passage>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    id: String,
    name: String,
    count: usize,
    at_ms: u64,
    before: String,
    #[serde(rename = "match")]
    found: String,
    after: String,
}

impl Transcript {
    fn summary(&self) -> Summary {
        Summary {
            id: self.id.clone(),
            name: self.name.clone(),
            source: self.source.clone(),
            source_exists: self.source.exists(),
            created_at: self.created_at,
            duration_ms: self.duration_ms,
            engine: self.engine.clone(),
            speakers: self.speakers.clone(),
            saved: self
                .saved
                .iter()
                .filter_map(|p| Some(p.file_name()?.to_string_lossy().into_owned()))
                .collect(),
        }
    }

    pub fn doc(&self) -> Doc {
        Doc {
            summary: self.summary(),
            passages: self.passages.clone(),
        }
    }

    /// Changes one passage's text. Its word timing no longer matches, so it
    /// is dropped; the passage keeps its own times.
    pub fn edit(&mut self, index: usize, text: &str) {
        if let Some(p) = self.passages.get_mut(index)
            && p.text != text
        {
            p.original.get_or_insert_with(|| p.text.clone());
            p.text = text.to_owned();
            p.words = None;
        }
    }

    /// Replaces every match of `find`, ignoring case. Returns how many.
    pub fn replace_all(&mut self, find: &str, replace: &str) -> usize {
        let mut total = 0;
        for i in 0..self.passages.len() {
            let (text, n) = replace_ignoring_case(&self.passages[i].text, find, replace);
            if n > 0 {
                total += n;
                self.edit(i, &text);
            }
        }
        total
    }
}

/// Where `find` occurs in `text`, ignoring case: byte ranges.
fn matches(text: &str, find: &str) -> Vec<(usize, usize)> {
    let needle: Vec<char> = find.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // Compare lowercased characters from here, as far as the needle.
        let mut lowered = Vec::new();
        let mut j = i;
        while j < chars.len() && lowered.len() < needle.len() {
            lowered.extend(chars[j].1.to_lowercase());
            j += 1;
        }
        if lowered == needle {
            let end = chars.get(j).map_or(text.len(), |&(at, _)| at);
            found.push((chars[i].0, end));
            i = j;
        } else {
            i += 1;
        }
    }
    found
}

fn replace_ignoring_case(text: &str, find: &str, replace: &str) -> (String, usize) {
    let found = matches(text, find);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for &(start, end) in &found {
        out.push_str(&text[at..start]);
        out.push_str(replace);
        at = end;
    }
    out.push_str(&text[at..]);
    (out, found.len())
}

/// Up to `n` characters on each side of a match, for a search snippet.
fn around(text: &str, start: usize, end: usize, n: usize) -> (String, String) {
    let before: String = text[..start]
        .chars()
        .rev()
        .take(n)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let after: String = text[end..].chars().take(n).collect();
    let lead = if before.len() < start { "…" } else { "" };
    let tail = if start + (end - start) + after.len() < text.len() {
        "…"
    } else {
        ""
    };
    (format!("{lead}{before}"), format!("{after}{tail}"))
}

pub struct Transcripts {
    dir: PathBuf,
    items: Mutex<Vec<Transcript>>,
    /// Held while rewriting saved files, so an older rewrite never lands
    /// after a newer one.
    writing: Mutex<()>,
}

impl Transcripts {
    /// The library in `data/transcripts`. An unreadable file is set aside.
    pub fn open(data: &Path) -> Self {
        let dir = data.join("transcripts");
        let items = json_store::load(&dir.join("transcripts.json"), "transcripts");
        Self {
            dir,
            items: Mutex::new(items),
            writing: Mutex::new(()),
        }
    }

    fn items(&self) -> std::sync::MutexGuard<'_, Vec<Transcript>> {
        self.items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Summary> {
        self.items().iter().rev().map(Transcript::summary).collect()
    }

    /// Rewrites the files transcript `id` saved next to its original, as
    /// it is now, with `names()` read now: speaker names in subtitles.
    /// With `subtitles_only`, only .srt and .vtt, which names alone change.
    pub fn rewrite_saved(&self, id: &str, names: impl FnOnce() -> bool, subtitles_only: bool) {
        let _writing = self
            .writing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(t) = self.get(id) else {
            return;
        };
        rewrite_saved(&t, names(), |f| {
            !subtitles_only || matches!(f, OutputFormat::Srt | OutputFormat::Vtt)
        });
    }

    pub fn all(&self) -> Vec<Transcript> {
        self.items().clone()
    }

    pub fn get(&self, id: &str) -> Option<Transcript> {
        self.items().iter().find(|t| t.id == id).cloned()
    }

    pub fn add(&self, transcript: Transcript) {
        let mut items = self.items();
        items.push(transcript);
        self.persist(&items);
    }

    pub fn update<T>(
        &self,
        id: &str,
        change: impl FnOnce(&mut Transcript) -> T,
    ) -> Option<(T, Transcript)> {
        let mut items = self.items();
        let item = items.iter_mut().find(|t| t.id == id)?;
        let out = change(item);
        let changed = item.clone();
        self.persist(&items);
        Some((out, changed))
    }

    /// Takes a transcript out of the library. The files it saved next to
    /// the original stay: they are the user's now.
    pub fn delete(&self, id: &str) {
        let mut items = self.items();
        items.retain(|t| t.id != id);
        self.persist(&items);
    }

    /// Transcripts mentioning `query`, by name or text, with the first match.
    pub fn search(&self, query: &str) -> Vec<Hit> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        self.items()
            .iter()
            .rev()
            .filter_map(|t| {
                let mut count = 0;
                let mut first = None;
                for p in &t.passages {
                    let found = matches(&p.text, query);
                    if let (None, Some(&(start, end))) = (&first, found.first()) {
                        first = Some((p, start, end));
                    }
                    count += found.len();
                }
                let named = !matches(&t.name, query).is_empty();
                if count == 0 && !named {
                    return None;
                }
                let (at_ms, before, found, after) = match first {
                    Some((p, start, end)) => {
                        let (before, after) = around(&p.text, start, end, 28);
                        (p.start_ms, before, p.text[start..end].to_owned(), after)
                    }
                    None => (0, String::new(), String::new(), String::new()),
                };
                Some(Hit {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    count,
                    at_ms,
                    before,
                    found,
                    after,
                })
            })
            .collect()
    }

    fn persist(&self, items: &[Transcript]) {
        json_store::save(&self.dir.join("transcripts.json"), items, "transcripts");
    }
}

fn format_of(path: &Path) -> Option<OutputFormat> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "srt" => Some(OutputFormat::Srt),
        "vtt" => Some(OutputFormat::Vtt),
        "txt" => Some(OutputFormat::Txt),
        "md" => Some(OutputFormat::Md),
        _ => None,
    }
}

/// Where a format goes next to the original: `name.srt`, unless a file
/// Viary did not write is there, then `name (Viary).srt`. None when both
/// are taken by the user's files.
fn target(t: &Transcript, format: OutputFormat) -> Option<PathBuf> {
    let ext = format.extension();
    let plain = t.source.with_extension(ext);
    let own = t.source.with_file_name(format!("{} (Viary).{ext}", t.name));
    [plain, own]
        .into_iter()
        .find(|path| !path.exists() || t.saved.contains(path))
}

/// Saves `formats` next to the original. Returns the files written; a
/// file that cannot be written is left out and logged.
pub fn save_next_to_source(t: &Transcript, formats: &[OutputFormat], names: bool) -> Vec<PathBuf> {
    formats
        .iter()
        .filter_map(|&format| {
            let Some(path) = target(t, format) else {
                tracing::warn!(source = %t.source.display(), "your own .{} files are in the way; not saving one", format.extension());
                return None;
            };
            let text = subtitles::render(format, &t.name, &t.passages, &t.speakers, names);
            match fs::write(&path, text) {
                Ok(()) => Some(path),
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), "cannot save next to the original");
                    None
                }
            }
        })
        .collect()
}

/// Rewrites the `wanted` formats of the files Viary saved.
fn rewrite_saved(t: &Transcript, names: bool, wanted: impl Fn(OutputFormat) -> bool) {
    for path in &t.saved {
        let Some(format) = format_of(path).filter(|&f| wanted(f)) else {
            continue;
        };
        let text = subtitles::render(format, &t.name, &t.passages, &t.speakers, names);
        if let Err(error) = fs::write(path, text) {
            tracing::warn!(%error, path = %path.display(), "cannot update a saved transcript file");
        }
    }
}

/// Writes the transcript as `format` to `path`, chosen in a save dialog.
pub fn export(
    t: &Transcript,
    format: OutputFormat,
    path: &Path,
    names: bool,
) -> std::io::Result<()> {
    fs::write(
        path,
        subtitles::render(format, &t.name, &t.passages, &t.speakers, names),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_ignoring_case_and_counts() {
        assert_eq!(
            replace_ignoring_case(
                "Modify beam search, then BEAM SEARCH again",
                "beam search",
                "beam-search"
            ),
            ("Modify beam-search, then beam-search again".to_owned(), 2)
        );
        assert_eq!(
            replace_ignoring_case("通义千问和通易千问", "通易千问", "通义千问"),
            ("通义千问和通义千问".to_owned(), 1)
        );
        assert_eq!(
            replace_ignoring_case("nothing", "", "x"),
            ("nothing".to_owned(), 0)
        );
    }

    #[test]
    fn edits_keep_the_original_and_drop_word_timing() {
        let mut t = Transcript {
            id: "1".into(),
            name: "call".into(),
            source: "/tmp/call.m4a".into(),
            created_at: 0,
            duration_ms: 1_000,
            engine: EngineLabel {
                name: "m".into(),
                kind: "k".into(),
                on_device: true,
            },
            passages: vec![Passage {
                start_ms: 0,
                end_ms: 1_000,
                text: "modify beam search".into(),
                speaker: None,
                words: Some(Vec::new()),
                original: None,
            }],
            speakers: Vec::new(),
            saved: Vec::new(),
        };
        assert_eq!(t.replace_all("modify", "modified"), 1);
        t.edit(0, "modified beam search on");
        let p = &t.passages[0];
        assert_eq!(p.original.as_deref(), Some("modify beam search"));
        assert_eq!(p.text, "modified beam search on");
        assert!(p.words.is_none());
    }

    #[test]
    fn saves_beside_files_it_did_not_write() {
        let dir = std::env::temp_dir().join(format!("viary-transcripts-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("call.srt"), "the user's own").unwrap();
        let mut t = Transcript {
            id: "1".into(),
            name: "call".into(),
            source: dir.join("call.m4a"),
            created_at: 0,
            duration_ms: 1_000,
            engine: EngineLabel {
                name: "m".into(),
                kind: "k".into(),
                on_device: true,
            },
            passages: Vec::new(),
            speakers: Vec::new(),
            saved: Vec::new(),
        };
        let saved = save_next_to_source(&t, &[OutputFormat::Srt, OutputFormat::Vtt], true);
        assert_eq!(
            saved,
            vec![dir.join("call (Viary).srt"), dir.join("call.vtt")]
        );
        assert_eq!(
            fs::read_to_string(dir.join("call.srt")).unwrap(),
            "the user's own"
        );
        // Once Viary owns a file, it updates that one.
        t.saved = saved;
        assert_eq!(target(&t, OutputFormat::Vtt), Some(dir.join("call.vtt")));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn snippets_mark_cut_text() {
        let text = "Is that with the modify beam search on, or greedy?";
        let (start, end) = matches(text, "BEAM search")[0];
        assert_eq!(&text[start..end], "beam search");
        let (before, after) = around(text, start, end, 7);
        assert_eq!(before, "…modify ");
        assert_eq!(after, " on, or…");
    }
}
