//! Live preview: show any moment of a scene without encoding a video.
//!
//! Each engine runs on its own thread because manim-rust's script host keeps its state
//! in thread-locals (one loaded scene per thread). Opening a scene type-strips and runs
//! its script once with `dry_run` + `record` (milliseconds for most scenes); after that
//! a seek replays at most one `play` of scene logic and rasterizes one frame on the CPU.
//!
//! Requests are coalesced: while the engine is busy only the newest seek survives, so
//! scrubbing and playback never queue up stale frames. The open scene's files are
//! watched, and an edit (by the assistant or an external editor) reloads it in place.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, anyhow};
use manim::config::Config;
use manim::events::ProgressMode;
use manim::script::{HostOptions, ScriptHost};
use manim::theme::Theme;

use crate::cues::CueTime;
use crate::{Frame, Output, TimelineInfo};

/// What to load.
#[derive(Clone, Debug)]
pub struct OpenScene {
    /// The scene script (`scenes/03-unroll.ts`).
    pub path: PathBuf,
    /// The video folder: imports and asset paths must stay inside it.
    pub root: PathBuf,
    /// House style (theme JSON) applied before `construct` runs.
    pub theme_json: String,
    /// Which exported scene; the first one when `None`.
    pub scene: Option<String>,
    /// JSON values exposed to the script as `globalThis[name]` (e.g. narration timing).
    pub globals: Vec<(String, String)>,
    /// Length of the scene's narration (seconds): the script sees it as
    /// `globalThis.NARRATION.seconds`, and the scene is held at the end until the voice
    /// has finished.
    pub narration: Option<f64>,
    /// Length of each narration sentence (seconds). Scripts call `this.narrate(n)` where
    /// sentence `n` should start; empty when the narration isn't split.
    pub narration_lines: Vec<f64>,
    /// When each cue mark in the narration is spoken (`this.cue(id)`).
    pub narration_cues: Vec<CueTime>,
    /// Run the timing and readability checks with the scene (dead air, too brief, too
    /// small, off-cue): their findings join the timeline's problems. Off for the live
    /// preview, thumbnails and renders; the assistant's checks turn it on.
    pub critic: Option<crate::critic::Critic>,
}

/// Silence after the narration ends before the next scene starts. The next scene's
/// voice starts as its picture fades in (0.5 s), so about 0.8 s of quiet is heard.
pub const NARRATION_TAIL: f64 = 1.3;
/// Scenes longer than their voice by more than this get retimed.
const RETIME_SLACK: f64 = 0.3;
/// Shortest hold a shortened pause keeps (the last one, after the voice, may go lower).
const MIN_WAIT: f64 = 0.4;
/// The most animations are sped up to fit the voice; beyond that motion looks rushed.
const MAX_SPEEDUP: f64 = 1.35;

