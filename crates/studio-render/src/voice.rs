//! Narration: text to a WAV file with Kokoro, the natural voice that runs on this
//! computer (downloaded once from Settings or onboarding). Each line is cached by a
//! hash of its text and voice settings.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, bail};

use crate::cache::Fnv;

/// Which voice and how fast.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoiceSpec {
    /// A Kokoro voice id ([`crate::kokoro::VOICES`]). `None`, or an id Kokoro doesn't
    /// know (such as a system voice saved by an older version), uses the default voice.
    pub voice: Option<String>,
    /// Speaking speed, 1.0 = normal.
    pub rate: f32,
}

/// The voice that will actually speak: a Kokoro voice id and the model it runs on.
#[derive(Clone, Debug, PartialEq)]
pub struct Speaker {
    pub voice: String,
    pub quality: crate::kokoro::Quality,
}

/// Shown when narration is asked for before the natural voice is downloaded.
pub const NOT_DOWNLOADED: &str = "The natural voice isn't downloaded yet (Settings → Voice & captions)";

static KOKORO_DIR: OnceLock<PathBuf> = OnceLock::new();
/// 0 = High, 1 = Standard (see [`crate::kokoro::Quality`]).
static KOKORO_QUALITY: AtomicU8 = AtomicU8::new(0);
static KOKORO: Mutex<Option<crate::kokoro::Kokoro>> = Mutex::new(None);

/// Where the Kokoro model lives (call once at startup).
pub fn set_kokoro_dir(dir: PathBuf) {
    let _ = KOKORO_DIR.set(dir);
}

pub fn kokoro_dir() -> Option<&'static Path> {
    KOKORO_DIR.get().map(PathBuf::as_path)
}

/// The model quality the user picked (it may not be downloaded yet).
pub fn set_kokoro_quality(q: crate::kokoro::Quality) {
    let i = crate::kokoro::Quality::ALL.iter().position(|&x| x == q).unwrap_or(0);
    KOKORO_QUALITY.store(i as u8, Ordering::Relaxed);
}

pub fn kokoro_quality() -> crate::kokoro::Quality {
    use crate::kokoro::Quality;
    Quality::ALL[(KOKORO_QUALITY.load(Ordering::Relaxed) as usize).min(Quality::ALL.len() - 1)]
}

/// The model that will speak: the picked quality if downloaded, else the other.
pub fn kokoro_usable() -> Option<crate::kokoro::Quality> {
    crate::kokoro::usable(kokoro_dir()?, kokoro_quality())
}

/// The natural (Kokoro) voice is downloaded.
pub fn kokoro_ready() -> bool {
    kokoro_usable().is_some()
}

/// espeak-ng for unknown words: Studio's copy or the system's.
pub fn espeak() -> Option<crate::kokoro::espeak::Espeak> {
    crate::kokoro::espeak::find(kokoro_dir())
}

/// Drop the loaded model so the next line reloads it (e.g. to pick up espeak-ng).
pub fn reset_kokoro() {
    *KOKORO.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

impl VoiceSpec {
    /// The voice id that will speak (the default when unset or unknown).
    pub fn voice_id(&self) -> &str {
        match self.voice.as_deref() {
            Some(v) if crate::kokoro::is_voice(v) => v,
            _ => crate::kokoro::DEFAULT_VOICE,
        }
    }

    /// `None` until the natural voice is downloaded.
    pub fn speaker(&self) -> Option<Speaker> {
        Some(Speaker { voice: self.voice_id().to_string(), quality: kokoro_usable()? })
    }

    fn key(&self, text: &str) -> u64 {
        let mut h = Fnv::default();
        h.write(b"voice-v3");
        h.write(text.as_bytes());
        let speaker = self.speaker();
        // Same text as the old `Speaker::Kokoro(id, quality)` debug form, so cached lines stay valid.
        if let Some(Speaker { voice, quality }) = &speaker {
            h.write(format!("Kokoro({voice:?}, {quality:?})").as_bytes());
        }
        // espeak-ng changes how unknown words are read.
        if speaker.is_some() && espeak().is_some() {
            h.write(b"espeak");
        }
        h.write(format!("{:.2}", self.rate).as_bytes());
        h.finish()
    }
}

/// Speak `text` into `out` (WAV). Returns its length in seconds.
pub fn synthesize(text: &str, spec: &VoiceSpec, out: &Path) -> Result<f64> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = out.with_extension("partial.wav");
    let _ = std::fs::remove_file(&tmp);
    let Speaker { voice, quality } = spec.speaker().context(NOT_DOWNLOADED)?;
    let dir = kokoro_dir().context("no voice folder")?;
    let mut slot = KOKORO.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_none_or(|k| k.quality != quality) {
        *slot = None;
        *slot = Some(crate::kokoro::Kokoro::load(dir, quality)?);
    }
    let samples = slot.as_mut().expect("loaded").speak(text, &voice, spec.rate)?;
    drop(slot);
    crate::kokoro::write_wav(&tmp, &samples)?;
    let secs = wav_seconds(&tmp)?;
    std::fs::rename(&tmp, out)?;
    Ok(secs)
}

