//! Scripts teachers already write: a lesson plan, a developer brief (branding, scenes,
//! colours, axes, LaTeX), the narration word for word with an on-screen action column, and
//! reviewer notes. [`parse`] reads the text a document extracts to (tables come out as
//! `a | b | c` rows, see `studio_docs`) into a [`TeacherScript`] that the planner and the
//! builder are given as fixed facts instead of material to summarise:
//!
//! - the narration is used verbatim ([`same_words`], [`carry_marks`] guard it),
//! - colours, axis ranges and equations are used as given,
//! - the brand table becomes the video's watermark and end card,
//! - reviewer notes are numbered in document order and applied only where the teacher's
//!   request asks for them ([`requested_notes`]).
//!
//! Nothing here calls a model. It reads the structure the teacher wrote, and says nothing
//! when a document isn't one of these scripts.

use serde::{Deserialize, Serialize};

use crate::latex;

/// A teacher's script, read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeacherScript {
    pub title: String,
    pub scenes: Vec<ScriptScene>,
    pub colours: Vec<Colour>,
    pub axes: Vec<Axes>,
    /// Other facts the brief gives (points, parameter values), as written.
    pub facts: Vec<String>,
    pub equations: Vec<Equation>,
    pub brand: Brand,
    pub notes: Vec<Note>,
}

/// One scene: a row of the developer brief's scene table, with the narration rows that fall
/// in its time range.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScriptScene {
    /// The scene class, e.g. `S03_OneEquationOneLine`.
    pub class: String,
    pub title: String,
    /// Seconds into the lesson.
    pub start: f64,
    pub end: f64,
    /// What the brief says the scene shows.
    pub shows: String,
    pub rows: Vec<Row>,
}

impl ScriptScene {
    /// The scene's narration, as written.
    pub fn say(&self) -> String {
        self.rows.iter().map(|r| r.say.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
    }

    /// The scene number in its class name (`S03_...` is 3).
    pub fn number(&self) -> Option<u32> {
        let digits: String = self.class.strip_prefix(['S', 's'])?.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }
}

/// One row of the narration table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Row {
    /// Seconds into the lesson.
    pub at: f64,
    pub say: String,
    /// The on-screen action column.
    pub action: String,
}

/// A colour the brief names, with the object it is for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Colour {
    pub object: String,
    /// "Teal".
    pub name: String,
    /// `#2EC4B6`, upper case.
    pub hex: String,
    /// What else the cell says ("dot radius 0.1, pulse twice").
    pub note: String,
}

/// An axis range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub step: Option<f64>,
}

impl std::fmt::Display for Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} to {}", self.min, self.max)?;
        if let Some(s) = self.step {
            write!(f, ", step {s}")?;
        }
        Ok(())
    }
}

/// A row of the axes table: the graphs of some scenes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Axes {
    /// The scene numbers it names (`S03, S04` is `[3, 4]`).
    pub scenes: Vec<u32>,
    /// The label as written ("S07 part a").
    pub label: String,
    pub x: Option<Range>,
    pub y: Option<Range>,
    /// The lines to draw, as written.
    pub lines: String,
}

/// An equation: the LaTeX the script gives, the Typst the engine takes, and whether that
/// rendered.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Equation {
    pub latex: String,
    pub typst: String,
    /// Commands the converter didn't know.
    pub warnings: Vec<String>,
    /// Why the engine couldn't draw `typst`, once checked.
    pub error: Option<String>,
}

/// The brand table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Brand {
    /// The logo's file name, as the script gives it.
    pub logo: Option<String>,
    pub watermark: Option<WatermarkSpec>,
    pub end_card: Option<EndCardSpec>,
}

/// Where and how the logo is drawn over the video.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatermarkSpec {
    /// 0..9 row by row from the top left, as in the watermark settings (2 = top right).
    pub anchor: Option<usize>,
    /// Fraction of the video's width.
    pub size: Option<f32>,
    pub opacity: Option<f32>,
    /// Gap to the edges, in pixels of a 1080p frame.
    pub margin_px: Option<f32>,
    /// Seconds the script shows it from and until.
    pub from: Option<f64>,
    pub until: Option<f64>,
}

/// The closing scene.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EndCardSpec {
    /// Fraction of the frame's width.
    pub logo_width: Option<f32>,
    /// The line under the logo.
    pub text: String,
    /// The brief's own words, for the builder.
    pub brief: String,
}

/// A reviewer's note, numbered in document order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    /// 1-based, in the order they appear.
    pub n: usize,
    /// Where in an earlier render the reviewer saw it, in seconds. The script's own
    /// timings differ, so this is only a label.
    pub at: f64,
    pub text: String,
}