/// Wraps the engine's run hook so `wait(s)` follows `globalThis.__WAIT_PLAN` and `play`
/// follows `globalThis.__PLAY_PLAN` (seconds per call, in order; null keeps the script's).
const TIMING_HOOK: &str = r#"(() => {
  const run = globalThis.__manim_run;
  if (run.__studio) return true;
  // An animation given what narrate() or cue() returned always has some length.
  const MIN_PLAY = 0.1;
  const wrapped = function (cls) {
    let p = cls.prototype;
    while (p && !Object.prototype.hasOwnProperty.call(p, "wait")) p = Object.getPrototypeOf(p);
    if (p && !p.__studioTiming) {
      const wait = p.wait, play = p.play;
      // The critic (a scene checked with one): which text is on screen when, and which line
      // of the script each animation comes from. The screen is sampled before and after
      // every play and before every wait, so a text added with add() is dated by the next
      // call and one an animation brings in or takes out by that animation's start.
      const TEXTY = { Text: 1, Math: 1, DecimalNumber: 1, Integer: 1 };
      const DRAWN = { Axes: 1, NumberLine: 1, NumberPlane: 1, ComplexPlane: 1, PolarPlane: 1 };
      // The scene file's frame of the stack (paths may hold spaces): the first one in a
      // scenes/ folder, else the first outside lib/.
      const where = () => {
        try {
          let other = null;
          for (const ln of String(new Error().stack).split("\n")) {
            const m = /\((.+\.ts):(\d+)(?::\d+)?\)\s*$/.exec(ln) || /^\s*at (.+\.ts):(\d+)(?::\d+)?\s*$/.exec(ln);
            if (!m) continue;
            const at = { file: m[1], line: +m[2] };
            if (/[\\/]scenes[\\/][^\\/]+$/.test(m[1])) return at;
            if (!other && !/[\\/]lib[\\/][^\\/]+$/.test(m[1])) other = at;
          }
          return other;
        } catch (e) {}
        return null;
      };
      const texts = (scene) => {
        const out = [];
        const items = scene.inspect();
        // inspect() leaves out what has no bounding box (an empty group): so must this.
        const ms = scene.mobjects.filter((m) => m.bbox);
        let budget = 4000;
        const walk = (m, key) => {
          if (budget-- <= 0 || DRAWN[m.kind]) return;
          if (TEXTY[m.kind]) {
            const bb = m.bbox;
            let text = "";
            try { text = m._q("text"); } catch (e) {}
            if (!bb || typeof text !== "string") return;
            // Size by the tallest glyph: the whole block's height grows with its lines.
            let tall = 0;
            const n = Math.min(m.length, 160);
            for (let j = 0; j < n && budget > 0; j++, budget--) {
              const g = m.get(j).bbox;
              if (g) tall = Math.max(tall, g[3] - g[1]);
            }
            out.push({ key, text, bb, tall: tall > 0 ? tall : bb[3] - bb[1], op: m.getFillOpacity() });
            return;
          }
          const n = m.length;
          for (let j = 0; j < n; j++) walk(m.get(j), key + "/" + j);
        };
        const aligned = ms.length === items.length && ms.every((m, i) => (m.kind || "VMobject") === items[i].kind);
        if (aligned) items.forEach((it, i) => { if (it.text) walk(ms[i], String(it.id)); });
        else items.forEach((it) => { if (it.text && TEXTY[it.kind]) out.push({ key: String(it.id), text: it.label, bb: it.bbox, tall: it.bbox[3] - it.bbox[1], op: it.opacity }); });
        return out;
      };
      const sample = (scene, t, at) => {
        const L = globalThis.__LOOK;
        const seen = {};
        for (const x of texts(scene)) {
          if (x.op < 0.05) continue;
          seen[x.key] = true;
          const live = L.live[x.key];
          if (live && live.label === x.text) { live.bb = x.bb; live.tall = x.tall; continue; }
          // A text that keeps changing under one key is redrawn by an updater (a live
          // readout): its moments of change say nothing about how long a viewer sees it.
          let dynamic = false;
          if (live) {
            live.to = t;
            dynamic = (L.flips[x.key] = (L.flips[x.key] || 0) + 1) >= 2;
            if (dynamic) for (const sp of L.spans) if (sp.key === x.key) sp.dynamic = true;
          }
          L.live[x.key] = { key: x.key, label: x.text, from: t, to: null, bb: x.bb, tall: x.tall, at, dynamic };
          L.spans.push(L.live[x.key]);
        }
        for (const k of Object.keys(L.live)) {
          if (!seen[k]) { L.live[k].to = t; delete L.live[k]; }
        }
      };
      // Notes a call and samples the screen as it is now; returns where the call is.
      const note = (scene) => {
        if (!globalThis.__LOOK_ON) return null;
        const at = where();
        globalThis.__LOOK.calls.push({ t: scene.time, at });
        return at;
      };
      p.wait = function (s = 1) {
        const plan = globalThis.__WAIT_PLAN;
        const i = (globalThis.__WAIT_IX = (globalThis.__WAIT_IX || 0) + 1) - 1;
        const at = note(this);
        if (globalThis.__LOOK_ON) sample(this, this.time, at);
        return wait.call(this, plan && plan[i] != null ? plan[i] : s);
      };
      p.play = function (...items) {
        const plan = globalThis.__PLAY_PLAN;
        const i = (globalThis.__PLAY_IX = (globalThis.__PLAY_IX || 0) + 1) - 1;
        const t0 = this.time;
        const at = note(this);
        if (globalThis.__LOOK_ON) sample(this, t0, at);
        if (plan && plan[i] != null) {
          const last = items[items.length - 1];
          if (last && Object.getPrototypeOf(last) === Object.prototype) items[items.length - 1] = { ...last, runTime: plan[i] };
          else items.push({ runTime: plan[i] });
        }
        const out = play.apply(this, items);
        checkRegions(this);
        if (globalThis.__LOOK_ON) sample(this, t0, at);
        return out;
      };
      // Holds until scene time `t`. The engine rounds a wait down to whole frames, which
      // would put the picture ahead of the voice, and more so with every sentence; this
      // rounds up, so a moment lands at most one frame late.
      const hold = (scene, t) => {
        const frames = Math.ceil((t - scene.time) * scene.fps - 1e-6);
        if (frames > 0) wait.call(scene, frames / scene.fps);
      };
      // Scenes that use lib/layout.ts: after every play, each thing placed in a region must
      // still be inside it, and any other text must sit inside one of the regions.
      const checkRegions = (scene) => {
        const layout = globalThis.__LAYOUT;
        if (!layout) return;
        const TOL = 0.08;
        const inside = (bb, b) => bb[0] >= b.left - TOL && bb[2] <= b.right + TOL && bb[1] >= b.bottom - TOL && bb[3] <= b.top + TOL;
        const spans = (bb) => "x " + bb[0].toFixed(1) + " to " + bb[2].toFixed(1) + ", y " + bb[1].toFixed(1) + " to " + bb[3].toFixed(1);
        const report = (key, a, message) => {
          if (globalThis.__REGION_SEEN[key]) return;
          globalThis.__REGION_SEEN[key] = true;
          globalThis.__REGION_PROBLEMS.push({ kind: "region", time: scene.time, play: null, a, b: null, message });
        };
        const placed = [];
        for (const { m, region } of layout.items) {
          if (!scene.isOnScreen(m)) continue;
          const bb = [m.getLeft()[0], m.getBottom()[1], m.getRight()[0], m.getTop()[1]];
          placed.push(bb);
          const box = layout.boxes[region];
          if (!box || inside(bb, box)) continue;
          let text = null;
          try { text = typeof m.text === "string" && m.text ? m.text : null; } catch (e) {}
          const what = text ? 'text "' + text + '"' : "a " + (m.kind || "group");
          const stack = layout.stacks && layout.stacks[region];
          const full = bb[1] < box.bottom - TOL && stack && stack.includes(m)
            ? "; the " + region + " column is full: this.play(" + region + ".clear()) before pushing more, or use fewer lines"
            : "";
          report(region + "|" + (text || m.kind), text || m.kind || "group", what + " goes outside the " + region + " region (it spans " + spans(bb) + ")" + full);
        }
        const zones = ["kicker", "work", "figure"].map((k) => layout.boxes[k]).filter(Boolean);
        for (const it of scene.inspect()) {
          if (!it.text || it.opacity < 0.05) continue;
          if (placed.some((bb) => bb.every((v, i) => Math.abs(v - it.bbox[i]) < 1e-3))) continue;
          if (zones.some((box) => inside(it.bbox, box))) continue;
          report("none|" + it.label, it.label, 'text "' + it.label + '" is not in a layout region (it spans ' + spans(it.bbox) + "); place it with heading, work.push, figure or full");
        }
      };
      const length = (n) => {
        const d = ((globalThis.NARRATION || {}).lines || [])[n - 1];
        return d != null ? d : 2.5;
      };
      // Starts sentence n, after any before it that the scene skipped: the voice speaks
      // them in order whatever the scene does.
      const begin = (scene, n) => {
        const narration = globalThis.NARRATION || {};
        const st = globalThis.__NARR;
        const known = n <= (narration.lines || []).length;
        for (let k = known ? 1 : n; k <= n; k++) {
          if (st.starts[k] != null) continue;
          hold(scene, st.end);
          scene.mark("narrate:" + k);
          st.starts[k] = scene.time;
          st.end = scene.time + length(k) + (narration.gap || 0);
        }
        return st.starts[n];
      };
      const cuesOf = () => (globalThis.NARRATION || {}).cues || [];
      // narrate(n): sentence n of the narration starts here. Holds until the previous
      // sentence has finished and the narrator has paused for breath, marks the moment,
      // and returns the seconds until the sentence's first cue, or its whole length.
      p.narrate = function (n) {
        note(this);
        const start = begin(this, n);
        const inside = cuesOf().filter((c) => c.line === n).map((c) => c.at);
        if (!inside.length) return length(n);
        return Math.max(MIN_PLAY, start + Math.min(...inside) - this.time);
      };
      // cue(id): the word after the mark {id} is spoken here. Starts its sentence if need
      // be, holds until that word, marks the moment, and returns the seconds until the
      // next cue or the end of the sentence. An id used again names its next mark.
      p.cue = function (id) {
        note(this);
        const all = cuesOf();
        const same = all.filter((c) => c.id === String(id));
        if (!same.length) return 1;
        const used = globalThis.__NARR.used;
        const k = Object.prototype.hasOwnProperty.call(used, id) ? used[id] : 0;
        used[id] = k + 1;
        const cue = same[Math.min(k, same.length - 1)];
        const start = begin(this, cue.line);
        hold(this, start + cue.at);
        this.mark("cue:" + id);
        const later = all.filter((c) => c.line === cue.line && c.at > cue.at + 1e-3).map((c) => c.at);
        const next = later.length ? Math.min(...later) : length(cue.line);
        return Math.max(MIN_PLAY, start + next - this.time);
      };
      p.__studioTiming = true;
    }
    globalThis.__WAIT_IX = 0;
    globalThis.__PLAY_IX = 0;
    globalThis.__NARR = { end: 0, starts: {}, used: {} };
    globalThis.__REGION_PROBLEMS = [];
    globalThis.__REGION_SEEN = {};
    globalThis.__LOOK = { spans: [], live: {}, flips: {}, calls: [] };
    return run.call(this, cls);
  };
  wrapped.__studio = true;
  globalThis.__manim_run = wrapped;
  return true;
})()"#;

/// New lengths for a narrated scene's pauses and animations, so it ends with its voice.
#[derive(Clone, Debug, PartialEq)]
pub struct TimingPlan {
    /// Seconds per `wait` call, in order.
    pub waits: Vec<f64>,
    /// Seconds per `play` call, in order; empty when animations keep their speed.
    pub plays: Vec<f64>,
}

/// Fit a scene to its narration instead of holding silent frames: cut the trailing hold
/// first, then the other pauses evenly (never below a short hold), then speed the
/// animations up by at most `MAX_SPEEDUP`. `None` when the scene already fits.
pub fn timing_plan(t: &TimelineInfo, narration: f64) -> Option<TimingPlan> {
    let target = narration + NARRATION_TAIL;
    let mut excess = t.duration - target;
    if excess <= RETIME_SLACK {
        return None;
    }
    let mut waits: Vec<f64> = t.plays.iter().filter(|p| p.wait).map(|p| p.run_time).collect();
    // The final pause, when the scene ends on one, is pure dead air.
    if t.plays.last().is_some_and(|p| p.wait)
        && let Some(last) = waits.last_mut()
    {
        let cut = (*last - 0.2).max(0.0).min(excess);
        *last -= cut;
        excess -= cut;
    }
    let room: f64 = waits.iter().map(|w| (w - MIN_WAIT).max(0.0)).sum();
    if excess > 1e-6 && room > 1e-6 {
        let k = (excess / room).min(1.0);
        excess -= room * k;
        for w in &mut waits {
            *w -= (*w - MIN_WAIT).max(0.0) * k;
        }
    }
    let anim: Vec<f64> = t.plays.iter().filter(|p| !p.wait).map(|p| p.run_time).collect();
    let anim_total: f64 = anim.iter().sum();
    let plays = if excess > RETIME_SLACK && anim_total > 1e-6 {
        let factor = ((anim_total - excess) / anim_total).max(1.0 / MAX_SPEEDUP);
        anim.iter().map(|r| r * factor).collect()
    } else {
        Vec::new()
    };
    if waits.is_empty() && plays.is_empty() {
        return None;
    }
    Some(TimingPlan { waits, plays })
}

