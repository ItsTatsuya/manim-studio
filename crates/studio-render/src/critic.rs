//! The render-and-look critic: timing and readability checks on a scene that has run, and
//! the stills for the look pass.
//!
//! The engine check finds errors and overlaps; this finds what a viewer would notice that
//! it can't. Every finding is a [`manim::check::Problem`] in the scene's timeline, so it goes
//! through the same repair loop and counts against the same limit:
//!
//! - **dead air**: nothing moves for more than 3 s while the voice is speaking, unless the
//!   scene marks the pause with `this.mark("hold")`;
//! - **too brief**: a text is on screen for less than 0.5 s plus 0.3 s per word;
//! - **too small**: reading text under 28 px at 1080p (the watermark is a separate layer the
//!   engine doesn't check);
//! - **off-cue**: a number, or a word the script marks with a cue, is on screen more than
//!   0.4 s before the voice says it.
//!
//! The scene's own script feeds them (see `TIMING_HOOK` in [`crate::preview`]): which text is
//! on screen when, and the source line of every animation. Nothing here costs a token.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use manim::check::Problem;
use manim::scene::TimelineInfo;
use manim::script::ScriptHost;
use serde::Deserialize;

use crate::cues::{CueTime, SpokenWord};
use crate::{Frame, preview};

/// Nothing moving for longer than this while the voice runs is dead air.
pub const DEAD_AIR: f64 = 3.0;
/// A text needs this long on screen, plus [`BRIEF_PER_WORD`] for each word.
pub const BRIEF_BASE: f64 = 0.5;
pub const BRIEF_PER_WORD: f64 = 0.3;
/// Reading text smaller than this (px at 1080p, the em: what CSS calls font-size) is too small.
pub const MIN_PX: f64 = 28.0;
/// A value this much before the voice says it is off-cue.
pub const OFF_CUE: f64 = 0.4;
/// The frame is 8 units tall: 1080 / 8 px per unit at 1080p.
const PX_PER_UNIT: f64 = 135.0;
/// The tallest a line of text is relative to its em, as a typical line of digits and capitals
/// measures; dividing a text's height by it gives an em that is never too small by much.
const LINE_HEIGHT_EM: f64 = 0.7;

/// What the critic needs besides the scene: the words the narrator says. Set on
/// [`preview::OpenScene::critic`] to run the checks with the scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Critic {
    pub words: Vec<SpokenWord>,
}

impl Critic {
    /// For a narration script (with its cue marks) spoken in sentences of `lines` seconds.
    pub fn new(say: &str, lines: &[f64], cues: &[CueTime], wavs: &[std::path::PathBuf]) -> Self {
        Critic { words: crate::cues::spoken_words(say, lines, cues, wavs) }
    }
}

/// A text block on screen: from when to when, and how big.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub label: String,
    pub from: f64,
    /// `None` while it is still there at the end of the scene.
    pub to: Option<f64>,
    /// Scene units: the block's width, and its tallest glyph (the block itself grows with
    /// every line it wraps to).
    pub width: f64,
    pub height: f64,
    /// The line of the script whose animation brought it in.
    pub line: Option<u32>,
    /// Redrawn by an updater (a live readout): how long it is on screen isn't measured.
    pub dynamic: bool,
}

/// What a scene's run left for the critic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub spans: Vec<Span>,
    /// Every `play`, `wait`, `narrate` and `cue` call: its scene time and script line.
    pub calls: Vec<(f64, Option<u32>)>,
}

impl Look {
    /// The line of the call that was running at scene time `t`.
    pub fn line_at(&self, t: f64) -> Option<u32> {
        self.calls.iter().rev().find(|(c, _)| *c <= t + 1e-6).and_then(|(_, l)| *l)
    }
}

#[derive(Deserialize)]
struct RawAt {
    file: String,
    line: u32,
}

#[derive(Deserialize)]
struct RawSpan {
    label: String,
    from: f64,
    to: Option<f64>,
    bb: [f64; 4],
    tall: f64,
    #[serde(default)]
    dynamic: bool,
    at: Option<RawAt>,
}

#[derive(Deserialize)]
struct RawCall {
    t: f64,
    at: Option<RawAt>,
}

#[derive(Deserialize, Default)]
struct RawLook {
    #[serde(default)]
    spans: Vec<RawSpan>,
    #[serde(default)]
    calls: Vec<RawCall>,
}

