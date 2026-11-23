//! Prompts. Token efficiency is designed in (DESIGN.md §6):
//!
//! - A short system prompt of our own replaces the CLI's coding-agent default. It holds
//!   the rules and a comment-free digest of the scene API, and is the same file every
//!   turn, so the provider's prompt cache serves it after the first call.
//! - Planning sends no API at all and asks for a small JSON storyboard.
//! - Edits name the one scene file in scope; repairs send only the failing line and a
//!   one-line diagnostic.

use serde::{Deserialize, Serialize};

/// The scene API declarations without comments or blank lines.
pub fn api_digest(types: &str) -> String {
    let mut out = String::with_capacity(types.len() / 2);
    let mut in_block = false;
    for line in types.lines() {
        let t = line.trim();
        if in_block {
            if t.contains("*/") {
                in_block = false;
            }
            continue;
        }
        if t.starts_with("/*") {
            in_block = !t.contains("*/");
            continue;
        }
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        // Inline doc comments after code.
        let code = match t.find(" /**") {
            Some(i) => t[..i].trim_end(),
            None => t,
        };
        out.push_str(code);
        out.push('\n');
    }
    out
}

/// How the API digest is written into the system prompt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DigestKind {
    /// Declarations without comments.
    #[default]
    Full,
    /// Also without `export`/`declare`/`readonly` keywords.
    Compact,
}

/// The prompt knobs the token benchmark (`tools/tokenbench`) compares. The app uses
/// `Recipe::default()`; change the defaults when the benchmark finds a better recipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Recipe {
    pub digest: DigestKind,
    /// Include a short worked scene in the system prompt.
    pub example: bool,
    /// Build each scene in its own fresh turn instead of all scenes in one turn.
    pub per_scene: bool,
}

impl Default for Recipe {
    fn default() -> Self {
        Self { digest: DigestKind::Full, example: false, per_scene: false }
    }
}

