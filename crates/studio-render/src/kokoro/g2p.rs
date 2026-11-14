//! English text → Kokoro phonemes: a port of misaki's lexicon G2P (hexgrad/misaki,
//! Apache-2.0), the phonemizer Kokoro was trained with. Words come from misaki's gold and
//! silver dictionaries with its stress, suffix (-s, -ed, -ing) and function-word rules.
//! Differences from misaki: no part-of-speech tagger (heteronyms use their default
//! reading), numbers and maths symbols are spelled out before lookup, and unknown words go
//! to espeak-ng when it is installed (run as a separate program), else are spelled out.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

const PRIMARY: char = 'ˈ';
const SECONDARY: char = 'ˌ';
const VOWELS: &str = "AIOQWYaiuæɑɒɔəɛɜɪʊʌᵻ";
const CONSONANTS: &str = "bdfhjklmnpstvwzðŋɡɹɾʃʒʤʧθ";
const US_TAUS: &str = "AIOWYiuæɑəɛɪɹʊʌ";
const PUNCTS: &str = ";:,.!?—…\"“”()";
const NON_QUOTE_PUNCTS: &str = ";:,.!?—…";

fn is_vowel(c: char) -> bool {
    VOWELS.contains(c)
}

#[derive(Debug)]
enum Entry {
    One(String),
    /// Readings by part of speech; always has "DEFAULT".
    Tags(HashMap<String, Option<String>>),
}

#[derive(Clone, Copy, Default)]
struct Ctx {
    /// Does the next word start with a vowel sound (None: punctuation or end).
    future_vowel: Option<bool>,
}

pub struct G2p {
    british: bool,
    gold: HashMap<String, Entry>,
    silver: HashMap<String, Entry>,
    espeak: Option<super::espeak::Espeak>,
}

fn load_dict(path: &Path) -> Result<HashMap<String, Entry>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let raw: HashMap<String, serde_json::Value> = serde_json::from_str(&text)?;
    let mut d = HashMap::with_capacity(raw.len() * 2);
    for (k, v) in raw {
        let e = match v {
            serde_json::Value::String(s) => Entry::One(s),
            serde_json::Value::Object(m) => {
                Entry::Tags(m.into_iter().map(|(t, v)| (t, v.as_str().map(str::to_string))).collect())
            }
            _ => continue,
        };
        d.insert(k, e);
    }
    // misaki's grow_dictionary: "word" also answers "Word" and the other way round.
    let mut extra = Vec::new();
    for k in d.keys() {
        if k.chars().count() < 2 {
            continue;
        }
        let lower = k.to_lowercase();
        let cap = capitalize(k);
        if *k == lower && *k != cap {
            extra.push((cap, k.clone()));
        } else if *k == capitalize(&lower) && *k != lower {
            extra.push((lower, k.clone()));
        }
    }
    for (new, from) in extra {
        if d.contains_key(&new) {
            continue;
        }
        let e = match &d[&from] {
            Entry::One(s) => Entry::One(s.clone()),
            Entry::Tags(m) => Entry::Tags(m.clone()),
        };
        d.insert(new, e);
    }
    Ok(d)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c.flat_map(|c| c.to_lowercase())).collect(),
        None => String::new(),
    }
}

fn is_alpha(s: &str) -> bool {
    !s.is_empty() && s.chars().all(char::is_alphabetic)
}

fn lexicon_chars(s: &str) -> bool {
    s.chars().all(|c| c == '\'' || c == '-' || c.is_ascii_alphabetic())
}