/// What the hook recorded in the last run, with lines in the TypeScript the teacher's
/// assistant wrote (the engine runs the type-stripped code, whose lines differ).
pub fn read(host: &ScriptHost) -> Look {
    let raw: RawLook = host.eval_json("globalThis.__LOOK || {}").unwrap_or_default();
    let mut lines = Lines::default();
    let mut at = |a: Option<RawAt>| a.and_then(|a| lines.source(&a.file, a.line));
    Look {
        spans: raw
            .spans
            .into_iter()
            .map(|s| Span {
                label: s.label,
                from: s.from,
                to: s.to,
                width: (s.bb[2] - s.bb[0]).abs(),
                height: s.tall.abs(),
                line: at(s.at),
                dynamic: s.dynamic,
            })
            .collect(),
        calls: raw.calls.into_iter().map(|c| (c.t, at(c.at))).collect(),
    }
}

/// Generated line → script line, per file.
#[derive(Default)]
struct Lines {
    files: HashMap<String, Option<manim::script::Transpiled>>,
}

impl Lines {
    fn source(&mut self, file: &str, line: u32) -> Option<u32> {
        let t = self.files.entry(file.to_string()).or_insert_with(|| {
            let src = std::fs::read_to_string(file).ok()?;
            manim::script::transpile(&src, Path::new(file)).ok()
        });
        t.as_ref().map(|t| t.source_line(line))
    }
}

/// One thing wrong, before it becomes a [`Problem`].
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub kind: Kind,
    /// Scene time it starts at.
    pub time: f64,
    pub line: Option<u32>,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    DeadAir,
    TooBrief,
    TooSmall,
    OffCue,
}

impl Kind {
    /// The `kind` of the [`Problem`] it becomes.
    pub fn name(self) -> &'static str {
        match self {
            Kind::DeadAir => "dead-air",
            Kind::TooBrief => "too-brief",
            Kind::TooSmall => "too-small",
            Kind::OffCue => "off-cue",
        }
    }

    pub const ALL: [Kind; 4] = [Kind::DeadAir, Kind::TooBrief, Kind::TooSmall, Kind::OffCue];
}

impl Finding {
    pub fn into_problem(self) -> Problem {
        let at = self.line.map(|l| format!(" (line {l})")).unwrap_or_default();
        Problem { kind: self.kind.name().into(), time: self.time, play: None, a: String::new(), b: None, message: format!("{}{at}", self.message) }
    }
}

/// How many of each kind of finding, in a timeline's problems.
pub fn count(problems: &[Problem]) -> [usize; 4] {
    let mut n = [0; 4];
    for p in problems {
        if let Some(i) = Kind::ALL.iter().position(|k| k.name() == p.kind) {
            n[i] += 1;
        }
    }
    n
}

/// The line a finding message names, `(line 14)`.
pub fn message_line(message: &str) -> Option<u32> {
    let i = message.rfind("(line ")?;
    message[i + 6..].split(')').next()?.trim().parse().ok()
}

/// The narration as the scene plays it.
pub struct Narration<'a> {
    /// Each sentence's length.
    pub lines: &'a [f64],
    /// Where each sentence starts in the scene.
    pub starts: Vec<f64>,
    /// When the voice has finished.
    pub end: f64,
    pub words: &'a [SpokenWord],
    pub cues: &'a [CueTime],
}

impl<'a> Narration<'a> {
    /// `None` for a scene without a voice.
    pub fn of(req: &'a preview::OpenScene, t: &TimelineInfo, critic: &'a Critic) -> Option<Self> {
        let total = req.narration?;
        let lines = &req.narration_lines[..];
        let starts = if lines.is_empty() {
            Vec::new()
        } else if preview::uses_narrate(t) {
            preview::narration_starts(t, lines)
        } else {
            crate::voice::sequential_starts(lines)
        };
        let end = if lines.is_empty() { total } else { preview::narration_end(&starts, lines) };
        Some(Narration { lines, starts, end, words: &critic.words, cues: &req.narration_cues })
    }

    /// The stretches of the scene in which the voice is speaking.
    pub fn windows(&self) -> Vec<(f64, f64)> {
        if self.lines.is_empty() {
            return vec![(0.0, self.end)];
        }
        self.starts.iter().zip(self.lines).map(|(s, d)| (*s, s + d)).collect()
    }
}

/// Every finding for a scene that has run.
pub fn critique(t: &TimelineInfo, look: &Look, narration: Option<&Narration>) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(n) = narration {
        out.extend(dead_air(t, look, n));
        out.extend(off_cue(look, n));
    }
    out.extend(too_brief(look));
    out.extend(too_small(look));
    out.sort_by(|a, b| a.time.total_cmp(&b.time));
    out
}

// ---------------------------------------------------------------------------- dead air

