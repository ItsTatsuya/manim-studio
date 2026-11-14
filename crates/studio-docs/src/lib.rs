//! Text out of the documents teachers attach: PDF, Word (.docx), PowerPoint (.pptx),
//! Markdown and plain text. The text is condensed once by the planning model into
//! source notes that later turns reuse (DESIGN.md §6.4), so this crate only extracts.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// File extensions we can read.
pub const SUPPORTED: &[&str] = &["pdf", "docx", "pptx", "md", "markdown", "txt"];

pub fn supported(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| SUPPORTED.contains(&e.to_ascii_lowercase().as_str()))
}

/// The document's text, tidied (runs of blank lines collapsed).
pub fn extract(path: &Path) -> Result<String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let text = match ext.as_str() {
        "md" | "markdown" | "txt" => std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
        "docx" => docx(path)?,
        "pptx" => pptx(path)?,
        "pdf" => pdf_extract::extract_text(path).map_err(|e| anyhow::anyhow!("reading the PDF: {e}"))?,
        _ => bail!("{} isn't a supported document (PDF, DOCX, PPTX, MD, TXT)", path.display()),
    };
    Ok(tidy(&text))
}

/// Cap text at about `max` characters, cutting at a paragraph when possible.
pub fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let cut = text[..end].rfind("\n\n").filter(|i| *i > max / 2).unwrap_or(end);
    format!("{}\n[…]", &text[..cut])
}

fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() {
            blank += 1;
            if blank == 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(&l);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

fn zip_entry(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<String> {
    let mut f = archive.by_name(name)?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Text inside `<{tag}>…</{tag}>` runs, with a newline at each `</{para}>`.
fn xml_text(xml: &str, tag: &str, para: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let para_close = format!("</{para}>");
    let mut out = String::new();
    let mut i = 0;
    while i < xml.len() {
        let next_text = xml[i..].find(&open).map(|p| p + i);
        let next_para = xml[i..].find(&para_close).map(|p| p + i);
        match (next_text, next_para) {
            (Some(t), p) if p.is_none_or(|p| t < p) => {
                // Skip tags like <a:tab> that only share the prefix.
                let after = &xml[t + open.len()..];
                if !after.starts_with('>') && !after.starts_with(' ') {
                    i = t + open.len();
                    continue;
                }
                let Some(gt) = xml[t..].find('>') else { break };
                let start = t + gt + 1;
                if xml[t..start].ends_with("/>") {
                    i = start;
                    continue;
                }
                let Some(end) = xml[start..].find(&close) else { break };
                out.push_str(&unescape(&xml[start..start + end]));
                i = start + end + close.len();
            }
            (_, Some(p)) => {
                out.push('\n');
                i = p + para_close.len();
            }
            _ => break,
        }
    }
    out
}

fn docx(path: &Path) -> Result<String> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut z = zip::ZipArchive::new(f).context("not a Word document")?;
    let xml = zip_entry(&mut z, "word/document.xml")?;
    Ok(xml_text(&xml, "w:t", "w:p"))
}

fn pptx(path: &Path) -> Result<String> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut z = zip::ZipArchive::new(f).context("not a PowerPoint file")?;
    let mut slides: Vec<(u32, String)> = (0..z.len())
        .filter_map(|i| {
            let name = z.by_index(i).ok()?.name().to_string();
            let n = name.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()?;
            Some((n, name))
        })
        .collect();
    slides.sort();
    let mut out = String::new();
    for (n, name) in slides {
        let xml = zip_entry(&mut z, &name)?;
        out.push_str(&format!("Slide {n}\n{}\n\n", xml_text(&xml, "a:t", "a:p")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_word_xml() {
        let xml = r#"<w:body><w:p><w:r><w:t>Area of</w:t></w:r><w:r><w:t xml:space="preserve"> a circle</w:t></w:r></w:p><w:p><w:r><w:tab/><w:t>A = &amp;pi;r²</w:t></w:r></w:p></w:body>"#;
        assert_eq!(xml_text(xml, "w:t", "w:p"), "Area of a circle\nA = &pi;r²\n");
    }

    #[test]
    fn tidies_and_truncates() {
        assert_eq!(tidy("a  b\n\n\n\nc"), "a b\n\nc");
        let t = truncate("aaaa\n\nbbbb\n\ncccc", 12);
        assert!(t.starts_with("aaaa\n\nbbbb") && t.ends_with("[…]"));
    }

    #[test]
    fn reads_a_docx_file() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("studio-docx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("notes.docx");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        w.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(br#"<w:document><w:body><w:p><w:r><w:t>Circles</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
        w.finish().unwrap();
        assert_eq!(extract(&p).unwrap(), "Circles");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