/// misaki's apply_stress.
fn apply_stress(ps: &str, stress: Option<f32>) -> String {
    let Some(stress) = stress else { return ps.to_string() };
    let has = |c| ps.contains(c);
    let any_stress = has(PRIMARY) || has(SECONDARY);
    let any_vowel = ps.chars().any(is_vowel);
    if stress < -1.0 {
        ps.replace([PRIMARY, SECONDARY], "")
    } else if stress == -1.0 || ((stress == 0.0 || stress == -0.5) && has(PRIMARY)) {
        ps.replace(SECONDARY, "").replace(PRIMARY, &SECONDARY.to_string())
    } else if (stress == 0.0 || stress == 0.5 || stress == 1.0) && !any_stress {
        if !any_vowel { ps.to_string() } else { restress(&format!("{SECONDARY}{ps}")) }
    } else if stress >= 1.0 && !has(PRIMARY) && has(SECONDARY) {
        ps.replace(SECONDARY, &PRIMARY.to_string())
    } else if stress > 1.0 && !any_stress {
        if !any_vowel { ps.to_string() } else { restress(&format!("{PRIMARY}{ps}")) }
    } else {
        ps.to_string()
    }
}

/// Move each stress mark to just before the next vowel.
fn restress(ps: &str) -> String {
    let chars: Vec<char> = ps.chars().collect();
    let mut keyed: Vec<(f32, char)> = chars.iter().enumerate().map(|(i, &c)| (i as f32, c)).collect();
    for (i, &c) in chars.iter().enumerate() {
        if (c == PRIMARY || c == SECONDARY)
            && let Some(j) = chars[i..].iter().position(|&v| is_vowel(v)) {
                keyed[i].0 = (i + j) as f32 - 0.5;
            }
    }
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
    keyed.into_iter().map(|(_, c)| c).collect()
}

impl G2p {
    /// Load the dictionaries for American (`us_*.json`) or British (`gb_*.json`) English.
    pub fn load(dir: &Path, british: bool) -> Result<Self> {
        let p = if british { "gb" } else { "us" };
        Ok(Self {
            british,
            gold: load_dict(&dir.join(format!("{p}_gold.json")))?,
            silver: load_dict(&dir.join(format!("{p}_silver.json")))?,
            espeak: super::espeak::find(Some(dir)),
        })
    }

    /// Phonemes for a line of narration, in Kokoro's alphabet.
    pub fn phonemize(&self, text: &str) -> String {
        let text = normalize(text);
        let tokens = tokenize(&text);
        let mut out: Vec<String> = vec![String::new(); tokens.len()];
        let mut ctx = Ctx::default();
        for (i, tk) in tokens.iter().enumerate().rev() {
            let ps = match tk.kind {
                Kind::Punct => {
                    let c = if tk.text == "-" || tk.text == "–" { "—" } else { tk.text.as_str() };
                    if PUNCTS.contains(c) { c.to_string() } else { String::new() }
                }
                Kind::Word => self.word(&tk.text, ctx).unwrap_or_default(),
            };
            ctx = next_ctx(ctx, &ps);
            out[i] = ps;
        }
        let mut s = String::new();
        for (tk, ps) in tokens.iter().zip(out) {
            s.push_str(&ps);
            if tk.space {
                s.push(' ');
            }
        }
        // Kokoro v1.0 was trained without flaps and glottal stops.
        s.trim().replace('ɾ', "T").replace('ʔ', "t")
    }

    fn word(&self, raw: &str, ctx: Ctx) -> Option<String> {
        let word = raw.replace(['‘', '’'], "'");
        let stress = if word == word.to_lowercase() {
            None
        } else if word == word.to_uppercase() {
            Some(2.0)
        } else {
            Some(0.5)
        };
        if let Some(ps) = self.get_word(&word, stress, ctx) {
            return Some(ps);
        }
        // Hyphenated or otherwise compound: say the parts.
        if word.contains('-') {
            let parts: Vec<String> =
                word.split('-').filter(|p| !p.is_empty()).filter_map(|p| self.word(p, ctx)).collect();
            if !parts.is_empty() {
                return Some(parts.join(" "));
            }
        }
        if let Some(ps) = self.espeak_word(&word) {
            return Some(ps);
        }
        // Last resort: spell it.
        if lexicon_chars(&word) { self.get_nnp(&word) } else { None }
    }

    fn entry(&self, word: &str, gold_only: bool) -> Option<&Entry> {
        self.gold.get(word).or_else(|| if gold_only { None } else { self.silver.get(word) })
    }

