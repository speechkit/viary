//! Past dictations, with their recordings while the retention period lasts.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use speechkit::{AudioBuffer, audio::encode_wav};

/// The most entries kept; older ones drop off.
const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Pasted into the focused app.
    Inserted,
    /// Left on the clipboard: no text field had focus.
    Copied,
    /// Taken back with Undo.
    Undone,
    /// Recognition failed; the recording, if kept, can be re-transcribed.
    Failed,
    /// Re-transcribed from History after a failure; never inserted.
    Transcribed,
}

/// The engine that produced an entry, as History shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineLabel {
    pub name: String,
    pub kind: String,
    pub on_device: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
    /// The app dictated into.
    pub app: String,
    pub duration_ms: u64,
    /// What was inserted: the punctuated text, or the raw text after "Use raw".
    pub text: String,
    /// The recognizer's own text, before punctuation.
    pub raw: String,
    /// The punctuated text, when a punctuation model changed it.
    pub punctuated: Option<String>,
    pub engine: EngineLabel,
    pub status: Status,
    pub error: Option<String>,
    /// The WAV file name under the recordings folder.
    pub recording: Option<String>,
}

impl Entry {
    pub fn words(&self) -> usize {
        count_words(&self.text)
    }
}

/// Words for the pill and History: Latin words, plus one per CJK character.
pub fn count_words(text: &str) -> usize {
    let cjk = text
        .chars()
        .filter(|c| matches!(*c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF))
        .count();
    let latin = text
        .split(|c: char| {
            c.is_whitespace()
                || !(c.is_alphanumeric() || c == '\'' || c == '-')
                || c as u32 >= 0x3040
        })
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count();
    latin + cjk
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

pub struct History {
    file: PathBuf,
    recordings: PathBuf,
    entries: Mutex<Vec<Entry>>,
}

impl History {
    /// The history in `dir`. An unreadable file is set aside first, as
    /// settings are, so the next save does not overwrite the only copy.
    pub fn open(dir: &Path) -> Self {
        let file = dir.join("history.json");
        let entries = crate::json_store::load(&file, "history");
        Self {
            file,
            recordings: dir.join("recordings"),
            entries: Mutex::new(entries),
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Entry> {
        self.entries().iter().rev().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<Entry> {
        self.entries().iter().find(|e| e.id == id).cloned()
    }

    pub fn add(&self, entry: Entry) {
        let mut entries = self.entries();
        entries.push(entry);
        let excess = entries.len().saturating_sub(MAX_ENTRIES);
        for old in entries.drain(..excess) {
            self.remove_recording(&old);
        }
        self.persist(&entries);
    }

    pub fn update(&self, id: &str, change: impl FnOnce(&mut Entry)) -> Option<Entry> {
        let mut entries = self.entries();
        let entry = entries.iter_mut().find(|e| e.id == id)?;
        change(entry);
        let updated = entry.clone();
        self.persist(&entries);
        Some(updated)
    }

    pub fn delete(&self, id: &str) {
        let mut entries = self.entries();
        if let Some(index) = entries.iter().position(|e| e.id == id) {
            let entry = entries.remove(index);
            self.remove_recording(&entry);
            self.persist(&entries);
        }
    }

    /// Saves `audio` for entry `id` and returns the file name.
    pub fn save_recording(&self, id: &str, audio: &AudioBuffer) -> Option<String> {
        let name = format!("{id}.wav");
        let result = fs::create_dir_all(&self.recordings)
            .map_err(|e| e.to_string())
            .and_then(|()| encode_wav(audio).map_err(|e| e.to_string()))
            .and_then(|bytes| {
                fs::write(self.recordings.join(&name), bytes).map_err(|e| e.to_string())
            });
        match result {
            Ok(()) => Some(name),
            Err(error) => {
                tracing::warn!(%error, "cannot save the recording");
                None
            }
        }
    }

    pub fn recording_path(&self, entry: &Entry) -> Option<PathBuf> {
        entry
            .recording
            .as_ref()
            .map(|name| self.recordings.join(name))
            .filter(|path| path.is_file())
    }

    /// Deletes recordings older than `days` (all of them for 0), and
    /// returns whether any were.
    pub fn purge_recordings(&self, days: u32) -> bool {
        let cutoff = now_ms()
            .saturating_sub(Duration::from_secs(u64::from(days) * 86_400).as_millis() as u64);
        let mut entries = self.entries();
        let mut changed = false;
        for entry in entries.iter_mut() {
            if entry.recording.is_some() && (days == 0 || entry.created_at < cutoff) {
                self.remove_recording(entry);
                entry.recording = None;
                changed = true;
            }
        }
        if changed {
            self.persist(&entries);
        }
        changed
    }

    fn remove_recording(&self, entry: &Entry) {
        if let Some(name) = &entry.recording {
            let _ = fs::remove_file(self.recordings.join(name));
        }
    }

    fn persist(&self, entries: &[Entry]) {
        crate::json_store::save(&self.file, entries, "history");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_latin_and_cjk_words() {
        assert_eq!(count_words("Let's move the demo to Friday."), 6);
        assert_eq!(count_words("我用Rust写代码"), 6);
        assert_eq!(count_words("  "), 0);
    }

    #[test]
    fn unreadable_history_is_backed_up_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("viary-history-bad-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let old = r#"[{"id":"1","status":"polished"}]"#;
        fs::write(dir.join("history.json"), old).unwrap();
        let history = History::open(&dir);
        assert!(history.list().is_empty());
        history.delete("nothing");
        history.purge_recordings(0);
        assert_eq!(fs::read_to_string(dir.join("history.json.bak")).unwrap(), old);
        let _ = fs::remove_dir_all(dir);
    }
}
