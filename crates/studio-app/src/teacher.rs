//! A teacher's script attached to a video (`studio_script::teacher`): what Studio read out
//! of it, kept in the video's folder (`teacher.json`) so a retry, a revision and the render
//! use the same facts the plan did.
//!
//! - The plan: the narration is the teacher's own and the planner only adds cue marks.
//! - The brand table becomes the video's watermark (the corner logo, drawn over the
//!   finished video) and its end scene.
//! - Equations come out of the script as LaTeX, are converted to Typst and are checked to
//!   render before anything is built.
//! - Reviewer notes are applied only where the teacher's request asks for them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use studio_agent::prompt::Storyboard;
use studio_agent::teacher as plan;
use studio_render::export::{Align, Mark, Watermark};
use studio_script::teacher::{TeacherScript, requested_notes};

use crate::app::Workspace;

const FILE: &str = "teacher.json";
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "webp"];

/// A picture a teacher attaches next to the script: the logo.
pub fn is_image(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| IMAGES.contains(&e.to_ascii_lowercase().as_str()))
}

/// What is kept for a video made from a teacher's script.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub script: TeacherScript,
    /// The logo in the video's folder (`assets/logo.png`), when it was attached.
    pub logo: Option<String>,
    /// The reviewer notes (numbers) the teacher has asked for so far.
    pub applying: Vec<usize>,
}

pub fn load(dir: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(dir.join(FILE)).ok()?).ok()
}

pub fn save(dir: &Path, saved: &Saved) {
    if let Ok(json) = serde_json::to_vec_pretty(saved) {
        let _ = crate::store::write_atomic(&dir.join(FILE), &json);
    }
}

impl Saved {
    /// The planning prompt: the teacher's narration and on-screen column, and only the
    /// reviewer notes the teacher asked for.
    pub fn plan_prompt(&self) -> String {
        let draft = plan::draft(&self.script, self.logo.as_deref(), &self.applying);
        plan::plan(&self.script, &draft, &self.applying)
    }

    /// Take the reviewer notes a request asks for. True when it asked for new ones.
    pub fn ask_for(&mut self, request: &str) -> bool {
        let before = self.applying.len();
        self.applying.extend(requested_notes(request, &self.script.notes));
        self.applying.sort_unstable();
        self.applying.dedup();
        self.applying.len() != before
    }

    /// The corner logo the brand table asks for, when the logo file is in the video.
    pub fn watermark(&self, dir: &Path) -> Option<Watermark> {
        let spec = self.script.brand.watermark.as_ref()?;
        let logo = self.logo.as_ref().map(|l| dir.join(l)).filter(|p| p.is_file())?;
        let a = spec.anchor.unwrap_or(2).min(8);
        Some(Watermark {
            mark: Mark::Image(logo),
            at: (Align::from_index(a % 3), Align::from_index(a / 3)),
            size: spec.size.unwrap_or(0.08).clamp(0.02, 0.5),
            opacity: spec.opacity.unwrap_or(0.7).clamp(0.1, 1.0),
            // Pixels of a 1080p frame, as a fraction of the shorter side.
            margin: (spec.margin_px.unwrap_or(24.) / 1080.).clamp(0., 0.3),
            color: None,
            shadow: false,
            // The end card carries the logo itself.
            until_last_scene: self.script.brand.end_card.is_some(),
        })
    }
}

