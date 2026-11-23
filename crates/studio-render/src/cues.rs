//! Cue times: when each marked word of a narration sentence is spoken.
//!
//! Kokoro's model returns audio only, so word starts are estimated. A sentence's measured
//! length is spread over its phonemes, with a short pause after punctuation, and each
//! pause is then snapped to the silence really there in the audio.
//!
//! The weights and constants were fitted on the model's own phoneme durations (Standard
//! model, US and UK voices, speeds 0.8 to 1.2). On sentences left out of the fitting, 98%
//! of word starts land within 0.25 s.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::kokoro::{EDGE, SAMPLE_RATE};

/// Bump when the estimate changes: stored word times are then worked out again.
pub(crate) const VERSION: u32 = 1;

/// Pause after punctuation before the audio is looked at.
const PAUSE: f64 = 0.1;
/// Quieter than this (RMS of a window) is silence.
const SILENCE_RMS: f32 = 0.004;
/// The window the RMS is measured over: 10 ms.
const WINDOW: usize = SAMPLE_RATE as usize / 100;
/// The fewest windows in a row that make a pause: 80 ms.
const SILENT_WINDOWS: usize = 8;
/// A pause snaps to a silence no further from its estimate than this.
const SNAP_REACH: f64 = 0.6;

/// What separates words in a phoneme string.
const SEPARATORS: &str = " ,;:.!?—…\"“”()";
/// Punctuation the narrator pauses at.
const PAUSES: &str = ",;:.!?";
/// Marks that take no time of their own.
const SILENT: &str = "ˈˌː";

/// A cue as a scene sees it in `NARRATION.cues`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CueTime {
    pub id: String,
    /// The sentence it is in (1-based, as in `this.narrate(n)`).
    pub line: usize,
    /// Seconds after that sentence starts.
    pub at: f64,
}

/// When a word of a sentence starts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Word {
    /// Byte offset in the sentence.
    pub at: usize,
    /// Seconds after the sentence starts.
    pub start: f64,
}

/// How long a phoneme lasts relative to a plain consonant.
fn weight(c: char, stressed: bool) -> f64 {
    let vowel = match c {
        ' ' => return 0.8,
        _ if SEPARATORS.contains(c) || SILENT.contains(c) => return 0.0,
        _ if "AIOQWYaiuɑɔɜ".contains(c) => 1.7,
        _ if "əᵻ".contains(c) => 0.4,
        _ if "æɒɛɪʊʌe".contains(c) => 1.3,
        _ if "szʃʒʧʤfθ".contains(c) => return 1.55,
        _ if "nmŋlɹw".contains(c) => return 1.4,
        _ => return 1.0,
    };
    if stressed { vowel + 1.15 } else { vowel }
}

fn weights(phones: &[char]) -> Vec<f64> {
    let mut stressed = false;
    phones
        .iter()
        .map(|&c| {
            let w = weight(c, stressed);
            if c == 'ˈ' {
                stressed = true;
            } else if "AIOQWYaiuɑɔɜəᵻæɒɛɪʊʌe".contains(c) {
                stressed = false;
            }
            w
        })
        .collect()
}

/// When each phoneme starts in a sentence `seconds` long whose audio is silent during
/// `silences`.
fn phone_times(phones: &[char], seconds: f64, silences: &[(f64, f64)]) -> Vec<f64> {
    let weights = weights(phones);
    let pauses: Vec<usize> =
        phones.windows(2).enumerate().filter(|(_, w)| PAUSES.contains(w[0]) && w[1] == ' ').map(|(i, _)| i).collect();
    let edge = f64::from(EDGE);
    let end = (seconds - edge).max(edge);
    // A knot is a phoneme index with the time the sound before it stops and the time it
    // starts again; phonemes spread evenly by weight between two knots.
    let spread = |knots: &[(usize, f64, f64)]| -> Vec<f64> {
        let mut times = vec![end; phones.len()];
        for pair in knots.windows(2) {
            let ((from, _, resume), (to, stop, _)) = (pair[0], pair[1]);
            // A pause that ends this stretch is the knot itself.
            let paused = |i: &usize| *i >= from && i + 1 < to && pauses.contains(i);
            let count = (from..to).filter(paused).count();
            let total: f64 = weights[from..to].iter().sum();
            let per_weight = (stop - resume - PAUSE * count as f64).max(0.05) / total.max(1e-6);
            let mut t = resume;
            for i in from..to {
                times[i] = t.min(end);
                t += weights[i] * per_weight + if paused(&i) { PAUSE } else { 0.0 };
            }
        }
        times
    };
    let mut knots = vec![(0, edge, edge), (phones.len(), end, end)];
    let first = spread(&knots);
    let mut left = silences;
    for &p in &pauses {
        let expected = first[p] + PAUSE / 2.0;
        let off = |(a, b): &(f64, f64)| ((a + b) / 2.0 - expected).abs();
        let Some((i, found)) = left.iter().enumerate().min_by(|(_, a), (_, b)| off(a).total_cmp(&off(b))) else { break };
        if off(found) < SNAP_REACH {
            knots.insert(knots.len() - 1, (p + 1, found.0, found.1));
            left = &left[i + 1..];
        }
    }
    if knots.len() > 2 { spread(&knots) } else { first }
}