// ---------------------------------------------------------------------------- reading

/// `m:ss` as seconds.
fn time(s: &str) -> Option<f64> {
    let (m, ss) = s.trim().split_once(':')?;
    if m.is_empty() || m.len() > 2 || ss.len() != 2 || !m.chars().all(|c| c.is_ascii_digit()) || !ss.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (m, ss): (f64, f64) = (m.parse().ok()?, ss.parse().ok()?);
    (ss < 60.).then_some(m * 60. + ss)
}

/// "1:15" for a number of seconds.
pub fn clock(seconds: f64) -> String {
    let s = seconds.round().max(0.) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A cell that is one time, or a range of two (`1:15–2:15`): the times in it.
fn times_of(cell: &str) -> Vec<f64> {
    let parts: Vec<&str> = cell.split(['–', '-', '—']).map(str::trim).collect();
    let t: Vec<f64> = parts.iter().filter_map(|p| time(p)).collect();
    if t.len() == parts.len() && (1..=2).contains(&t.len()) { t } else { Vec::new() }
}

/// The text as cells: a `|` row splits into its cells, any other line is one.
fn cells(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.contains(" | ") || line.starts_with("| ") {
            for c in line.trim_matches('|').split('|') {
                let c = c.trim();
                // A Markdown table's rule line.
                if c.is_empty() || c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')) && c.contains('-') {
                    continue;
                }
                out.push(c.to_string());
            }
        } else {
            out.push(line.to_string());
        }
    }
    out
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// `3. Script, part 1` or `Beat 4 – Example 1`.
fn is_heading(cell: &str) -> bool {
    let c = cell.trim();
    let numbered = c.split_once(". ").is_some_and(|(n, rest)| !n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit()) && !rest.trim().is_empty() && c.len() < 90);
    numbered || lower(c).starts_with("beat ") && c.len() < 120
}

/// A table's column title.
fn is_column_title(cell: &str) -> bool {
    matches!(lower(cell.trim()).as_str(), "time" | "narration" | "on-screen action" | "on screen action" | "on-screen" | "action" | "visual" | "timing")
}

/// A scene class: `S03_OneEquationOneLine`.
fn is_scene_class(cell: &str) -> bool {
    let c = cell.trim();
    let Some(rest) = c.strip_prefix(['S', 's']) else { return false };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].starts_with('_') && rest.len() > digits + 1 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// `OneEquationOneLine` as "One equation one line".
fn title_of(class: &str) -> String {
    let name = class.split_once('_').map_or(class, |(_, n)| n);
    let mut words: Vec<String> = Vec::new();
    for ch in name.chars() {
        if ch == '_' {
            words.push(String::new());
        } else if ch.is_uppercase() || words.is_empty() || ch.is_ascii_digit() && words.last().is_some_and(|w| !w.ends_with(|c: char| c.is_ascii_digit())) {
            words.push(ch.to_string());
        } else if let Some(w) = words.last_mut() {
            w.push(ch);
        }
    }
    let mut out = words.into_iter().filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect::<Vec<_>>().join(" ");
    if let Some(first) = out.chars().next() {
        out.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
    }
    out
}

/// A number written with a real or a typographic minus.
fn number(s: &str) -> Option<f64> {
    s.trim().replace('−', "-").parse().ok()
}

/// `0 to 40, step 10`, `−1 to 6, step 1`.
fn range(cell: &str) -> Option<Range> {
    let c = cell.replace('−', "-");
    let (lo, rest) = c.split_once(" to ")?;
    let (hi, step) = match rest.split_once(',') {
        Some((hi, tail)) => (hi, tail.trim().strip_prefix("step").and_then(number)),
        None => (rest, None),
    };
    Some(Range { min: number(lo)?, max: number(hi)?, step })
}