/// Length of a PCM WAV file in seconds (reads the RIFF header).
pub fn wav_seconds(path: &Path) -> Result<f64> {
    let b = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        bail!("{} is not a WAV file", path.display());
    }
    let (mut byte_rate, mut data) = (0u32, 0u32);
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let size = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]);
        if id == b"fmt " && i + 20 <= b.len() {
            byte_rate = u32::from_le_bytes([b[i + 16], b[i + 17], b[i + 18], b[i + 19]]);
        } else if id == b"data" {
            // Some writers leave the size 0 or oversized while streaming; trust the file.
            data = if size == 0 || size as usize > b.len() - i - 8 { (b.len() - i - 8) as u32 } else { size };
            break;
        }
        i += 8 + size as usize + (size as usize & 1);
    }
    if byte_rate == 0 {
        bail!("{} has no audio format", path.display());
    }
    Ok(data as f64 / byte_rate as f64)
}

/// A narration line, synthesized once and cached in `cache_dir`.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub wav: PathBuf,
    pub seconds: f64,
}

/// The cached WAV for `text`, synthesizing it if needed.
pub fn line(text: &str, spec: &VoiceSpec, cache_dir: &Path) -> Result<Line> {
    let wav = cache_dir.join(format!("{:016x}.wav", spec.key(text)));
    if wav.is_file()
        && let Ok(seconds) = wav_seconds(&wav) {
            return Ok(Line { wav, seconds });
        }
    let seconds = synthesize(text, spec, &wav)?;
    Ok(Line { wav, seconds })
}

/// Pause between sentences that aren't placed by the scene.
pub const SENTENCE_GAP: f64 = 0.3;

/// A narration line split into sentences (short fragments join the sentence before).
pub fn sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        cur.push(ch);
        let end = matches!(ch, '.' | '!' | '?') && chars.get(i + 1).is_none_or(|c| c.is_whitespace());
        if end {
            let s = cur.trim().to_string();
            if !s.is_empty() {
                out.push(s);
            }
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    let mut merged: Vec<String> = Vec::new();
    for s in out {
        match merged.last_mut() {
            Some(prev) if s.split_whitespace().count() < 3 => {
                prev.push(' ');
                prev.push_str(&s);
            }
            _ => merged.push(s),
        }
    }
    merged
}

/// Sentences one after another from 0 with `SENTENCE_GAP` between them.
pub fn sequential_starts(lines: &[f64]) -> Vec<f64> {
    let mut t = 0.0;
    lines
        .iter()
        .map(|d| {
            let s = t;
            t += d + SENTENCE_GAP;
            s
        })
        .collect()
}

/// Mono samples of a 16-bit PCM (or 32-bit float) WAV, resampled to 24 kHz.
fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let b = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        bail!("{} is not a WAV file", path.display());
    }
    let (mut fmt, mut channels, mut rate, mut bits) = (1u16, 1u16, 24_000u32, 16u16);
    let mut i = 12;
    let mut data: &[u8] = &[];
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let size = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let body = &b[i + 8..(i + 8 + size).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            fmt = u16::from_le_bytes([body[0], body[1]]);
            channels = u16::from_le_bytes([body[2], body[3]]).max(1);
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            data = body;
            break;
        }
        i += 8 + size + (size & 1);
    }
    let frame = channels as usize * (bits as usize / 8).max(1);
    let mono: Vec<f32> = data
        .chunks_exact(frame)
        .map(|f| {
            let sample = |c: usize| -> f32 {
                let o = c * (bits as usize / 8);
                match (fmt, bits) {
                    (3, 32) => f32::from_le_bytes([f[o], f[o + 1], f[o + 2], f[o + 3]]),
                    (_, 16) => i16::from_le_bytes([f[o], f[o + 1]]) as f32 / 32768.0,
                    (_, 8) => (f[o] as f32 - 128.0) / 128.0,
                    _ => 0.0,
                }
            };
            (0..channels as usize).map(sample).sum::<f32>() / channels as f32
        })
        .collect();
    if rate == crate::kokoro::SAMPLE_RATE || mono.is_empty() {
        return Ok(mono);
    }
    let ratio = rate as f64 / crate::kokoro::SAMPLE_RATE as f64;
    let n = (mono.len() as f64 / ratio) as usize;
    Ok((0..n)
        .map(|k| {
            let x = k as f64 * ratio;
            let j = x as usize;
            let f = (x - j as f64) as f32;
            let a = mono[j.min(mono.len() - 1)];
            let b = mono[(j + 1).min(mono.len() - 1)];
            a + (b - a) * f
        })
        .collect())
}

