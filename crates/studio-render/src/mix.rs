//! The sound around the voice: a quiet music bed that dips whenever the narrator speaks,
//! and soft sounds where a dot pops or the answer box comes in.
//!
//! Everything is made here, from sine waves, so there is no music licence to carry and no
//! file to download. The bed is built once for the whole lesson at export (not per scene),
//! so it runs on through the cross-dissolves instead of restarting at each one, and
//! turning it on or off never re-renders a scene. Scenes only record *where* a sound
//! belongs (the `sfx:` marks the lesson kit leaves, saved beside each cached scene).

use serde::{Deserialize, Serialize};

use crate::kokoro::SAMPLE_RATE;

const RATE: f64 = SAMPLE_RATE as f64;

/// Which extras a teacher turned on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioMix {
    /// A quiet bed under the whole lesson, dipping under the voice.
    pub music: bool,
    /// A soft tick as each dot pops in.
    pub pops: bool,
    /// A soft chime as the answer box appears.
    pub answers: bool,
}

impl AudioMix {
    pub fn any(self) -> bool {
        self.music || self.pops || self.answers
    }
}

/// A sound a scene asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sfx {
    Pop,
    Answer,
}

/// Where in a scene (seconds from its start) a sound belongs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub kind: Sfx,
    pub at: f64,
}

/// The `sfx:pop` / `sfx:answer` marks the lesson kit leaves on a scene's timeline.
pub fn hits(markers: &[manim::scene::Marker]) -> Vec<Hit> {
    markers
        .iter()
        .filter_map(|m| {
            let kind = match m.name.strip_prefix("sfx:")? {
                "pop" => Sfx::Pop,
                "answer" => Sfx::Answer,
                _ => return None,
            };
            Some(Hit { kind, at: m.time })
        })
        .collect()
}

/// Read the hits a scene saved (none when it saved nothing).
pub fn load_hits(path: &std::path::Path) -> Vec<Hit> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

// ---------------------------------------------------------------------------- levels

/// The bed's loudest sample (about −26 dBFS): well under speech, which peaks near 0.5.
const BED_LEVEL: f32 = 0.05;
/// How much of the bed is left while the voice speaks (about −9 dB).
const DUCKED: f32 = 0.35;
/// A 20 ms stretch of speech is louder than this (RMS).
const SPEECH_RMS: f32 = 0.01;
const WINDOW: usize = (SAMPLE_RATE / 50) as usize;
/// The bed starts dipping this long before the voice does, so a word never starts on a
/// full-strength bed.
const LOOKAHEAD: f64 = 0.08;
/// How fast the bed dips, and how slowly it comes back (so a comma doesn't pump it).
const ATTACK: f64 = 0.12;
const RELEASE: f64 = 0.6;
/// The bed fades in and out at the ends of the lesson.
const FADE_IN: f64 = 1.0;
const FADE_OUT: f64 = 1.5;
/// Soft sounds: peaks of about −20 dBFS.
const POP_LEVEL: f32 = 0.10;
const ANSWER_LEVEL: f32 = 0.09;
/// Two sounds of one kind closer than this are one (a dense row of dots is one soft roll,
/// not a buzz).
const MIN_GAP: f64 = 0.06;

/// How much of the bed to keep at each sample of `voice` (1.0 untouched, [`DUCKED`] under
/// speech), `samples` long.
pub fn duck(voice: &[f32], samples: usize) -> Vec<f32> {
    let windows = samples.div_ceil(WINDOW).max(1);
    let loud: Vec<bool> = (0..windows)
        .map(|w| {
            let part = voice.get(w * WINDOW..((w + 1) * WINDOW).min(voice.len())).unwrap_or(&[]);
            !part.is_empty() && (part.iter().map(|s| s * s).sum::<f32>() / part.len() as f32).sqrt() > SPEECH_RMS
        })
        .collect();
    let look = (LOOKAHEAD * RATE / WINDOW as f64).ceil() as usize;
    let step = WINDOW as f64 / RATE;
    let (attack, release) = (1.0 - (-step / ATTACK).exp(), 1.0 - (-step / RELEASE).exp());
    let mut level = 0.0f64;
    let mut at_window = Vec::with_capacity(windows);
    for w in 0..windows {
        let target = if loud[w..(w + look + 1).min(windows)].iter().any(|&l| l) { 1.0 } else { 0.0 };
        level += (target - level) * if target > level { attack } else { release };
        at_window.push(1.0 - level as f32 * (1.0 - DUCKED));
    }
    // Between window centres the gain moves in a line, so there are no steps to hear.
    (0..samples)
        .map(|i| {
            let x = (i as f64 / WINDOW as f64 - 0.5).max(0.0);
            let (a, f) = (x as usize, (x - x.floor()) as f32);
            let (g0, g1) = (at_window[a.min(windows - 1)], at_window[(a + 1).min(windows - 1)]);
            g0 + (g1 - g0) * f
        })
        .collect()
}

