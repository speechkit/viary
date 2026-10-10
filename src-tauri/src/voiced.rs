//! Whether there was a voice under what the engine heard. A VAD can take
//! room noise for speech, and an offline model then writes fluent text
//! for it: a long silence came back as dozens of made-up words, inserted
//! into whatever had the focus. A segment is kept only if some of its
//! audio stands out from the recording's quiet parts, and is not near
//! digital silence.

use std::time::Duration;

use speechkit::{AudioBuffer, asr::Transcript};

/// The stretch the level is measured over.
const FRAME: Duration = Duration::from_millis(20);
/// Quieter than this is no voice at all, however short the recording.
const SILENT_DBFS: f32 = -55.0;
/// How far above the recording's quiet parts a voice is.
const ABOVE_QUIET_DB: f32 = 12.0;
/// Which share of the frames is the quiet part: the room between words.
const QUIET_SHARE: f64 = 0.1;
/// Below this, the quiet part may be speech too: a quick "yes" has no
/// room before or after it to learn the room from.
const LEARN_QUIET_FROM: Duration = Duration::from_secs(3);
/// How much voiced audio a segment needs.
const MIN_VOICED: Duration = Duration::from_millis(200);

/// `transcript` without the segments that have no voice under them.
/// `origin` is where `audio` starts on the segments' clock.
pub fn keep_heard(mut transcript: Transcript, audio: &AudioBuffer, origin: Duration) -> Transcript {
    let levels = levels(audio);
    let threshold = threshold(&levels, audio.duration());
    let frames_needed = (MIN_VOICED.as_millis() / FRAME.as_millis()) as usize;
    transcript.segments.retain(|segment| {
        if segment.text.trim().is_empty() {
            return true;
        }
        let at = |time: Duration| {
            let offset = time.saturating_sub(origin);
            usize::try_from(offset.as_millis() / FRAME.as_millis()).unwrap_or(usize::MAX)
        };
        let (start, end) = (at(segment.start), at(segment.end).max(at(segment.start) + 1));
        // Outside the recording kept: there is nothing to judge it by.
        let Some(under) = levels.get(start..end.min(levels.len())).filter(|l| !l.is_empty()) else {
            return true;
        };
        let voiced = under.iter().filter(|&&level| level >= threshold).count();
        let heard = voiced >= frames_needed.min(under.len());
        if !heard {
            tracing::info!(text = %segment.text, "dropped a segment with no voice under it");
        }
        heard
    });
    transcript
}

/// Each frame's level, in dBFS.
fn levels(audio: &AudioBuffer) -> Vec<f32> {
    let frame = usize::try_from(audio.sample_rate.frames_in(FRAME)).unwrap_or(usize::MAX).max(1);
    audio
        .samples
        .chunks(frame)
        .map(|chunk| {
            let power = chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32;
            10.0 * (power + 1e-12).log10()
        })
        .collect()
}

/// The level a voice reaches in this recording.
fn threshold(levels: &[f32], length: Duration) -> f32 {
    if length < LEARN_QUIET_FROM || levels.is_empty() {
        return SILENT_DBFS;
    }
    let mut sorted = levels.to_vec();
    sorted.sort_by(f32::total_cmp);
    let quiet = sorted[((sorted.len() - 1) as f64 * QUIET_SHARE) as usize];
    (quiet + ABOVE_QUIET_DB).max(SILENT_DBFS)
}

#[cfg(test)]
mod tests {
    use speechkit::{
        SampleRate,
        asr::{Segment, UtteranceId},
    };

    use super::*;

    const RATE: SampleRate = SampleRate::HZ_16000;

    /// Noise of about `dbfs`, steady, for `seconds`.
    fn noise(dbfs: f32, seconds: f32) -> Vec<f32> {
        let amplitude = 10f32.powf(dbfs / 20.0) * 3f32.sqrt();
        let mut state = 0x2545_f491_u32;
        (0..(seconds * 16_000.0) as usize)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32 * 2.0 - 1.0) * amplitude
            })
            .collect()
    }

    /// A voice-like tone of about `dbfs`, for `seconds`.
    fn voice(dbfs: f32, seconds: f32) -> Vec<f32> {
        let amplitude = 10f32.powf(dbfs / 20.0) * 2f32.sqrt();
        (0..(seconds * 16_000.0) as usize)
            .map(|i| (i as f32 * 220.0 * std::f32::consts::TAU / 16_000.0).sin() * amplitude)
            .collect()
    }

    fn segment(text: &str, start: f32, end: f32) -> Segment {
        Segment {
            utterance: UtteranceId(0),
            text: text.into(),
            start: Duration::from_secs_f32(start),
            end: Duration::from_secs_f32(end),
        }
    }

    fn heard(samples: Vec<f32>, segments: Vec<Segment>) -> String {
        let audio = AudioBuffer::new(RATE, samples);
        keep_heard(Transcript::new(segments, audio.duration()), &audio, Duration::ZERO).text()
    }

    #[test]
    fn a_long_silence_is_not_text() {
        let text = heard(noise(-70.0, 12.0), vec![segment("made up", 0.5, 11.5)]);
        assert_eq!(text, "");
    }

    #[test]
    fn steady_room_noise_is_not_text() {
        // Loud enough to pass any fixed floor; no louder than itself.
        let text = heard(noise(-35.0, 42.0), vec![segment("made up", 1.0, 20.0), segment("too", 21.0, 41.0)]);
        assert_eq!(text, "");
    }

    #[test]
    fn speech_over_room_noise_is_kept_and_the_noise_after_it_is_not() {
        let mut samples = noise(-45.0, 2.0);
        samples.extend(voice(-20.0, 2.0).iter().zip(noise(-45.0, 2.0)).map(|(v, n)| v + n));
        samples.extend(noise(-45.0, 8.0));
        let text = heard(samples, vec![segment("hello", 1.8, 4.2), segment("made up", 5.0, 11.0)]);
        assert_eq!(text, "hello");
    }

    #[test]
    fn a_quick_word_with_no_room_around_it_is_kept() {
        let text = heard(voice(-30.0, 0.6), vec![segment("yes", 0.0, 0.6)]);
        assert_eq!(text, "yes");
    }

    #[test]
    fn a_quick_silence_is_not_text() {
        let text = heard(noise(-75.0, 1.0), vec![segment("made up", 0.0, 1.0)]);
        assert_eq!(text, "");
    }

    #[test]
    fn segment_times_are_read_from_the_origin() {
        let mut samples = noise(-60.0, 4.0);
        samples.extend(voice(-20.0, 1.0));
        let audio = AudioBuffer::new(RATE, samples);
        let origin = Duration::from_secs(100);
        let transcript = Transcript::new(vec![segment("hello", 104.0, 105.0)], audio.duration());
        assert_eq!(keep_heard(transcript, &audio, origin).text(), "hello");
    }
}
