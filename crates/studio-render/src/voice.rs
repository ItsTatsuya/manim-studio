//! Narration: text to a WAV file with a voice that runs on this computer: Kokoro, the
//! default, or Chatterbox, which speaks in the voice of a reference clip (each downloaded
//! once from Settings). Each line is cached by a hash of its text and voice settings.
//!
//! An engine hands back raw 24 kHz samples; the quiet around them is trimmed here, once,
//! for every engine, because the word timing behind `{cue}` marks ([`crate::cues`]) assumes
//! speech starts and ends within a short edge of the file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, bail};

use crate::cache::Fnv;
use crate::cues::CueTime;

/// What turns text into speech.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// Kokoro-82M: eleven ready-made voices, adjustable speed. The default.
    #[default]
    Kokoro,
    /// Chatterbox Turbo: speaks like a reference clip (voice cloning), at one speed.
    Chatterbox,
}

impl Engine {
    pub const ALL: [Engine; 2] = [Engine::Kokoro, Engine::Chatterbox];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Kokoro => "Kokoro",
            Engine::Chatterbox => "Chatterbox (your own voice)",
        }
    }

    /// Whether the speed setting changes how this engine speaks.
    pub fn has_speed(self) -> bool {
        self == Engine::Kokoro
    }
}

/// Which voice and how fast.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoiceSpec {
    /// Which engine speaks. Kokoro unless a teacher chose otherwise.
    pub engine: Engine,
    /// Chatterbox: the reference clip to sound like (a prepared clip, see
    /// [`crate::chatterbox::prepare_reference`]).
    pub reference: Option<PathBuf>,
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
static CHATTERBOX_DIR: OnceLock<PathBuf> = OnceLock::new();
static CHATTERBOX: Mutex<Option<crate::chatterbox::Chatterbox>> = Mutex::new(None);
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

/// Where the Chatterbox model lives (call once at startup).
pub fn set_chatterbox_dir(dir: PathBuf) {
    let _ = CHATTERBOX_DIR.set(dir);
}

pub fn chatterbox_dir() -> Option<&'static Path> {
    CHATTERBOX_DIR.get().map(PathBuf::as_path)
}

/// The Chatterbox voice is downloaded. It runs on the ONNX Runtime library that sits with
/// Kokoro's files.
pub fn chatterbox_ready() -> bool {
    matches!((chatterbox_dir(), kokoro_dir()), (Some(m), Some(r)) if crate::chatterbox::installed(m, r))
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
    *crate::lock(&KOKORO) = None;
    *crate::lock(&CHATTERBOX) = None;
    crate::kokoro::forget_dictionaries();
}

impl VoiceSpec {
    /// The voice id that will speak (the default when unset or unknown).
    pub fn voice_id(&self) -> &str {
        match self.voice.as_deref() {
            Some(v) if crate::kokoro::is_voice(v) => v,
            _ => crate::kokoro::DEFAULT_VOICE,
        }
    }