/// Install Studio's scene hooks (`narrate`, `cue`, timing plans) before the next `run`.
pub fn install_hooks(host: &ScriptHost, plan: Option<&TimingPlan>) -> Result<()> {
    let empty: Vec<f64> = Vec::new();
    host.set_global_json("__WAIT_PLAN", &serde_json::to_string(plan.map_or(&empty, |p| &p.waits))?)?;
    host.set_global_json("__PLAY_PLAN", &serde_json::to_string(plan.map_or(&empty, |p| &p.plays))?)?;
    let _: bool = host.eval_json(TIMING_HOOK)?;
    Ok(())
}

/// The `NARRATION` global a scene sees.
pub fn narration_json(seconds: f64, lines: &[f64], cues: &[CueTime]) -> String {
    serde_json::json!({ "seconds": seconds, "lines": lines, "cues": cues, "gap": crate::voice::SENTENCE_GAP }).to_string()
}

/// The scene places its sentences itself with `this.narrate(n)`.
pub fn uses_narrate(t: &TimelineInfo) -> bool {
    t.markers.iter().any(|m| m.name.starts_with("narrate:"))
}

/// Where each sentence starts: at its `narrate(n)` mark, else right after the sentence
/// before it. Either way a short gap follows each sentence.
pub fn narration_starts(t: &TimelineInfo, lines: &[f64]) -> Vec<f64> {
    let mut starts = Vec::with_capacity(lines.len());
    let mut end: Option<f64> = None;
    for (i, d) in lines.iter().enumerate() {
        let name = format!("narrate:{}", i + 1);
        let marked = t.markers.iter().find(|m| m.name == name).map(|m| m.time);
        let s = match (marked, end) {
            (Some(m), Some(e)) => m.max(e + crate::voice::SENTENCE_GAP),
            (Some(m), None) => m,
            (None, Some(e)) => e + crate::voice::SENTENCE_GAP,
            (None, None) => 0.0,
        };
        starts.push(s);
        end = Some(s + d);
    }
    starts
}

/// When the last sentence ends.
pub fn narration_end(starts: &[f64], lines: &[f64]) -> f64 {
    starts.iter().zip(lines).map(|(s, d)| s + d).fold(0.0, f64::max)
}

#[derive(Clone, Debug)]
pub enum PreviewEvent {
    /// The script ran; `timeline` has its duration, plays, markers and layout problems.
    Loaded { generation: u64, path: PathBuf, scene: String, timeline: TimelineInfo, elapsed: Duration },
    /// Loading or running failed (syntax error, exception, timeout). `error` names the
    /// TypeScript file and line plus the offending source line.
    Failed { generation: u64, path: PathBuf, error: String },
    /// The frame at scene time `time`.
    Frame { generation: u64, time: f64, frame: Frame },
}

enum Cmd {
    Open(u64, Box<OpenScene>),
    Seek(f64),
    Close,
}

/// A preview engine thread. Dropping it stops the thread.
pub struct Preview {
    tx: mpsc::Sender<Cmd>,
    events: async_channel::Receiver<PreviewEvent>,
    generation: AtomicU64,
}

const SCRIPT_TIMEOUT: Duration = Duration::from_secs(20);
const MEMORY_LIMIT: usize = 768 * 1024 * 1024;
/// QuickJS, Typst layout and deep scene graphs want more than the default 2 MB.
const STACK: usize = 32 * 1024 * 1024;

pub fn load(req: &OpenScene, output: Output) -> Result<(ScriptHost, String, TimelineInfo)> {
    let first = load_with(req, output, None)?;
    // Scenes that place their sentences with narrate() are already in step with the voice.
    if uses_narrate(&first.2) {
        return Ok(first);
    }
    // Otherwise narrated scenes end with their voice: shorten the pauses and run again.
    // (Padding only happens when the voice outlasts the scene, and then there is no plan.)
    let Some(plan) = req.narration.and_then(|n| timing_plan(&first.2, n)) else { return Ok(first) };
    // The engine keeps per-thread state: the first host must be gone before the next loads.
    drop(first);
    load_with(req, output, Some(&plan)).or_else(|_| load_with(req, output, None))
}

fn load_with(req: &OpenScene, output: Output, plan: Option<&TimingPlan>) -> Result<(ScriptHost, String, TimelineInfo)> {
    let host = ScriptHost::load(
        &req.path,
        HostOptions { sandbox: Some(req.root.clone()), timeout: Some(SCRIPT_TIMEOUT), memory_limit: Some(MEMORY_LIMIT) },
    )?;
    for (name, json) in &req.globals {
        host.set_global_json(name, json)?;
    }
    if let Some(n) = req.narration {
        host.set_global_json("NARRATION", &narration_json(n, &req.narration_lines, &req.narration_cues))?;
    }
    host.set_global_json("__LOOK_ON", if req.critic.is_some() { "true" } else { "false" })?;
    install_hooks(&host, plan)?;
    let scene = match &req.scene {
        Some(s) => s.clone(),
        None => host.scenes().into_iter().next().ok_or_else(|| anyhow!("no scenes"))?,
    };
    let config = Config {
        pixel_width: output.width,
        pixel_height: output.height,
        fps: output.fps,
        dry_run: true,
        record: true,
        check: true,
        progress: ProgressMode::None,
        theme: Some(Theme::from_json(&req.theme_json).context("house style")?),
        ..Config::default()
    };
    host.run(&scene, &config)?;
    if let Some(n) = req.narration {
        let t = host.timeline().ok_or_else(|| anyhow!("the scene did not run"))?;
        let until = if uses_narrate(&t) && !req.narration_lines.is_empty() {
            narration_end(&narration_starts(&t, &req.narration_lines), &req.narration_lines)
        } else {
            n
        };
        pad_for_narration(&host, until);
    }
    let mut timeline = host.timeline().ok_or_else(|| anyhow!("the scene did not run"))?;
    timeline.problems.extend(region_problems(&host));
    if let Some(critic) = &req.critic {
        let look = crate::critic::read(&host);
        let narration = crate::critic::Narration::of(req, &timeline, critic);
        let found = crate::critic::critique(&timeline, &look, narration.as_ref());
        timeline.problems.extend(found.into_iter().map(crate::critic::Finding::into_problem));
    }
    Ok((host, scene, timeline))
}

/// What the region check (in [`install_hooks`]) found during the last run.
fn region_problems(host: &ScriptHost) -> Vec<manim::check::Problem> {
    #[derive(serde::Deserialize)]
    struct Found {
        kind: String,
        time: f64,
        a: String,
        message: String,
    }
    let found: Vec<Found> = host.eval_json("globalThis.__REGION_PROBLEMS || []").unwrap_or_default();
    found.into_iter().map(|f| manim::check::Problem { kind: f.kind, time: f.time, play: None, a: f.a, b: None, message: f.message }).collect()
}

/// Hold the last frame until the narration (plus a short tail) has finished.
pub fn pad_for_narration(host: &ScriptHost, seconds: f64) {
    host.with_scene(|s| {
        let extra = seconds + NARRATION_TAIL - s.duration();
        if extra > 1e-3 {
            s.wait(extra);
        }
    });
}

/// The files a scene depends on and their modification times, in a stable order.
type Watch = Vec<(PathBuf, Option<SystemTime>)>;

fn watch(req: &OpenScene) -> Watch {
    crate::cache::dependencies(&req.path, &req.root)
        .into_iter()
        .map(|d| {
            let m = std::fs::metadata(&d).and_then(|m| m.modified()).ok();
            (d, m)
        })
        .collect()
}

/// Whether any watched file changed (or vanished), by stat alone: no re-parsing.
fn touched(w: &Watch) -> bool {
    w.iter().any(|(p, m)| std::fs::metadata(p).and_then(|m| m.modified()).ok() != *m)
}