/// A script read, with what to tell the teacher.
pub struct Prepared {
    pub saved: Saved,
    /// Lines for the chat: whether all is well, and the text.
    pub lines: Vec<(bool, String)>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Read a teacher's script out of the attached documents' text, check its equations in the
/// engine and put its logo in the video's assets. Slow (it runs the engine): call it off the
/// UI thread. `None` when no document is such a script.
pub fn prepare(request: &str, texts: &[String], images: &[PathBuf], dir: Option<&Path>, style: usize) -> Option<Prepared> {
    let mut script = texts.iter().find_map(|t| studio_script::teacher::parse(t))?;
    let mut lines = vec![(
        true,
        format!(
            "Using your script as written: {}{}.",
            plural(script.scenes.len(), "scene", "scenes"),
            [
                (script.colours.len(), "colour"),
                (script.axes.len(), "set of axes"),
                (script.equations.len(), "equation"),
            ]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, w)| format!(", {}", plural(*n, w, &format!("{w}s"))))
            .collect::<String>()
        ),
    )];

    // Equations: LaTeX in, Typst out, and each one drawn once before anything is built.
    if !script.equations.is_empty() {
        let typst: Vec<String> = script.equations.iter().map(|e| e.typst.clone()).collect();
        match studio_render::equations::check_in_style(&typst, style) {
            Ok(found) => {
                for (e, f) in script.equations.iter_mut().zip(found) {
                    e.error = f;
                }
                let bad: Vec<String> = script
                    .equations
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| e.error.as_ref().map(|why| format!("Equation {} ({}) didn't render: {why}. The builder writes it by hand.", i + 1, e.latex)))
                    .collect();
                if bad.is_empty() {
                    lines.push((true, format!("All {} converted to Typst and render.", plural(script.equations.len(), "equation", "equations"))));
                }
                lines.extend(bad.into_iter().map(|b| (false, b)));
            }
            Err(e) => lines.push((false, format!("Couldn't check the equations ({e:#}); each scene is still checked when it is built."))),
        }
    }

    // The logo: the attached picture the script names (or the only one), copied into the video.
    let wanted = script.brand.logo.clone();
    let named = |p: &&PathBuf| {
        wanted.as_ref().is_some_and(|w| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(w)))
    };
    let picked = images.iter().find(named).or_else(|| if images.len() == 1 { images.first() } else { None });
    let logo = match (picked, dir) {
        (Some(p), Some(dir)) => crate::attach::copy_into(p, &dir.join("assets")).ok().and_then(|d| d.file_name().map(|n| format!("assets/{}", n.to_string_lossy()))),
        // Attached earlier (a retry): already in the video.
        (None, Some(dir)) => wanted.as_ref().filter(|w| dir.join("assets").join(w).is_file()).map(|w| format!("assets/{w}")),
        _ => None,
    };
    match (&logo, &wanted) {
        (Some(l), _) if script.brand.watermark.is_some() || script.brand.end_card.is_some() => {
            lines.push((true, format!("Using {l} as the watermark and on the end card.")));
        }
        (None, Some(w)) => lines.push((
            false,
            format!("The script names a logo ({w}) but it wasn't attached, so the end card has text only and there is no corner logo. Attach it next to the script to use it."),
        )),
        _ => {}
    }

    // Reviewer notes wait for the teacher to ask.
    let applying = requested_notes(request, &script.notes);
    if !script.notes.is_empty() {
        lines.push((
            true,
            if applying.is_empty() {
                format!(
                    "{} found. None is applied unless you ask: for example, “apply reviewer notes 2–4”.",
                    plural(script.notes.len(), "reviewer note", "reviewer notes")
                )
            } else {
                format!("Applying reviewer {} {} of {} found.", if applying.len() == 1 { "note" } else { "notes" }, join_numbers(&applying), script.notes.len())
            },
        ));
    }
    Some(Prepared { saved: Saved { script, logo, applying }, lines })
}

/// `2, 3 and 5`.
fn join_numbers(n: &[usize]) -> String {
    let s: Vec<String> = n.iter().map(usize::to_string).collect();
    match s.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => s.join(""),
    }
}

