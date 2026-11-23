//! Prompts and checks for a teacher's own script (`studio_script::teacher`). The teacher
//! wrote the narration, the colours, the axes and the equations; Studio reads them out of
//! the document instead of asking a model to summarise them, so they reach the builder as
//! they were written. The planner is asked for one thing, the `{id}` cue marks, and what it
//! sends back is checked: a scene whose words changed gets the teacher's words back.

use anyhow::{Result, bail};
use serde::Deserialize;
use studio_script::teacher::{Note, ScriptScene, TeacherScript, clock, requested_notes, same_words};

use crate::prompt::{Beat, Storyboard};

/// The frame is this many units wide (`design/polish-brief.md`, section 2).
const FRAME_WIDTH: f32 = 14.2;

/// The beats of a script, as the teacher wrote them: narration word for word, and a
/// `visual` that carries the on-screen column, the axes, the points and the end card for
/// the builder. `logo` is the logo's path in the video's assets, when it was attached, and
/// `applying` the numbers of the reviewer notes the teacher asked for.
pub fn draft(script: &TeacherScript, logo: Option<&str>, applying: &[usize]) -> Storyboard {
    let last = script.scenes.len().saturating_sub(1);
    let scenes = script
        .scenes
        .iter()
        .enumerate()
        .map(|(i, s)| Beat {
            title: s.title.clone(),
            say: s.say(),
            visual: visual(script, s, (i == last).then_some(logo)),
            show: if s.shows.is_empty() { s.rows.first().map(|r| r.action.clone()).unwrap_or_default() } else { s.shows.clone() },
            seconds: ((s.end - s.start).round() as u32).max(5),
        })
        .collect();
    Storyboard { title: title(&script.title), grade: String::new(), scenes, source_notes: source_notes(script, logo, applying) }
}

/// "Video 1.01 – Two Equations: What Does Solving Mean?" as a short video title.
fn title(heading: &str) -> String {
    let t = heading.split_once(['–', '—']).map_or(heading, |(_, t)| t);
    t.trim().to_string()
}

/// What the builder is told for one scene. `end` is `Some` on the last scene (with the logo's
/// path if there is one).
fn visual(script: &TeacherScript, s: &ScriptScene, end: Option<Option<&str>>) -> String {
    let mut out = String::new();
    if !s.shows.is_empty() {
        out.push_str(&format!("{}. ", s.shows.trim_end_matches('.')));
    }
    // The on-screen column, by the sentences it goes with.
    let mut done = 0;
    for (k, row) in s.rows.iter().enumerate() {
        let said: Vec<&str> = s.rows[..=k].iter().map(|r| r.say.as_str()).collect();
        let upto = studio_script::sentences(&said.join(" ")).len();
        let at = match upto.saturating_sub(done) {
            0 | 1 => format!("sentence {upto}"),
            _ => format!("sentences {}-{upto}", done + 1),
        };
        done = upto;
        if !row.action.is_empty() {
            out.push_str(&format!("[{at}] {} ", row.action.trim()));
        }
    }
    let number = s.number();
    for a in script.axes.iter().filter(|a| number.is_some_and(|n| a.scenes.contains(&n))) {
        let range = |r: Option<studio_script::teacher::Range>| r.map_or_else(|| "as drawn".to_string(), |r| r.to_string());
        out.push_str(&format!("Axes{}: x {}; y {}.", if a.label.contains("part") { format!(" ({})", a.label) } else { String::new() }, range(a.x), range(a.y)));
        if !a.lines.is_empty() {
            out.push_str(&format!(" Lines: {}.", a.lines.trim_end_matches('.')));
        }
        out.push(' ');
    }
    if let Some(n) = number {
        for fact in script.facts.iter().flat_map(|f| f.split(". ")).filter(|f| mentions_scene(f, n)) {
            out.push_str(&format!("{}. ", fact.trim().trim_end_matches('.')));
        }
    }
    if let (Some(logo), Some(card)) = (end, &script.brand.end_card) {
        let width = card.logo_width.map(|w| format!(", {:.0}% of the frame's width (about {:.1} units)", w * 100., w * FRAME_WIDTH)).unwrap_or_default();
        let text = if card.text.is_empty() { String::new() } else { format!(" Under it the text “{}” in the heading role.", card.text) };
        match logo {
            Some(path) => out.push_str(&format!("End card, drawn exactly like this: ImageMobject(\"{path}\") at the centre{width}.{text} The watermark is not drawn on this scene. ")),
            None => out.push_str(&format!("End card: the logo file wasn't attached, so draw only the text.{text} ")),
        }
    }
    out.trim().to_string()
}