impl Preview {
    /// Start a preview engine that renders frames at `output` size.
    pub fn spawn(output: Output) -> Self {
        let (tx, rx) = mpsc::channel();
        let (etx, events) = async_channel::unbounded();
        std::thread::Builder::new()
            .name("studio-preview".into())
            .stack_size(STACK)
            .spawn(move || preview_thread(output, rx, etx))
            .expect("spawning the preview thread");
        Preview { tx, events, generation: AtomicU64::new(0) }
    }

    /// Load a scene. Returns the generation its events carry; older events are stale.
    pub fn open(&self, scene: OpenScene) -> u64 {
        let g = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.tx.send(Cmd::Open(g, Box::new(scene)));
        g
    }

    /// Show scene time `t` (seconds) of the open scene.
    pub fn seek(&self, t: f64) {
        let _ = self.tx.send(Cmd::Seek(t));
    }

    /// Unload the scene (stops watching its files).
    pub fn close(&self) {
        let _ = self.tx.send(Cmd::Close);
    }

    /// The generation of the latest `open`.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn events(&self) -> async_channel::Receiver<PreviewEvent> {
        self.events.clone()
    }
}

struct Current {
    generation: u64,
    req: OpenScene,
    /// Dependencies at the last load; statted every idle tick, re-derived now and then.
    watch: Watch,
    checked: Instant,
    /// Idle ticks since the dependency list was last re-derived (new imports).
    ticks: u32,
}

/// How often (in idle ticks of 500 ms) the dependency list itself is re-derived.
const REPARSE_TICKS: u32 = 10;

fn preview_thread(output: Output, rx: mpsc::Receiver<Cmd>, tx: async_channel::Sender<PreviewEvent>) {
    let mut host: Option<ScriptHost> = None;
    let mut current: Option<Current> = None;
    let mut last_t = 0.0;
    // A frame of the open scene has been shown.
    let mut drawn = false;
    loop {
        let first = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(c) => Some(c),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let mut open: Option<(u64, OpenScene)> = None;
        let mut seek: Option<f64> = None;
        for cmd in first.into_iter().chain(rx.try_iter()) {
            match cmd {
                Cmd::Open(g, req) => {
                    open = Some((g, *req));
                    seek = None;
                }
                Cmd::Seek(t) => seek = Some(t),
                Cmd::Close => {
                    open = None;
                    seek = None;
                    host = None;
                    current = None;
                }
            }
        }
        // Idle: reload when the scene's files change. Statting the cached dependency
        // list is cheap; the list itself is re-derived only every few seconds.
        if open.is_none() && seek.is_none()
            && let Some(cur) = &mut current
            && cur.checked.elapsed() >= Duration::from_millis(500)
        {
            cur.checked = Instant::now();
            cur.ticks += 1;
            let changed = touched(&cur.watch)
                || (cur.ticks >= REPARSE_TICKS && {
                    cur.ticks = 0;
                    let w = watch(&cur.req);
                    !w.is_empty() && w != cur.watch
                });
            if changed {
                open = Some((cur.generation, cur.req.clone()));
                seek = Some(last_t);
            }
        }
        let reloaded = open.is_some();
        if let Some((generation, req)) = open {
            // Drop the old scene first: the engine keeps one scene per thread.
            host = None;
            drawn = false;
            let w = watch(&req);
            let started = Instant::now();
            match crate::guard(|| load(&req, output)) {
                Ok((h, scene, timeline)) => {
                    let _ = tx.send_blocking(PreviewEvent::Loaded {
                        generation,
                        path: req.path.clone(),
                        scene,
                        timeline,
                        elapsed: started.elapsed(),
                    });
                    host = Some(h);
                }
                Err(e) => {
                    let _ = tx.send_blocking(PreviewEvent::Failed {
                        generation,
                        path: req.path.clone(),
                        error: format!("{e:#}"),
                    });
                }
            }
            // Keep watching a failed scene too, so fixing the file brings it back.
            current = Some(Current { generation, req, watch: w, checked: Instant::now(), ticks: 0 });
            if seek.is_none() && host.is_some() {
                seek = Some(0.0);
            }
        }
        // Nothing changed since this moment was shown: don't draw and send it again.
        if !reloaded && drawn && seek == Some(last_t) {
            continue;
        }
        if let (Some(t), Some(cur)) = (seek, &current)
            && host.is_some()
        {
            let mut result = frame_near(host.as_ref().expect("checked"), t);
            if result.as_ref().err().is_some_and(crate::is_engine_panic) {
                // The engine panicked inside this host: drop it (its state lives in
                // thread-locals) and rebuild it from the script before trying again.
                host = None;
                result = match crate::guard(|| load(&cur.req, output)) {
                    Ok((h, ..)) => {
                        let r = frame_near(&h, t);
                        if !r.as_ref().err().is_some_and(crate::is_engine_panic) {
                            host = Some(h);
                        }
                        r
                    }
                    Err(e) => Err(e),
                };
            }
            match result {
                Ok(rgba) => {
                    last_t = t;
                    drawn = true;
                    let frame = Frame::from_rgba(output.width, output.height, rgba);
                    let _ = tx.send_blocking(PreviewEvent::Frame { generation: cur.generation, time: t, frame });
                }
                // The scene ran (the check passed) but this moment couldn't be drawn (a
                // safety net for engine seeking bugs). Keep the last frame rather than
                // flashing an error.
                Err(_) if drawn => {}
                Err(e) => {
                    let _ = tx.send_blocking(PreviewEvent::Failed {
                        generation: cur.generation,
                        path: cur.req.path.clone(),
                        error: format!("{e:#}"),
                    });
                }
            }
        }
    }
}

/// A frame at `t`, or at the next moments an animation starts if the engine can't seek
/// to `t` itself. Engine panics are caught (see [`crate::guard`]); a panic is returned
/// at once so the caller can rebuild the host instead of poking a broken one.
fn frame_near(host: &ScriptHost, t: f64) -> Result<Vec<u8>> {
    let try_at = |t: f64| -> Result<Vec<u8>> { crate::guard(|| host.frame_at(t)) };
    let first = match try_at(t) {
        Ok(f) => return Ok(f),
        Err(e) if crate::is_engine_panic(&e) => return Err(e),
        Err(e) => e,
    };
    let later: Vec<f64> = crate::guard(|| Ok(host.timeline()))
        .ok()
        .flatten()
        .map(|tl| tl.plays.iter().map(|p| p.start + 1e-3).filter(|s| *s > t).take(4).collect())
        .unwrap_or_default();
    for s in later {
        match try_at(s) {
            Ok(f) => return Ok(f),
            Err(e) if crate::is_engine_panic(&e) => return Err(e),
            Err(_) => {}
        }
    }
    Err(first)
}

// ---------------------------------------------------------------------- thumbnails

#[derive(Clone, Debug)]
pub enum ThumbEvent {
    Ready { key: u64, path: PathBuf, frame: Frame, timeline: TimelineInfo },
    Failed { key: u64, path: PathBuf, error: String },
    /// A short looping clip (`request_clip`): frames from the start of the scene,
    /// `1 / fps` seconds apart.
    Clip { key: u64, path: PathBuf, frames: Vec<Frame>, fps: f64 },
}

impl ThumbEvent {
    pub fn key(&self) -> u64 {
        match self {
            ThumbEvent::Ready { key, .. } | ThumbEvent::Failed { key, .. } | ThumbEvent::Clip { key, .. } => *key,
        }
    }
}

/// One queued job: a still, or a clip at the given frame rate.
type ThumbJob = (u64, Box<OpenScene>, Option<f64>);

/// Longest clip `request_clip` renders, so a long scene can't eat memory.
const CLIP_MAX_SECONDS: f64 = 8.0;

/// Renders one still per scene (scene strip, storyboard, queue rows) on its own engine
/// thread, so it never disturbs the live preview. Also reports each scene's real
/// duration and layout problems.
pub struct Thumbnailer {
    tx: mpsc::Sender<ThumbJob>,
    events: async_channel::Receiver<ThumbEvent>,
}

