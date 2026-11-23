//! Chatterbox's text tokenizer: GPT-2 byte-level BPE read from the model's own
//! `tokenizer.json` (vocabulary, merges, the `[laugh]`-style tags and the end-of-text
//! tokens its template adds), so ids match what the model was trained with.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use fancy_regex::Regex;
use serde_json::Value;

pub struct Tokenizer {
    vocab: HashMap<String, i64>,
    /// Merge rank of each pair of pieces (lower merges first).
    ranks: HashMap<(String, String), usize>,
    /// Tags that are one token whatever they are next to (`[chuckle]`), longest first.
    added: Vec<(String, i64)>,
    /// What the template puts after the text.
    tail: Vec<i64>,
    split: Regex,
    to_char: [char; 256],
}

/// GPT-2's byte-to-letter table: printable bytes stand for themselves, the rest move up
/// to unused code points, so every byte has a visible, non-space character.
fn byte_chars() -> [char; 256] {
    let mut table = ['\0'; 256];
    let mut spare = 0u32;
    for b in 0..=255u32 {
        let own = (33..=126).contains(&b) || (161..=172).contains(&b) || (174..=255).contains(&b);
        table[b as usize] = if own {
            char::from_u32(b).unwrap_or('?')
        } else {
            let c = char::from_u32(256 + spare).unwrap_or('?');
            spare += 1;
            c
        };
    }
    table
}