/// Whether a sentence of the facts names scene `n` (`S03`).
fn mentions_scene(fact: &str, n: u32) -> bool {
    fact.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w.strip_prefix(['S', 's']).and_then(|d| d.parse::<u32>().ok()) == Some(n))
}

/// The script's colours, axes, equations and brand as fixed facts for every build turn, and
/// the reviewer notes the teacher asked for. Written in code, not condensed by a model, so a
/// revision can't drift from what the teacher gave.
pub fn source_notes(script: &TeacherScript, logo: Option<&str>, applying: &[usize]) -> String {
    let mut out = String::from("The teacher's script gives these; use them exactly as given.\n");
    if !script.colours.is_empty() {
        out.push_str("Colours (exact hex strings, written in code as plain strings: the exception to the palette rule; use them where the script says, palette() elsewhere):\n");
        for c in &script.colours {
            let note = if c.note.is_empty() { String::new() } else { format!(" ({})", c.note) };
            out.push_str(&format!("- {}: {} {}{note}\n", c.object, c.name, c.hex));
        }
    }
    if !script.axes.is_empty() {
        out.push_str("Axes and lines, by scene:\n");
        for a in &script.axes {
            let range = |r: Option<studio_script::teacher::Range>| r.map_or_else(|| "as drawn".to_string(), |r| r.to_string());
            let lines = if a.lines.is_empty() { String::new() } else { format!(". Lines: {}", a.lines.trim_end_matches('.')) };
            out.push_str(&format!("- {}: x {}; y {}{lines}\n", a.label, range(a.x), range(a.y)));
        }
    }
    if !script.equations.is_empty() {
        out.push_str("Equations as Typst for MathTex, each checked to render. Use these strings, not your own:\n");
        for (i, e) in script.equations.iter().enumerate() {
            match &e.error {
                None => out.push_str(&format!("{}. {}\n", i + 1, e.typst)),
                Some(why) => out.push_str(&format!("{}. LaTeX {} (it didn't convert to a Typst that renders: {why}; write it by hand in Typst)\n", i + 1, e.latex)),
            }
        }
    }
    match (logo, &script.brand.logo) {
        (Some(path), _) => out.push_str(&format!("Brand: the logo is {path}. Studio draws it as the corner watermark on the finished video, so never draw it yourself, except on the end card.\n")),
        (None, Some(name)) => out.push_str(&format!("Brand: the logo {name} was not attached. Studio draws the corner watermark itself; don't draw it.\n")),
        _ => {}
    }
    // Reviewer notes: only the ones the teacher asked for. Their wording changes are already in
    // the narration; what they say about the screen is for the builder.
    let wanted: Vec<&Note> = script.notes.iter().filter(|n| applying.contains(&n.n)).collect();
    if !wanted.is_empty() {
        out.push_str("Reviewer notes the teacher asked for, from someone who watched an earlier render (use the words to find the scene). Do what each says about what is on screen; changes to the wording are already in the narration:\n");
        for n in &wanted {
            out.push_str(&format!("[{}] {}\n", n.n, n.text));
        }
    }
    if script.notes.len() > wanted.len() {
        out.push_str("sources/*.md also holds other reviewer notes the teacher did not ask for: don't act on them.\n");
    }
    out.trim_end().to_string()
}