/// Where the words start in a phoneme string.
fn word_indices(phones: &[char]) -> Vec<usize> {
    (0..phones.len())
        .filter(|&i| !SEPARATORS.contains(phones[i]) && (i == 0 || SEPARATORS.contains(phones[i - 1])))
        .collect()
}

/// The start of each whitespace-separated word of `text`. `say` turns text into phonemes;
/// a word is found by counting the spoken words before it, so "35" may be two of them.
pub(crate) fn word_times(text: &str, seconds: f64, silences: &[(f64, f64)], say: impl Fn(&str) -> String) -> Vec<Word> {
    let phones: Vec<char> = say(text).chars().collect();
    let times = phone_times(&phones, seconds, silences);
    let spoken = word_indices(&phones);
    let mut out = Vec::new();
    let mut in_word = false;
    for (at, c) in text.char_indices() {
        if !c.is_whitespace() && !in_word {
            let before = match text[..at].trim_end() {
                "" => 0,
                prefix => word_indices(&say(prefix).chars().collect::<Vec<_>>()).len(),
            };
            let start = spoken.get(before).map_or(seconds, |&i| times[i]);
            out.push(Word { at, start: f64::from(ms(start)) / 1000.0 });
        }
        in_word = !c.is_whitespace();
    }
    out
}

/// Word times without the voice's dictionaries or its audio: by spelling.
pub(crate) fn spelled_word_times(text: &str, seconds: f64) -> Vec<Word> {
    word_times(text, seconds, &[], |t| crate::kokoro::normalize(t).to_lowercase())
}

/// Stretches of silence inside a sentence's audio, in seconds. The quiet edges of the
/// clip don't count.
pub(crate) fn silences(samples: &[f32]) -> Vec<(f64, f64)> {
    let quiet: Vec<bool> = samples
        .chunks(WINDOW)
        .map(|w| (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt() < SILENCE_RMS)
        .collect();
    let seconds = |i: usize| (i * WINDOW) as f64 / f64::from(SAMPLE_RATE);
    let mut out = Vec::new();
    let mut i = 0;
    while i < quiet.len() {
        let from = i;
        while i < quiet.len() && quiet[i] {
            i += 1;
        }
        if i - from >= SILENT_WINDOWS && from > 10 && i + 8 < quiet.len() {
            out.push((seconds(from), seconds(i)));
        }
        i = i.max(from + 1);
    }
    out
}

/// The cues of sentence `line` (1-based), given when its words start.
pub(crate) fn resolve(line: usize, sentence: &studio_script::Marked, words: &[Word], seconds: f64) -> Vec<CueTime> {
    sentence
        .marks
        .iter()
        .map(|m| {
            let word = words.iter().rev().find(|w| w.at <= m.at).filter(|_| m.at < sentence.text.len());
            CueTime { id: m.id.clone(), line, at: word.map_or(seconds, |w| w.start) }
        })
        .collect()
}

/// Cue times for a script that hasn't been spoken, from each sentence's length in seconds.
pub fn estimate(script: &str, lines: &[f64]) -> Vec<CueTime> {
    studio_script::marked_sentences(script)
        .iter()
        .zip(lines)
        .enumerate()
        .filter(|(_, (s, _))| !s.marks.is_empty())
        .flat_map(|(i, (s, &seconds))| resolve(i + 1, s, &spelled_word_times(&s.text, seconds), seconds))
        .collect()
}

/// One word of a narration as it is heard: for the critic, which checks what is on screen
/// against what has been said.
#[derive(Clone, Debug, PartialEq)]
pub struct SpokenWord {
    /// The word as written, lower case, without punctuation around it.
    pub text: String,
    /// The sentence it is in (1-based, as in `this.narrate(n)`).
    pub line: usize,
    /// Seconds after that sentence starts.
    pub at: f64,
    /// The id of the cue mark in front of it, if the script marks this word.
    pub cue: Option<String>,
}

/// Every word of a narration script with when it starts. Word times come from the files
/// stored beside each sentence's WAV (`wavs`, one per sentence, when they exist), else from
/// the spelling; a marked word takes the time of its cue, which was worked out from the
/// audio.
pub fn spoken_words(script: &str, lines: &[f64], cues: &[CueTime], wavs: &[std::path::PathBuf]) -> Vec<SpokenWord> {
    let mut out = Vec::new();
    for (i, sentence) in studio_script::marked_sentences(script).iter().enumerate() {
        let Some(&seconds) = lines.get(i) else { break };
        let words = wavs
            .get(i)
            .and_then(|w| load(&w.with_extension("words.json")))
            .unwrap_or_else(|| spelled_word_times(&sentence.text, seconds));
        for (k, w) in words.iter().enumerate() {
            let end = words.get(k + 1).map_or(sentence.text.len(), |n| n.at);
            let text = sentence.text[w.at..end].trim().trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
            if text.is_empty() {
                continue;
            }
            // The same word `resolve` gives a mark.
            let mark = sentence.marks.iter().find(|m| {
                m.at < sentence.text.len() && words.iter().rev().find(|x| x.at <= m.at).is_some_and(|x| x.at == w.at)
            });
            let at = mark.and_then(|m| cues.iter().find(|c| c.line == i + 1 && c.id == m.id)).map_or(w.start, |c| c.at);
            out.push(SpokenWord { text, line: i + 1, at, cue: mark.map(|m| m.id.clone()) });
        }
    }
    out
}

/// Word times as stored: byte offset and whole milliseconds, so a time read back is the
/// time that was worked out (a scene's cache key includes its cue times).
#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    words: Vec<(usize, u32)>,
}