/// Stretches in which nothing moves, from consecutive waits.
fn still_spans(t: &TimelineInfo) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    for p in t.plays.iter().filter(|p| p.wait || p.name == "Wait") {
        let end = p.start + p.run_time;
        match out.last_mut() {
            Some(last) if p.start <= last.1 + 1e-3 => last.1 = last.1.max(end),
            _ => out.push((p.start, end)),
        }
    }
    out
}

fn dead_air(t: &TimelineInfo, look: &Look, n: &Narration) -> Vec<Finding> {
    let windows = n.windows();
    let held = |from: f64, to: f64| t.markers.iter().any(|m| m.name.eq_ignore_ascii_case("hold") && m.time >= from - 0.05 && m.time <= to + 1e-6);
    still_spans(t)
        .into_iter()
        .filter_map(|(from, to)| {
            let spoken: f64 = windows.iter().map(|(a, b)| (to.min(*b) - from.max(*a)).max(0.0)).sum();
            if spoken <= DEAD_AIR || held(from, to) {
                return None;
            }
            Some(Finding {
                kind: Kind::DeadAir,
                time: from,
                line: look.line_at(from),
                message: format!(
                    "nothing moves for {:.1} s while the voice is speaking: give what is being said some motion for that long (an emphasis such as Indicate, a step coming in), or call this.mark(\"hold\") before a pause that should stand still",
                    spoken
                ),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------- text spans

/// Words in a label, as a viewer reads them: pieces with a letter or digit in them.
fn word_count(label: &str) -> usize {
    label.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count().max(1)
}

/// What a viewer sees of a text, for a message.
fn shown(label: &str) -> String {
    let flat: String = label.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 40 {
        format!("{}…", flat.chars().take(39).collect::<String>())
    } else {
        flat
    }
}

fn too_brief(look: &Look) -> Vec<Finding> {
    look.spans
        .iter()
        .filter(|s| s.label.chars().any(char::is_alphanumeric) && !s.dynamic)
        .filter_map(|s| {
            let seen = s.to? - s.from;
            // A text swapped for another in the same instant (a counter) was never on screen.
            if seen < 0.05 {
                return None;
            }
            let need = BRIEF_BASE + BRIEF_PER_WORD * word_count(&s.label) as f64;
            (seen < need - 1e-6).then(|| Finding {
                kind: Kind::TooBrief,
                time: s.from,
                line: s.line,
                message: format!(
                    "text \"{}\" is on screen for only {seen:.1} s; a viewer needs {need:.1} s (0.5 s plus 0.3 s a word): keep it up longer, or leave it out",
                    shown(&s.label)
                ),
            })
        })
        .collect()
}

/// The em of a text in px at 1080p, from its height (one line is assumed: more lines only
/// make the guess larger, so a text is never called too small for having several).
pub fn em_px(height_units: f64) -> f64 {
    height_units * PX_PER_UNIT / LINE_HEIGHT_EM
}

fn too_small(look: &Look) -> Vec<Finding> {
    look.spans
        .iter()
        .filter(|s| s.label.chars().any(char::is_alphanumeric) && s.height > 0.0)
        .filter_map(|s| {
            let px = em_px(s.height);
            (px < MIN_PX).then(|| Finding {
                kind: Kind::TooSmall,
                time: s.from,
                line: s.line,
                message: format!(
                    "text \"{}\" is about {px:.0} px tall at 1080p; reading text must be at least {MIN_PX:.0} px: use fewer or shorter lines, or split it, instead of shrinking it",
                    shown(&s.label)
                ),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------- off-cue

/// Numbers in a label, as digit runs (`3.5` is one).
fn numbers(label: &str) -> Vec<String> {
    let chars: Vec<char> = label.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let from = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || (chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))) {
                i += 1;
            }
            out.push(chars[from..i].iter().collect());
        } else {
            i += 1;
        }
    }
    out
}

/// The words a voice says for `text`, lower case.
fn said(text: &str) -> Vec<String> {
    crate::kokoro::normalize(text).to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// A word of the narration with when it is said in the scene.
struct Heard<'a> {
    word: &'a SpokenWord,
    /// The words it is spoken as ("20" is "twenty").
    said: Vec<String>,
    at: f64,
}

/// A value or term on screen before the voice says the word the script marks with a cue.
/// Only what the script cues counts: the script says which words go with something on
/// screen, so a number that is merely said later is not a finding.
fn off_cue(look: &Look, n: &Narration) -> Vec<Finding> {
    let heard: Vec<Heard> = n
        .words
        .iter()
        .filter_map(|w| Some(Heard { word: w, said: said(&w.text), at: n.starts.get(w.line.checked_sub(1)?)? + w.at }))
        .collect();
    if !heard.iter().any(|h| h.word.cue.is_some()) {
        return Vec::new();
    }
    // The spoken words in order, each with the text word it came from.
    let flat: Vec<(&str, usize)> = heard.iter().enumerate().flat_map(|(i, h)| h.said.iter().map(move |s| (s.as_str(), i))).collect();
    // The words (of the narration) a number is said as, where it starts.
    let spoken_at = |num: &str| -> Vec<usize> {
        let words = said(num);
        if words.is_empty() || words.len() > flat.len() {
            return Vec::new();
        }
        (0..=flat.len() - words.len()).filter(|&i| words.iter().enumerate().all(|(k, w)| flat[i + k].0 == w)).map(|i| flat[i].1).collect()
    };
    look.spans
        .iter()
        .filter_map(|s| {
            // A lone small number is a counter or a step number, not a value.
            if s.dynamic || s.label.trim().parse::<u32>().is_ok_and(|v| v < 10) {
                return None;
            }
            // Every place the voice says something this text shows.
            let mut said_here: Vec<usize> = numbers(&s.label).iter().flat_map(|num| spoken_at(num)).collect();
            let tokens: Vec<String> = s.label.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect();
            said_here.extend(heard.iter().enumerate().filter(|(_, x)| x.word.cue.is_some() && x.word.text.chars().count() >= 4 && tokens.contains(&x.word.text)).map(|(h, _)| h));
            // Said without a cue before it is on screen: it was announced.
            if said_here.iter().any(|&h| heard[h].word.cue.is_none() && heard[h].at <= s.from + OFF_CUE) {
                return None;
            }
            // The first cued mention is the cue the text waits for.
            let h = said_here.into_iter().filter(|&h| heard[h].word.cue.is_some()).min_by(|&a, &b| heard[a].at.total_cmp(&heard[b].at))?;
            let x = &heard[h];
            let gap = x.at - s.from;
            if gap <= OFF_CUE {
                return None;
            }
            let id = x.word.cue.as_deref().unwrap_or_default();
            Some(Finding {
                kind: Kind::OffCue,
                time: s.from,
                line: s.line,
                message: format!(
                    "text \"{}\" appears {gap:.1} s before the voice says \"{}\" (sentence {}, at {:.1} s): call this.cue(\"{id}\") right before it",
                    shown(&s.label),
                    x.word.text,
                    x.word.line,
                    x.at
                ),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------- look pass

/// A moment of a scene for the look pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Moment {
    pub time: f64,
    /// The line of the script that brought in what is on screen then.
    pub line: Option<u32>,
    /// What the voice has said so far (its last few words).
    pub said: String,
}

/// The most stills of one scene the look pass sends.
pub const MAX_LOOKS: usize = 12;
/// Stills closer together than this are one.
const MIN_APART: f64 = 0.5;
/// How many words of what was said go with a still.
const SAID_WORDS: usize = 24;

/// The times to look at: the end of each cue (up to the next cue, or the end of its
/// sentence), and the end of every sentence. `None` when the scene has no voice split in
/// sentences, then the thumbnail moment and the middle are used by the caller.
pub fn look_times(duration: f64, n: &Narration) -> Vec<f64> {
    let mut times: Vec<f64> = Vec::new();
    for (k, (&start, &len)) in n.starts.iter().zip(n.lines).enumerate() {
        let end = start + len;
        let mut ends: Vec<f64> = n.cues.iter().filter(|c| c.line == k + 1).map(|c| start + c.at).filter(|t| *t > start + 0.05 && *t < end - 0.05).collect();
        ends.sort_by(f64::total_cmp);
        ends.push(end);
        times.extend(ends.into_iter().map(|t| t - 0.08));
    }
    // The finished picture.
    times.push(duration - 0.1);
    times.retain(|t| *t > 0.0 && *t <= duration);
    times.sort_by(f64::total_cmp);
    let mut kept: Vec<f64> = Vec::new();
    for t in times {
        // Of two stills close together the later one shows the finished moment.
        match kept.last_mut() {
            Some(last) if t - *last < MIN_APART => *last = t,
            _ => kept.push(t),
        }
    }
    while kept.len() > MAX_LOOKS {
        // Drop the middle of the closest pair.
        let i = (1..kept.len()).min_by(|&a, &b| (kept[a] - kept[a - 1]).total_cmp(&(kept[b] - kept[b - 1]))).unwrap_or(1);
        kept.remove(i - 1);
    }
    kept
}

/// What the voice has said by scene time `t`: the last words of it.
pub fn said_by(n: &Narration, t: f64) -> String {
    let words: Vec<&str> = n
        .words
        .iter()
        .filter(|w| n.starts.get(w.line.wrapping_sub(1)).is_some_and(|s| s + w.at <= t + 1e-6))
        .map(|w| w.text.as_str())
        .collect();
    let from = words.len().saturating_sub(SAID_WORDS);
    words[from..].join(" ")
}

/// Stills of a scene for the look pass, each with its moment.
pub fn look_frames(req: &preview::OpenScene, output: crate::Output) -> Result<Vec<(Moment, Frame)>> {
    let mut req = req.clone();
    req.critic.get_or_insert_with(Critic::default);
    let req = req;
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            crate::guard(|| {
                let (host, _, timeline) = preview::load(&req, output)?;
                let look = read(&host);
                let critic = req.critic.clone().unwrap_or_default();
                let narration = Narration::of(&req, &timeline, &critic);
                let times = match narration.as_ref().filter(|n| !n.lines.is_empty()) {
                    Some(n) => look_times(timeline.duration, n),
                    None => {
                        let key = preview::thumbnail_time(&timeline).clamp(0.0, timeline.duration);
                        let mid = timeline.duration / 2.0;
                        if (mid - key).abs() > 3.0 { vec![mid, key] } else { vec![key] }
                    }
                };
                times
                    .into_iter()
                    .map(|t| {
                        let moment = Moment { time: t, line: look.line_at(t), said: narration.as_ref().map(|n| said_by(n, t)).unwrap_or_default() };
                        Ok((moment, Frame::from_rgba(output.width, output.height, host.frame_at(t)?)))
                    })
                    .collect()
            })
        })
        .map_err(|e| anyhow::anyhow!("spawning the look thread: {e}"))?
        .join()
        .map_err(|_| anyhow::anyhow!("the engine panicked"))?
}

/// Tiles per row of a contact sheet and rows per sheet.
pub const SHEET_COLS: usize = 2;
pub const SHEET_ROWS: usize = 3;
/// Width of a tile in a contact sheet (a 16:9 still scaled down).
pub const TILE_WIDTH: usize = 768;
const GUTTER: usize = 8;

/// 3 × 5 digits for the tile numbers.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Frames tiled into contact sheets, `SHEET_COLS` × `SHEET_ROWS` to a sheet, each tile
/// numbered from `first` in its top left corner (left to right, then down).
pub fn contact_sheets(frames: &[&Frame], first: usize) -> Vec<Frame> {
    frames
        .chunks(SHEET_COLS * SHEET_ROWS)
        .enumerate()
        .map(|(s, chunk)| {
            let tile_h = chunk.first().map_or(TILE_WIDTH * 9 / 16, |f| TILE_WIDTH * f.height as usize / f.width.max(1) as usize);
            let rows = chunk.len().div_ceil(SHEET_COLS);
            let (w, h) = (SHEET_COLS * TILE_WIDTH + (SHEET_COLS + 1) * GUTTER, rows * tile_h + (rows + 1) * GUTTER);
            let mut bgra = vec![0u8; w * h * 4];
            for px in bgra.as_chunks_mut::<4>().0 {
                px.copy_from_slice(&[0x40, 0x40, 0x40, 0xFF]);
            }
            for (i, f) in chunk.iter().enumerate() {
                let (x0, y0) = (GUTTER + (i % SHEET_COLS) * (TILE_WIDTH + GUTTER), GUTTER + (i / SHEET_COLS) * (tile_h + GUTTER));
                blit_scaled(f, &mut bgra, w, x0, y0, TILE_WIDTH, tile_h);
                label(&mut bgra, w, x0, y0, first + s * SHEET_COLS * SHEET_ROWS + i);
            }
            Frame { width: w as u32, height: h as u32, bgra }
        })
        .collect()
}

/// `src` scaled into the box at (x0, y0) of a `w`-wide BGRA buffer, bilinear.
fn blit_scaled(src: &Frame, dst: &mut [u8], w: usize, x0: usize, y0: usize, tw: usize, th: usize) {
    let (sw, sh) = (src.width as usize, src.height as usize);
    if sw == 0 || sh == 0 {
        return;
    }
    let at = |x: usize, y: usize, c: usize| f32::from(src.bgra[(y.min(sh - 1) * sw + x.min(sw - 1)) * 4 + c]);
    for y in 0..th {
        let fy = ((y as f32 + 0.5) * sh as f32 / th as f32 - 0.5).max(0.0);
        let (iy, ty) = (fy as usize, fy.fract());
        for x in 0..tw {
            let fx = ((x as f32 + 0.5) * sw as f32 / tw as f32 - 0.5).max(0.0);
            let (ix, tx) = (fx as usize, fx.fract());
            let o = ((y0 + y) * w + x0 + x) * 4;
            for c in 0..3 {
                let top = at(ix, iy, c) * (1.0 - tx) + at(ix + 1, iy, c) * tx;
                let bottom = at(ix, iy + 1, c) * (1.0 - tx) + at(ix + 1, iy + 1, c) * tx;
                dst[o + c] = (top * (1.0 - ty) + bottom * ty).round() as u8;
            }
            dst[o + 3] = 0xFF;
        }
    }
}

/// The number `n` (1-based) on a dark plate in the corner of the tile at (x0, y0).
fn label(dst: &mut [u8], w: usize, x0: usize, y0: usize, n: usize) {
    const SCALE: usize = 4;
    let digits: Vec<usize> = n.to_string().bytes().map(|b| usize::from(b - b'0')).collect();
    let (plate_w, plate_h) = (digits.len() * 4 * SCALE + SCALE, 7 * SCALE);
    for y in 0..plate_h {
        for x in 0..plate_w {
            let o = ((y0 + y) * w + x0 + x) * 4;
            dst[o..o + 4].copy_from_slice(&[0x10, 0x10, 0x10, 0xFF]);
        }
    }
    for (k, d) in digits.iter().enumerate() {
        for (row, bits) in DIGITS[*d].iter().enumerate() {
            for col in 0..3 {
                if bits >> (2 - col) & 1 == 1 {
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            let (x, y) = (x0 + SCALE + (k * 4 + col) * SCALE + dx, y0 + SCALE + row * SCALE + dy);
                            let o = (y * w + x) * 4;
                            dst[o..o + 4].copy_from_slice(&[0x40, 0xD7, 0xFF, 0xFF]);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(label: &str, from: f64, to: Option<f64>, height: f64) -> Span {
        Span { label: label.into(), from, to, width: 2.0, height, line: Some(7), dynamic: false }
    }

    fn word(text: &str, line: usize, at: f64, cue: Option<&str>) -> SpokenWord {
        SpokenWord { text: text.into(), line, at, cue: cue.map(str::to_string) }
    }

    #[test]
    fn brief_texts_are_found() {
        // 0.5 s plus 0.3 s for each of the three words: 1.4 s.
        let look = Look { spans: vec![span("c is twenty", 2.0, Some(3.2), 0.4), span("c is twenty", 5.0, Some(6.5), 0.4), span("on screen to the end", 9.0, None, 0.4), span("-", 1.0, Some(1.1), 0.4)], calls: vec![] };
        let found = too_brief(&look);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("only 1.2 s") && found[0].message.contains("1.4 s"), "{}", found[0].message);
        assert_eq!((found[0].time, found[0].line), (2.0, Some(7)));
    }

    #[test]
    fn small_text_is_measured_at_1080p() {
        // A digit of the default 48 size is 0.48 units tall: about 93 px of em.
        assert!((em_px(0.4795) - 92.5).abs() < 1.0);
        let look = Look { spans: vec![span("fine print", 0.0, None, 0.14), span("big enough", 0.0, None, 0.16), span(".", 0.0, None, 0.01)], calls: vec![] };
        let found = too_small(&look);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("fine print") && found[0].message.contains("at least 28 px"), "{}", found[0].message);
    }

    #[test]
    fn dead_air_needs_voice_and_no_hold_mark() {
        use manim::scene::{Marker, PlayInfo};
        let play = |index, name: &str, start, run_time, wait| PlayInfo { index, name: name.into(), start, run_time, frames: 0, wait };
        let t = |markers: Vec<Marker>| TimelineInfo {
            scene: "S".into(),
            fps: 30.0,
            width: 1920,
            height: 1080,
            duration: 12.0,
            plays: vec![play(0, "Create", 0.0, 1.0, false), play(1, "Wait", 1.0, 4.2, true), play(2, "FadeIn", 5.2, 0.5, false), play(3, "Wait", 5.7, 2.0, true), play(4, "Wait", 7.7, 0.0, true), play(5, "Wait", 7.7, 4.3, true)],
            markers,
            sounds: vec![],
            problems: vec![],
        };
        let lines = [11.0];
        let n = |end| Narration { lines: &lines, starts: vec![0.0], end, words: &[], cues: &[] };
        let look = Look { spans: vec![], calls: vec![(0.0, Some(3)), (1.0, Some(4)), (5.2, Some(5)), (5.7, Some(6)), (7.7, Some(9))] };
        // The voice runs the whole time: the 4.2 s and the 6.3 s waits are both dead air.
        let found = dead_air(&t(vec![]), &look, &n(11.0));
        let at: Vec<(f64, Option<u32>)> = found.iter().map(|f| (f.time, f.line)).collect();
        assert_eq!(at, [(1.0, Some(4)), (5.7, Some(6))], "{found:?}");
        assert!(found[0].message.contains("4.2 s"), "{}", found[0].message);
        // A hold mark inside the pause excuses it.
        let hold = Marker { name: "hold".into(), time: 1.0, kind: "marker".into() };
        let found = dead_air(&t(vec![hold]), &look, &n(11.0));
        assert_eq!(found.len(), 1, "{found:?}");
        // After the voice is done nothing is dead.
        let late = Narration { lines: &[2.0], starts: vec![0.0], end: 2.0, words: &[], cues: &[] };
        assert!(dead_air(&t(vec![]), &look, &late).is_empty());
    }

    #[test]
    fn values_before_their_words_are_off_cue() {
        // "If r is ten, c is twenty. They meet at one hundred twenty six." in sentences of 2.5 s and 3 s.
        let words = [
            word("if", 1, 0.0, None),
            word("r", 1, 0.2, None),
            word("is", 1, 0.5, None),
            word("ten", 1, 0.8, None),
            word("c", 1, 1.3, None),
            word("is", 1, 1.5, None),
            word("twenty", 1, 1.8, Some("c20")),
            word("they", 2, 0.0, None),
            word("meet", 2, 0.3, None),
            word("126", 2, 1.0, Some("sum")),
        ];
        let lines = [2.5, 3.0];
        let n = Narration { lines: &lines, starts: vec![0.0, 2.9], end: 5.9, words: &words, cues: &[] };
        let look = Look {
            spans: vec![
                // Said at 1.8 s, shown at 0.2 s.
                span("c = 20", 0.2, None, 0.4),
                // On the word.
                span("r = 10", 0.7, None, 0.4),
                // 126 is said at 3.9 s.
                span("x = 126", 1.5, None, 0.4),
                // Not mentioned at all, and a lone step number.
                span("y = 77", 0.1, None, 0.4),
                span("2", 0.1, None, 0.4),
            ],
            calls: vec![],
        };
        let found = off_cue(&look, &n);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].kind, Kind::OffCue);
        assert!(found[0].message.contains("\"c = 20\"") && found[0].message.contains("1.6 s before the voice says \"twenty\"") && found[0].message.contains("this.cue(\"c20\")"), "{}", found[0].message);
        assert!(found[1].message.contains("\"x = 126\"") && found[1].message.contains("this.cue(\"sum\")"), "{}", found[1].message);
    }

    /// Only what the script cues counts, and a value said twice is measured against the
    /// first time it is cued.
    #[test]
    fn only_cued_values_are_off_cue_and_the_first_cue_counts() {
        let plain = [word("is", 1, 0.0, None), word("seventy", 1, 2.0, None)];
        let n = Narration { lines: &[3.0], starts: vec![0.0], end: 3.0, words: &plain, cues: &[] };
        let look = Look { spans: vec![span("3 r + 2 c = 70", 0.0, None, 0.4)], calls: vec![] };
        assert!(off_cue(&look, &n).is_empty(), "no cue marks, nothing to be off");
        // "twenty" is cued at 1.8 s and again in sentence 2 (at 3.4 s).
        let twice = [word("c", 1, 1.0, None), word("twenty", 1, 1.8, Some("a")), word("again", 2, 0.0, None), word("twenty", 2, 0.5, Some("b"))];
        let n = Narration { lines: &[2.5, 2.0], starts: vec![0.0, 2.9], end: 4.9, words: &twice, cues: &[] };
        let look = Look { spans: vec![span("c = 20", 2.0, None, 0.4), span("c = 20", 0.2, None, 0.4)], calls: vec![] };
        let found = off_cue(&look, &n);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("1.6 s before") && found[0].message.contains("this.cue(\"a\")"), "{}", found[0].message);
        // Said before without a cue, it was announced.
        let told = [word("twenty", 1, 0.2, None), word("twenty", 1, 1.8, Some("a"))];
        let n = Narration { lines: &[2.5], starts: vec![0.0], end: 2.5, words: &told, cues: &[] };
        let look = Look { spans: vec![span("c = 20", 0.1, None, 0.4)], calls: vec![] };
        assert!(off_cue(&look, &n).is_empty());
    }

    #[test]
    fn a_marked_term_is_off_cue_too() {
        let words = [word("this", 1, 0.0, None), word("is", 1, 0.3, None), word("gauss", 1, 0.7, None), word("elimination", 1, 1.2, Some("ge"))];
        let n = Narration { lines: &[2.5], starts: vec![0.0], end: 2.5, words: &words, cues: &[] };
        let look = Look { spans: vec![span("Gauss elimination", 0.0, None, 0.4), span("Gauss elimination", 1.1, None, 0.4)], calls: vec![] };
        let found = off_cue(&look, &n);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("\"elimination\"") && found[0].message.contains("this.cue(\"ge\")"), "{}", found[0].message);
    }

    #[test]
    fn findings_become_problems_with_their_line() {
        let f = Finding { kind: Kind::DeadAir, time: 4.0, line: Some(14), message: "nothing moves".into() };
        let p = f.into_problem();
        assert_eq!((p.kind.as_str(), p.time), ("dead-air", 4.0));
        assert_eq!(p.message, "nothing moves (line 14)");
        assert_eq!(message_line(&p.message), Some(14));
        assert_eq!(message_line("no line here"), None);
        assert_eq!(count(&[p.clone(), p, Problem { kind: "overlap".into(), time: 0.0, play: None, a: String::new(), b: None, message: String::new() }]), [2, 0, 0, 0]);
    }

    #[test]
    fn looks_follow_the_ends_of_cues() {
        let cues = [CueTime { id: "a".into(), line: 1, at: 1.0 }, CueTime { id: "b".into(), line: 1, at: 2.2 }];
        let lines = [3.0, 2.0];
        let n = Narration { lines: &lines, starts: vec![0.0, 3.4], end: 5.4, words: &[], cues: &cues };
        let times = look_times(6.7, &n);
        // End of the lead-in to cue a, of cue a, of cue b (the sentence), of sentence 2, and the finish.
        let want = [0.92, 2.12, 2.92, 5.32, 6.6];
        assert_eq!(times.len(), want.len(), "{times:?}");
        for (t, w) in times.iter().zip(want) {
            assert!((t - w).abs() < 1e-6, "{times:?}");
        }
        // Cues too close together make one still, and a long scene is thinned.
        let many: Vec<CueTime> = (0..40).map(|i| CueTime { id: format!("c{i}"), line: 1, at: 0.2 + i as f64 * 0.3 }).collect();
        let n = Narration { lines: &[14.0], starts: vec![0.0], end: 14.0, words: &[], cues: &many };
        let times = look_times(15.0, &n);
        assert!(times.len() <= MAX_LOOKS && times.windows(2).all(|w| w[1] - w[0] >= MIN_APART - 1e-9), "{times:?}");
    }

    #[test]
    fn said_so_far_is_the_last_words() {
        let words = [word("if", 1, 0.0, None), word("r", 1, 0.3, None), word("is", 1, 0.6, None), word("ten", 1, 0.9, None), word("then", 2, 0.0, None)];
        let n = Narration { lines: &[1.5, 1.0], starts: vec![0.0, 1.9], end: 2.9, words: &words, cues: &[] };
        assert_eq!(said_by(&n, 0.7), "if r is");
        assert_eq!(said_by(&n, 2.0), "if r is ten then");
    }

    #[test]
    fn contact_sheets_tile_and_number_the_stills() {
        let still = |v: u8| Frame { width: 320, height: 180, bgra: vec![v; 320 * 180 * 4] };
        let frames: Vec<Frame> = (0..7).map(|i| still(i as u8 * 30)).collect();
        let refs: Vec<&Frame> = frames.iter().collect();
        let sheets = contact_sheets(&refs, 1);
        // Six to a sheet, two columns by three rows.
        assert_eq!(sheets.len(), 2);
        let tile_h = TILE_WIDTH * 9 / 16;
        assert_eq!((sheets[0].width as usize, sheets[0].height as usize), (2 * TILE_WIDTH + 3 * GUTTER, 3 * tile_h + 4 * GUTTER));
        assert_eq!(sheets[1].height as usize, tile_h + 2 * GUTTER);
        // The third still (value 60) sits at the start of the second row.
        let px = |s: &Frame, x: usize, y: usize| s.bgra[(y * s.width as usize + x) * 4];
        assert_eq!(px(&sheets[0], GUTTER + 300, GUTTER + tile_h + GUTTER + 200), 60);
        // The number plate is dark with a coloured digit, and the picture beside it is untouched.
        assert_eq!(px(&sheets[0], GUTTER + 2, GUTTER + 2), 0x10);
        assert_eq!(px(&sheets[0], GUTTER + 400, GUTTER + 200), 0);
    }
}