    fn get_nnp(&self, word: &str) -> Option<String> {
        let mut ps = String::new();
        for c in word.chars().filter(|c| c.is_alphabetic()) {
            match self.gold.get(&c.to_uppercase().to_string())? {
                Entry::One(s) => ps.push_str(s),
                Entry::Tags(m) => ps.push_str(m.get("DEFAULT")?.as_deref()?),
            }
        }
        if ps.is_empty() {
            return None;
        }
        let ps = apply_stress(&ps, Some(0.0));
        // Last secondary stress becomes primary.
        Some(match ps.rfind(SECONDARY) {
            Some(i) => format!("{}{PRIMARY}{}", &ps[..i], &ps[i + SECONDARY.len_utf8()..]),
            None => ps,
        })
    }

    fn gold_default(&self, word: &str) -> Option<String> {
        match self.gold.get(word)? {
            Entry::One(s) => Some(s.clone()),
            Entry::Tags(m) => m.get("DEFAULT").cloned().flatten(),
        }
    }

    /// misaki's get_special_case, assuming the common part of speech (no tagger).
    fn special_case(&self, word: &str, stress: Option<f32>, ctx: Ctx) -> Option<String> {
        match word {
            "a" | "A" => Some("ɐ".into()),
            "am" | "Am" | "AM" => {
                if ctx.future_vowel.is_none() || word != "am" || stress.is_some_and(|s| s > 0.0) {
                    self.gold_default("am")
                } else {
                    Some("ɐm".into())
                }
            }
            "an" | "An" | "AN" => Some("ɐn".into()),
            "I" => Some(format!("{SECONDARY}I")),
            "to" | "To" | "TO" => match ctx.future_vowel {
                None => self.gold_default("to"),
                Some(false) => Some("tə".into()),
                Some(true) => Some("tʊ".into()),
            },
            "in" | "In" | "IN" => Some(if ctx.future_vowel.is_none() { format!("{PRIMARY}ɪn") } else { "ɪn".into() }),
            "the" | "The" | "THE" => Some(if ctx.future_vowel == Some(true) { "ði".into() } else { "ðə".into() }),
            "vs" | "vs." => self.lookup("versus", None, ctx),
            _ => None,
        }
    }

    fn is_known(&self, word: &str) -> bool {
        if self.gold.contains_key(word) || self.silver.contains_key(word) {
            return true;
        }
        if !is_alpha(word) || !lexicon_chars(word) {
            return false;
        }
        if word.chars().count() == 1 {
            return true;
        }
        if word == word.to_uppercase() && self.gold.contains_key(&word.to_lowercase()) {
            return true;
        }
        let rest: String = word.chars().skip(1).collect();
        rest == rest.to_uppercase()
    }

    fn lookup(&self, word: &str, stress: Option<f32>, ctx: Ctx) -> Option<String> {
        let mut word = word.to_string();
        if word == word.to_uppercase() && !self.gold.contains_key(&word) {
            word = word.to_lowercase();
        }
        let ps = match self.entry(&word, false) {
            Some(Entry::One(s)) => Some(s.clone()),
            Some(Entry::Tags(m)) => {
                let key = if ctx.future_vowel.is_none() && m.contains_key("None") { "None" } else { "DEFAULT" };
                m.get(key).cloned().flatten().or_else(|| m.get("DEFAULT").cloned().flatten())
            }
            None => None,
        };
        match ps {
            Some(ps) => Some(apply_stress(&ps, stress)),
            None => self.get_nnp(&word),
        }
    }

    fn suffix_s(&self, stem: &str) -> Option<String> {
        let last = stem.chars().last()?;
        Some(if "ptkfθ".contains(last) {
            format!("{stem}s")
        } else if "szʃʒʧʤ".contains(last) {
            format!("{stem}{}z", if self.british { 'ɪ' } else { 'ᵻ' })
        } else {
            format!("{stem}z")
        })
    }

