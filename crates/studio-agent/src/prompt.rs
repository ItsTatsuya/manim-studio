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
import { palette, heading } from "../lib/kit";

export default class Diameter extends Scene {
  construct() {
    const c = palette(this.theme);
    // 1. "Here is a circle."  2. "The radius runs from the centre to the edge."
    // 3. "The diameter is twice the radius."
    this.play(Write(heading("Radius and diameter")), { runTime: 1 });
    const circle = Circle(2, { color: c.primary });
    this.play(Create(circle), { runTime: this.narrate(1) });
    const r = Line(circle.getCenter(), circle.getCenter().add(RIGHT.mul(2)), { color: c.accent });
    const label = MathTex("r", { color: c.accent }).nextTo(r, DOWN, 0.15);
    this.play(Create(r), FadeIn(label), { runTime: this.narrate(2) });
    const rule = Text("The diameter is twice the radius", { role: "body" }).toEdge(DOWN, 1.2);
    const eq = MathTex("d = 2 r").nextTo(rule, DOWN, 0.3);
    this.play(Write(rule), Write(eq), { runTime: this.narrate(3) });
    this.mark("thumbnail");
    this.wait(1);
  }
}"#;

/// A narration line split into sentences, numbered for `this.narrate(n)`. Must match
/// `studio_render::voice::sentences` (checked by a test in tools/tokenbench).
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

/// `1. "…"  2. "…"` for a prompt.
pub fn numbered(text: &str) -> String {
    sentences(text).iter().enumerate().map(|(i, s)| format!("{}. \"{}\"", i + 1, s)).collect::<Vec<_>>().join("  ")
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
- lib/kit.ts: helpers. `import {{ palette, heading, card }} from "../lib/kit";`
  palette(this.theme) gives colours; heading(text) is a top-left slide title; card(content, fill, stroke) boxes content.
- assets/: images, SVGs and sounds, referenced by relative path.

Rules:
- Scenes are TypeScript run by manim-rust: the Manim Community API in camelCase. Import only from "manim" and relative files. No DOM, Node, fs or network.
- Never hard-code colours. `const c = palette(this.theme);` then c.primary, c.secondary, c.accent, c.positive, c.negative, c.text, c.muted, c.key_fill, c.background. Text uses roles: Text("…", {{ role: "title" | "heading" | "body" | "caption" | "hand" }}).
- MathTex takes Typst math, not LaTeX: "A = pi r^2", "1/2 dot b h", "sqrt(x)", "\"cm\"^2", "integral_0^1 x dif x".
- The frame is 14.2 × 8 units, origin at the centre, y up. Keep text within x ±6.5, y ±3.6; wrap long text with maxWidth.
- Text must not overlap other text once an animation ends, and must not cross-fade over other text. When new text takes the place of old text, FadeOut or Transform the old one in the same play() call; place new text with nextTo/arrange/VGroup relative to what is on screen, never at a position something else already uses.
- The top band (y > 2.6) is the heading's. Put body content, equations and diagrams below it.
- Call this.mark("thumbnail") at the scene's most representative moment.
- Each scene is narrated, sentence by sentence (numbered in the storyboard). Call `this.narrate(n)` immediately before the animation that sentence n describes, for every sentence, in order. It returns that sentence's length in seconds; use it for the runTime of what it describes. If the previous sentence is still being spoken, narrate() first waits for it, so the voice always matches what is on screen. Studio holds the last frame until the last sentence ends. Don't time anything off `NARRATION.seconds`.
- Pace for the class: let narrate() set the pace; add this.wait(seconds) only for a short beat after a key result.
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
    /// What appears on screen.
    pub visual: String,
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
{{"title": "short video title", "grade": "e.g. Grade 8", "scenes": [{{"title": "scene title", "say": "the narration for this scene, in the teacher's voice", "visual": "what is drawn and animated, concretely", "seconds": 20}}]{notes}}}
Use {lo} to {hi} scenes. Start with a hook and build the idea with visuals that can be drawn with shapes, graphs, text and equations. {ending} Narration is read aloud by a voice: write it as speech ("pi r squared", "x over two"), no symbols, markup or LaTeX. The narration sets each scene's length, so it must fill the scene: about {wps} spoken words per second ("seconds": 20 means about {words20} words), talking through what appears on screen as it appears. The scenes are one continuous video: link each scene's narration to the one before it."#,
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
        "Revise this storyboard. Teacher's note: {}\n\nCurrent storyboard:\n{}\n\nReply with only the revised storyboard JSON in the same shape.",
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

/// One scene that failed the engine check.
pub struct Failure {
    /// Path relative to the video folder.
    pub file: String,
    pub diagnostic: String,
    /// The source lines involved (`  12 | code`), when known.
    pub excerpt: Option<String>,
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
        .take(4)
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
    fn scene_count_scales_with_length() {
        assert_eq!(opts(30).scene_range(), (2, 3));
        assert_eq!(opts(120).scene_range(), (3, 6));
        assert_eq!(opts(360).scene_range(), (8, 12));
    }

    #[test]
    fn empty_options_keep_the_old_defaults() {
        let o = PlanOptions { approach: "", level: "", ending: "", words_per_second: 0., ..opts(60) };
        let p = plan(&o);
        assert!(!p.contains("Audience:"));
        assert!(p.contains("End with one practice question."));
        assert!(p.contains("about 2.5 spoken words per second"));
    }
}