fn ms(seconds: f64) -> u32 {
    (seconds.max(0.0) * 1000.0).round() as u32
}

/// The word times stored beside a sentence's WAV.
pub(crate) fn load(path: &Path) -> Option<Vec<Word>> {
    let stored: Stored = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (stored.version == VERSION).then(|| stored.words.into_iter().map(|(at, t)| Word { at, start: f64::from(t) / 1000.0 }).collect())
}

pub(crate) fn store(path: &Path, words: &[Word]) -> Result<()> {
    let tmp = path.with_extension("partial");
    let words = words.iter().map(|w| (w.at, ms(w.start))).collect();
    std::fs::write(&tmp, serde_json::to_vec(&Stored { version: VERSION, words })?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(text: &str) -> String {
        text.to_string()
    }

    fn starts(words: &[Word]) -> Vec<f64> {
        words.iter().map(|w| w.start).collect()
    }

    #[test]
    fn words_share_the_measured_length() {
        // Eight consonants and a space over 0.88 s of speech.
        let got = starts(&word_times("bbbb bbbb", 1.0, &[], same));
        assert_eq!(got, [0.06, 0.54]);
        let longer = starts(&word_times("bbbb bbbb", 2.0, &[], same));
        assert!(longer[1] > 1.0, "{longer:?}");
    }

    #[test]
    fn pauses_snap_to_the_silence() {
        let text = "bbbb, bbbb";
        let guess = starts(&word_times(text, 2.0, &[], same))[1];
        let snapped = starts(&word_times(text, 2.0, &[(1.3, 1.5)], same))[1];
        assert!(guess < 1.2, "{guess}");
        // The word after the comma starts when the silence is over, never inside it.
        assert!((1.5..1.6).contains(&snapped), "{snapped}");
        // A silence nowhere near the comma is some other sound's gap.
        let far = starts(&word_times("bb, bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", 4.0, &[(3.5, 3.7)], same))[1];
        assert!(far < 1.0, "{far}");
    }

    #[test]
    fn stored_times_read_back_exactly() {
        let path = std::env::temp_dir().join(format!("studio-words-{}.json", std::process::id()));
        let words = word_times("The total is 1,250, so we {x}stop.", 3.21, &[(1.4, 1.6)], same);
        store(&path, &words).unwrap();
        assert_eq!(load(&path).as_deref(), Some(&words[..]));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn finds_pauses_in_audio() {
        let tone = |secs: f32| vec![0.2f32; (secs * SAMPLE_RATE as f32) as usize];
        let gap = |secs: f32| vec![0.0f32; (secs * SAMPLE_RATE as f32) as usize];
        let audio = [gap(0.06), tone(0.5), gap(0.3), tone(0.5), gap(0.05), tone(0.5), gap(0.06)].concat();
        let found = silences(&audio);
        assert_eq!(found.len(), 1, "edges and short gaps are not pauses: {found:?}");
        assert!((found[0].0 - 0.56).abs() < 0.02 && (found[0].1 - 0.86).abs() < 0.02, "{found:?}");
    }

    #[test]
    fn marks_follow_numbers_read_as_words() {
        let script = "The total is 1,250 and the {share}share is {half}half. {next}Then we stop here {end}";
        let cues = estimate(script, &[4.0, 2.0]);
        assert_eq!(cues.iter().map(|c| (c.id.as_str(), c.line)).collect::<Vec<_>>(), [("share", 1), ("half", 1), ("next", 2), ("end", 2)]);
        // "1,250" is five spoken words, so "share" comes well after the middle.
        assert!(cues[0].at > 2.3 && cues[0].at < cues[1].at && cues[1].at < 4.0, "{cues:?}");
        assert!(cues[2].at < 0.1, "{cues:?}");
        assert_eq!(cues[3].at, 2.0, "a mark after the last word waits for the sentence to end");
    }
}