    fn stem_s(&self, word: &str, stress: Option<f32>, ctx: Ctx) -> Option<String> {
        let n = word.len();
        if n < 3 || !word.ends_with('s') {
            return None;
        }
        let stem = if !word.ends_with("ss") && self.is_known(&word[..n - 1]) {
            word[..n - 1].to_string()
        } else if (word.ends_with("'s") || (n > 4 && word.ends_with("es") && !word.ends_with("ies"))) && self.is_known(&word[..n - 2]) {
            word[..n - 2].to_string()
        } else if n > 4 && word.ends_with("ies") && self.is_known(&format!("{}y", &word[..n - 3])) {
            format!("{}y", &word[..n - 3])
        } else {
            return None;
        };
        self.suffix_s(&self.lookup(&stem, stress, ctx)?)
    }

    fn suffix_ed(&self, stem: &str) -> Option<String> {
        let chars: Vec<char> = stem.chars().collect();
        let last = *chars.last()?;
        let weak = if self.british { 'ɪ' } else { 'ᵻ' };
        Some(if "pkfθʃsʧ".contains(last) {
            format!("{stem}t")
        } else if last == 'd' {
            format!("{stem}{weak}d")
        } else if last != 't' {
            format!("{stem}d")
        } else if self.british || chars.len() < 2 {
            format!("{stem}ɪd")
        } else if US_TAUS.contains(chars[chars.len() - 2]) {
            format!("{}ɾᵻd", chars[..chars.len() - 1].iter().collect::<String>())
        } else {
            format!("{stem}ᵻd")
        })
    }

    fn stem_ed(&self, word: &str, stress: Option<f32>, ctx: Ctx) -> Option<String> {
        let n = word.len();
        if n < 4 || !word.ends_with('d') {
            return None;
        }
        let stem = if !word.ends_with("dd") && self.is_known(&word[..n - 1]) {
            &word[..n - 1]
        } else if n > 4 && word.ends_with("ed") && !word.ends_with("eed") && self.is_known(&word[..n - 2]) {
            &word[..n - 2]
        } else {
            return None;
        };
        self.suffix_ed(&self.lookup(stem, stress, ctx)?)
    }

    fn suffix_ing(&self, stem: &str) -> Option<String> {
        let chars: Vec<char> = stem.chars().collect();
        let last = *chars.last()?;
        if self.british {
            if "əː".contains(last) {
                return None;
            }
        } else if chars.len() > 1 && last == 't' && US_TAUS.contains(chars[chars.len() - 2]) {
            return Some(format!("{}ɾɪŋ", chars[..chars.len() - 1].iter().collect::<String>()));
        }
        Some(format!("{stem}ɪŋ"))
    }

    fn stem_ing(&self, word: &str, stress: Option<f32>, ctx: Ctx) -> Option<String> {
        let n = word.len();
        if n < 5 || !word.ends_with("ing") {
            return None;
        }
        let base = &word[..n - 3];
        let doubled = {
            let b = base.as_bytes();
            (b.len() >= 2 && b[b.len() - 1] == b[b.len() - 2] && b"bcdgklmnprstvxz".contains(&b[b.len() - 1])) || word.ends_with("cking")
        };
        let stem = if n > 5 && self.is_known(base) {
            base.to_string()
        } else if self.is_known(&format!("{base}e")) {
            format!("{base}e")
        } else if n > 5 && doubled && self.is_known(&word[..n - 4]) {
            word[..n - 4].to_string()
        } else {
            return None;
        };
        self.suffix_ing(&self.lookup(&stem, stress, ctx)?)
    }