/// What the planner is asked. It gets each scene's narration and on-screen column and
/// replies with the narration plus cue marks, nothing else. `applying` are the numbers of the
/// reviewer notes the teacher's request asked for; the other notes are not mentioned.
pub fn plan(script: &TeacherScript, draft: &Storyboard, applying: &[usize]) -> String {
    let mut p = String::from(
        "A teacher wrote this lesson script. Its narration is word for word and stays exactly as written. Your one job is to add cue marks to it.\n\n",
    );
    for (i, (s, b)) in script.scenes.iter().zip(&draft.scenes).enumerate() {
        p.push_str(&format!("Scene {} \"{}\" ({}-{})\n  Narration: {}\n  On screen: {}\n", i + 1, s.title, clock(s.start), clock(s.end), b.say, b.visual));
    }
    if !applying.is_empty() {
        p.push_str("\nThe teacher asked for these reviewer notes to be applied. Each is a suggestion from someone who watched an earlier render (the time is in that render, so use the words to find the scene):\n");
        for n in script.notes.iter().filter(|n| applying.contains(&n.n)) {
            p.push_str(&format!("[{}] {} {}\n", n.n, clock(n.at), n.text));
        }
        p.push_str("If a note asks for different words, change the narration of the one scene it is about, only as far as the note says, and list its number in that scene's \"applied\". A note about what appears on screen needs no change here: the builder gets it. Every other word stays as written. Ignore every other note.\n");
    }
    p.push_str(&format!(
        r#"
Reply with only this JSON, with exactly {n} scenes in order:
{{"scenes": [{{"say": "the scene's narration, with cue marks", "applied": []}}]}}
"say" is the scene's narration with the same words in the same order, spacing and punctuation: do not add, drop, reorder or reword anything, and do not correct spelling or grammar. Add cue marks only: wherever the "On screen" text puts something on screen in the middle of a sentence (a dot, a number, an equation, a labelled value, a named term), put a short id in braces directly before the word that names it, as in "If r is ten, c is {{c20}}twenty." A mark goes before the number itself, is never read aloud, and has an id of its own in that scene, starting with a letter. A scene needs a mark only where something appears at a word; leave the others without."#,
        n = script.scenes.len(),
    ));
    p
}

/// One scene of the planner's reply.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Marked {
    pub say: String,
    #[serde(default)]
    pub applied: Vec<usize>,
}

/// The scenes out of the planner's reply (tolerates prose or fences around the JSON).
pub fn parse_marks(text: &str) -> Result<Vec<Marked>> {
    #[derive(Deserialize)]
    struct Reply {
        scenes: Vec<Marked>,
    }
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else { bail!("the reply had no JSON") };
    if end < start {
        bail!("the reply had no JSON");
    }
    Ok(serde_json::from_str::<Reply>(&text[start..=end])?.scenes)
}

/// The storyboard after the planner's reply, with the teacher's wording enforced.
pub struct Merged {
    pub storyboard: Storyboard,
    /// Scenes (1-based) whose words the reply changed and that got the teacher's words back.
    pub restored: Vec<usize>,
    /// Reviewer notes (numbers) that were applied.
    pub applied: Vec<usize>,
}

/// Merge the planner's marks into the draft. A scene keeps the reply's text only when it
/// says the same words as the script, or when the reply lists reviewer notes the teacher
/// asked for; otherwise the script's own words come back, carrying over the marks that
/// still fit.
pub fn merge(draft: Storyboard, reply: &[Marked], applying: &[usize]) -> Merged {
    let mut storyboard = draft;
    let (mut restored, mut applied): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
    for (i, beat) in storyboard.scenes.iter_mut().enumerate() {
        let Some(r) = reply.get(i) else { continue };
        // A note changes the words of one scene: the first that claims it.
        let asked: Vec<usize> = r.applied.iter().copied().filter(|n| applying.contains(n) && !applied.contains(n)).collect();
        if same_words(&beat.say, &r.say) {
            beat.say = r.say.trim().to_string();
        } else if !asked.is_empty() && asked.len() == r.applied.len() {
            beat.say = r.say.trim().to_string();
            applied.extend(asked);
        } else {
            beat.say = studio_script::teacher::carry_marks(&beat.say, &r.say);
            restored.push(i + 1);
        }
    }
    applied.sort_unstable();
    applied.dedup();
    Merged { storyboard, restored, applied }
}