/// The storyboard for a finished plan or revise turn of a video made from a teacher's
/// script, and what to tell the teacher. A plan reply is only cue marks, and every scene's
/// words are checked against the script; a revision keeps the facts as they were and says
/// which scenes' words it changed from `before`.
pub fn from_reply(saved: &Saved, before: Option<&Storyboard>, planning: bool, reply: &str) -> (Result<Storyboard, String>, Vec<String>) {
    let logo = saved.logo.as_deref();
    if !planning {
        return match studio_agent::prompt::parse_storyboard(reply) {
            Ok(mut sb) => {
                plan::keep_facts(&saved.script, logo, &saved.applying, &mut sb);
                let changed = before.map(|b| plan::reworded(b, &sb)).unwrap_or_default();
                let lines = if changed.is_empty() {
                    Vec::new()
                } else {
                    vec![format!("Changed the wording of {} {}.", if changed.len() == 1 { "scene" } else { "scenes" }, join_numbers(&changed))]
                };
                (Ok(sb), lines)
            }
            Err(e) => (Err(e.to_string()), Vec::new()),
        };
    }
    let draft = plan::draft(&saved.script, logo, &saved.applying);
    let scenes = draft.scenes.len();
    let applying = saved.applying.as_slice();
    let Ok(marks) = plan::parse_marks(reply) else {
        return (Ok(draft), vec!["The assistant's reply had no cue marks, so your script is used as written, without them.".into()]);
    };
    let mut lines = Vec::new();
    if marks.len() < scenes {
        lines.push(format!("The assistant marked {} of {scenes} scenes; the rest have no cue marks.", marks.len()));
    }
    let merged = plan::merge(draft, &marks, applying);
    if !merged.restored.is_empty() {
        let which = merged.restored.iter().map(usize::to_string).collect::<Vec<_>>();
        lines.push(format!(
            "The assistant changed the wording of {} {}; your script's words were put back.",
            if which.len() == 1 { "scene" } else { "scenes" },
            join_numbers(&merged.restored)
        ));
    }
    if !merged.applied.is_empty() {
        lines.push(format!("Applied reviewer {} {}.", if merged.applied.len() == 1 { "note" } else { "notes" }, join_numbers(&merged.applied)));
    }
    (Ok(merged.storyboard), lines)
}

