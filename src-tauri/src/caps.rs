//! What recognition reports beyond passage text and times.
//!
//! speechkit 0.5 gives each segment its text, start and end, and nothing
//! about who spoke, when each word was said, or how sure it was. These
//! belong in speechkit: Viary does not work them out itself, and never
//! makes up word times by dividing a passage's span. Until speechkit has
//! them, every capability is off and the UI falls back to whole passages.
//!
//! For development, `VIARY_MOCK_SPEECHKIT=1` turns them on and replaces a
//! note's passages with a scripted fixture whose speakers, word times and
//! confidences were written by hand (fixtures/voice-note.json).

use serde::{Deserialize, Serialize};

use crate::notes::{Passage, Speaker, Word};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechCaps {
    pub speakers: bool,
    pub word_timings: bool,
    pub word_confidence: bool,
    /// The capabilities come from the dev fixture, not from speechkit.
    pub mock: bool,
}

fn mocked() -> bool {
    cfg!(debug_assertions) && std::env::var_os("VIARY_MOCK_SPEECHKIT").is_some_and(|v| v == "1")
}

/// What the current build can report. Release builds never mock.
pub fn current() -> SpeechCaps {
    if mocked() {
        return SpeechCaps {
            speakers: true,
            word_timings: true,
            word_confidence: true,
            mock: true,
        };
    }
    SpeechCaps::default()
}

#[derive(Deserialize)]
struct Fixture {
    speakers: Vec<String>,
    passages: Vec<FixturePassage>,
}

#[derive(Deserialize)]
struct FixturePassage {
    speaker: usize,
    /// Text, start ms, end ms, confidence.
    words: Vec<(String, u64, u64, f32)>,
}

/// The fixture's passages that end within `duration_ms`, and their
/// speakers. Only when mocking.
pub fn mock_passages(duration_ms: u64) -> Option<(Vec<Passage>, Vec<Speaker>)> {
    if !mocked() {
        return None;
    }
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/voice-note.json")).ok()?;
    let mut passages: Vec<Passage> = fixture
        .passages
        .into_iter()
        .filter_map(|p| {
            let words: Vec<Word> = p
                .words
                .into_iter()
                .map(|(text, start_ms, end_ms, confidence)| Word {
                    text,
                    start_ms,
                    end_ms,
                    confidence: Some(confidence),
                })
                .collect();
            let (start_ms, end_ms) = (words.first()?.start_ms, words.last()?.end_ms);
            (end_ms <= duration_ms).then(|| Passage {
                start_ms,
                end_ms,
                text: words
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                speaker: Some(p.speaker),
                words: Some(words),
                original: None,
            })
        })
        .collect();
    // Only the speakers heard in this much of the script, numbered anew.
    let mut heard: Vec<usize> = passages.iter().filter_map(|p| p.speaker).collect();
    heard.sort_unstable();
    heard.dedup();
    for passage in &mut passages {
        passage.speaker = passage
            .speaker
            .and_then(|s| heard.iter().position(|&h| h == s));
    }
    let speakers = heard
        .iter()
        .map(|&i| Speaker {
            name: fixture.speakers[i].clone(),
        })
        .collect();
    Some((passages, speakers))
}