    /// This voice can speak now: its engine's model is downloaded, and Chatterbox has a
    /// reference clip.
    pub fn ready(&self) -> bool {
        match self.engine {
            Engine::Kokoro => kokoro_ready(),
            Engine::Chatterbox => chatterbox_ready() && self.reference.as_deref().is_some_and(Path::is_file),
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
        if self.engine == Engine::Chatterbox {
            // Another model, and another voice for every reference clip. It has no speed.
            h.write(b"Chatterbox(turbo-q4)");
            if let Some(clip) = self.reference.as_deref().and_then(|p| std::fs::read(p).ok()) {
                h.write(&clip);
            }
            return h.finish();
        }
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
    if !crate::kokoro::supported() {
        bail!("{}", crate::kokoro::UNSUPPORTED);
    }
    let samples = match spec.engine {
        Engine::Kokoro => speak_kokoro(text, spec)?,
        Engine::Chatterbox => speak_chatterbox(text, spec)?,
    };
    crate::kokoro::write_wav(&tmp, &samples)?;
    let secs = wav_seconds(&tmp)?;
    std::fs::rename(&tmp, out)?;
    Ok(secs)
}

fn speak_kokoro(text: &str, spec: &VoiceSpec) -> Result<Vec<f32>> {
    let Speaker { voice, quality } = spec.speaker().context(NOT_DOWNLOADED)?;
    let dir = kokoro_dir().context("no voice folder")?;
    let mut slot = crate::lock(&KOKORO);
    if slot.as_ref().is_none_or(|k| k.quality != quality) {
        *slot = None;
        *slot = Some(crate::guard(|| crate::kokoro::Kokoro::load(dir, quality))?);
    }
    // A panic inside the model leaves it in an unknown state: drop it so the next line reloads.
    let spoken = crate::guard(|| slot.as_mut().expect("loaded").speak(text, &voice, spec.rate));
    if spoken.as_ref().err().is_some_and(crate::is_engine_panic) {
        *slot = None;
    }
    spoken
}

/// Shown when Chatterbox is asked to speak without a model or a reference clip.
pub const CHATTERBOX_MISSING: &str = "The Chatterbox voice isn't ready: download it and choose a reference clip (Settings → Voice & captions)";

fn speak_chatterbox(text: &str, spec: &VoiceSpec) -> Result<Vec<f32>> {
    if !spec.ready() {
        bail!("{CHATTERBOX_MISSING}");
    }
    let (models, runtime) = (chatterbox_dir().context("no voice folder")?, kokoro_dir().context("no voice folder")?);
    let reference = read_wav(spec.reference.as_deref().context(CHATTERBOX_MISSING)?)?;
    let mut slot = crate::lock(&CHATTERBOX);
    if slot.is_none() {
        *slot = Some(crate::guard(|| crate::chatterbox::Chatterbox::load(models, runtime))?);
    }
    let spoken = crate::guard(|| slot.as_mut().expect("loaded").speak(text, &reference));
    if spoken.as_ref().err().is_some_and(crate::is_engine_panic) {
        *slot = None;
    }
    Ok(finish_speech(&spoken?))
}

/// An engine's raw samples as narration: the quiet around the speech goes, so cue timing
/// counts from the first sound and the scene adds the gap between sentences, and the
/// loudest peak is held under full scale (the model's output can overshoot, which would
/// clip).
fn finish_speech(raw: &[f32]) -> Vec<f32> {
    const CEILING: f32 = 0.95;
    let trimmed = crate::kokoro::trim_silence(raw);
    let peak = trimmed.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let scale = if peak > CEILING { CEILING / peak } else { 1.0 };
    trimmed.iter().map(|s| s * scale).collect()
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

/// Pause after each sentence (added to the WAV's own short padding), placed by the scene
/// or not.
pub const SENTENCE_GAP: f64 = 0.4;

/// A narration script's sentences, without cue marks (short fragments join the sentence
/// before).
pub use studio_script::sentences;

/// A script as it is heard and shown: without cue marks.
pub use studio_script::strip as plain;

/// One spoken sentence.
#[derive(Clone, Debug, PartialEq)]
pub struct SpokenSentence {
    pub wav: PathBuf,
    pub seconds: f64,
    /// The words spoken (no cue marks).
    pub text: String,
}

/// A narration script, spoken.
#[derive(Clone, Debug, PartialEq)]
pub struct Spoken {
    pub sentences: Vec<SpokenSentence>,
    pub cues: Vec<CueTime>,
}

/// Speak a script sentence by sentence (each cached, so the scene can place it) and time
/// its cue marks.
pub fn speak(script: &str, spec: &VoiceSpec, cache_dir: &Path) -> Result<Spoken> {
    let mut spoken = Spoken { sentences: Vec::new(), cues: Vec::new() };
    for (i, sentence) in studio_script::marked_sentences(script).into_iter().enumerate() {
        let Line { wav, seconds } = line(&sentence.text, spec, cache_dir)?;
        if !sentence.marks.is_empty() {
            let words = word_times(&sentence.text, spec, &wav, seconds);
            spoken.cues.extend(crate::cues::resolve(i + 1, &sentence, &words, seconds));
        }
        spoken.sentences.push(SpokenSentence { wav, seconds, text: sentence.text });
    }
    Ok(spoken)
}

/// When each word of a spoken sentence starts, kept beside its WAV.
fn word_times(text: &str, spec: &VoiceSpec, wav: &Path, seconds: f64) -> Vec<crate::cues::Word> {
    let stored = wav.with_extension("words.json");
    if let Some(words) = crate::cues::load(&stored) {
        return words;
    }
    let timed = crate::guard(|| {
        // Chatterbox speaks US English, and reads the same dictionaries Kokoro does; without
        // them (Kokoro never downloaded) the fallback below counts by spelling.
        let accent = if spec.engine == Engine::Kokoro { spec.voice_id() } else { crate::kokoro::DEFAULT_VOICE };
        let g2p = crate::kokoro::g2p_for(kokoro_dir().context("no voice folder")?, accent)?;
        let quiet = crate::cues::silences(&read_wav(wav)?);
        Ok(crate::cues::word_times(text, seconds, &quiet, |t| g2p.phonemize(t)))
    });
    match timed {
        Ok(words) => {
            // Only a cache: if it can't be written the times are worked out again.
            let _ = crate::cues::store(&stored, &words);
            words
        }
        // Without the dictionaries a cue is still close, and narration must not fail for it.
        Err(_) => crate::cues::spelled_word_times(text, seconds),
    }
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
/// The most 24 kHz samples a narration track may hold (one hour): a damaged header or a
/// wild start time must not ask for gigabytes.
const MAX_SAMPLES: usize = 24_000 * 3600;

pub(crate) fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let b = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse_wav(&b).with_context(|| format!("reading {}", path.display()))
}

/// [`read_wav`] on the file's bytes. Errors on a bad header: sample rate 0, no channels
/// or an absurd length.
fn parse_wav(b: &[u8]) -> Result<Vec<f32>> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        bail!("not a WAV file");
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
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            data = body;
            break;
        }
        i += 8 + size + (size & 1);
    }
    if rate == 0 || channels == 0 {
        bail!("the WAV header is damaged (sample rate {rate}, {channels} channels)");
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
    if n > MAX_SAMPLES {
        bail!("the audio is too long");
    }
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
        // A cache hit counts as fresh: the app prunes old tracks by age, and this one is
        // about to be applied.
        if let Ok(f) = std::fs::File::options().write(true).open(&wav) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        return Ok(Line { wav, seconds });
    }
    std::fs::create_dir_all(cache_dir)?;
    let rate = crate::kokoro::SAMPLE_RATE as f64;
    let mut out: Vec<f32> = Vec::new();
    for (p, s) in parts.iter().zip(starts) {
        let samples = read_wav(p)?;
        let at = (s.max(0.0) * rate) as usize;
        let end = at.saturating_add(samples.len());
        if end > MAX_SAMPLES {
            bail!("the narration track would be too long");
        }
        if out.len() < end {
            out.resize(end, 0.0);
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

/// Add the audio of `wav` into `out` (24 kHz mono) starting at second `at`, growing `out`
/// as needed. Used to hear a whole lesson's narration at once.
pub fn overlay(out: &mut Vec<f32>, wav: &Path, at: f64) -> Result<()> {
    let samples = read_wav(wav)?;
    let from = (at.max(0.0) * crate::kokoro::SAMPLE_RATE as f64) as usize;
    let end = from.saturating_add(samples.len());
    if end > MAX_SAMPLES {
        bail!("the narration track would be too long");
    }
    if out.len() < end {
        out.resize(end, 0.0);
    }
    for (k, v) in samples.iter().enumerate() {
        out[from + k] += v;
    }
    Ok(())
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
    let sentences = studio_script::split_sentences(&studio_script::strip(text));
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
    }

    /// Whatever an engine puts around its speech, cue timing sees speech start and end
    /// within the usual short edge, and loud output does not clip.
    #[test]
    fn engine_output_is_trimmed_for_cue_timing() {
        let rate = crate::kokoro::SAMPLE_RATE as usize;
        let mut raw = vec![0.0f32; rate * 3 / 2];
        raw.extend((0..rate * 2).map(|i| 1.4 * (i as f32 * 0.07).sin()));
        raw.extend(vec![0.0f32; rate * 2]);
        let kept = finish_speech(&raw);
        let edge = (crate::kokoro::EDGE * rate as f32) as usize;
        assert!((kept.len() as i64 - (rate * 2 + 2 * edge) as i64).abs() < 2 * edge as i64, "{}", kept.len());
        assert!(kept.iter().all(|s| s.abs() <= 0.951), "held under full scale");
        // The first word starts at the edge, not at the 1.5 s the engine left empty.
        let seconds = kept.len() as f64 / rate as f64;
        let words = crate::cues::word_times("bbbb bbbb", seconds, &crate::cues::silences(&kept), |t| t.to_string());
        assert!((words[0].start - f64::from(crate::kokoro::EDGE)).abs() < 0.01, "{words:?}");
        assert!(words[1].start < seconds - 0.2, "{words:?}");
        assert!(finish_speech(&vec![0.0; 1000]).is_empty());
    }

    /// With the real models (set `CHATTERBOX_DIR`, `CHATTERBOX_RUNTIME`, `CHATTERBOX_REFERENCE`;
    /// skipped otherwise): a marked script is spoken by Chatterbox and its cues are timed
    /// from the audio like Kokoro's, for any engine.
    #[test]
    fn chatterbox_lines_get_cue_times() {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let (Some(models), Some(runtime), Some(clip)) = (var("CHATTERBOX_DIR"), var("CHATTERBOX_RUNTIME"), var("CHATTERBOX_REFERENCE")) else { return };
        set_chatterbox_dir(models);
        set_kokoro_dir(runtime);
        let dir = std::env::temp_dir().join(format!("studio-cb-speak-{}", std::process::id()));
        let reference = dir.join("reference.wav");
        crate::chatterbox::prepare_reference(&clip, &reference).unwrap();
        let spec = VoiceSpec { engine: Engine::Chatterbox, reference: Some(reference), ..Default::default() };
        assert!(spec.ready());
        let spoken = speak("If r is ten, then c is {c20}twenty. And {d}they line up exactly.", &spec, &dir).unwrap();
        assert_eq!(spoken.sentences.len(), 2);
        assert_eq!(spoken.cues.len(), 2);
        let edge = f64::from(crate::kokoro::EDGE);
        for (cue, sentence) in spoken.cues.iter().zip(&spoken.sentences) {
            eprintln!("{} at {:.2} of {:.2} s", cue.id, cue.at, sentence.seconds);
            assert!(cue.at > edge && cue.at < sentence.seconds, "{cue:?}");
        }
        // The comma pause is really there in the audio, so cue times can snap to it.
        let pauses = crate::cues::silences(&read_wav(&spoken.sentences[0].wav).unwrap());
        eprintln!("pauses in sentence 1: {pauses:?}");
        assert!(!pauses.is_empty(), "the comma pause is found with the same silence threshold Kokoro uses");
        // The first cue is near the end of its sentence, the second near the start of the second half.
        assert!(spoken.cues[0].at > spoken.sentences[0].seconds * 0.5, "{:?}", spoken.cues);
        for s in &spoken.sentences {
            let samples = read_wav(&s.wav).unwrap();
            let loud = |x: &[f32]| x.iter().any(|v| v.abs() > 0.02);
            let edge_samples = (0.2 * crate::kokoro::SAMPLE_RATE as f64) as usize;
            assert!(loud(&samples[..edge_samples]) && loud(&samples[samples.len() - edge_samples..]), "speech reaches both ends of {}", s.text);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn engines_and_speeds() {
        assert_eq!(VoiceSpec::default().engine, Engine::Kokoro, "Kokoro stays the default");
        assert!(Engine::Kokoro.has_speed() && !Engine::Chatterbox.has_speed());
        // A Chatterbox line never shares a cached file with a Kokoro line, and each reference clip is its own voice.
        let dir = std::env::temp_dir().join(format!("studio-engine-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));
        crate::kokoro::write_wav(&a, &vec![0.1; 4000]).unwrap();
        crate::kokoro::write_wav(&b, &vec![0.2; 4000]).unwrap();
        let spec = |engine, reference: Option<&PathBuf>| VoiceSpec { engine, reference: reference.cloned(), rate: 1.0, ..Default::default() };
        let keys = [
            spec(Engine::Kokoro, None).key("Hello."),
            spec(Engine::Chatterbox, Some(&a)).key("Hello."),
            spec(Engine::Chatterbox, Some(&b)).key("Hello."),
        ];
        assert!(keys[0] != keys[1] && keys[1] != keys[2] && keys[0] != keys[2]);
        let slow = VoiceSpec { rate: 0.8, ..spec(Engine::Chatterbox, Some(&a)) };
        assert_eq!(slow.key("Hello."), keys[1], "no speed control, so no new cache entry for it");
        assert!(!spec(Engine::Chatterbox, Some(&a)).ready(), "not without the model");
        let _ = std::fs::remove_dir_all(&dir);
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
    fn captions_keep_decimals_whole() {
        let c = cues("Pi is about 3.14 here. It never ends!", 0.0, 4.0);
        assert_eq!(c.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["Pi is about 3.14 here.", "It never ends!"]);
        let c = cues("Take 2.5 and 0.75", 0.0, 2.0);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn captions_never_show_cue_marks() {
        let c = cues("If r is ten, c is {c20}twenty. {line}They line up.", 0.0, 4.0);
        assert_eq!(c.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["If r is ten, c is twenty.", "They line up."]);
    }

    fn wav_bytes(rate: u32, channels: u16, samples: usize) -> Vec<u8> {
        let data = vec![0u8; samples * 2 * channels as usize];
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2 * channels as u32).to_le_bytes());
        b.extend_from_slice(&(2 * channels).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        b
    }

    #[test]
    fn damaged_wav_headers_are_refused() {
        assert!(parse_wav(&wav_bytes(0, 1, 100)).is_err(), "sample rate 0");
        assert!(parse_wav(&wav_bytes(8000, 0, 100)).is_err(), "no channels");
        assert!(parse_wav(b"RIFF").is_err());
        // A tiny rate stretches the audio past the limit.
        assert!(parse_wav(&wav_bytes(1, 1, 200_000)).is_err());
        // Ordinary files still resample: 1 s at 8 kHz becomes 1 s at 24 kHz.
        assert_eq!(parse_wav(&wav_bytes(8000, 1, 8000)).unwrap().len(), 24_000);
    }

}