/// A slow, warm pad: an open chord (A, E, A, C♯, E, B) of sine pairs a hair apart, each
/// swelling and fading at its own slow rate. `samples` long, before fades.
pub fn bed(samples: usize) -> Vec<f32> {
    const CHORD: [(f64, f64, f64); 6] = [
        // Hz, weight, swell rate in Hz
        (110.0, 1.0, 0.043),
        (164.81, 0.8, 0.061),
        (220.0, 0.9, 0.037),
        (277.18, 0.55, 0.071),
        (329.63, 0.6, 0.053),
        (493.88, 0.3, 0.089),
    ];
    let total: f64 = CHORD.iter().map(|(_, w, _)| w * 2.0).sum();
    let tau = std::f64::consts::TAU;
    (0..samples)
        .map(|i| {
            let t = i as f64 / RATE;
            let mut v = 0.0;
            for (k, &(hz, weight, swell)) in CHORD.iter().enumerate() {
                let amp = weight * (0.7 + 0.3 * (tau * swell * t + k as f64 * 1.3).sin());
                // Two voices a few cents apart beat slowly against each other.
                v += amp * ((tau * hz * 0.9992 * t).sin() + (tau * hz * 1.0008 * t + k as f64).sin());
            }
            (v / total) as f32 * BED_LEVEL
        })
        .collect()
}

/// A soft tick: a short sine sliding down from about 620 Hz.
fn pop_sound() -> Vec<f32> {
    let n = (0.14 * RATE) as usize;
    let mut phase = 0.0f64;
    (0..n)
        .map(|i| {
            let t = i as f64 / RATE;
            let hz = 380.0 + 240.0 * (-t / 0.03).exp();
            phase += std::f64::consts::TAU * hz / RATE;
            let attack = (t / 0.004).min(1.0);
            (phase.sin() * attack * (-t / 0.035).exp()) as f32 * POP_LEVEL
        })
        .collect()
}

/// A soft chime: two bell-like notes, an octave and a fourth apart.
fn answer_sound() -> Vec<f32> {
    let n = (0.9 * RATE) as usize;
    let tau = std::f64::consts::TAU;
    (0..n)
        .map(|i| {
            let t = i as f64 / RATE;
            let note = |at: f64, hz: f64| -> f64 {
                let u = t - at;
                if u < 0.0 {
                    return 0.0;
                }
                let bell = (tau * hz * u).sin() + 0.25 * (tau * hz * 2.76 * u).sin() * (-u / 0.08).exp();
                bell * (u / 0.005).min(1.0) * (-u / 0.22).exp()
            };
            ((note(0.0, 659.25) + note(0.11, 880.0) * 0.8) * 0.5) as f32 * ANSWER_LEVEL * 2.0
        })
        .collect()
}

/// Add `sound` to `out` starting at second `at`.
fn lay(out: &mut [f32], sound: &[f32], at: f64) {
    let from = (at.max(0.0) * RATE) as usize;
    for (o, s) in out.iter_mut().skip(from).zip(sound) {
        *o += s;
    }
}

