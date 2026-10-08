//! Subtitles and documents from a transcript.
//!
//! With word timing, cues follow the usual reading limits: up to 42
//! characters on each of two lines, shown for 1 to 7 seconds. Without it,
//! which is every engine in speechkit 0.5, each passage is one cue with the
//! passage's own times: splitting a passage would mean guessing when its
//! words were said.

use serde::Serialize;

use crate::{
    dictionary::is_cjk,
    notes::{Passage, Speaker, Word, clock},
    settings::OutputFormat,
};

/// Characters per line, counting a CJK character as two.
const LINE: usize = 42;
const MIN_MS: u64 = 1_000;
const MAX_MS: u64 = 7_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cue {
    pub n: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub speaker: Option<usize>,
    pub warning: Option<String>,
}

fn width(text: &str) -> usize {
    text.chars().map(|c| if is_cjk(c) { 2 } else { 1 }).sum()
}

/// `text` fit for a cue: its lines joined into one, since a blank line
/// ends a cue, as words are joined (no space beside CJK); spacing within a
/// line stays as written. Without the `-->` that marks cue times.
fn cue_text(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    join(&lines).replace("-->", "→")
}

/// Joins words as they are written: a space between Latin words, none
/// around CJK characters or before punctuation.
fn join(words: &[&str]) -> String {
    let mut out = String::new();
    for word in words {
        let space = out.chars().next_back().is_some_and(|c| !is_cjk(c))
            && word
                .chars()
                .next()
                .is_some_and(|c| !is_cjk(c) && !".,!?;:".contains(c));
        if space {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Wraps `text` into lines of at most `LINE`, breaking at spaces, or
/// anywhere in CJK text.
fn wrap(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        if !line.is_empty() && width(&line) + 1 + width(word) > LINE {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        for c in word.chars() {
            if is_cjk(c) && width(&line) + 2 > LINE {
                lines.push(std::mem::take(&mut line));
            }
            line.push(c);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines.join("\n")
}

/// `text` on one line, or on two of about the same width, each within
/// `LINE`; None when it does not fit in two.
fn two_lines(text: &str) -> Option<String> {
    if width(text) <= LINE {
        return Some(text.to_owned());
    }
    // Of the breaks that fit, a space or between CJK characters, the one
    // nearest the middle.
    let total = width(text);
    let mut best: Option<(usize, usize)> = None;
    let mut seen: usize = 0;
    for (i, (at, c)) in text.char_indices().enumerate() {
        if c == ' ' || (is_cjk(c) && i > 0) {
            let first = seen;
            let second = total - seen - usize::from(c == ' ');
            if first <= LINE && second <= LINE {
                let uneven = first.abs_diff(second);
                if best.is_none_or(|(_, d)| uneven < d) {
                    best = Some((at, uneven));
                }
            }
        }
        seen += if is_cjk(c) { 2 } else { 1 };
    }
    let (at, _) = best?;
    let (first, second) = text.split_at(at);
    Some(format!("{}\n{}", first.trim_end(), second.trim_start()))
}

/// A cue's text on two balanced lines, or wrapped if it is longer.
fn balance(text: &str) -> String {
    two_lines(text).unwrap_or_else(|| wrap(text))
}

/// Word groups of one passage, each within the reading limits.
fn groups<'a>(words: &'a [Word], prefix: &str) -> Vec<&'a [Word]> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..words.len() {
        if i > start {
            let texts: Vec<&str> = words[start..=i].iter().map(|w| w.text.as_str()).collect();
            let lead = if start == 0 { prefix } else { "" };
            let too_wide = two_lines(&format!("{lead}{}", join(&texts))).is_none();
            let too_long = words[i].end_ms.saturating_sub(words[start].start_ms) > MAX_MS;
            // A sentence that ended makes a natural break, once the cue
            // has been on screen long enough.
            let previous = &words[i - 1];
            let sentence = previous.text.ends_with(['.', '!', '?', '。', '！', '？'])
                && previous.end_ms.saturating_sub(words[start].start_ms) >= MIN_MS;
            if too_wide || too_long || sentence {
                out.push(&words[start..i]);
                start = i;
            }
        }
    }
    if start < words.len() {
        out.push(&words[start..]);
    }
    out
}

/// The cues for `passages`. With `names`, a cue that starts a speaker's
/// turn begins with their name.
pub fn cues(passages: &[Passage], speakers: &[Speaker], names: bool) -> Vec<Cue> {
    let mut cues: Vec<Cue> = Vec::new();
    let mut last_speaker = None;
    for p in passages {
        let name = p
            .speaker
            .and_then(|s| speakers.get(s))
            .filter(|_| names && p.speaker != last_speaker)
            .map(|s| format!("{}: ", s.name))
            .unwrap_or_default();
        last_speaker = p.speaker.or(last_speaker);
        match &p.words {
            Some(words) if !words.is_empty() => {
                for (i, group) in groups(words, &name).into_iter().enumerate() {
                    let texts: Vec<&str> = group.iter().map(|w| w.text.as_str()).collect();
                    let prefix = if i == 0 { name.as_str() } else { "" };
                    cues.push(Cue {
                        n: 0,
                        start_ms: group[0].start_ms,
                        end_ms: group[group.len() - 1].end_ms,
                        text: balance(&cue_text(&format!("{prefix}{}", join(&texts)))),
                        speaker: p.speaker,
                        warning: None,
                    });
                }
            }
            _ => cues.push(Cue {
                n: 0,
                start_ms: p.start_ms,
                end_ms: p.end_ms,
                text: wrap(&cue_text(&format!("{name}{}", p.text))),
                speaker: p.speaker,
                warning: None,
            }),
        }
    }
    // Number them, give short cues time to be read, and note overlaps
    // between speakers, which stay as separate cues.
    for i in 0..cues.len() {
        cues[i].n = i + 1;
        let next = cues.get(i + 1).map(|c| (c.start_ms, c.speaker));
        let cue = &mut cues[i];
        if cue.end_ms.saturating_sub(cue.start_ms) < MIN_MS {
            let room = next.map_or(u64::MAX, |(start, _)| start.max(cue.end_ms));
            cue.end_ms = (cue.start_ms + MIN_MS).min(room);
        }
        if i > 0 {
            let (before, after) = cues.split_at_mut(i);
            let previous = &before[i - 1];
            let cue = &mut after[0];
            if previous.end_ms > cue.start_ms && previous.speaker != cue.speaker {
                let who = previous
                    .speaker
                    .and_then(|s| speakers.get(s))
                    .map_or("the previous speaker", |s| s.name.as_str());
                let by = previous.end_ms - cue.start_ms;
                cue.warning = Some(format!(
                    "Overlaps {who} by {}.{} s · kept as two cues",
                    by / 1000,
                    (by % 1000) / 100
                ));
            }
        }
    }
    cues
}

fn cue_time(ms: u64, separator: char) -> String {
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

/// The transcript as `format`. `title` heads the Markdown document.
pub fn render(
    format: OutputFormat,
    title: &str,
    passages: &[Passage],
    speakers: &[Speaker],
    names: bool,
) -> String {
    let who = |p: &Passage| {
        p.speaker
            .and_then(|s| speakers.get(s))
            .map(|s| format!(" {}:", s.name))
            .unwrap_or_default()
    };
    let mut out = String::new();
    match format {
        OutputFormat::Srt => {
            for cue in cues(passages, speakers, names) {
                out.push_str(&format!(
                    "{}\n{} --> {}\n{}\n\n",
                    cue.n,
                    cue_time(cue.start_ms, ','),
                    cue_time(cue.end_ms, ','),
                    cue.text
                ));
            }
        }
        OutputFormat::Vtt => {
            out.push_str("WEBVTT\n\n");
            for cue in cues(passages, speakers, names) {
                out.push_str(&format!(
                    "{} --> {}\n{}\n\n",
                    cue_time(cue.start_ms, '.'),
                    cue_time(cue.end_ms, '.'),
                    cue.text
                ));
            }
        }
        OutputFormat::Txt => {
            for p in passages {
                out.push_str(&format!("[{}]{} {}\n\n", clock(p.start_ms), who(p), p.text));
            }
        }
        OutputFormat::Md => {
            out.push_str(&format!("# {title}\n\n"));
            for p in passages {
                out.push_str(&format!(
                    "**[{}]{}** {}\n\n",
                    clock(p.start_ms),
                    who(p),
                    p.text
                ));
            }
        }
    }
    out.trim_end().to_owned() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start_ms: u64, end_ms: u64) -> Word {
        Word {
            text: text.into(),
            start_ms,
            end_ms,
            confidence: None,
        }
    }

    fn passage(
        start_ms: u64,
        end_ms: u64,
        text: &str,
        speaker: Option<usize>,
        words: Option<Vec<Word>>,
    ) -> Passage {
        Passage {
            start_ms,
            end_ms,
            text: text.into(),
            speaker,
            words,
            original: None,
        }
    }

    #[test]
    fn without_word_timing_each_passage_is_one_cue_with_its_own_times() {
        let long = "We finally have numbers for the Zipformer model on the new runners, and latency is down about a third across every test we ran.";
        let cues = cues(&[passage(14_020, 26_400, long, None, None)], &[], true);
        assert_eq!(cues.len(), 1);
        assert_eq!((cues[0].start_ms, cues[0].end_ms), (14_020, 26_400));
        assert!(cues[0].text.lines().all(|l| width(l) <= LINE));
        assert_eq!(cues[0].text.replace('\n', " "), long);
    }

    #[test]
    fn edited_line_breaks_and_arrows_do_not_break_cues() {
        let edited = "First paragraph.\n\nSecond --> third.";
        let srt = render(OutputFormat::Srt, "t", &[passage(0, 2_000, edited, None, None)], &[], true);
        assert_eq!(srt, "1\n00:00:00,000 --> 00:00:02,000\nFirst paragraph. Second → third.\n");
        assert_eq!(cue_text("我们周四发布。\n请准备。"), "我们周四发布。请准备。");
        // Spacing within a line is the writer's.
        assert_eq!(cue_text("我用 Rust 写代码 ... 好"), "我用 Rust 写代码 ... 好");
    }

    #[test]
    fn with_word_timing_cues_keep_to_two_lines_and_seven_seconds() {
        let words: Vec<Word> = (0..30)
            .map(|i| word("syllable", i * 400, i * 400 + 350))
            .collect();
        let text = vec!["syllable"; 30].join(" ");
        let speakers = [Speaker {
            name: "Mei Lin".into(),
        }];
        let cues = cues(
            &[passage(0, 12_000, &text, Some(0), Some(words))],
            &speakers,
            true,
        );
        assert!(cues.len() > 1);
        assert!(cues[0].text.starts_with("Mei Lin: "));
        assert!(!cues[1].text.starts_with("Mei Lin"));
        for cue in &cues {
            assert!(cue.text.lines().count() <= 2, "{cue:?}");
            assert!(cue.text.lines().all(|l| width(l) <= LINE), "{cue:?}");
            assert!(cue.end_ms - cue.start_ms <= MAX_MS);
        }
    }

    #[test]
    fn short_cues_get_a_second_and_overlaps_are_noted() {
        let speakers = [
            Speaker {
                name: "Mei Lin".into(),
            },
            Speaker {
                name: "Speaker 2".into(),
            },
        ];
        let cues = cues(
            &[
                passage(0, 2_000, "Beam search.", Some(0), None),
                passage(1_600, 1_900, "Right.", Some(1), None),
            ],
            &speakers,
            false,
        );
        assert_eq!(cues[1].end_ms, 2_600);
        assert_eq!(
            cues[1].warning.as_deref(),
            Some("Overlaps Mei Lin by 0.4 s · kept as two cues")
        );
    }

    #[test]
    fn srt_and_vtt_times() {
        let p = [passage(3_725_250, 3_727_000, "你好。", None, None)];
        let srt = render(OutputFormat::Srt, "t", &p, &[], false);
        assert_eq!(srt, "1\n01:02:05,250 --> 01:02:07,000\n你好。\n");
        let vtt = render(OutputFormat::Vtt, "t", &p, &[], false);
        assert!(vtt.starts_with("WEBVTT\n\n01:02:05.250 --> 01:02:07.000\n"));
    }

    #[test]
    fn cjk_words_join_without_spaces() {
        assert_eq!(join(&["我们", "周四", "发布", "。"]), "我们周四发布。");
        assert_eq!(join(&["Okay,", "quick", "sync."]), "Okay, quick sync.");
    }
}