/// The moment a thumbnail shows: a `mark("thumbnail")` if the scene has one, else the
/// end of the last animation (or the start of a closing fade-out).
pub fn thumbnail_time(t: &TimelineInfo) -> f64 {
    if let Some(m) = t.markers.iter().find(|m| m.name.eq_ignore_ascii_case("thumbnail")) {
        return m.time;
    }
    let plays: Vec<_> = t.plays.iter().filter(|p| !p.wait).collect();
    match plays.last() {
        Some(p) if ["FadeOut", "Uncreate", "Unwrite", "ShrinkToCenter"].iter().any(|n| p.name.contains(n)) => p.start,
        Some(p) => p.start + p.run_time,
        None => t.duration,
    }
}

impl Thumbnailer {
    pub fn spawn(output: Output) -> Self {
        let (tx, rx) = mpsc::channel::<ThumbJob>();
        let (etx, events) = async_channel::unbounded();
        std::thread::Builder::new()
            .name("studio-thumbs".into())
            .stack_size(STACK)
            .spawn(move || {
                // Stills go first; of the clip requests (hover previews) only the newest
                // is kept, so style previews nobody is looking at never delay thumbnails.
                let mut pending: Vec<ThumbJob> = Vec::new();
                let add = |pending: &mut Vec<ThumbJob>, job: ThumbJob| {
                    // Newest request per key wins.
                    pending.retain(|(k, _, _)| *k != job.0);
                    if job.2.is_some() {
                        pending.retain(|(_, _, clip)| clip.is_none());
                    }
                    pending.push(job);
                };
                loop {
                    if pending.is_empty() {
                        match rx.recv() {
                            Ok(job) => add(&mut pending, job),
                            Err(_) => return,
                        }
                    }
                    for job in rx.try_iter() {
                        add(&mut pending, job);
                    }
                    let at = pending.iter().position(|(_, _, clip)| clip.is_none()).unwrap_or(0);
                    let (key, req, clip) = pending.remove(at);
                    let path = req.path.clone();
                    let result = match clip {
                        None => render_thumb(&req, output)
                            .map(|(frame, timeline)| ThumbEvent::Ready { key, path: path.clone(), frame, timeline }),
                        Some(fps) => render_clip(&req, output, fps)
                            .map(|frames| ThumbEvent::Clip { key, path: path.clone(), frames, fps }),
                    };
                    let ev = result.unwrap_or_else(|e| ThumbEvent::Failed { key, path, error: format!("{e:#}") });
                    if etx.send_blocking(ev).is_err() {
                        return;
                    }
                }
            })
            .expect("spawning the thumbnail thread");
        Thumbnailer { tx, events }
    }

    /// Queue a thumbnail; `key` comes back with the result.
    pub fn request(&self, key: u64, scene: OpenScene) {
        let _ = self.tx.send((key, Box::new(scene), None));
    }

    /// Queue a short clip of the scene at `fps` (previews that play on hover). Only the
    /// newest pending clip request is kept: an older one still waiting is dropped without
    /// an event, and thumbnails are rendered before any clip.
    pub fn request_clip(&self, key: u64, scene: OpenScene, fps: f64) {
        let _ = self.tx.send((key, Box::new(scene), Some(fps.max(1.0))));
    }

    pub fn events(&self) -> async_channel::Receiver<ThumbEvent> {
        self.events.clone()
    }
}

/// A fresh host per call (dropped, panicked or not, when the guard returns), so a caught
/// engine panic never leaves a broken host behind for the next request.
fn render_thumb(req: &OpenScene, output: Output) -> Result<(Frame, TimelineInfo)> {
    crate::guard(|| {
        let (host, _, timeline) = load(req, output)?;
        let rgba = host.frame_at(thumbnail_time(&timeline))?;
        Ok((Frame::from_rgba(output.width, output.height, rgba), timeline))
    })
}

fn render_clip(req: &OpenScene, output: Output, fps: f64) -> Result<Vec<Frame>> {
    crate::guard(|| {
        let (host, _, timeline) = load(req, output)?;
        let seconds = timeline.duration.clamp(0.0, CLIP_MAX_SECONDS);
        let count = (seconds * fps).floor() as usize + 1;
        (0..count).map(|i| Ok(Frame::from_rgba(output.width, output.height, host.frame_at(i as f64 / fps)?))).collect()
    })
}

/// Frames for the assistant to look at (the visual review): the scene's key moment, and
/// its middle when that is a different picture. Each comes with its time in seconds.
pub fn review_frames(req: &OpenScene, output: Output) -> Result<Vec<(f64, Frame)>> {
    let req = req.clone();
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            crate::guard(|| {
                let (host, _, timeline) = load(&req, output)?;
                let key = thumbnail_time(&timeline).clamp(0.0, timeline.duration);
                let mid = timeline.duration / 2.0;
                let mut times = vec![key];
                if (mid - key).abs() > 3.0 {
                    times.insert(0, mid);
                }
                times
                    .into_iter()
                    .map(|t| Ok((t, Frame::from_rgba(output.width, output.height, host.frame_at(t)?))))
                    .collect()
            })
        })
        .context("spawning the review thread")?
        .join()
        .map_err(|_| anyhow!("the engine panicked"))?
}

/// Run a scene once (no frames) and return its timeline: duration, plays, markers and
/// layout problems. The quick check before a render or after an assistant edit.
pub fn check(req: &OpenScene) -> Result<TimelineInfo> {
    let req = req.clone();
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || crate::guard(|| load(&req, Output::new(1280, 720, 30.0)).map(|(_, _, t)| t)))
        .context("spawning the check thread")?
        .join()
        .map_err(|_| anyhow!("the engine panicked"))?
}

/// Whether a path looks like a scene script.
pub fn is_script(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "ts" || e == "js" || e == "mjs")
}

#[cfg(test)]
mod retime_tests {
    use super::*;

    fn scene(dir: &Path) -> OpenScene {
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        let path = dir.join("scenes/01-t.ts");
        std::fs::write(
            &path,
            r#"import { Scene, Circle, Create } from "manim";
export default class T extends Scene {
  construct() {
    this.play(Create(Circle(1)), { runTime: 1 });
    this.wait(2);
    this.play(Create(Circle(2)), { runTime: 1 });
    this.wait(6);
  }
}"#,
        )
        .unwrap();
        OpenScene { path, root: dir.to_path_buf(), theme_json: crate::style::json(0).into(), scene: None, globals: vec![], narration: None, narration_lines: vec![], narration_cues: vec![], critic: None }
    }