    fn get_word(&self, word: &str, stress: Option<f32>, ctx: Ctx) -> Option<String> {
        if let Some(ps) = self.special_case(word, stress, ctx) {
            return Some(ps);
        }
        let lower = word.to_lowercase();
        let mut word = word.to_string();
        let rest: String = word.chars().skip(1).collect();
        if word.chars().count() > 1
            && is_alpha(&word.replace('\'', ""))
            && word != lower
            && !self.gold.contains_key(&word)
            && !self.silver.contains_key(&word)
            && (word == word.to_uppercase() || rest == rest.to_lowercase())
            && (self.gold.contains_key(&lower)
                || self.silver.contains_key(&lower)
                || self.stem_s(&lower, stress, ctx).is_some()
                || self.stem_ed(&lower, stress, ctx).is_some()
                || self.stem_ing(&lower, stress, ctx).is_some())
        {
            word = lower;
        }
        if self.is_known(&word) {
            return self.lookup(&word, stress, ctx);
        }
        if let Some(stem) = word.strip_suffix("s'")
            && self.is_known(&format!("{stem}'s")) {
                return self.lookup(&format!("{stem}'s"), stress, ctx);
            }
        if let Some(stem) = word.strip_suffix('\'')
            && self.is_known(stem) {
                return self.lookup(stem, stress, ctx);
            }
        self.stem_s(&word, stress, ctx)
            .or_else(|| self.stem_ed(&word, stress, ctx))
            .or_else(|| self.stem_ing(&word, Some(stress.unwrap_or(0.5)), ctx))
    }

    /// espeak-ng (GPL) as a separate program, mapped to misaki's alphabet. Optional.
    fn espeak_word(&self, word: &str) -> Option<String> {
        let mut ps = self.espeak.as_ref()?.ipa(word, self.british)?;
        const E2M: [(&str, &str); 19] = [
            ("ʔˌn\u{329}", "ʔn"), ("ʔn\u{329}", "ʔn"), ("a^ɪ", "I"), ("a^ʊ", "W"), ("d^ʒ", "ʤ"), ("e^ɪ", "A"),
            ("t^ʃ", "ʧ"), ("ɔ^ɪ", "Y"), ("ə^l", "ᵊl"), ("ʲo", "jo"), ("ʲə", "jə"), ("ʲ", ""), ("ɚ", "əɹ"),
            ("r", "ɹ"), ("x", "k"), ("ç", "k"), ("ɐ", "ə"), ("ɬ", "l"), ("\u{303}", ""),
        ];
        for (a, b) in E2M {
            ps = ps.replace(a, b);
        }
        ps = ps.replace('e', "A");
        // A syllabic consonant (n + U+0329) becomes ᵊ + consonant.
        let chars: Vec<char> = ps.chars().collect();
        let mut out = String::with_capacity(ps.len());
        for (i, &c) in chars.iter().enumerate() {
            if c == '\u{329}' {
                continue;
            }
            if chars.get(i + 1) == Some(&'\u{329}') && !c.is_whitespace() {
                out.push('ᵊ');
            }
            out.push(c);
        }
        ps = out;
        if self.british {
            ps = ps.replace("e^ə", "ɛː").replace("iə", "ɪə").replace("ə^ʊ", "Q");
        } else {
            ps = ps.replace("o^ʊ", "O").replace("ɜːɹ", "ɜɹ").replace("ɜː", "ɜɹ").replace("ɪə", "iə").replace('ː', "");
        }
        Some(ps.replace('o', "ɔ").replace('^', ""))
    }
}


fn next_ctx(ctx: Ctx, ps: &str) -> Ctx {
    for c in ps.chars() {
        if NON_QUOTE_PUNCTS.contains(c) {
            return Ctx { future_vowel: None };
        }
        if is_vowel(c) {
            return Ctx { future_vowel: Some(true) };
        }
        if CONSONANTS.contains(c) {
            return Ctx { future_vowel: Some(false) };
        }
    }
    ctx
}

// ---------------------------------------------------------------------- tokens

#[derive(PartialEq, Debug)]
enum Kind {
    Word,
    Punct,
}

#[derive(Debug)]
struct Token {
    text: String,
    kind: Kind,
    /// Followed by whitespace.
    space: bool,
}

fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0;
    let word_char = |c: char| c.is_alphanumeric();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            if let Some(t) = out.last_mut() {
                t.space = true;
            }
            i += 1;
            continue;
        }
        if word_char(c) {
            let start = i;
            while i < chars.len()
                && (word_char(chars[i])
                    || ("'’-".contains(chars[i]) && i + 1 < chars.len() && word_char(chars[i + 1]) && i > start))
            {
                i += 1;
            }
            // A trailing apostrophe (plural possessive: "students'").
            if i < chars.len() && "'’".contains(chars[i]) && chars[i - 1] == 's' {
                i += 1;
            }
            out.push(Token { text: chars[start..i].iter().collect(), kind: Kind::Word, space: false });
            continue;
        }
        out.push(Token { text: c.to_string(), kind: Kind::Punct, space: false });
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------- numbers and symbols

const ONES: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve",
    "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

fn under_thousand(n: u64) -> String {
    let mut parts = Vec::new();
    if n >= 100 {
        parts.push(format!("{} hundred", ONES[(n / 100) as usize]));
    }
    let r = n % 100;
    if r > 0 || n == 0 {
        parts.push(if r < 20 {
            ONES[r as usize].to_string()
        } else if r.is_multiple_of(10) {
            TENS[(r / 10) as usize].to_string()
        } else {
            format!("{}-{}", TENS[(r / 10) as usize], ONES[(r % 10) as usize])
        });
    }
    parts.join(" ")
}

pub(crate) fn number_words(n: u64) -> String {
    if n < 1000 {
        return under_thousand(n);
    }
    const SCALES: [(u64, &str); 4] =
        [(1_000_000_000_000, "trillion"), (1_000_000_000, "billion"), (1_000_000, "million"), (1_000, "thousand")];
    let mut parts = Vec::new();
    let mut n = n;
    for (size, name) in SCALES {
        if n >= size {
            parts.push(format!("{} {name}", number_words(n / size)));
            n %= size;
        }
    }
    if n > 0 {
        parts.push(under_thousand(n));
    }
    parts.join(" ")
}

fn ordinal(words: &str) -> String {
    let (head, last) = match words.rfind([' ', '-']) {
        Some(i) => (&words[..=i], &words[i + 1..]),
        None => ("", words),
    };
    let last = match last {
        "one" => "first".to_string(),
        "two" => "second".to_string(),
        "three" => "third".to_string(),
        "five" => "fifth".to_string(),
        "eight" => "eighth".to_string(),
        "nine" => "ninth".to_string(),
        "twelve" => "twelfth".to_string(),
        w if w.ends_with('y') => format!("{}ieth", &w[..w.len() - 1]),
        w => format!("{w}th"),
    };
    format!("{head}{last}")
}

fn year_words(n: u64) -> Option<String> {
    if !(1100..=1999).contains(&n) && !(2010..=2099).contains(&n) {
        return None;
    }
    let (hi, lo) = (n / 100, n % 100);
    Some(match lo {
        0 => format!("{} hundred", under_thousand(hi)),
        1..=9 => format!("{} oh {}", under_thousand(hi), ONES[lo as usize]),
        _ => format!("{} {}", under_thousand(hi), under_thousand(lo)),
    })
}