impl Tokenizer {
    pub fn from_json(json: &[u8]) -> Result<Self> {
        let t: Value = serde_json::from_slice(json).context("reading tokenizer.json")?;
        let model = &t["model"];
        if model["type"] != "BPE" {
            bail!("the tokenizer is not byte-pair encoding");
        }
        let vocab: HashMap<String, i64> =
            model["vocab"].as_object().context("no vocabulary")?.iter().filter_map(|(k, v)| Some((k.clone(), v.as_i64()?))).collect();
        let ranks = model["merges"]
            .as_array()
            .context("no merges")?
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                // Newer files list each merge as a pair, older ones as "a b".
                let pair = match m {
                    Value::Array(p) => (p.first()?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string()),
                    Value::String(s) => {
                        let (a, b) = s.split_once(' ')?;
                        (a.to_string(), b.to_string())
                    }
                    _ => return None,
                };
                Some((pair, i))
            })
            .collect();
        let mut added: Vec<(String, i64)> = t["added_tokens"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| Some((x["content"].as_str()?.to_string(), x["id"].as_i64()?))).collect())
            .unwrap_or_default();
        added.sort_by_key(|(s, _)| std::cmp::Reverse(s.len()));
        // Single-text template: the text, then the special tokens it lists.
        let mut tail = Vec::new();
        let mut seen_text = false;
        for part in t["post_processor"]["single"].as_array().into_iter().flatten() {
            if part.get("Sequence").is_some() {
                seen_text = true;
            } else if let Some(name) = part["SpecialToken"]["id"].as_str() {
                let id = t["post_processor"]["special_tokens"][name]["ids"].get(0).and_then(Value::as_i64).context("a template token has no id")?;
                if !seen_text {
                    bail!("the tokenizer puts tokens before the text, which isn't handled");
                }
                tail.push(id);
            }
        }
        let split = Regex::new(r"'(?:s|t|re|ve|m|ll|d)| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+(?!\S)|\s+")?;
        Ok(Self { vocab, ranks, added, tail, split, to_char: byte_chars() })
    }

    /// Token ids for `text`, as the model expects them.
    pub fn encode(&self, text: &str) -> Result<Vec<i64>> {
        let mut ids = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            // The nearest tag (longest if several start there), else the rest as plain text.
            let next = self
                .added
                .iter()
                .filter_map(|(tag, id)| rest.find(tag.as_str()).map(|at| (at, tag.len(), *id)))
                .min_by_key(|&(at, len, _)| (at, std::cmp::Reverse(len)));
            let Some((at, len, id)) = next else {
                self.plain(rest, &mut ids)?;
                break;
            };
            self.plain(&rest[..at], &mut ids)?;
            ids.push(id);
            rest = &rest[at + len..];
        }
        ids.extend(&self.tail);
        Ok(ids)
    }

    fn plain(&self, text: &str, ids: &mut Vec<i64>) -> Result<()> {
        for piece in self.split.find_iter(text) {
            let piece = piece?.as_str();
            let mut symbols: Vec<String> = piece.bytes().map(|b| self.to_char[b as usize].to_string()).collect();
            loop {
                let best = (0..symbols.len().saturating_sub(1))
                    .filter_map(|i| self.ranks.get(&(symbols[i].clone(), symbols[i + 1].clone())).map(|&r| (r, i)))
                    .min();
                let Some((_, i)) = best else { break };
                let merged = format!("{}{}", symbols[i], symbols[i + 1]);
                symbols.splice(i..i + 2, [merged]);
            }
            for s in symbols {
                ids.push(*self.vocab.get(&s).with_context(|| format!("the tokenizer doesn't know {s:?}"))?);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny tokenizer shaped like the real file: bytes, a few merges, a tag, a template.
    fn tiny() -> Tokenizer {
        let chars = byte_chars();
        let mut vocab: HashMap<String, i64> = (0..=255usize).map(|b| (chars[b].to_string(), b as i64)).collect();
        for (i, s) in ["he", "hel", "hell", "hello", "Ġw", "Ġwo"].iter().enumerate() {
            vocab.insert((*s).to_string(), 300 + i as i64);
        }
        let json = serde_json::json!({
            "model": { "type": "BPE", "vocab": vocab, "merges": [["h","e"],["he","l"],["hel","l"],["hell","o"],["Ġ","w"],["Ġw","o"]] },
            "added_tokens": [{ "id": 900, "content": "[laugh]" }, { "id": 901, "content": "[laugh] [laugh]" }],
            "post_processor": {
                "single": [{ "Sequence": { "id": "A" } }, { "SpecialToken": { "id": "<|eot|>" } }, { "SpecialToken": { "id": "<|eot|>" } }],
                "special_tokens": { "<|eot|>": { "ids": [999] } }
            }
        });
        Tokenizer::from_json(json.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn bytes_get_visible_letters() {
        let c = byte_chars();
        assert_eq!((c[b'a' as usize], c[b' ' as usize], c[b'\n' as usize], c[0]), ('a', 'Ġ', 'Ċ', 'Ā'));
        assert_eq!(c.iter().collect::<std::collections::HashSet<_>>().len(), 256);
    }

    #[test]
    fn merges_in_rank_order_and_adds_the_template_tail() {
        let t = tiny();
        // "hello" merges all the way; " wo" merges to one piece; "r" is a byte; two end tokens follow.
        assert_eq!(t.encode("hello wor").unwrap(), [303, 305, b'r' as i64, 999, 999]);
        assert_eq!(t.encode("").unwrap(), [999, 999]);
    }

    #[test]
    fn tags_are_single_tokens_wherever_they_are() {
        let t = tiny();
        assert_eq!(t.encode("hello[laugh]hello").unwrap(), [303, 900, 303, 999, 999]);
        // The longest tag that starts at the same place wins.
        assert_eq!(t.encode("[laugh] [laugh]").unwrap(), [901, 999, 999]);
    }

    /// The real file, when it is on this computer (skipped otherwise). The expected ids come
    /// from the model's own `tokenizers` library.
    #[test]
    fn matches_the_published_tokenizer() {
        let Some(dir) = std::env::var_os("CHATTERBOX_DIR").map(std::path::PathBuf::from).or_else(|| super::super::model_dir().map(std::path::Path::to_path_buf)) else {
            return;
        };
        let Ok(json) = std::fs::read(dir.join("tokenizer.json")) else { return };
        let t = Tokenizer::from_json(&json).unwrap();
        let cases: [(&str, &[i64]); 6] = [
            ("Hello, world.", &[15496, 11, 995, 13, 50256, 50256]),
            ("If r is ten, c is twenty.", &[1532, 374, 318, 3478, 11, 269, 318, 8208, 13, 50256, 50256]),
            ("Oh, that's hilarious! [chuckle] Um anyway.", &[5812, 11, 326, 338, 20105, 0, 220, 50274, 21039, 6949, 13, 50256, 50256]),
            ("  leading and  double  space", &[220, 3756, 290, 220, 4274, 220, 2272, 50256, 50256]),
            ("3r + 2c = 126 → (10, 20)", &[18, 81, 1343, 362, 66, 796, 19710, 15168, 357, 940, 11, 1160, 8, 50256, 50256]),
            ("It's 100% done.\nNew line", &[1026, 338, 1802, 4, 1760, 13, 198, 3791, 1627, 50256, 50256]),
        ];
        for (text, want) in cases {
            assert_eq!(t.encode(text).unwrap(), want, "{text:?}");
        }
    }
}
