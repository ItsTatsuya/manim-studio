//! Narration scripts. A cue mark names the moment the word after it is spoken:
//! `If r is ten, c is {c20}twenty.` Marks are never spoken or shown as captions, and
//! sentences are always split on the text without them, so adding a mark never renumbers
//! `this.narrate(n)`.

/// An id is at most this long.
const MAX_ID: usize = 32;

/// Punctuation that sticks to the word before it.
const CLOSING: [char; 6] = [',', '.', ';', ':', '!', '?'];

/// A cue mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub id: String,
    /// Byte offset of the marked word in the text without marks (its length when the
    /// mark comes after the last word).
    pub at: usize,
}

/// Text with its marks taken out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marked {
    pub text: String,
    pub marks: Vec<Mark>,
}

/// The id and byte length of a mark at the start of `s`. An id starts with a letter, so
/// set notation like `{1, 2}` is left alone.
fn mark_at(s: &str) -> Option<(&str, usize)> {
    let body = s.strip_prefix('{')?;
    let id = &body[..body.find('}')?];
    let ok = id.len() <= MAX_ID
        && id.starts_with(|c: char| c.is_ascii_alphabetic())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then_some((id, id.len() + 2))
}

/// Take the marks out of `script`.
pub fn parse(script: &str) -> Marked {
    let mut text = String::with_capacity(script.len());
    let mut marks = Vec::new();
    let mut rest = script;
    while let Some(c) = rest.chars().next() {
        let Some((id, len)) = mark_at(rest) else {
            text.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        };
        rest = &rest[len..];
        marks.push(Mark { id: id.to_string(), at: text.len() });
        // Marks in a row share their word; the spacing is settled after the last one.
        if mark_at(rest).is_some() {
            continue;
        }
        if text.is_empty() || text.ends_with(char::is_whitespace) {
            // "ten, {x} c": the mark leaves one space, not two.
            rest = rest.trim_start_matches(' ');
            // "twenty {x}." and a mark at the very end leave none.
            if rest.is_empty() || rest.starts_with(CLOSING) {
                text.truncate(text.trim_end_matches(' ').len());
            }
        } else if text.ends_with(CLOSING) && rest.starts_with(char::is_alphabetic) {
            // "ten.{x}Next" would hide the end of the sentence.
            text.push(' ');
        }
    }
    for m in &mut marks {
        // On to the word the mark points at.
        let at = m.at.min(text.len());
        m.at = text[at..].find(|c: char| !c.is_whitespace()).map_or(text.len(), |i| at + i);
    }
    Marked { text, marks }
}

/// `script` as it is spoken and shown: without marks.
pub fn strip(script: &str) -> String {
    parse(script).text
}

/// Split at `.`, `!` or `?` that is followed by whitespace or the end of the text, so
/// "3.14" stays whole. Every piece is trimmed and non-empty. `text` has no marks.
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        cur.push(ch);
        if matches!(ch, '.' | '!' | '?') && chars.peek().is_none_or(|c| c.is_whitespace()) {
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
    out
}

/// The spoken sentences of a script, as `this.narrate(n)` numbers them: without marks,
/// short fragments joined to the sentence before.
pub fn sentences(script: &str) -> Vec<String> {
    merged(&strip(script))
}

fn merged(text: &str) -> Vec<String> {
    let mut merged: Vec<String> = Vec::new();
    for s in split_sentences(text) {
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

/// [`sentences`], each with the marks inside it (offsets within that sentence).
pub fn marked_sentences(script: &str) -> Vec<Marked> {
    let Marked { text, marks } = parse(script);
    let solid = |s: &str| s.chars().filter(|c| !c.is_whitespace()).count();
    let mut out: Vec<Marked> = merged(&text).into_iter().map(|text| Marked { text, marks: Vec::new() }).collect();
    let Some(last) = out.len().checked_sub(1) else { return out };
    // Joining fragments can change the spacing but no other character, so a mark is
    // carried over by how many of those come before it.
    let sizes: Vec<usize> = out.iter().map(|s| solid(&s.text)).collect();
    for m in marks {
        let mut before = solid(&text[..m.at]);
        let mut i = 0;
        while i < last && before >= sizes[i] {
            before -= sizes[i];
            i += 1;
        }
        let s = &out[i].text;
        let at = s.char_indices().filter(|(_, c)| !c.is_whitespace()).nth(before).map_or(s.len(), |(b, _)| b);
        out[i].marks.push(Mark { id: m.id, at });
    }
    out
}

/// A sentence with its marks written back in (for a prompt).
pub fn with_marks(sentence: &Marked) -> String {
    let mut out = String::with_capacity(sentence.text.len() + sentence.marks.len() * 8);
    let mut from = 0;
    for m in &sentence.marks {
        out.push_str(&sentence.text[from..m.at]);
        out.push_str(&format!("{{{}}}", m.id));
        from = m.at;
    }
    out.push_str(&sentence.text[from..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sentence number, id and word of each mark.
    fn pointed_at(script: &str) -> Vec<(usize, String, String)> {
        marked_sentences(script)
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                s.marks.iter().map(move |m| (i + 1, m.id.clone(), s.text[m.at..].split_whitespace().next().unwrap_or("").to_string()))
            })
            .collect()
    }

    #[test]
    fn marks_are_never_spoken() {
        for (script, spoken) in [
            ("If r is ten, {c20}c is twenty.", "If r is ten, c is twenty."),
            ("If r is ten, {c20} c is twenty.", "If r is ten, c is twenty."),
            ("If r is ten,{c20}c is twenty.", "If r is ten, c is twenty."),
            ("If r is ten, c is twenty {end}.", "If r is ten, c is twenty."),
            ("{start} If r is ten, c is twenty. {end}", "If r is ten, c is twenty."),
            ("Say {a}{b}both here.", "Say both here."),
            ("The set {1, 2} and {} stay, so does {open", "The set {1, 2} and {} stay, so does {open"),
            ("Über {é} π²", "Über {é} π²"),
        ] {
            assert_eq!(strip(script), spoken, "{script}");
        }
    }

    #[test]
    fn marks_point_at_the_next_word() {
        let script = "If r is ten, c is {c20}twenty. {line}They lie on {one}one line. Yes.{ok} Next we {cross}cross π²{sq} over.";
        let want = [(1, "c20", "twenty."), (2, "line", "They"), (2, "one", "one"), (3, "ok", "Next"), (3, "cross", "cross"), (3, "sq", "over.")];
        assert_eq!(pointed_at(script), want.map(|(n, id, w)| (n, id.to_string(), w.to_string())));
        assert_eq!(pointed_at("One line of answers. {end}"), [(1, "end".to_string(), String::new())]);
    }

    #[test]
    fn marks_never_renumber_sentences() {
        let plain = "Here's a pizza.  If we double it, do we? Yes. Let's see!\nIt is 3.14 times bigger.";
        let marked = "Here's a {p}pizza.{q}  If we double it, do we? {y}Yes. Let's {see} see!\nIt is {pi}3.14 times bigger.";
        assert_eq!(sentences(marked), sentences(plain));
    }

    #[test]
    fn marks_can_be_written_back() {
        let s = marked_sentences("If r is ten, c is {a}{b} twenty.");
        assert_eq!(with_marks(&s[0]), "If r is ten, c is {a}{b}twenty.");
    }
}