/// The scene numbers an axes row names: `S03, S04` → [3, 4]; `S07 part a` → [7].
fn scene_refs(cell: &str) -> Vec<u32> {
    let c = cell.trim();
    if !c.starts_with(['S', 's']) || c.contains('_') {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut rest = c;
    while let Some(i) = rest.find(['S', 's']) {
        let digits: String = rest[i + 1..].chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse() {
            out.push(n);
        }
        rest = &rest[i + 1..];
    }
    // Only a table's scene column: `S07 part a`, `S03, S04`, not a sentence.
    if out.is_empty() || c.split_whitespace().count() > 6 || c.ends_with('.') { Vec::new() } else { out }
}

/// `#2EC4B6` colours in a cell: (name before it, hex, what follows).
fn colour_of(cell: &str) -> Option<(String, String, String)> {
    let at = cell.find('#')?;
    let hex = cell.get(at..at + 7)?;
    if !hex[1..].chars().all(|c| c.is_ascii_hexdigit()) || cell[at + 7..].starts_with(|c: char| c.is_ascii_hexdigit()) {
        return None;
    }
    let name = cell[..at].trim().to_string();
    let note = cell[at + 7..].trim().trim_start_matches([',', ';', '.', '–', '-']).trim().to_string();
    Some((name, hex.to_uppercase(), note))
}

/// Numbers followed by `%` or `px` in a sentence: (value, unit, the words after it).
fn quantities(text: &str) -> Vec<(f32, String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            i += 1;
        }
        // A time (`9:45`) is not a quantity.
        if chars.get(i) == Some(&':') || start > 0 && chars[start - 1] == ':' {
            continue;
        }
        let value: f32 = chars[start..i].iter().collect::<String>().parse().unwrap_or(0.);
        let mut j = i;
        while chars.get(j) == Some(&' ') {
            j += 1;
        }
        let unit: String = if chars.get(j) == Some(&'%') {
            "%".into()
        } else if chars[j.min(chars.len())..].iter().take(2).collect::<String>().eq_ignore_ascii_case("px") {
            "px".into()
        } else {
            continue;
        };
        // The words that say what the number is, up to the next comma.
        let after: String = chars[(j + unit.len()).min(chars.len())..].iter().take_while(|c| !matches!(c, ',' | ';')).take(28).collect::<String>().to_lowercase();
        out.push((value, unit, after));
    }
    out
}

/// A file name with a picture extension in the text.
fn picture_name(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | ';' | '"' | '“' | '”' | '\'')))
        .find(|w| {
            let l = lower(w);
            [".png", ".jpg", ".jpeg", ".webp", ".svg"].iter().any(|e| l.ends_with(e))
        })
        .map(str::to_string)
}

/// The first quoted phrase (`“Thank you”` or `"Thank you"`).
fn quoted(text: &str) -> Option<String> {
    let (open, close) = [('“', '”'), ('"', '"'), ('‘', '’')].into_iter().find(|(o, _)| text.contains(*o))?;
    let from = text.find(open)? + open.len_utf8();
    let len = text[from..].find(close)?;
    Some(text[from..from + len].trim().to_string()).filter(|q| !q.is_empty())
}

fn watermark_spec(text: &str) -> WatermarkSpec {
    let l = lower(text);
    let y = if l.contains("top") { Some(0) } else if l.contains("bottom") { Some(2) } else if l.contains("middle") || l.contains("centre") || l.contains("center") { Some(1) } else { None };
    let x = if l.contains("left") { Some(0) } else if l.contains("right") { Some(2) } else if l.contains("centre") || l.contains("center") { Some(1) } else { None };
    let anchor = match (x, y) {
        (None, None) => None,
        (x, y) => Some(y.unwrap_or(0) * 3 + x.unwrap_or(2)),
    };
    let mut w = WatermarkSpec { anchor, ..Default::default() };
    for (v, unit, after) in quantities(text) {
        match unit.as_str() {
            "%" if after.contains("opac") => w.opacity = Some(v / 100.),
            "%" if after.contains("width") || after.contains("wide") || after.contains("screen") => w.size = Some(v / 100.),
            "px" if after.contains("margin") || l.contains("margin") => w.margin_px = Some(v),
            _ => {}
        }
    }
    // "from 0:00 to 9:45"
    let times: Vec<f64> = text.split(|c: char| !(c.is_ascii_digit() || c == ':')).filter_map(|t| time(t.trim_matches(':'))).collect();
    if let [from, until, ..] = times[..] {
        w.from = Some(from);
        w.until = Some(until);
    }
    w
}

fn end_card_spec(text: &str) -> EndCardSpec {
    let width = quantities(text).into_iter().find(|(_, u, after)| u == "%" && (after.contains("width") || after.contains("wide") || after.contains("screen"))).map(|(v, ..)| v / 100.);
    EndCardSpec { logo_width: width, text: quoted(text).unwrap_or_default(), brief: text.trim().to_string() }
}

/// The value after a labelled cell (`Watermark` → its description), for the first cell
/// that is exactly the label.
fn after_label<'a>(cells: &'a [String], label: &str) -> Option<&'a str> {
    let i = cells.iter().position(|c| lower(c.trim().trim_end_matches(':')) == label)?;
    cells.get(i + 1).map(String::as_str)
}