/// The bed and soft sounds for a lesson `seconds` long, to be mixed with the narration.
/// `voice` is the narration as it will be heard (24 kHz mono, from the start of the
/// lesson), used only to find where to dip the bed. Empty when nothing is switched on.
pub fn render(voice: &[f32], seconds: f64, hits: &[Hit], mix: AudioMix) -> Vec<f32> {
    if !mix.any() || seconds <= 0.0 {
        return Vec::new();
    }
    let samples = (seconds * RATE) as usize;
    let mut out = if mix.music {
        let mut bed = bed(samples);
        let keep = duck(voice, samples);
        for (i, (b, k)) in bed.iter_mut().zip(&keep).enumerate() {
            let t = i as f64 / RATE;
            let fade = (t / FADE_IN).min(((seconds - t) / FADE_OUT).max(0.0)).min(1.0) as f32;
            *b *= k * fade;
        }
        bed
    } else {
        vec![0.0; samples]
    };
    let (pop, answer) = (pop_sound(), answer_sound());
    let mut last: [f64; 2] = [f64::NEG_INFINITY; 2];
    let mut sorted = hits.to_vec();
    sorted.sort_by(|a, b| a.at.total_cmp(&b.at));
    for h in sorted {
        let (wanted, sound, slot) = match h.kind {
            Sfx::Pop => (mix.pops, &pop, 0),
            Sfx::Answer => (mix.answers, &answer, 1),
        };
        if !wanted || !h.at.is_finite() || h.at - last[slot] < MIN_GAP {
            continue;
        }
        last[slot] = h.at;
        lay(&mut out, sound, h.at);
    }
    for s in &mut out {
        *s = s.clamp(-1.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn at(seconds: f64) -> usize {
        (seconds * RATE) as usize
    }

    /// Four seconds of silence with a second of speech from 1.0 s.
    fn speech() -> Vec<f32> {
        let mut v = vec![0.0f32; at(4.0)];
        for (i, s) in v[at(1.0)..at(2.0)].iter_mut().enumerate() {
            *s = 0.3 * (i as f32 * 0.05).sin();
        }
        v
    }

    #[test]
    fn the_bed_dips_under_the_voice_and_comes_back() {
        let keep = duck(&speech(), at(4.0));
        assert!((keep[at(0.3)] - 1.0).abs() < 1e-3, "untouched before the voice");
        assert!(keep[at(0.95)] < 0.99, "already dipping just before the first word: {}", keep[at(0.95)]);
        assert!(keep[at(1.6)] < DUCKED + 0.02, "down under the voice: {}", keep[at(1.6)]);
        assert!(keep[at(2.3)] < 0.8, "a breath after the voice it is still low: {}", keep[at(2.3)]);
        assert!(keep[at(3.9)] > 0.93, "and back up after: {}", keep[at(3.9)]);
        assert!(keep.iter().all(|&k| (DUCKED - 1e-3..=1.0 + 1e-3).contains(&k)));
    }

    #[test]
    fn a_comma_does_not_pump_the_bed() {
        // 0.25 s of quiet between two phrases leaves the bed well down.
        let mut v = speech();
        for (i, s) in v[at(2.0)..at(2.25)].iter_mut().enumerate() {
            *s = 0.0 * i as f32;
        }
        for (i, s) in v[at(2.25)..at(3.0)].iter_mut().enumerate() {
            *s = 0.3 * (i as f32 * 0.05).sin();
        }
        let keep = duck(&v, at(4.0));
        assert!(keep[at(2.2)] < 0.55, "{}", keep[at(2.2)]);
    }

    #[test]
    fn the_bed_is_quiet_and_steady() {
        let b = bed(at(3.0));
        let peak = b.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak <= BED_LEVEL + 1e-6 && peak > BED_LEVEL * 0.2, "{peak}");
        // No sudden jumps: a sample never moves more than a smooth tone's top slope allows.
        assert!(b.windows(2).all(|w| (w[1] - w[0]).abs() < 0.01));
    }

    #[test]
    fn music_ducks_under_the_voice_and_fades_at_the_ends() {
        let mix = AudioMix { music: true, ..Default::default() };
        let mut voice = speech();
        voice.resize(at(10.0), 0.0);
        let out = render(&voice, 10.0, &[], mix);
        assert_eq!(out.len(), at(10.0));
        let (under, after) = (rms(&out[at(1.3)..at(1.8)]), rms(&out[at(5.0)..at(5.5)]));
        assert!(under < after * 0.6, "ducked {under}, open {after}");
        assert!(out[0].abs() < 1e-3 && rms(&out[..at(0.05)]) < rms(&out[at(0.5)..at(0.55)]) * 0.2, "fades in");
        assert!(rms(&out[at(9.95)..]) < rms(&out[at(5.0)..at(5.05)]) * 0.2, "fades out");
    }

    #[test]
    fn each_sound_has_its_own_switch() {
        let hits = [Hit { kind: Sfx::Pop, at: 0.5 }, Hit { kind: Sfx::Answer, at: 2.0 }];
        let voice = vec![0.0; at(4.0)];
        assert!(render(&voice, 4.0, &hits, AudioMix::default()).is_empty(), "everything off adds nothing");
        let pops = render(&voice, 4.0, &hits, AudioMix { pops: true, ..Default::default() });
        assert!(rms(&pops[at(0.5)..at(0.65)]) > 0.01, "the pop is heard");
        assert_eq!(rms(&pops[at(2.0)..at(2.9)]), 0.0, "the answer chime is off");
        let answers = render(&voice, 4.0, &hits, AudioMix { answers: true, ..Default::default() });
        assert_eq!(rms(&answers[at(0.5)..at(0.65)]), 0.0, "the pop is off");
        assert!(rms(&answers[at(2.0)..at(2.4)]) > 0.01, "the chime is heard");
        assert!(answers.iter().chain(&pops).all(|s| s.abs() < 0.2), "soft");
    }

    #[test]
    fn a_roll_of_pops_is_not_a_buzz() {
        let voice = vec![0.0; at(2.0)];
        let mix = AudioMix { pops: true, ..Default::default() };
        let fast: Vec<Hit> = (0..3).map(|i| Hit { kind: Sfx::Pop, at: 0.5 + i as f64 * 0.02 }).collect();
        let one = render(&voice, 2.0, &fast[..1], mix);
        let many = render(&voice, 2.0, &fast, mix);
        assert_eq!(one, many);
    }

    #[test]
    fn scenes_ask_for_sounds_with_marks() {
        let m = |name: &str, time: f64| manim::scene::Marker { name: name.into(), time, kind: "marker".into() };
        let got = hits(&[m("narrate:1", 0.0), m("sfx:pop", 1.5), m("sfx:answer", 4.0), m("sfx:boom", 5.0), m("cue:x", 2.0)]);
        assert_eq!(got, [Hit { kind: Sfx::Pop, at: 1.5 }, Hit { kind: Sfx::Answer, at: 4.0 }]);
        // What a scene saves is what the export reads.
        let json = serde_json::to_string(&got).unwrap();
        assert_eq!(json, r#"[{"kind":"pop","at":1.5},{"kind":"answer","at":4.0}]"#);
        assert_eq!(serde_json::from_str::<Vec<Hit>>(&json).unwrap(), got);
    }
}