/// One narration track from sentence WAVs placed at `starts` (seconds), cached in
/// `cache_dir` by content.
pub fn compose(parts: &[PathBuf], starts: &[f64], cache_dir: &Path) -> Result<Line> {
    let mut h = Fnv::default();
    h.write(b"compose-v1");
    for (p, s) in parts.iter().zip(starts) {
        h.write(p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default().as_bytes());
        h.write(format!("{:.3}", s).as_bytes());
    }
    let wav = cache_dir.join(format!("track-{:016x}.wav", h.finish()));
    if wav.is_file()
        && let Ok(seconds) = wav_seconds(&wav)
    {
        return Ok(Line { wav, seconds });
    }
    std::fs::create_dir_all(cache_dir)?;
    let rate = crate::kokoro::SAMPLE_RATE as f64;
    let mut out: Vec<f32> = Vec::new();
    for (p, s) in parts.iter().zip(starts) {
        let samples = read_wav(p)?;
        let at = (s.max(0.0) * rate) as usize;
        if out.len() < at + samples.len() {
            out.resize(at + samples.len(), 0.0);
        }
        for (k, v) in samples.iter().enumerate() {
            out[at + k] += v;
        }
    }
    let tmp = wav.with_extension("partial.wav");
    crate::kokoro::write_wav(&tmp, &out)?;
    std::fs::rename(&tmp, &wav)?;
    Ok(Line { seconds: out.len() as f64 / rate, wav })
}

/// Where a video keeps its narration audio.
pub fn cache_dir(video_dir: &Path) -> PathBuf {
    video_dir.join(".cache").join("voice")
}

// ---------------------------------------------------------------------- captions

/// One caption cue.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Split a scene's narration into sentence cues spread over `seconds` by word count,
/// starting at `offset`.
pub fn cues(text: &str, offset: f64, seconds: f64) -> Vec<Cue> {
    let mut sentences: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        cur.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            let s = cur.trim().to_string();
            if !s.is_empty() {
                sentences.push(s);
            }
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        sentences.push(cur.trim().to_string());
    }
    // Long sentences split at commas so a cue stays readable (~12 words).
    let mut parts: Vec<String> = Vec::new();
    for s in sentences {
        if s.split_whitespace().count() > 14 && s.contains(", ") {
            let mut acc = String::new();
            for piece in s.split_inclusive(", ") {
                acc.push_str(piece);
                if acc.split_whitespace().count() >= 7 {
                    parts.push(acc.trim().to_string());
                    acc.clear();
                }
            }
            if !acc.trim().is_empty() {
                parts.push(acc.trim().to_string());
            }
        } else {
            parts.push(s);
        }
    }
    let words: usize = parts.iter().map(|p| p.split_whitespace().count().max(1)).sum();
    let mut t = offset;
    parts
        .into_iter()
        .map(|p| {
            let d = seconds * p.split_whitespace().count().max(1) as f64 / words.max(1) as f64;
            let cue = Cue { start: t, end: t + d, text: p };
            t += d;
            cue
        })
        .collect()
}

fn srt_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// SubRip text for cues.
pub fn srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, c) in cues.iter().enumerate() {
        out.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, srt_time(c.start), srt_time(c.end), c.text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_sentences() {
        assert_eq!(sentences("Here's a pizza. If we double it, do we? Yes. Let's see!"), vec!["Here's a pizza.", "If we double it, do we? Yes. Let's see!"]);
        assert_eq!(sentences("Pi is about 3.14 here."), vec!["Pi is about 3.14 here."]);
    }

    #[test]
    fn composes_sentences_into_one_track() {
        let dir = std::env::temp_dir().join(format!("studio-compose-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.wav");
        let b = dir.join("b.wav");
        crate::kokoro::write_wav(&a, &vec![0.3; 24_000]).unwrap();
        crate::kokoro::write_wav(&b, &vec![0.3; 12_000]).unwrap();
        let line = compose(&[a, b], &[0.0, 2.0], &dir).unwrap();
        assert!((line.seconds - 2.5).abs() < 0.01, "{}", line.seconds);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cues_cover_the_scene() {
        let c = cues("Peel each ring off. Lay it flat, and the strips stack into a triangle!", 10.0, 6.0);
        assert_eq!(c.len(), 2);
        assert!((c[0].start - 10.0).abs() < 1e-9);
        assert!((c.last().unwrap().end - 16.0).abs() < 1e-9);
        assert_eq!(srt_time(3725.5), "01:02:05,500");
        assert!(srt(&c).starts_with("1\n00:00:10,000 --> "));
    }

    #[test]
    fn reads_wav_length() {
        let dir = std::env::temp_dir().join(format!("studio-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.wav");
        // 1 second of 8 kHz mono 16-bit silence.
        let data = vec![0u8; 16000];
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&8000u32.to_le_bytes());
        b.extend_from_slice(&16000u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        std::fs::write(&p, b).unwrap();
        assert!((wav_seconds(&p).unwrap() - 1.0).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