fn is_notes_heading(cell: &str) -> bool {
    let l = lower(cell.trim().trim_end_matches(':'));
    cell.len() <= 40 && ["suggestions", "suggestion", "reviewer", "review notes", "review comments", "feedback", "comments", "reviewers' notes"].iter().any(|k| l.starts_with(k))
}

/// Read a teacher's script. `None` when the text isn't one: it needs a narration table (a
/// time, then what is said, then what appears) with at least two timed rows.
pub fn parse(text: &str) -> Option<TeacherScript> {
    let all = cells(text);
    let first_row = all.iter().position(|c| time(c).is_some())?;
    // Reviewer notes come after the script; their lines start with times too.
    let notes_at = all.iter().enumerate().skip(first_row).find(|(_, c)| is_notes_heading(c)).map(|(i, _)| i);
    let script_end = notes_at.unwrap_or(all.len());

    // Narration rows: a time, then the words, then the action.
    let mut rows: Vec<(usize, Row)> = Vec::new();
    let mut consumed = vec![false; all.len()];
    let mut k = 0;
    while k < script_end {
        let Some(at) = time(&all[k]) else {
            k += 1;
            continue;
        };
        let free = |i: usize| i < script_end && time(&all[i]).is_none() && !is_heading(&all[i]) && !is_column_title(&all[i]);
        let mut row = Row { at, ..Default::default() };
        consumed[k] = true;
        let mut next = k + 1;
        if free(next) {
            row.say = all[next].clone();
            consumed[next] = true;
            next += 1;
            if free(next) && !is_scene_class(&all[next]) {
                row.action = all[next].clone();
                consumed[next] = true;
                next += 1;
            }
        }
        if !row.say.is_empty() {
            rows.push((k, row));
        }
        k = next;
    }
    if rows.len() < 2 {
        return None;
    }

    let mut script = TeacherScript { title: text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_string(), ..Default::default() };

    // Scenes: the brief's scene table.
    let mut k = 0;
    while k + 1 < script_end {
        if is_scene_class(&all[k]) {
            let span = times_of(&all[k + 1]);
            if let [start, end] = span[..] {
                let shows = all.get(k + 2).filter(|c| !is_scene_class(c) && !is_heading(c)).cloned().unwrap_or_default();
                script.scenes.push(ScriptScene { class: all[k].trim().to_string(), title: title_of(&all[k]), start, end, shows, rows: Vec::new() });
                k += 3;
                continue;
            }
        }
        k += 1;
    }
    if script.scenes.is_empty() {
        script.scenes = scenes_from_beats(&all[..script_end], &rows);
    }
    for (_, row) in &rows {
        // The scene whose time range holds the row (the last one that started before it).
        let ix = script.scenes.iter().rposition(|s| s.start <= row.at + 1e-6).unwrap_or(0);
        script.scenes[ix].rows.push(row.clone());
    }
    script.scenes.retain(|s| !s.rows.is_empty() || !s.shows.is_empty());
    if script.scenes.iter().all(|s| s.rows.is_empty()) {
        return None;
    }

    // Colours, axes and facts: the brief's tables, outside the narration rows.
    let mut k = 0;
    while k < script_end {
        if consumed[k] {
            k += 1;
            continue;
        }
        let cell = &all[k];
        if let Some((name, hex, note)) = colour_of(cell) {
            let object = k.checked_sub(1).filter(|p| !consumed[*p] && colour_of(&all[*p]).is_none() && !is_column_title(&all[*p])).map(|p| all[p].clone());
            if let Some(object) = object.filter(|o| !is_heading(o) && o.len() < 90) {
                script.colours.push(Colour { object, name, hex, note });
            }
        } else if let refs @ [_, ..] = &scene_refs(cell)[..]
            && let Some(x) = all.get(k + 1).and_then(|c| range(c))
        {
            let y = all.get(k + 2).and_then(|c| range(c));
            let lines = if y.is_some() { all.get(k + 3).cloned().unwrap_or_default().replace('−', "-") } else { String::new() };
            script.axes.push(Axes { scenes: refs.to_vec(), label: cell.trim().to_string(), x: Some(x), y, lines });
            k += if y.is_some() { 3 } else { 1 };
        } else if cell.contains("Dots in S") || cell.contains("dots in S") {
            script.facts.push(cell.trim().replace('−', "-"));
        }
        k += 1;
    }

    // Brand.
    let brief: Vec<String> = all[..script_end].iter().enumerate().filter(|(i, _)| !consumed[*i]).map(|(_, c)| c.clone()).collect();
    script.brand = Brand {
        logo: after_label(&brief, "logo file").and_then(picture_name).or_else(|| after_label(&brief, "logo").and_then(picture_name)),
        watermark: after_label(&brief, "watermark").map(watermark_spec),
        end_card: after_label(&brief, "end card").map(end_card_spec),
    };

    // Equations: the lines under the LaTeX heading, up to the next numbered section.
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    if let Some(h) = lines.iter().position(|l| lower(l).contains("latex") && l.len() < 90 && !l.contains('\\')) {
        // The list ends where the script goes back to prose, a table or a new section.
        let math = |l: &str| l.contains(['\\', '^', '_', '=', '+', '<', '>', '/']);
        let ends = |l: &str| is_heading(l) || l.contains(" | ") || time(l.split_whitespace().next().unwrap_or("")).is_some() || !l.is_empty() && !math(l);
        for l in lines[h + 1..].iter().take_while(|l| !ends(l)) {
            if l.is_empty() {
                continue;
            }
            let c = latex::to_typst(l);
            script.equations.push(Equation { latex: (*l).to_string(), typst: c.typst, warnings: c.warnings, error: None });
        }
    }

    // Reviewer notes: lines that start with a time, and what follows them.
    if let Some(from) = notes_at {
        for c in &all[from + 1..] {
            let end = c.find(|ch: char| !(ch.is_ascii_digit() || ch == ':')).unwrap_or(c.len());
            match time(c[..end].trim_end_matches(':')) {
                Some(at) => {
                    let text = c[end..].trim_start_matches([' ', ':', '-', '–']).trim().to_string();
                    script.notes.push(Note { n: script.notes.len() + 1, at, text });
                }
                None => {
                    if let Some(last) = script.notes.last_mut() {
                        last.text.push(' ');
                        last.text.push_str(c.trim());
                    }
                }
            }
        }
    }
    Some(script)
}