/// The revise prompt for a storyboard made from a teacher's script: the usual one, plus
/// the rule that the teacher's words change only where the note says.
pub fn revise(script: &TeacherScript, current: &Storyboard, note: &str) -> String {
    // The facts a model must not drift on are written back after its reply (`keep_facts`).
    let mut p = crate::prompt::revise(current, note);
    p.push_str("\nThe narration is the teacher's own script, word for word. Change its words only where the note asks, in the scenes it names, and leave every other sentence exactly as it is. Keep the colours, axis ranges, points and brand details in each scene's \"visual\", and \"source_notes\", as they are.");
    let applying = requested_notes(note, &script.notes);
    let wanted: Vec<&Note> = script.notes.iter().filter(|n| applying.contains(&n.n)).collect();
    if !wanted.is_empty() {
        p.push_str("\nThe teacher asked for these reviewer notes (from an earlier render of the video; use the words to find the scene). Apply each to the one scene it is about, as far as it says:\n");
        for n in wanted {
            p.push_str(&format!("[{}] {}\n", n.n, n.text));
        }
    }
    p
}

/// After a revision: the facts a model must not drift on are put back as they were.
pub fn keep_facts(script: &TeacherScript, logo: Option<&str>, applying: &[usize], revised: &mut Storyboard) {
    revised.source_notes = source_notes(script, logo, applying);
}