/// Spell out numbers, currency and the maths symbols lessons use, so every word is in
/// the dictionary.
pub(crate) fn normalize(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 16);
    let mut i = 0;
    let symbol = |c: char| -> Option<&'static str> {
        Some(match c {
            '%' => " percent",
            '&' => " and ",
            '+' => " plus ",
            '=' => " equals ",
            '≈' => " is about ",
            '≠' => " is not equal to ",
            '<' => " is less than ",
            '>' => " is greater than ",
            '≤' => " is at most ",
            '≥' => " is at least ",
            '×' => " times ",
            '÷' => " divided by ",
            '±' => " plus or minus ",
            '√' => " the square root of ",
            '²' => " squared",
            '³' => " cubed",
            '°' => " degrees",
            'π' => " pi ",
            'θ' => " theta ",
            'α' => " alpha ",
            'β' => " beta ",
            'Δ' => " delta ",
            'λ' => " lambda ",
            'μ' => " mu ",
            'σ' => " sigma ",
            'Σ' => " sigma ",
            '∞' => " infinity ",
            '@' => " at ",
            _ => return None,
        })
    };
    while i < chars.len() {
        let c = chars[i];
        let prev_word = i > 0 && chars[i - 1].is_alphanumeric();
        // Currency before a number.
        let currency = match c {
            '$' => Some(("dollar", "dollars")),
            '£' => Some(("pound", "pounds")),
            '€' => Some(("euro", "euros")),
            _ => None,
        };
        let num_start = if currency.is_some() { i + 1 } else { i };
        let negative = c == '-' && !prev_word && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit());
        let num_start = if negative { i + 1 } else { num_start };
        if chars.get(num_start).is_some_and(|d| d.is_ascii_digit()) && !prev_word {
            let mut j = num_start;
            let mut int = String::new();
            let mut grouped = false;
            while j < chars.len() {
                if chars[j].is_ascii_digit() {
                    int.push(chars[j]);
                } else if chars[j] == ',' && chars.get(j + 1).is_some_and(|d| d.is_ascii_digit()) && j + 3 < chars.len() + 1 {
                    grouped = true;
                } else {
                    break;
                }
                j += 1;
            }
            let mut frac = String::new();
            if chars.get(j) == Some(&'.') && chars.get(j + 1).is_some_and(|d| d.is_ascii_digit()) {
                j += 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    frac.push(chars[j]);
                    j += 1;
                }
            }
            let suffix: String = chars[j..chars.len().min(j + 2)].iter().collect::<String>().to_lowercase();
            let is_ord = frac.is_empty()
                && ["st", "nd", "rd", "th"].contains(&suffix.as_str())
                && !chars.get(j + 2).is_some_and(|c| c.is_alphanumeric());
            let value: u64 = int.parse().unwrap_or(0);
            let mut words = if int.len() > 15 {
                int.chars().map(|d| ONES[d.to_digit(10).unwrap_or(0) as usize]).collect::<Vec<_>>().join(" ")
            } else if !grouped && frac.is_empty() && !is_ord && currency.is_none() && int.len() == 4 {
                year_words(value).unwrap_or_else(|| number_words(value))
            } else {
                number_words(value)
            };
            if is_ord {
                words = ordinal(&words);
                j += 2;
            }
            if !frac.is_empty() {
                let digits: Vec<&str> = frac.chars().map(|d| ONES[d.to_digit(10).unwrap_or(0) as usize]).collect();
                words = format!("{words} point {}", digits.join(" "));
            }
            if negative {
                words = format!("minus {words}");
            }
            if let Some((one, many)) = currency {
                words = format!("{words} {}", if value == 1 && frac.is_empty() { one } else { many });
            }
            out.push_str(&words);
            i = j;
            continue;
        }
        match symbol(c) {
            Some(s) => out.push_str(s),
            None => out.push(c),
        }
        i += 1;
    }
    // Collapse the spaces symbols added.
    let mut s = String::with_capacity(out.len());
    for w in out.split(' ').filter(|w| !w.is_empty()) {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(w);
    }
    s.replace(" ,", ",").replace(" .", ".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spells_numbers_and_symbols() {
        assert_eq!(normalize("A = πr²"), "A equals pi r squared");
        assert_eq!(normalize("3.14"), "three point one four");
        assert_eq!(normalize("1,250 people"), "one thousand two hundred fifty people");
        assert_eq!(normalize("in 1998 and 2024"), "in nineteen ninety-eight and twenty twenty-four");
        assert_eq!(normalize("the 21st time"), "the twenty-first time");
        assert_eq!(normalize("$5 and -3"), "five dollars and minus three");
        assert_eq!(normalize("50%"), "fifty percent");
        assert_eq!(normalize("x2"), "x2");
    }

    #[test]
    fn stress_moves_before_vowel() {
        assert_eq!(restress("ˈkæt"), "kˈæt");
        assert_eq!(apply_stress("kˈæt", Some(-1.0)), "kˌæt");
    }
}