/// Scenes from the script's "Beat 3 – Concept build (1:15–4:30)" headings, for scripts
/// without a scene table; failing that, one scene per minute of narration.
fn scenes_from_beats(cells: &[String], rows: &[(usize, Row)]) -> Vec<ScriptScene> {
    let mut out: Vec<ScriptScene> = Vec::new();
    for (i, c) in cells.iter().enumerate() {
        if !lower(c).starts_with("beat ") {
            continue;
        }
        // Rows after the heading, before the next one.
        let at = rows.iter().find(|(k, _)| *k > i).map(|(_, r)| r.at);
        let Some(start) = at else { continue };
        let title = c.split_once(['–', '-', '—']).map_or(c.as_str(), |(_, t)| t);
        let title = title.split(" (").next().unwrap_or(title).trim().to_string();
        if out.last().is_some_and(|s| (s.start - start).abs() < 1e-6) {
            continue;
        }
        out.push(ScriptScene { class: format!("S{:02}", out.len() + 1), title, start, ..Default::default() });
    }
    if out.is_empty() {
        let mut start = f64::NEG_INFINITY;
        for (_, r) in rows {
            if r.at >= start + 60. {
                start = r.at;
                out.push(ScriptScene { class: format!("S{:02}", out.len() + 1), title: format!("Part {}", out.len() + 1), start, ..Default::default() });
            }
        }
    }
    let ends: Vec<f64> = out.iter().skip(1).map(|s| s.start).chain(rows.last().map(|(_, r)| r.at + 15.)).collect();
    for (s, e) in out.iter_mut().zip(ends) {
        s.end = e;
    }
    out
}

// ---------------------------------------------------------------------------- notes