/// The scenes (1-based) whose words differ between two storyboards of the same script.
pub fn reworded(before: &Storyboard, after: &Storyboard) -> Vec<usize> {
    before.scenes.iter().zip(&after.scenes).enumerate().filter(|(_, (a, b))| !same_words(&a.say, &b.say)).map(|(i, _)| i + 1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_script::teacher::parse;

    /// An invented script in the shape a Word document extracts to.
    const SCRIPT: &str = "\
Video 3.02 – Two Pens, One Bill: Pairs of Equations
Brand | Instruction
Logo file | acme_logo.png (transparent PNG)
Watermark | Logo in the top-left corner, about 6% of screen width, 55% opacity, 30 px margin.
End card | Logo at the centre, 40% of screen width, with “Thanks for watching – Acme” below it.
Scene class | Time | What it shows
S01_Hook | 0:00–0:30 | A bill with two lines of prices
S02_FirstLine | 0:30–1:00 | Axes, two dots, a violet line
S03_EndCard | 1:00–1:15 | The logo and a thank-you line
Colours and objects
Object | Colour
First equation and its line | Violet #7B2CBF
Final answer | Green #57CC99 in a rounded box
Axes for each graph
Scene | x range | y range | Lines
S02 | 0 to 50, step 10 | −5 to 30, step 5 | Violet: y = 20 − 0.5x
Dots in S02: (0, 20), (10, 15). Dots in S01: none.
LaTeX strings
kx + 2y = 3
\\foo{x}
Time | Narration | On-screen action
0:00 | A shop sells pens and pads. Two pens and three pads cost forty rupees. | A bill slides in.
0:15 | If p is five, q is ten. | Dots appear.
0:30 | Take the first bill. Plot it. | Axes appear.
0:45 | The points lie on one line. | A violet line draws itself.
1:00 | Thanks for watching. | The logo fades in at the centre.
Suggestions:
0:20 : Show the word “bill” on screen.
0:50 : Say “line” slowly.
";

    fn script() -> TeacherScript {
        let mut s = parse(SCRIPT).expect("script");
        s.equations[1].error = Some("unknown function".into());
        s
    }

    #[test]
    fn the_draft_keeps_the_teachers_words_and_facts() {
        let s = script();
        let d = draft(&s, Some("assets/acme_logo.png"), &[]);
        assert_eq!(d.title, "Two Pens, One Bill: Pairs of Equations");
        assert_eq!(d.scenes.len(), 3);
        assert_eq!(d.scenes[0].say, "A shop sells pens and pads. Two pens and three pads cost forty rupees. If p is five, q is ten.");
        assert_eq!(d.scenes[0].seconds, 30);
        // The on-screen column goes with its sentences.
        let v = &d.scenes[0].visual;
        assert!(v.starts_with("A bill with two lines of prices. [sentences 1-2] A bill slides in. [sentence 3] Dots appear."), "{v}");
        assert!(v.contains("Dots in S01: none"), "{v}");
        // The axes, lines and points of scene 2, as given, and only there.
        let v = &d.scenes[1].visual;
        assert!(v.contains("Axes: x 0 to 50, step 10; y -5 to 30, step 5. Lines: Violet: y = 20 - 0.5x."), "{v}");
        assert!(v.contains("Dots in S02: (0, 20), (10, 15)") && !v.contains("S01"), "{v}");
        assert!(!d.scenes[0].visual.contains("Axes"));
        // The end card is written into the last scene.
        let v = &d.scenes[2].visual;
        assert!(v.contains("ImageMobject(\"assets/acme_logo.png\") at the centre, 40% of the frame's width (about 5.7 units)"), "{v}");
        assert!(v.contains("“Thanks for watching – Acme”") && v.contains("watermark is not drawn"), "{v}");
        assert!(!d.scenes[1].visual.contains("End card"));
    }

    #[test]
    fn colours_equations_and_brand_reach_the_builder() {
        let n = source_notes(&script(), Some("assets/acme_logo.png"), &[]);
        assert!(n.contains("- First equation and its line: Violet #7B2CBF"), "{n}");
        assert!(n.contains("- Final answer: Green #57CC99 (in a rounded box)"), "{n}");
        assert!(n.contains("1. k x + 2 y = 3"), "{n}");
        assert!(n.contains("2. LaTeX \\foo{x} (it didn't convert to a Typst that renders: unknown function; write it by hand in Typst)"), "{n}");
        assert!(n.contains("the logo is assets/acme_logo.png") && n.contains("never draw it yourself"), "{n}");
        // Without the logo file the end card is text only.
        let d = draft(&script(), None, &[]);
        assert!(d.scenes[2].visual.contains("the logo file wasn't attached"), "{}", d.scenes[2].visual);
        assert!(source_notes(&script(), None, &[]).contains("acme_logo.png was not attached"));
    }

    #[test]
    fn the_planner_is_asked_for_marks_and_sees_only_requested_notes() {
        let s = script();
        let d = draft(&s, None, &[]);
        let p = plan(&s, &d, &[]);
        assert!(p.contains("Narration: A shop sells pens and pads."), "{p}");
        assert!(p.contains("[sentences 1-2] A bill slides in."), "{p}");
        assert!(p.contains("c is {c20}twenty") && p.contains("do not add, drop, reorder or reword anything"), "{p}");
        // No target length, scene count, ending or speech-writing instructions of the open planner.
        for open in ["seconds", "practice question", "Use 2 to", "spoken words per second"] {
            assert!(!p.contains(open), "{open}");
        }
        assert!(!p.contains("reviewer notes") && !p.contains("Show the word"), "{p}");
        let p = plan(&s, &d, &[2]);
        assert!(p.contains("[2] 0:50 Say “line” slowly.") && !p.contains("Show the word"), "{p}");
        assert!(p.contains("list its number in that scene's \"applied\""), "{p}");
    }

    #[test]
    fn the_teachers_wording_is_enforced() {
        let s = script();
        let d = draft(&s, None, &[]);
        let reply = parse_marks(
            "```json\n{\"scenes\": [\
              {\"say\": \"A shop sells pens and pads. Two pens and three pads cost forty rupees. If p is five, q is {q10}ten.\"},\
              {\"say\": \"Take the first bill, then plot it. {pl}Plot it. The points lie on one line.\"},\
              {\"say\": \"Thank you for watching.\", \"applied\": [1]}]}\n```",
        )
        .unwrap();
        assert_eq!(reply.len(), 3);
        let m = merge(d.clone(), &reply, &[]);
        // Scene 1: same words, marks kept.
        assert_eq!(m.storyboard.scenes[0].say, "A shop sells pens and pads. Two pens and three pads cost forty rupees. If p is five, q is {q10}ten.");
        // Scene 2 was reworded: the teacher's words come back, with the mark that still fits.
        assert_eq!(m.storyboard.scenes[1].say, "Take the first bill. {pl}Plot it. The points lie on one line.");
        assert!(same_words(&m.storyboard.scenes[1].say, &d.scenes[1].say));
        // Scene 3 claims a note nobody asked for and changes a word.
        assert_eq!(m.storyboard.scenes[2].say, d.scenes[2].say);
        assert_eq!(m.restored, [2, 3]);
        assert!(m.applied.is_empty());
        // A scene the reply left out keeps the script's words too.
        let m = merge(d.clone(), &reply[..1], &[]);
        assert_eq!(m.storyboard.scenes[2].say, d.scenes[2].say);
    }

    #[test]
    fn requested_notes_may_change_their_scene_and_only_that_one() {
        let s = script();
        let d = draft(&s, None, &[]);
        let reply = vec![
            Marked { say: d.scenes[0].say.clone(), applied: vec![] },
            Marked { say: "Take the first bill. Now plot it. The points lie on one line.".into(), applied: vec![1] },
            Marked { say: "Bye.".into(), applied: vec![9] },
        ];
        let m = merge(d.clone(), &reply, &[1, 2]);
        assert_eq!(m.storyboard.scenes[1].say, "Take the first bill. Now plot it. The points lie on one line.");
        assert_eq!(m.applied, [1]);
        // Note 9 was not asked for (nor does it exist): scene 3 keeps the script's words.
        assert_eq!(m.storyboard.scenes[2].say, d.scenes[2].say);
        assert_eq!(m.restored, [3]);
    }

    #[test]
    fn a_revision_keeps_the_words_and_the_facts() {
        let s = script();
        let d = draft(&s, None, &[]);
        let p = revise(&s, &d, "Make scene 2 gentler");
        assert!(p.contains("Change its words only where the note asks") && !p.contains("Show the word"), "{p}");
        let p = revise(&s, &d, "Apply reviewer note 1");
        assert!(p.contains("[1] Show the word “bill” on screen."), "{p}");
        let mut drifted = d.clone();
        drifted.source_notes = "something else".into();
        keep_facts(&s, None, &[], &mut drifted);
        assert_eq!(drifted.source_notes, d.source_notes);
    }

    /// A note about the screen changes no word of the narration, so it reaches the builder
    /// instead: through the source notes, and only when the teacher asked for it.
    #[test]
    fn requested_notes_reach_the_builder_and_others_do_not() {
        let s = script();
        let build = |applying: &[usize]| crate::prompt::build(&draft(&s, None, applying));
        let asked = build(&[1]);
        assert!(asked.contains("[1] Show the word “bill” on screen."), "{asked}");
        assert!(asked.contains("Do what each says about what is on screen"), "{asked}");
        assert!(!asked.contains("Say “line” slowly"), "{asked}");
        assert!(asked.contains("don't act on them"), "{asked}");
        let none = build(&[]);
        assert!(!none.contains("Show the word") && !none.contains("Say “line”"), "{none}");
        assert!(none.contains("don't act on them"), "{none}");
        // Every note asked for: nothing is left to warn about.
        assert!(!build(&[1, 2]).contains("don't act on them"));
        // The axes are facts for every turn, and a revision gets them back.
        assert!(none.contains("- S02: x 0 to 50, step 10; y -5 to 30, step 5. Lines: Violet: y = 20 - 0.5x"), "{none}");
    }

    /// One note changes the words of one scene, however many claim it.
    #[test]
    fn a_note_unlocks_one_scene() {
        let s = script();
        let d = draft(&s, None, &[]);
        let reply = vec![
            Marked { say: d.scenes[0].say.clone(), applied: vec![] },
            Marked { say: "Take the first bill. Now plot it. The points lie on one line.".into(), applied: vec![1] },
            Marked { say: "Thanks, all, for watching.".into(), applied: vec![1] },
        ];
        let m = merge(d.clone(), &reply, &[1]);
        assert!(m.storyboard.scenes[1].say.contains("Now plot it"));
        assert_eq!(m.storyboard.scenes[2].say, d.scenes[2].say);
        assert_eq!((m.restored.as_slice(), m.applied.as_slice()), ([3].as_slice(), [1].as_slice()));
        assert_eq!(reworded(&d, &m.storyboard), [2]);
    }

    #[test]
    fn the_reply_must_be_json() {
        assert!(parse_marks("no json here").is_err());
        assert!(parse_marks("{\"title\": \"x\"}").is_err());
    }
}