impl Workspace {
    /// The watermark for a video's export: the script's brand table when the video has one
    /// (and its logo), else the default from Settings. A project that turned the watermark
    /// off has none.
    pub fn video_watermark(&self, video: (usize, usize)) -> Option<Watermark> {
        let project = self.projects.get(video.0)?;
        if project.no_watermark {
            return None;
        }
        let from_script = project.videos.get(video.1).and_then(|v| v.dir.as_deref()).and_then(|dir| load(dir)?.watermark(dir));
        from_script.or_else(|| self.default_watermark())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An invented script in the shape a Word document extracts to.
    const SCRIPT: &str = "\
Video 4.01 – Two Pens, One Bill
Logo file | acme_logo.png
Watermark | Logo in the bottom-left corner from 0:00 to 1:00: about 6% of screen width, 55% opacity, 30 px margin.
End card | Logo at the centre, 40% of screen width, with “Thanks – Acme” below it.
S01_Hook | 0:00–0:30 | A bill
S02_EndCard | 0:30–1:00 | The logo
LaTeX strings
kx + 2y = 3
\\foo{x}
Time | Narration | On-screen action
0:00 | A shop sells pens. If p is five, q is ten. | A bill slides in.
0:20 | Take the first bill. | Axes appear.
0:30 | Thanks for watching. | The logo fades in.
Suggestions:
0:10 : Show the word “bill”.
";

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("studio-teacher-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_script_is_read_checked_and_its_logo_attached() {
        let dir = temp("prepare");
        let logo = dir.join("Acme_Logo.PNG");
        std::fs::write(&logo, b"not really a png").unwrap();
        let video = dir.join("video");
        std::fs::create_dir_all(&video).unwrap();
        let texts = vec!["Circles. The area of a circle.".to_string(), SCRIPT.to_string()];
        let p = prepare("Make a lesson from the attached documents.", &texts, std::slice::from_ref(&logo), Some(&video), 0).expect("a script");
        let lines: Vec<&str> = p.lines.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(lines[0], "Using your script as written: 2 scenes, 2 equations.");
        // `kx + 2y = 3` is converted with spaced letters and renders; `\foo` is an unknown command and doesn't.
        assert_eq!(p.saved.script.equations[0].typst, "k x + 2 y = 3");
        assert!(p.saved.script.equations[0].error.is_none());
        assert!(p.saved.script.equations[1].error.is_some(), "{:?}", p.saved.script.equations[1]);
        assert!(lines.iter().any(|l| l.starts_with("Equation 2 (\\foo{x}) didn't render")), "{lines:?}");
        // The picture is matched by name, whatever its case, and copied into the video.
        assert_eq!(p.saved.logo.as_deref(), Some("assets/Acme_Logo.PNG"));
        assert!(video.join("assets/Acme_Logo.PNG").is_file());
        assert!(lines.iter().any(|l| l.starts_with("Using assets/Acme_Logo.PNG as the watermark and on the end card")), "{lines:?}");
        // The note waits for the teacher to ask.
        assert!(lines.last().unwrap().starts_with("1 reviewer note found. None is applied"), "{lines:?}");
        assert!(p.saved.applying.is_empty());
        let asked = prepare("Apply reviewer note 1", &texts, &[logo], Some(&video), 0).unwrap();
        assert_eq!(asked.lines.last().unwrap().1, "Applying reviewer note 1 of 1 found.");
        assert_eq!(asked.saved.applying, [1]);
        // The note goes to the builder with the storyboard, and the plan asks the planner about it.
        assert!(studio_agent::prompt::build(&plan::draft(&asked.saved.script, None, &asked.saved.applying)).contains("[1] Show the word “bill”."));
        assert!(asked.saved.plan_prompt().contains("[1] 0:10 Show the word “bill”."));
        assert!(!p.saved.plan_prompt().contains("Show the word"));
        // Nothing to say about a document that isn't a script.
        assert!(prepare("", &texts[..1], &[], None, 0).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_brand_table_becomes_the_videos_watermark() {
        let dir = temp("brand");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/logo.png"), b"x").unwrap();
        let script = studio_script::teacher::parse(SCRIPT).unwrap();
        let mut saved = Saved { script, logo: Some("assets/logo.png".into()), ..Default::default() };
        let w = saved.watermark(&dir).expect("watermark");
        assert_eq!(w.mark, Mark::Image(dir.join("assets/logo.png")));
        assert_eq!(w.at, (Align::Start, Align::End));
        assert_eq!((w.size, w.opacity), (0.06, 0.55));
        assert!((w.margin - 30. / 1080.).abs() < 1e-6);
        assert!(w.until_last_scene, "the end card has its own logo");
        // Saved with the video and read back.
        save(&dir, &saved);
        assert_eq!(load(&dir).unwrap().watermark(&dir), Some(w));
        // No logo file, no watermark: the default from Settings applies.
        saved.logo = None;
        assert!(saved.watermark(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A later request can ask for more notes; a revision says which scenes' words it changed.
    #[test]
    fn a_revision_asks_for_notes_and_reports_what_it_reworded() {
        let mut saved = Saved { script: studio_script::teacher::parse(SCRIPT).unwrap(), ..Default::default() };
        assert!(!saved.ask_for("Make it calmer"));
        assert!(saved.ask_for("Apply reviewer note 1"));
        assert!(!saved.ask_for("Apply reviewer note 1 again"));
        assert_eq!(saved.applying, [1]);
        let before = plan::draft(&saved.script, None, &[]);
        let mut after = before.clone();
        after.scenes[1].say = "Thanks, everyone.".into();
        let reply = serde_json::to_string(&after).unwrap();
        let (sb, lines) = from_reply(&saved, Some(&before), false, &reply);
        assert_eq!(lines, ["Changed the wording of scene 2."]);
        // The facts come back whatever the model wrote, with the notes asked for.
        assert!(sb.unwrap().source_notes.contains("[1] Show the word “bill”."));
    }

    #[test]
    fn replies_are_checked_against_the_script() {
        let saved = Saved { script: studio_script::teacher::parse(SCRIPT).unwrap(), logo: Some("assets/logo.png".into()), ..Default::default() };
        let reply = r#"{"scenes": [{"say": "A shop sells pens. If p is five, q is {q}ten. Take the first bill."}, {"say": "Thank you all for watching."}]}"#;
        let (sb, lines) = from_reply(&saved, None, true, reply);
        let sb = sb.unwrap();
        assert_eq!(sb.scenes[0].say, "A shop sells pens. If p is five, q is {q}ten. Take the first bill.");
        assert_eq!(sb.scenes[1].say, "Thanks for watching.");
        assert_eq!(lines, ["The assistant changed the wording of scene 2; your script's words were put back."]);
        // The end card and colours are in the storyboard whatever the reply says.
        assert!(sb.scenes[1].visual.contains("ImageMobject(\"assets/logo.png\")"), "{}", sb.scenes[1].visual);
        // Not JSON: the script is used as written.
        let (sb, lines) = from_reply(&saved, None, true, "Sorry, I can't.");
        assert_eq!(sb.unwrap().scenes[1].say, "Thanks for watching.");
        assert_eq!(lines.len(), 1);
    }
}