    #[test]
    fn narrated_scenes_end_with_their_voice() {
        let dir = std::env::temp_dir().join(format!("studio-retime-{}", std::process::id()));
        let mut req = scene(&dir);
        let out = Output::new(320, 180, 30.0);
        assert!((load(&req, out).unwrap().2.duration - 10.0).abs() < 0.1, "unnarrated scenes keep their timing");
        // Short voice: the final hold is cut so the scene ends NARRATION_TAIL after the voice.
        req.narration = Some(3.5);
        let d = load(&req, out).unwrap().2.duration;
        assert!((d - (3.5 + NARRATION_TAIL)).abs() < 0.1, "{d}");
        // The cases below are scene lengths (voice + tail) against these 10 s of animations
        // and pauses.
        // Voice shorter than the animations alone: pauses shrink to the minimum and the
        // animations speed up, but no faster than MAX_SPEEDUP.
        req.narration = Some(1.8 - NARRATION_TAIL);
        let d = load(&req, out).unwrap().2.duration;
        assert!((d - (2.0 / MAX_SPEEDUP + MIN_WAIT + 0.2)).abs() < 0.12, "{d}");
        // Pauses alone are enough: animations keep their speed.
        req.narration = Some(2.7 - NARRATION_TAIL);
        let d = load(&req, out).unwrap().2.duration;
        assert!((d - 2.7).abs() < 0.1, "{d}");
        // In between: a moderate speed-up lands on the voice.
        req.narration = Some(2.1 - NARRATION_TAIL);
        let d = load(&req, out).unwrap().2.duration;
        assert!((d - 2.1).abs() < 0.1, "{d}");
        // Long voice: the scene is held, never shortened.
        req.narration = Some(12.0);
        let d = load(&req, out).unwrap().2.duration;
        assert!((d - (12.0 + NARRATION_TAIL)).abs() < 0.1, "{d}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `narrate(n)` places each sentence: a short animation waits for its sentence, a long
    /// one delays the next sentence, and the scene ends with the last sentence.
    #[test]
    fn narrate_places_sentences() {
        let dir = std::env::temp_dir().join(format!("studio-narrate-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        let path = dir.join("scenes/01-n.ts");
        std::fs::write(
            &path,
            r#"import { Scene, Circle, Square, Create } from "manim";
export default class N extends Scene {
  construct() {
    this.narrate(1);
    this.play(Create(Circle(1)), { runTime: 0.5 });
    this.narrate(2);
    this.play(Create(Square(1)), { runTime: 3 });
    this.narrate(3);
    this.wait(0.2);
  }
}"#,
        )
        .unwrap();
        let lines = vec![1.5, 2.0, 1.0];
        let total = crate::preview::narration_end(&crate::voice::sequential_starts(&lines), &lines);
        let req = OpenScene { path, root: dir.clone(), theme_json: crate::style::json(0).into(), scene: None, globals: vec![], narration: Some(total), narration_lines: lines.clone(), narration_cues: vec![], critic: None };
        let (_, _, t) = load(&req, Output::new(320, 180, 30.0)).unwrap();
        assert!(uses_narrate(&t));
        let starts = narration_starts(&t, &lines);
        // Sentence 2 waits for sentence 1 and a breath; sentence 3 is already late.
        let g = crate::voice::SENTENCE_GAP;
        let want = [0.0, 1.5 + g, 4.5 + g];
        for (s, w) in starts.iter().zip(want) {
            assert!((s - w).abs() < 0.05, "{starts:?}");
        }
        assert!((t.duration - (5.5 + g + NARRATION_TAIL)).abs() < 0.1, "{}", t.duration);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A narrated scene's timeline, its sentence starts and its cues.
    fn cued(tag: &str, say: &str, lines: &[f64], body: &str) -> (TimelineInfo, Vec<f64>, Vec<CueTime>) {
        let dir = std::env::temp_dir().join(format!("studio-cue-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        let path = dir.join("scenes/01-c.ts");
        let script = format!(
            "import {{ Scene, Circle, Dot, Create, FadeIn }} from \"manim\";\n\
             export default class C extends Scene {{ construct() {{ {body} }} }}"
        );
        std::fs::write(&path, script).unwrap();
        let total = narration_end(&crate::voice::sequential_starts(lines), lines);
        let cues = crate::cues::estimate(say, lines);
        let req = OpenScene { path, root: dir.clone(), theme_json: crate::style::json(0).into(), scene: None, globals: vec![], narration: Some(total), narration_lines: lines.to_vec(), narration_cues: cues.clone(), critic: None };
        let (_, _, t) = load(&req, Output::new(320, 180, 30.0)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let starts = narration_starts(&t, lines);
        (t, starts, cues)
    }

    /// Each dot pops when its number is spoken. Measured against when the voice says it,
    /// not against the `cue:` mark, which a late cue would drag along.
    #[test]
    fn cues_pop_dots_on_their_numbers() {
        let say = "We want points on the first line. If p is four, q is {a}nine. If p is eight, q is {b}three. If p is two, q is {c}twelve.";
        let lines = [2.4, 2.1, 2.2, 2.0];
        let (t, starts, cues) = cued(
            "dots",
            say,
            &lines,
            r#"this.play(Create(Circle(1)), { runTime: this.narrate(1) });
            for (const [n, id] of [[2, "a"], [3, "b"], [4, "c"]]) {
              this.play(Create(Circle(n / 2)), { runTime: this.narrate(n) });
              this.play(FadeIn(Dot()), { runTime: Math.min(0.25, this.cue(id)) });
            }"#,
        );
        let plays: Vec<f64> = t.plays.iter().filter(|p| !p.wait).map(|p| p.start).collect();
        assert_eq!((plays.len(), cues.len()), (7, 3));
        for (k, cue) in cues.iter().enumerate() {
            let spoken = starts[cue.line - 1] + cue.at;
            let pop = plays[2 + 2 * k];
            assert!(cue.at > 1.0, "the number is said late in its sentence: {cue:?}");
            assert!((pop - spoken).abs() < 0.25, "{} pops at {pop}, spoken at {spoken}", cue.id);
            assert!(pop > spoken - 1e-6, "{} is shown before it is said", cue.id);
        }
        // The sentences stay a breath apart: the cues don't stretch the narration.
        for k in 1..lines.len() {
            let want = starts[k - 1] + lines[k - 1] + crate::voice::SENTENCE_GAP;
            assert!((starts[k] - want).abs() < 0.1, "{starts:?}");
        }
    }

    /// `cue` starts the sentences before it, works without `narrate`, takes a repeated id
    /// in order and lets an unknown one through.
    #[test]
    fn cues_work_without_narrate() {
        let say = "First we look at it. Then it is {x}six here. Now it is {x}seven here.";
        let lines = [1.5, 1.8, 1.9];
        let (t, starts, cues) = cued(
            "alone",
            say,
            &lines,
            r#"this.cue("nothing");
            this.play(FadeIn(Dot()), { runTime: 0.2 });
            this.cue("x");
            this.narrate(2);
            this.play(FadeIn(Dot()), { runTime: 0.2 });
            this.cue("x");
            this.play(FadeIn(Dot()), { runTime: 0.2 });"#,
        );
        assert!(uses_narrate(&t));
        let plays: Vec<f64> = t.plays.iter().filter(|p| !p.wait).map(|p| p.start).collect();
        assert!(plays[0] < 0.05, "an unknown cue doesn't wait: {plays:?}");
        for (pop, cue) in plays[1..].iter().zip(&cues) {
            let spoken = starts[cue.line - 1] + cue.at;
            assert!((pop - spoken).abs() < 0.05, "{pop} for {cue:?}, spoken at {spoken}");
        }
        assert_eq!(t.markers.iter().filter(|m| m.name == "narrate:2").count(), 1, "cue already started sentence 2");
    }

    /// A scene run in a folder with the layout kit; its region problems.
    fn laid_out(tag: &str, body: &str) -> Vec<String> {
        run_in_kit(tag, body, 0).into_iter().filter(|m| m.contains("region")).map(|m| m.rsplit_once(" at ").map_or(m.clone(), |(a, _)| a.to_string())).collect()
    }

    /// Every problem of a scene run with the full kit, in house style `style`.
    fn run_in_kit(tag: &str, body: &str, style: usize) -> Vec<String> {
        let dir = std::env::temp_dir().join(format!("studio-kit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        crate::kit::install(&dir).unwrap();
        let path = dir.join("scenes/01-l.ts");
        std::fs::write(&path, format!("import {{ Scene, Text, MathTex, Dot, Create, FadeIn, Write }} from \"manim\";\n\
             import {{ heading, work, figure, full }} from \"../lib/layout\";\n\
             import {{ lesson }} from \"../lib/lesson\";\n\
             export default class L extends Scene {{ construct() {{ {body} }} }}")).unwrap();
        let req = OpenScene { path, root: dir.clone(), theme_json: crate::style::json(style).into(), scene: None, globals: vec![], narration: None, narration_lines: vec![], narration_cues: vec![], critic: None };
        let t = load(&req, Output::new(320, 180, 30.0)).unwrap_or_else(|e| panic!("{e:#}"));
        let _ = std::fs::remove_dir_all(&dir);
        t.2.problems.iter().map(|p| format!("{} at {:.2}", p.message, p.time)).collect()
    }

    #[test]
    fn regions_place_and_stack_content() {
        let found = laid_out("ok", r#"
            this.play(Write(heading("Two equations, two lines")), { runTime: 0.3 });
            for (const tex of ["3 r + 2 c = 70", "r + 4 c = 90", "r = 10", "c = 20"]) this.play(FadeIn(work.push(MathTex(tex))), { runTime: 0.2 });
            const ax = figure.axes({ xRange: [0, 40, 10], yRange: [0, 40, 10], numbers: true, xLabel: "r (notebook price)", yLabel: "c (pen-box price)" });
            this.play(Create(ax), FadeIn(ax.labels), { runTime: 0.3 });
            const d = Dot(ax.c2p(10, 20));
            this.play(FadeIn(d), FadeIn(MathTex("(10, 20)").nextTo(d)), { runTime: 0.2 });
            this.play(work.clear(), { runTime: 0.2 });
            this.play(FadeIn(work.push(Text("A sentence long enough that the work column has to shrink it to fit", { role: "body" }))), { runTime: 0.2 });
            this.play(FadeIn(full.place(Text("Solving = finding the points common to all the lines.", { role: "heading" }))), { runTime: 0.2 });"#);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn content_outside_its_region_is_a_problem() {
        let found = laid_out("bad", r#"
            for (let i = 0; i < 12; i++) this.play(FadeIn(work.push(MathTex("x = " + i))), { runTime: 0.1 });
            this.play(FadeIn(Text("Loose text", { role: "body" }).moveTo([-0.9, 0, 0])), { runTime: 0.1 });
            const t = figure.place(Text("Moved away", { role: "body" }));
            this.play(FadeIn(t), { runTime: 0.1 });
            this.play(t.animate.shift([-4, 0, 0]), { runTime: 0.1 });"#);
        // Steps 9 to 12 run off the bottom of the work column: each is named.
        let below = found.iter().filter(|m| m.starts_with("text \"x = ") && m.contains("outside the work region")).count();
        assert_eq!((below, found.len()), (4, 6), "{found:?}");
        assert!(found.iter().any(|m| m.starts_with("text \"Loose text\" is not in a layout region")), "{found:?}");
        assert!(found.iter().any(|m| m.starts_with("text \"Moved away\" goes outside the figure region")), "{found:?}");
        assert!(found[0].ends_with("the work column is full: this.play(work.clear()) before pushing more, or use fewer lines"), "{found:?}");
    }

    /// Every lesson piece runs, keeps to its region and passes the layout check, in every
    /// house style: the S03 and S04 moments of a two-lines lesson, built only from kit pieces.
    #[test]
    fn lesson_pieces_run_cleanly_in_every_style() {
        let body = r#"
            const k = lesson(this);
            const c = k.c;
            this.play(Write(heading("Two equations, two lines")), { runTime: 0.3 });
            const one = work.push(k.EquationCard("3 r + 2 c = 70", c.primary, "Order 1"));
            const two = work.push(k.EquationCard("r + 4 c = 90", c.secondary, "Order 2"));
            this.play(one.enter(), two.enter(), { runTime: 0.4 });
            const p = k.Plot({ xRange: [0, 40], yRange: [0, 40], step: 10, xLabel: "r", yLabel: "c" });
            this.play(p.show(), two.dim(), { runTime: 0.5 });
            const steps = k.Steps(["r = 0 arrow c = 35", "r = 10 arrow c = 20", "r = 20 arrow c = 5"]);
            for (const pt of [[0, 35], [10, 20], [20, 5]]) this.play(steps.next(), p.dot(pt, c.primary), { runTime: 0.3 });
            this.play(p.line((r) => 35 - 1.5 * r, c.primary), { runTime: 0.4 });
            this.play(p.probe((r) => 35 - 1.5 * r, 0, 20, (r, v) => "3 dot " + Math.round(r) + " + 2 dot " + Math.round(v) + " = 70"), { runTime: 0.8 });
            this.play(p.probeTo([30, 18]), { runTime: 0.3 });
            this.play(p.probeOut(), { runTime: 0.2 });
            this.play(work.clear(), { runTime: 0.3 });
            const idea = work.push(k.Callout("One equation = one line of answers."));
            this.play(idea.enter(), { runTime: 0.3 });
            for (const pt of [[2, 22], [30, 15]]) this.play(p.dot(pt, c.secondary), { runTime: 0.2 });
            this.play(p.line((r) => 22.5 - 0.25 * r, c.secondary), { runTime: 0.4 });
            this.play(p.line((r) => 35 - 1.5 * r, c.secondary, { dashed: true }), { runTime: 0.3 });
            this.play(p.crossing([10, 20]), { runTime: 1.2 });
            this.play(p.push([10, 20]), { runTime: 0.6 });
            this.play(p.pull(), { runTime: 0.5 });
            this.play(p.slopeTriangle((r) => 22.5 - 0.25 * r, 24, 8, "-1/4"), { runTime: 0.3 });
            const ans = work.push(k.AnswerBox("r = 10, quad c = 20", "notebook and pen box"));
            this.play(ans.enter(), { runTime: 0.4 });
            this.play(k.StrikeOut(ans), { runTime: 0.2 });
            const card = k.StatementCard("Solving = finding the points common to all the lines.", "common");
            this.play(card.enter(), { runTime: 0.4 });
            this.play(card.exit(), { runTime: 0.2 });"#;
        let cards = r#"
            const k = lesson(this);
            this.play(Write(heading("Exam practice")), { runTime: 0.3 });
            this.play(k.PauseCard(0.3));
            const table = k.CaseTable([["k", "ratios", "solutions"], ["6", "1/2, 1/2, 1/2", "infinitely many"], ["-6", "-1/2, -1/2, 1/2", "none"]]);
            for (let i = 0; i < 3; i++) this.play(table.next(), { runTime: 0.2 });"#;
        for style in 0..crate::style::STYLES.len() {
            for (name, scene) in [("graph", body), ("cards", cards)] {
                let found = run_in_kit(&format!("lesson-{name}-{style}"), scene, style);
                assert!(found.is_empty(), "{name} in style {style}: {found:?}");
            }
        }
    }

    /// Scenes that don't use the layout kit are checked as before.
    #[test]
    fn scenes_without_regions_are_not_region_checked() {
        let found = laid_out("free", r#"this.play(FadeIn(Text("Anywhere", { role: "body" }).moveTo([-0.9, 0, 0])), { runTime: 0.1 });"#);
        assert!(found.is_empty(), "{found:?}");
    }

    /// A narrated scene run with the critic: its findings, `(kind, message)`. The body starts
    /// at line 6 of the script.
    fn critiqued(tag: &str, say: &str, lines: &[f64], body: &str) -> Vec<(String, String)> {
        critiqued_in(tag, say, lines, body, 0)
    }

    fn critiqued_in(tag: &str, say: &str, lines: &[f64], body: &str, style: usize) -> Vec<(String, String)> {
        let dir = std::env::temp_dir().join(format!("studio-critic-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        crate::kit::install(&dir).unwrap();
        let path = dir.join("scenes/01-c.ts");
        std::fs::write(
            &path,
            format!(
                "import {{ Scene, Text, MathTex, Dot, Circle, Create, FadeIn, FadeOut, Write, Indicate, VGroup }} from \"manim\";\n\
                 import {{ heading, work, figure, full }} from \"../lib/layout\";\n\
                 import {{ lesson }} from \"../lib/lesson\";\n\
                 export default class C extends Scene {{\n  construct() {{\n{}\n  }}\n}}",
                body.trim_matches('\n')
            ),
        )
        .unwrap();
        let total = narration_end(&crate::voice::sequential_starts(lines), lines);
        let cues = crate::cues::estimate(say, lines);
        let critic = crate::critic::Critic::new(say, lines, &cues, &[]);
        let req = OpenScene { path, root: dir.clone(), theme_json: crate::style::json(style).into(), scene: None, globals: vec![], narration: Some(total), narration_lines: lines.to_vec(), narration_cues: cues, critic: Some(critic) };
        let t = load(&req, Output::new(320, 180, 30.0)).unwrap_or_else(|e| panic!("{e:#}")).2;
        let _ = std::fs::remove_dir_all(&dir);
        t.problems.into_iter().filter(|p| crate::critic::Kind::ALL.iter().any(|k| k.name() == p.kind)).map(|p| (p.kind, p.message)).collect()
    }

    /// Nothing moving while the voice runs is dead air, with the line it follows; a hold
    /// mark excuses it.
    #[test]
    fn dead_air_is_found_and_a_hold_excuses_it() {
        let say = "Here is the first idea. It takes a while to say, so the picture has time to show it.";
        let body = r#"
            this.narrate(1);
            this.play(FadeIn(Dot()), { runTime: 0.5 });
            this.narrate(2);
            this.play(FadeIn(Circle(1)), { runTime: 0.5 });"#;
        let found = critiqued("air gap", say, &[2.0, 6.0], body);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].0 == "dead-air" && found[0].1.contains("nothing moves for 5.5 s") && found[0].1.ends_with("(line 9)"), "{found:?}");
        // The wait before sentence 2 is only 1.9 s, and a hold mark excuses the rest.
        let held = critiqued("held gap", say, &[2.0, 6.0], &format!("{body}\n            this.mark(\"hold\");"));
        assert!(held.is_empty(), "{held:?}");
    }

    /// A scene that moves with its voice and shows each value on its word has no findings.
    #[test]
    fn a_scene_that_moves_with_its_voice_is_clean() {
        let say = "We set r to {r}ten and look at it for a moment, then at the line that follows from it.";
        let body = r#"
            this.play(Write(heading("A clean scene")), { runTime: 0.8 });
            this.cue("r");
            const eq = work.push(MathTex("r = 10"));
            this.play(FadeIn(eq), { runTime: 0.4 });
            this.play(Indicate(eq), { runTime: 1.2 });
            this.play(Indicate(eq), { runTime: 1.2 });"#;
        let found = critiqued("clean", say, &[4.0], body);
        assert!(found.is_empty(), "{found:?}");
    }

    /// The S03 and S04 moments of a two-lines lesson, built from kit pieces and paced by the
    /// voice as a builder is told to: no finding in any house style (a phantom one would
    /// burn a repair try on a scene that is fine).
    #[test]
    fn lesson_pieces_paced_by_the_voice_pass_the_critic() {
        let say = "Two equations, two lines. The first order says {a}three r plus two c is seventy, and the second says {b}r plus four c is ninety. \
                   Pick {p}r equals ten and the first line gives c equals twenty. Try a few more points and they all fall on one line. \
                   Draw the second line the same way, and they cross at one point. That point is {x}ten and twenty. \
                   So solving means finding the points common to all the lines.";
        let lines = [1.6, 8.0, 5.0, 5.5, 4.5, 4.0, 4.5];
        let body = r#"
            const k = lesson(this);
            const c = k.c;
            this.narrate(1);
            this.play(Write(heading("Two equations, two lines")), { runTime: 1.2 });
            const one = work.push(k.EquationCard("3 r + 2 c = 70", c.primary, "Order 1"));
            const two = work.push(k.EquationCard("r + 4 c = 90", c.secondary, "Order 2"));
            this.narrate(2);
            this.play(one.enter(), { runTime: Math.min(1.2, this.cue("a")) });
            this.play(one.focus(), { runTime: 2.0 });
            this.play(two.enter(), { runTime: Math.min(1.2, this.cue("b")) });
            this.play(two.focus(), { runTime: 1.5 });
            const p = k.Plot({ xRange: [0, 40], yRange: [0, 40], step: 10, xLabel: "r", yLabel: "c" });
            this.narrate(3);
            this.play(p.show(), two.dim(), { runTime: 1.5 });
            const steps = k.Steps(["r = 10 arrow c = 20"]);
            this.play(steps.next(), p.dot([10, 20], c.primary), { runTime: Math.min(1.2, this.cue("p")) });
            this.play(Indicate(one.eq), { runTime: 1.5 });
            this.narrate(4);
            const more = k.Steps(["r = 0 arrow c = 35", "r = 20 arrow c = 5"]);
            for (const pt of [[0, 35], [20, 5]]) this.play(more.next(), p.dot(pt, c.primary), { runTime: 1.4 });
            this.play(p.line((r) => 35 - 1.5 * r, c.primary), { runTime: 2.0 });
            this.narrate(5);
            this.play(work.clear(), { runTime: 0.8 });
            this.play(p.line((r) => 22.5 - 0.25 * r, c.secondary), { runTime: 2.0 });
            this.play(p.probe((r) => 22.5 - 0.25 * r, 0, 20), { runTime: 1.8 });
            this.narrate(6);
            const ans = work.push(k.AnswerBox("r = 10, quad c = 20", "notebook and pen box"));
            this.cue("x");
            this.play(p.crossing([10, 20]), ans.enter(), { runTime: 1.5 });
            this.play(Indicate(ans), { runTime: 1.5 });
            this.narrate(7);
            const card = k.StatementCard("Solving = finding the points common to all the lines.", "common");
            this.play(card.enter(), { runTime: 1.0 });
            this.play(Indicate(ans), { runTime: 2.0 });"#;
        for style in 0..crate::style::STYLES.len() {
            let found = critiqued_in(&format!("kit{style}"), say, &lines, body, style);
            assert!(found.is_empty(), "style {style}: {found:#?}");
        }
    }

    /// Brief texts, tiny texts and values shown before they are said, each with its line.
    #[test]
    fn brief_small_and_off_cue_texts_are_found() {
        let say = "We start with the line. Then the value is {v}twenty.";
        let body = r#"
            this.narrate(1);
            const a = work.push(MathTex("c = 20"));
            this.play(FadeIn(a), { runTime: 0.5 });
            this.play(FadeOut(a), { runTime: 0.3 });
            this.narrate(2);
            const t = work.push(Text("tiny print here", { fontSize: 9 }));
            this.play(FadeIn(t), { runTime: 0.5 });
            this.cue("v");
            this.play(Indicate(t), { runTime: 1.0 });"#;
        let found = critiqued("many", say, &[2.5, 2.5], body);
        let of = |kind: &str| found.iter().filter(|(k, _)| k == kind).map(|(_, m)| m.as_str()).collect::<Vec<_>>();
        let brief = of("too-brief");
        assert!(brief.len() == 1 && brief[0].contains("\"c = 20\"") && brief[0].contains("only 0.5 s") && brief[0].ends_with("(line 8)"), "{found:?}");
        let small = of("too-small");
        assert!(small.len() == 1 && small[0].contains("\"tiny print here\"") && small[0].ends_with("(line 12)"), "{found:?}");
        let off = of("off-cue");
        assert!(off.len() == 1 && off[0].contains("\"c = 20\"") && off[0].contains("before the voice says \"twenty\""), "{found:?}");
        assert!(of("dead-air").is_empty(), "{found:?}");
    }

    /// An empty group on screen (figure labels nobody filled) has no bounding box, which
    /// must not throw off the walk through what is shown: the text is still found.
    #[test]
    fn an_empty_group_on_screen_does_not_hide_text_from_the_critic() {
        let say = "A sentence about nothing in particular that goes on for a bit.";
        let body = r#"
            const empty = VGroup();
            this.add(empty);
            const t = work.push(Text("tiny print here", { fontSize: 9 }));
            this.play(FadeIn(t), { runTime: 0.5 });
            this.play(Indicate(t), { runTime: 2.5 });"#;
        let found = critiqued("empty", say, &[4.0], body);
        assert!(found.iter().any(|(k, m)| k == "too-small" && m.contains("tiny print here")), "{found:?}");
    }

    /// Like the preview thread: one scene after another on the same thread, retimed.
    #[test]
    fn sequential_retimed_scenes_on_one_thread() {
        let dir = std::env::temp_dir().join(format!("studio-retime-seq-{}", std::process::id()));
        let scenes = crate::sample::install(&dir).unwrap();
        let out = Output::new(320, 180, 30.0);
        for path in scenes.iter().take(3) {
            let req = OpenScene { path: path.clone(), root: dir.clone(), theme_json: crate::style::json(0).into(), scene: None, globals: vec![], narration: Some(6.0), narration_lines: vec![], narration_cues: vec![], critic: None };
            let (host, _, t) = load(&req, out).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
            // Raw engine seeks, backwards and forwards: the sample's scene 2 creates its
            // tracker and redrawn mobjects mid-scene (fixed in manim-rust 3deae09).
            for f in [0.9, 0.2, 0.5, 0.05, 0.9] {
                host.frame_at(t.duration * f).unwrap_or_else(|e| panic!("{} at {f}: {e:#}", path.display()));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