/// The digest for a recipe.
pub fn digest_for(types: &str, kind: DigestKind) -> String {
    let d = api_digest(types);
    match kind {
        DigestKind::Full => d,
        DigestKind::Compact => d
            .lines()
            .map(|l| {
                let mut l = l;
                for p in ["export ", "declare ", "readonly "] {
                    l = l.strip_prefix(p).unwrap_or(l);
                }
                l.replace(" readonly ", " ")
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// A worked scene showing the house idioms: palette, heading, roles, pacing to the
/// narration, the thumbnail mark.
pub const EXAMPLE_SCENE: &str = r#"import { Scene, Text, MathTex, Circle, Line, Create, Write, FadeIn, DOWN, RIGHT } from "manim";
import { palette } from "../lib/kit";
import { heading, work, figure } from "../lib/layout";

export default class Diameter extends Scene {
  construct() {
    const c = palette(this.theme);
    // 1. "Here is a circle."  2. "The radius runs from the centre to the {r}edge."
    // 3. "The diameter is twice the radius."
    this.play(Write(heading("Radius and diameter")), { runTime: 1 });
    const circle = figure.place(Circle(2, { color: c.primary }));
    this.play(Create(circle), { runTime: this.narrate(1) });
    const r = Line(circle.getCenter(), circle.getCenter().add(RIGHT.mul(2)), { color: c.accent });
    this.play(Create(r), { runTime: this.narrate(2) });
    const label = MathTex("r", { color: c.accent }).nextTo(r, DOWN, 0.15);
    this.play(FadeIn(label), { runTime: Math.min(0.4, this.cue("r")) });
    const rule = work.push(Text("The diameter is twice the radius", { role: "body" }));
    const eq = work.push(MathTex("d = 2 r"));
    this.play(Write(rule), Write(eq), { runTime: this.narrate(3) });
    this.mark("thumbnail");
    this.wait(1);
  }
}"#;

/// The narration's sentences, as the voice speaks them and `this.narrate(n)` numbers them.
pub use studio_script::sentences;

/// `1. "…"  2. "…"` for a prompt, with the cue marks the scene can wait for.
pub fn numbered(text: &str) -> String {
    studio_script::marked_sentences(text)
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. \"{}\"", i + 1, studio_script::with_marks(s)))
        .collect::<Vec<_>>()
        .join("  ")
}

/// System prompt for building and editing with a recipe.
pub fn build_system_with(digest: &str, recipe: Recipe) -> String {
    let mut s = build_system(digest);
    if recipe.example {
        s.push_str("\nExample scene (scenes/02-diameter.ts):\n");
        s.push_str(EXAMPLE_SCENE);
        s.push('\n');
    }
    s
}

/// The source line a diagnostic points at, for a repair prompt: `  12 | code`.
pub fn repair_excerpt(path: &std::path::Path, line: Option<u32>) -> Option<String> {
    let l = line?;
    let src = std::fs::read_to_string(path).ok()?;
    let code = src.lines().nth(l.checked_sub(1)? as usize)?;
    Some(format!("  {l} | {}", code.trim_end()))
}

/// System prompt for building and editing (written to `AGENTS.md` in the video folder).
pub fn build_system(digest: &str) -> String {
    format!(
        r#"You build and edit scenes for Manim Studio, which teachers use to make short narrated lesson videos.

The current folder is one video:
- scenes/NN-slug.ts: one scene per file, played in file-name order. Each file is
  `export default class Name extends Scene {{ construct() {{ ... }} }}`.
- lib/kit.ts: helpers. `import {{ palette, card }} from "../lib/kit";`
  palette(this.theme) gives colours; card(content, fill, stroke) boxes content.
- lib/layout.ts: where things go (Studio keeps it current; don't edit it). `import {{ heading, work, figure, full }} from "../lib/layout";`
- lib/lesson.ts: the pieces lessons are made of (Studio keeps it current; don't edit it). `import {{ lesson }} from "../lib/lesson";` then `const k = lesson(this);` (k.c is the palette). Play what their methods return.
  k.EquationCard(tex, colour, tag): an equation card; work.push it, then .enter(), .focus(), .dim().
  k.Steps([tex, ...]): numbered worked steps; `this.play(steps.next())` pushes the next one into work.
  k.AnswerBox(tex, sub?, colour?): the final answer in a green box; work.push it, then .enter().
  k.Callout(text, sub?): one idea in a line; work.push it, then .enter().
  `const p = k.Plot({{ xRange: [0, 40], yRange: [0, 40], step: 10, xLabel, yLabel }})` fills figure: p.show(); p.line(f, colour, {{ dashed }}) draws y = f(x) end to end (the same f drawn again with dashed: true lands exactly on the first); p.dot([x, y], colour, label?) pops; p.crossing([x, y], colour?) pulses a dot twice and marks it on both axes; p.slopeTriangle(f, x, run, label); p.probe(f, from, to, readout?) slides a ring along the line with a live readout, p.probeTo([x, y]) moves it off, p.probeOut(); p.push([x, y]) pushes the camera in on that point (the figure zooms inside its panel, everything else stays put) and p.pull() goes back, one push per moment worth lingering on; p.at([x, y]) is the scene point.
  k.StatementCard(text, accentWord?): a full-width card over the dimmed stage; .enter(), .exit().
  k.StrikeOut(m) crosses out a mistake; k.PauseCard(seconds) (play it without a runTime); k.CaseTable(rows) with table.next().
  Build lesson content from these pieces; draw other shapes only for what they don't cover.
- lib/art.ts (yours to create): SVG pictures used by more than one scene, as functions taking the palette.
- assets/: images, SVGs and sounds, referenced by relative path.

Rules:
- Scenes are TypeScript run by manim-rust: the Manim Community API in camelCase. Import only from "manim" and relative files. No DOM, Node, fs or network.
- Never hard-code colours. `const c = palette(this.theme);` then c.primary, c.secondary, c.accent, c.positive, c.negative, c.text, c.muted, c.key_fill, c.background. Text uses roles: Text("…", {{ role: "title" | "heading" | "body" | "caption" | "hand" }}).
- MathTex takes Typst math, not LaTeX: "A = pi r^2", "1/2 dot b h", "sqrt(x)", "\"cm\"^2", "integral_0^1 x dif x".
- Place content only through regions. Never pass coordinates for text (no moveTo, shift, toEdge or toCorner with numbers or vectors). heading(text) is the title, in a band above the left column. work.push(m) stacks equations, worked steps and answers down the left column, each below the last; it holds about seven lines, so when a new part of the working starts, `this.play(work.clear())` fades the stack out and the next push starts at the top again. figure is the right-hand panel: `figure.axes({{ xRange, yRange, numbers: true, xLabel, yLabel }})` gives axes that fill it, with their titles in `ax.labels` (show them with the axes; use c2p for points), `figure.place(m)` centres any other drawing. `full.place(m)` is a full-width statement card. A region shrinks what is too big for it. A tick or note beside a step goes in with it: `work.push(VGroup(step, note).arrange(RIGHT))`. A label on a drawing goes beside what it labels with nextTo. Studio checks after every animation that everything is inside its region.
- Text must not overlap other text once an animation ends, and must not cross-fade over other text. When new text takes the place of old text, FadeOut or Transform the old one in the same play() call; place new text with nextTo/arrange/VGroup relative to what is on screen, never at a position something else already uses.
- Text in a box: card(Text(...), fill, stroke) sizes the box to the text. Never move text onto a fixed-size Rectangle. A label beside an icon, tick or shape goes there with nextTo(it, RIGHT, {{ buff: 0.25 }}), never at its own hard-coded position.
- Draw pictures (people, desks, devices, documents, icons, objects) as one SVG, not as piles of Rectangles and Circles: `SVGMobject(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 90">…</svg>`, {{ height: 2 }})` with colours from the palette (`fill="${{c.primary}}"`). Use rect, circle, ellipse, line, polyline, polygon and path with flat fills and strokes; no gradients, filters, images or <text> (labels are Text objects placed beside the picture, so they follow the house style). To point at a picture, scale it briefly (`pic.animate.scale(1.1)` and back) or draw SurroundingRectangle around it; Indicate and colour changes paint every part of it one colour.
- Call this.mark("thumbnail") at the scene's most representative moment.
- Each scene is narrated, sentence by sentence (numbered in the storyboard). Call `this.narrate(n)` immediately before the animation that sentence n describes, for every sentence, in order. It returns that sentence's length in seconds (or the seconds until its first cue); use it for the runTime of what it describes. If the previous sentence is still being spoken, narrate() first waits for it, so the voice always matches what is on screen. Studio holds the last frame until the last sentence ends. Don't time anything off `NARRATION.seconds`.
- A `{{id}}` inside the narration is a cue: the moment the next word is spoken. Call `this.cue(id)` immediately before the animation for that moment, for every cue, in order. Never show a value before the voice says it. cue() waits for that moment and returns the seconds until the next cue or the end of the sentence, so keep that animation no longer: a pop takes `Math.min(0.3, this.cue("c20"))`.
- Pace for the class: let narrate() and cue() set the pace; add this.wait(seconds) only for a short beat after a key result.
- The scenes play back to back as one continuous video, with a short dissolve between them. Don't open a scene with the video title or a scene number; carry on from the previous scene (reuse its heading wording, colours and positions for recurring objects), and end on the scene's key result rather than fading everything out.
- Change only the files you are asked to change. Don't open other scenes unless asked.
- Work in as few steps as possible: write several files at once with parallel tool calls, and don't read files back after writing them.
- Studio runs every scene through the engine after you finish and sends back any error, so don't try to run or test anything yourself.
- Reply to the teacher in one or two plain sentences. No code in replies.

Scene API (TypeScript declarations):
{digest}"#
    )
}

/// System prompt for planning (no API: planning never writes code).
pub const PLAN_SYSTEM: &str = "You plan short narrated lesson videos for teachers as storyboards. \
You never write code. You reply with JSON only, no prose, no code fences.";

/// One beat of a storyboard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Beat {
    pub title: String,
    /// What the narrator says.
    pub say: String,
    /// What appears on screen, for the assistant that writes the scene (drawing terms).
    pub visual: String,
    /// The same, in plain words for the teacher reading the script.
    #[serde(default)]
    pub show: String,
    pub seconds: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Storyboard {
    pub title: String,
    #[serde(default)]
    pub grade: String,
    pub scenes: Vec<Beat>,
    /// Outline and key facts from the teacher's documents, condensed once and reused by
    /// later turns instead of the documents themselves.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_notes: String,
}

pub struct PlanOptions<'a> {
    pub request: &'a str,
    /// Summaries of attached documents, already condensed.
    pub sources: &'a [String],
    /// Target length in seconds.
    pub target_seconds: u32,
    pub style_name: &'a str,
    /// How to explain (intuitive, step by step, ...), as an instruction. Empty: default.
    pub approach: &'a str,
    /// Who it is for, e.g. "High school (ages 14 to 18)". Empty: infer from the request.
    pub level: &'a str,
    /// Narration speed in spoken words per second (about 2.5 is normal).
    pub words_per_second: f32,
    /// How to end, as an instruction. Empty: one practice question.
    pub ending: &'a str,
}

impl PlanOptions<'_> {
    /// A sensible scene count range for the target length.
    pub fn scene_range(&self) -> (u32, u32) {
        let t = self.target_seconds as f32;
        let lo = (t / 45.).round().clamp(2., 8.) as u32;
        let hi = ((t / 20.).round() as u32).clamp(lo + 1, 12);
        (lo, hi)
    }
}

pub fn plan(o: &PlanOptions) -> String {
    let mut p = format!(
        "Plan a lesson video of about {} seconds in the \"{}\" house style.\n\nTeacher's request:\n{}\n",
        o.target_seconds,
        o.style_name,
        o.request.trim()
    );
    if !o.level.trim().is_empty() {
        p.push_str(&format!("\nAudience: {}. Pitch the language, examples and difficulty for them.\n", o.level.trim()));
    }
    if !o.approach.trim().is_empty() {
        p.push_str(&format!("\nApproach: {}\n", o.approach.trim()));
    }
    for (i, s) in o.sources.iter().enumerate() {
        p.push_str(&format!("\nSource {} (the teacher's material; follow it):\n{}\n", i + 1, s.trim()));
    }
    let (lo, hi) = o.scene_range();
    let wps = if o.words_per_second > 0. { (o.words_per_second * 10.).round() / 10. } else { 2.5 };
    let ending = if o.ending.trim().is_empty() { "End with one practice question." } else { o.ending.trim() };
    let notes = if o.sources.is_empty() {
        ""
    } else {
        r#", "source_notes": "the sources condensed for later steps: an outline and the key facts, definitions, numbers and examples, at most 250 words""#
    };
    p.push_str(&format!(
        r#"
Reply with only this JSON:
{{"title": "short video title", "grade": "e.g. Grade 8", "scenes": [{{"title": "scene title", "say": "the narration for this scene, in the teacher's voice", "show": "one plain sentence a teacher would use to say what the viewer sees, no code or drawing terms", "visual": "what is drawn and animated, concretely", "seconds": 20}}]{notes}}}
Use {lo} to {hi} scenes. Start with a hook and build the idea with visuals that can be drawn with shapes, graphs, text and equations. {ending} Narration is read aloud by a voice: write it as speech ("pi r squared", "x over two"), no symbols, markup or LaTeX. The one exception is cue marks: wherever something appears on screen in the middle of a sentence (a dot, a number, a named term), put a short id in braces directly before the word that names it, as in "If r is ten, c is {{c20}}twenty." A mark goes before the number itself, is never read aloud, and has an id of its own in that scene, starting with a letter. Say in "visual" what appears at each cue. The narration sets each scene's length, so it must fill the scene: about {wps} spoken words per second ("seconds": 20 means about {words20} words), talking through what appears on screen as it appears. The scenes are one continuous video: link each scene's narration to the one before it."#,
        lo = lo,
        hi = hi,
        ending = ending,
        wps = wps,
        words20 = (wps * 20.).round() as u32,
    ));
    p
}

/// Revise a storyboard from a teacher's note (planning model, no code).
pub fn revise(current: &Storyboard, note: &str) -> String {
    format!(
        "Revise this storyboard. Keep the {{id}} cue marks in the narration with the words they precede. Teacher's note: {}\n\nCurrent storyboard:\n{}\n\nReply with only the revised storyboard JSON in the same shape.",
        note.trim(),
        serde_json::to_string(current).unwrap_or_default()
    )
}

/// Pull the storyboard JSON out of a model reply (tolerates prose or fences around it).
pub fn parse_storyboard(text: &str) -> anyhow::Result<Storyboard> {
    let start = text.find('{').ok_or_else(|| anyhow::anyhow!("the plan had no JSON"))?;
    let end = text.rfind('}').filter(|e| *e > start).ok_or_else(|| anyhow::anyhow!("the plan had no JSON"))?;
    let sb: Storyboard = serde_json::from_str(&text[start..=end])?;
    if sb.scenes.is_empty() {
        anyhow::bail!("the plan had no scenes");
    }
    Ok(sb)
}

/// File name for scene `i` (0-based): `scenes/03-unroll-the-rings.ts`.
pub fn scene_file(i: usize, title: &str) -> String {
    let mut slug = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.len() >= 28 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    format!("scenes/{:02}-{}.ts", i + 1, if slug.is_empty() { "scene" } else { slug })
}

/// Where the documents' full text lives, for details the notes leave out. Read on demand
/// only, so the notes stay the cheap default.
const SOURCES_HINT: &str = "The full text of the documents is in sources/*.md. Read a file there only if a scene needs a detail the notes lack.\n";

/// Build every scene of an approved storyboard.
pub fn build(sb: &Storyboard) -> String {
    let mut p = format!("Build the approved storyboard \"{}\" ({}). Write one file per scene:\n", sb.title, sb.grade);
    if !sb.source_notes.trim().is_empty() {
        p.push_str(&format!("\nThe teacher's source notes (keep facts and numbers consistent with them):\n{}\n", sb.source_notes.trim()));
        p.push_str(SOURCES_HINT);
    }
    for (i, b) in sb.scenes.iter().enumerate() {
        p.push_str(&format!(
            "\n{}  ({} s) {}\n  Narration: {}\n  On screen: {}\n",
            scene_file(i, &b.title),
            b.seconds,
            b.title,
            numbered(&b.say),
            b.visual
        ));
    }
    p.push_str("\nWrite all the files in one step (parallel Write calls), then reply with one sentence.");
    p
}

/// Build one scene of an approved storyboard in its own turn (`Recipe::per_scene`). The
/// other scene titles give it the arc without their details.
pub fn build_scene(sb: &Storyboard, i: usize) -> String {
    let b = &sb.scenes[i];
    let titles: Vec<String> = sb.scenes.iter().enumerate().map(|(k, s)| format!("{}. {}", k + 1, s.title)).collect();
    let mut p = format!(
        "Video \"{}\" ({}), scenes: {}.\n\nWrite {} only  ({} s) {}\n  Narration: {}\n  On screen: {}\n",
        sb.title,
        sb.grade,
        titles.join("; "),
        scene_file(i, &b.title),
        b.seconds,
        b.title,
        numbered(&b.say),
        b.visual
    );
    if !sb.source_notes.trim().is_empty() {
        p.push_str(&format!("\nSource notes (keep facts consistent):\n{}\n", sb.source_notes.trim()));
        p.push_str(SOURCES_HINT);
    }
    p.push_str("\nWrite the file in one step, then reply with one sentence.");
    p
}

/// A teacher's change request, scoped to one scene when one is selected. `assets`
/// lists files in assets/ the scene may use.
pub fn edit(request: &str, scope: Option<(&str, &str, &str)>, assets: &[String]) -> String {
    let mut p = match scope {
        Some((file, title, say)) if !say.trim().is_empty() => format!(
            "{}\n\nScope: {file} ({title}). Change only this file. Its narration: {}",
            request.trim(),
            numbered(say)
        ),
        Some((file, title, _)) => format!("{}\n\nScope: {file} ({title}). Change only this file.", request.trim()),
        None => format!("{}\n\nScope: the scenes/ folder. Change as few files as possible.", request.trim()),
    };
    if !assets.is_empty() {
        p.push_str(&format!("\nAssets available (ImageMobject / SVGMobject / addSound with these paths): {}", assets.join(", ")));
    }
    p
}

/// One picture for the visual review: a frame of a chapter, and the script that draws it.
pub struct ReviewFrame {
    /// Path of the picture, relative to the video folder.
    pub image: String,
    /// Path of the chapter's script, relative to the video folder.
    pub file: String,
    pub chapter: usize,
    /// Where in the chapter the frame is, in seconds.
    pub at: f64,
}

/// Ask the assistant to look at how the finished chapters look and fix what is wrong. The
/// frames are pictures it opens with its file reader.
pub fn review(frames: &[ReviewFrame]) -> String {
    let mut p = String::from(
        "Look at how the video looks. Open each picture below (they are frames of the finished chapters, as a viewer sees them) and check:\n\
- text overlapping other text, a drawing or a label\n\
- anything cut off by, or touching, the edge of the frame\n\
- text too small to read from the back of a classroom\n\
- crowded or uneven spacing, or a label that is not next to what it labels\n\n",
    );
    for f in frames {
        p.push_str(&format!("{}  chapter {} at {:.1} s  ({})\n", f.image, f.chapter, f.at, f.file));
    }
    p.push_str(
        "\nFix each problem with the smallest change to that chapter's script file, placing things through the layout regions (heading, work.push, figure, full) and nextTo instead of coordinates. Leave the narration, timing and story as they are, and leave chapters that look clean alone. Reply with one sentence: what you fixed, or \"Looks clean.\"",
    );
    p
}

/// One scene that failed the engine check.
pub struct Failure {
    /// Path relative to the video folder.
    pub file: String,
    pub diagnostic: String,
    /// The source lines involved (`  12 | code`), when known.
    pub excerpt: Option<String>,
}

/// Most problems of one scene a repair is told about at once.
const PROBLEMS_PER_REPAIR: usize = 4;

/// A scene's check problems (message, scene time) as one diagnostic: the first few, then
/// how many more, so one repair can fix them together.
pub fn problems(found: &[(&str, f64)]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    for (message, time) in found {
        if seen.contains(message) {
            continue;
        }
        seen.push(message);
        if parts.len() < PROBLEMS_PER_REPAIR {
            parts.push(format!("{message} at {time:.1} s"));
        }
    }
    let more = seen.len().saturating_sub(parts.len());
    let mut out = parts.join("; also ");
    if more > 0 {
        out.push_str(&format!(" (and {more} more like these)"));
    }
    out
}

/// Every failed check of a turn, sent back in one repair turn with only what is needed.
pub fn repair_all(failures: &[Failure]) -> String {
    let mut p = String::new();
    for f in failures {
        p.push_str(&format!("{} fails the engine check: {}\n", f.file, f.diagnostic.trim()));
        if let Some(x) = &f.excerpt {
            p.push_str(x.trim_end());
            p.push('\n');
        }
    }
    p.push_str(if failures.len() > 1 {
        "Fix each with the smallest change (edit the files in parallel). Reply with one sentence."
    } else {
        "Fix it with the smallest change. Reply with one sentence."
    });
    p
}

/// The lines creating the texts a layout problem names (`text "A" overlaps "B"`).
pub fn layout_excerpt(path: &std::path::Path, message: &str) -> Option<String> {
    let quoted: Vec<&str> = message.split('"').skip(1).step_by(2).filter(|q| !q.trim().is_empty()).collect();
    if quoted.is_empty() {
        return None;
    }
    let src = std::fs::read_to_string(path).ok()?;
    let lines: Vec<String> = src
        .lines()
        .enumerate()
        .filter(|(_, l)| quoted.iter().any(|q| l.contains(q)))
        .take(8)
        .map(|(i, l)| format!("  {} | {}", i + 1, l.trim()))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_drops_comments() {
        let d = api_digest("// header\n/** doc */\nexport const A: number;\n/**\n * long\n */\nexport function f(): void; /** trailing */\n\n");
        assert_eq!(d, "export const A: number;\nexport function f(): void;\n");
    }

    #[test]
    fn parses_storyboard_with_noise() {
        let t = "Here you go:\n```json\n{\"title\":\"T\",\"grade\":\"G\",\"scenes\":[{\"title\":\"A b!\",\"say\":\"s\",\"visual\":\"v\",\"seconds\":12}],\"source_notes\":\"n\"}\n```";
        let sb = parse_storyboard(t).unwrap();
        assert_eq!(sb.scenes[0].seconds, 12);
        assert_eq!(sb.source_notes, "n");
        assert_eq!(scene_file(0, &sb.scenes[0].title), "scenes/01-a-b.ts");
    }
}

#[cfg(test)]
mod plan_option_tests {
    use super::*;

    fn opts(seconds: u32) -> PlanOptions<'static> {
        PlanOptions {
            request: "area of a circle",
            sources: &[],
            target_seconds: seconds,
            style_name: "Slate",
            approach: "Keep the theory brief.",
            level: "High school (ages 14 to 18)",
            words_per_second: 2.9,
            ending: "End with a short recap of the key points. No practice question.",
        }
    }

    #[test]
    fn options_reach_the_plan_prompt() {
        let p = plan(&opts(120));
        assert!(p.contains("Audience: High school (ages 14 to 18)"));
        assert!(p.contains("Approach: Keep the theory brief."));
        assert!(p.contains("No practice question."));
        assert!(p.contains("about 2.9 spoken words per second"));
        assert!(p.contains("about 58 words"));
        assert!(!p.contains("End with one practice question"));
    }

    #[test]
    fn a_repair_hears_about_several_problems() {
        let found = [("a", 1.0), ("b", 2.0), ("a", 3.0), ("c", 4.0), ("d", 5.0), ("e", 6.0), ("f", 7.0)];
        assert_eq!(problems(&found), "a at 1.0 s; also b at 2.0 s; also c at 4.0 s; also d at 5.0 s (and 2 more like these)");
        assert_eq!(problems(&found[..1]), "a at 1.0 s");
    }

    #[test]
    fn cues_reach_the_prompts() {
        assert!(plan(&opts(120)).contains("c is {c20}twenty"));
        let system = build_system("");
        assert!(system.contains("A `{id}` inside the narration is a cue"));
        assert!(system.contains("Never show a value before the voice says it."));
        assert!(system.contains("Place content only through regions. Never pass coordinates for text"));
        assert!(system.contains("Build lesson content from these pieces"));
        // Marks stay visible to the builder and don't change the numbering.
        assert_eq!(
            numbered("If r is ten, c is {c20}twenty. {line}They lie on one line."),
            "1. \"If r is ten, c is {c20}twenty.\"  2. \"{line}They lie on one line.\""
        );
    }

}