/// The notes the teacher's request asks for (1-based numbers, in order). None unless the
/// request says so: "apply reviewer notes 2–4", "use all the suggestions", "apply the 2:15
/// suggestion". A request that only mentions the notes, or says to ignore them, asks for none.
pub fn requested_notes(request: &str, notes: &[Note]) -> Vec<usize> {
    if notes.is_empty() {
        return Vec::new();
    }
    let mut want: Vec<usize> = Vec::new();
    for clause in request.split(['.', ';', '\n', '!', '?']) {
        let l = lower(clause).replace('’', "'");
        let about_notes = ["note", "suggestion", "feedback", "comment", "reviewer"].iter().any(|k| l.contains(k));
        let refused = ["don't", "do not", "dont", "ignore", "skip", "without", "leave out", "never"].iter().any(|k| l.contains(k));
        if !about_notes || refused {
            continue;
        }
        let mut picked: Vec<usize> = Vec::new();
        // Times that name a note, and numbers.
        let mut named = false;
        for word in l.split(|c: char| !(c.is_ascii_digit() || c == ':')) {
            if let Some(t) = time(word) {
                named = true;
                picked.extend(notes.iter().filter(|n| (n.at - t).abs() < 1.).map(|n| n.n));
            }
        }
        let nums = numbers_in(&l);
        named |= !nums.is_empty();
        picked.extend(nums.into_iter().filter(|n| (1..=notes.len()).contains(n)));
        // No numbers at all: "apply the reviewer's notes" or "use all the notes" means every
        // one. "Use my notes" doesn't: it needs the reviewer or "all".
        let words: Vec<&str> = l.split(|c: char| !c.is_alphabetic()).collect();
        let has = |ws: &[&str]| words.iter().any(|w| ws.contains(w));
        let verb = has(&["apply", "use", "include", "follow", "incorporate", "address", "implement", "take"]);
        let all = has(&["all", "every", "each"]);
        let reviewer = ["reviewer", "suggestion", "feedback", "comment", "review note"].iter().any(|k| l.contains(k));
        if !named && (reviewer && (verb || all) || all && l.contains("note")) {
            picked.extend(notes.iter().map(|n| n.n));
        }
        want.extend(picked);
    }
    want.sort_unstable();
    want.dedup();
    want
}

/// The note numbers in a clause: `2`, `#2`, `2-4`, `2 to 4`, `1, 3 and 5` (times excluded).
fn numbers_in(clause: &str) -> Vec<usize> {
    let chars: Vec<char> = clause.chars().collect();
    let mut found: Vec<(usize, usize, usize)> = Vec::new(); // value, start, end
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let clock = chars.get(i) == Some(&':') || start > 0 && chars[start - 1] == ':';
        let decimal = chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if !clock && !decimal && let Ok(v) = chars[start..i].iter().collect::<String>().parse() {
            found.push((v, start, i));
        }
    }
    let mut out = Vec::new();
    let mut k = 0;
    while k < found.len() {
        let (v, _, end) = found[k];
        out.push(v);
        if let Some(&(w, start, _)) = found.get(k + 1) {
            let between: String = chars[end..start].iter().collect();
            if matches!(between.trim(), "-" | "–" | "—" | "to" | "through" | "thru" | "until") && w >= v {
                out.extend(v + 1..=w);
                k += 1;
            }
        }
        k += 1;
    }
    out
}

// ---------------------------------------------------------------------------- verbatim

/// Whitespace collapsed to single spaces.
pub fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The two texts say the same words (cue marks and spacing aside).
pub fn same_words(a: &str, b: &str) -> bool {
    squash(&crate::strip(a)) == squash(&crate::strip(b))
}

/// A word without its punctuation, in lower case.
fn bare(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

/// `original` (a teacher's narration) with the cue marks `reply` puts in it. Each mark
/// goes before the same word of `original`; a mark whose word isn't there is dropped. The
/// result says the original's words whatever `reply` did to them.
pub fn carry_marks(original: &str, reply: &str) -> String {
    let words: Vec<&str> = original.split_whitespace().collect();
    let marked = crate::parse(reply);
    let mut at: Vec<Vec<&str>> = vec![Vec::new(); words.len() + 1];
    let mut from = 0;
    for m in &marked.marks {
        let Some(word) = marked.text[m.at.min(marked.text.len())..].split_whitespace().next() else {
            at[words.len()].push(&m.id);
            continue;
        };
        let want = bare(word);
        if let Some(i) = words[from..].iter().position(|w| bare(w) == want && !want.is_empty()) {
            from += i;
            at[from].push(&m.id);
        }
    }
    let mut out = String::with_capacity(original.len() + marked.marks.len() * 8);
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        for id in &at[i] {
            out.push_str(&format!("{{{id}}}"));
        }
        out.push_str(w);
    }
    for id in &at[words.len()] {
        out.push_str(&format!("{{{id}}}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An invented script in the shape `studio_docs` extracts a Word document to.
    pub(crate) const FIXTURE: &str = "\
Video 2.07 – Two Pens, One Bill: Solving Pairs of Equations
Acme Academy · Algebra · Module 2 · 6:00
1. Lesson plan
The video teaches one idea.
2. Developer brief
Branding
Item | Instruction
Logo file | acme_logo.png (transparent PNG)
Watermark | Logo in the bottom-left corner from 0:00 to 5:45: about 6% of screen width, 55% opacity, 30 px margin.
End card | Logo at the centre, 40% of screen width, with “Thanks for watching – Acme Academy” below it.
Scene class | Time | What it shows
S01_Hook | 0:00–0:30 | A bill with two lines of prices
S02_FirstLine | 0:30–1:30 | Axes, two dots, a violet line
S03_EndCard | 1:30–2:00 | The Acme logo and a thank-you line
Colours and objects
Object | Colour
First equation and its line | Violet #7B2CBF
Crossing point | Amber #FFB703, dot radius 0.12, pulse twice
Final answer | Green #57CC99 in a rounded box
Axes for each graph
Scene | x range | y range | Lines (as y = …)
S02 | 0 to 50, step 10 | −5 to 30, step 5 | Violet: y = 20 − 0.5x
S02, S03 | −1 to 6, step 1 | 0 to 8, step 2 | Violet: y = x + 1
Dots in S02: (0, 20), (10, 15).
LaTeX strings (MathTex, ready to paste)
2p + 3q = 40 \\qquad p + q = 15
\\frac{p}{q} \\neq \\frac{2}{3}
kx + y = 3
3. Script
Beat 1 – Hook (0:00–0:30)
Time | Narration | On-screen action
0:00 | A shop sells pens and pads. Two pens and three pads cost forty rupees. | A bill slides in.
0:15 | How many pens can we buy? If p is five, q is ten. | Dots appear.
Beat 2 – First line (0:30–1:30)
0:30 | Take the first bill. Plot it. | Axes appear, 0 to 50.
1:00 | The points lie on one line. | A violet line draws itself.
Beat 3 – End card
1:30 | Thanks for watching. | Screen clears. The Acme logo fades in at the centre.
Suggestions:
0:20 : Show the word “bill” on screen.
1:05: Say “parameter” slowly.
Instead, say “a free number”.
1:25 : Slower pace.
";

    fn script() -> TeacherScript {
        parse(FIXTURE).expect("a teacher script")
    }

    #[test]
    fn narration_rows_go_to_the_scenes_by_time() {
        let s = script();
        assert_eq!(s.title, "Video 2.07 – Two Pens, One Bill: Solving Pairs of Equations");
        let names: Vec<_> = s.scenes.iter().map(|s| (s.class.as_str(), s.title.as_str(), s.rows.len())).collect();
        assert_eq!(names, [("S01_Hook", "Hook", 2), ("S02_FirstLine", "First line", 2), ("S03_EndCard", "End card", 1)]);
        assert_eq!(title_of("S06_Example1"), "Example 1");
        assert_eq!(s.scenes[0].say(), "A shop sells pens and pads. Two pens and three pads cost forty rupees. How many pens can we buy? If p is five, q is ten.");
        assert_eq!(s.scenes[0].rows[1].action, "Dots appear.");
        assert_eq!((s.scenes[1].start, s.scenes[1].end), (30., 90.));
        assert_eq!(s.scenes[1].number(), Some(2));
        assert_eq!(s.scenes[1].shows, "Axes, two dots, a violet line");
    }

    #[test]
    fn colours_axes_and_facts_are_read_as_given() {
        let s = script();
        let c: Vec<_> = s.colours.iter().map(|c| (c.object.as_str(), c.name.as_str(), c.hex.as_str(), c.note.as_str())).collect();
        assert_eq!(
            c,
            [
                ("First equation and its line", "Violet", "#7B2CBF", ""),
                ("Crossing point", "Amber", "#FFB703", "dot radius 0.12, pulse twice"),
                ("Final answer", "Green", "#57CC99", "in a rounded box"),
            ]
        );
        assert_eq!(s.axes.len(), 2);
        assert_eq!(s.axes[0].scenes, [2]);
        assert_eq!(s.axes[0].x, Some(Range { min: 0., max: 50., step: Some(10.) }));
        assert_eq!(s.axes[0].y, Some(Range { min: -5., max: 30., step: Some(5.) }));
        assert_eq!(s.axes[0].y.unwrap().to_string(), "-5 to 30, step 5");
        assert_eq!(s.axes[0].lines, "Violet: y = 20 - 0.5x");
        assert_eq!(s.axes[1].scenes, [2, 3]);
        assert_eq!(s.facts, ["Dots in S02: (0, 20), (10, 15)."]);
    }

    #[test]
    fn the_brand_table_is_read() {
        let b = script().brand;
        assert_eq!(b.logo.as_deref(), Some("acme_logo.png"));
        let w = b.watermark.expect("watermark");
        assert_eq!((w.anchor, w.size, w.opacity, w.margin_px), (Some(6), Some(0.06), Some(0.55), Some(30.)));
        assert_eq!((w.from, w.until), (Some(0.), Some(345.)));
        let e = b.end_card.expect("end card");
        assert_eq!((e.logo_width, e.text.as_str()), (Some(0.4), "Thanks for watching – Acme Academy"));
    }

    #[test]
    fn equations_are_converted_to_typst() {
        let s = script();
        let t: Vec<_> = s.equations.iter().map(|e| e.typst.as_str()).collect();
        assert_eq!(t, ["2 p + 3 q = 40 wide p + q = 15", "p/q != 2/3", "k x + y = 3"]);
        assert!(s.equations.iter().all(|e| e.warnings.is_empty() && e.error.is_none()));
        assert_eq!(s.equations[1].latex, "\\frac{p}{q} \\neq \\frac{2}{3}");
    }

    #[test]
    fn reviewer_notes_are_numbered_in_order() {
        let n = script().notes;
        let got: Vec<_> = n.iter().map(|n| (n.n, n.at, n.text.as_str())).collect();
        assert_eq!(
            got,
            [
                (1, 20., "Show the word “bill” on screen."),
                (2, 65., "Say “parameter” slowly. Instead, say “a free number”."),
                (3, 85., "Slower pace."),
            ]
        );
    }

    #[test]
    fn other_documents_are_not_scripts() {
        assert_eq!(parse("Circles\n\nThe area of a circle is pi r squared.\nSlide 1\nRadius"), None);
        assert_eq!(parse("Time | Narration | On-screen action\n0:00 | Only one row. | Nothing."), None);
    }

    #[test]
    fn scripts_without_a_scene_table_split_at_beats() {
        let text = "Beat 1 – Hook (0:00–0:30)\n0:00 | One two three. | A\n0:15 | Four five six. | B\nBeat 2 – Idea (0:30–1:00)\n0:30 | Seven eight nine. | C";
        let s = parse(text).expect("script");
        let got: Vec<_> = s.scenes.iter().map(|s| (s.title.as_str(), s.rows.len())).collect();
        assert_eq!(got, [("Hook", 2), ("Idea", 1)]);
    }

    #[test]
    fn flat_lines_read_like_table_rows() {
        let flat = FIXTURE.replace(" | ", "\n");
        let (a, b) = (script(), parse(&flat).expect("flat script"));
        assert_eq!(a.scenes[0].say(), b.scenes[0].say());
        assert_eq!(a.colours, b.colours);
        assert_eq!(a.brand, b.brand);
    }

    #[test]
    fn notes_are_applied_only_where_the_request_asks() {
        let n3: Vec<Note> = (1..=6).map(|i| Note { n: i, at: i as f64 * 20., text: String::new() }).collect();
        for (request, want) in [
            ("Make a lesson from the attached script.", vec![]),
            ("Apply reviewer notes 2–4", vec![2, 3, 4]),
            ("use notes 1 and 3, and 6", vec![1, 3, 6]),
            ("Apply notes 2 to 3.", vec![2, 3]),
            ("Apply all the reviewer suggestions", vec![1, 2, 3, 4, 5, 6]),
            ("Please use the reviewer's notes.", vec![1, 2, 3, 4, 5, 6]),
            ("Ignore the reviewer notes", vec![]),
            ("Do not apply the suggestions. Keep it short.", vec![]),
            ("The reviewer sent notes.", vec![]),
            ("Apply the 1:00 suggestion", vec![3]),
            ("Make it 3 minutes long. Apply notes #5", vec![5]),
            ("Apply note 9", vec![]),
            ("Apply all notes", vec![1, 2, 3, 4, 5, 6]),
            ("Use my notes and keep it short.", vec![]),
            ("Use simple words, and review the pacing at the end.", vec![]),
        ] {
            assert_eq!(requested_notes(request, &n3), want, "{request}");
        }
        assert!(requested_notes("Apply all notes", &[]).is_empty());
    }

    #[test]
    fn wording_is_guarded_and_marks_are_carried() {
        let said = "If r is ten, c is twenty. Look!";
        assert!(same_words(said, "If r is ten,  c is {c20}twenty. {x}Look!"));
        assert!(!same_words(said, "If r is ten, c is twenty. Look at that!"));
        // The reply rewords one sentence: the original's words come back with the marks that still fit.
        let reply = "If r equals ten, c is {c20}twenty. {x}Look at that!";
        let fixed = carry_marks(said, reply);
        assert_eq!(fixed, "If r is ten, c is {c20}twenty. {x}Look!");
        assert!(same_words(&fixed, said));
        // A mark on a word that is gone is dropped; one at the very end stays at the end.
        assert_eq!(carry_marks("Plain words here.", "Plain {gone}other words {end}"), "Plain words here.{end}");
    }
}
