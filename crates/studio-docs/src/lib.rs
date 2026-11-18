//! Text out of the documents teachers attach: PDF, Word (.docx), PowerPoint (.pptx),
//! Markdown and plain text. The text is condensed once by the planning model into
//! source notes that later turns reuse (DESIGN.md §6.4), so this crate only extracts.

use std::cell::Cell;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// File extensions we can read.
pub const SUPPORTED: &[&str] = &["pdf", "docx", "pptx", "md", "markdown", "txt"];

/// Largest PDF we will hand to the parser.
const MAX_PDF_BYTES: u64 = 100 * 1024 * 1024;
/// Largest .md / .txt file we will read.
const MAX_TEXT_BYTES: u64 = 20 * 1024 * 1024;
/// Largest single XML part (a .docx body, one .pptx slide) after inflating.
const MAX_XML_BYTES: u64 = 50 * 1024 * 1024;
/// Largest total of XML read out of one archive.
const MAX_ARCHIVE_XML_BYTES: u64 = 150 * 1024 * 1024;

pub fn supported(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| SUPPORTED.contains(&e.to_ascii_lowercase().as_str()))
}

thread_local! {
    static IN_PDF_GUARD: Cell<bool> = const { Cell::new(false) };
}

/// True while the calling thread is inside the guarded PDF parser. The parser panics on
/// malformed files; the panic is caught and turned into an error, so an app-wide crash
/// hook should treat a panic raised while this is true as recovered, not as a crash. The
/// hook runs on the panicking thread before unwinding, which is why a thread-local works.
pub fn in_pdf_guard() -> bool {
    IN_PDF_GUARD.with(Cell::get)
}

/// Restores the flag on drop, so it is reset on unwind too.
struct PdfGuard(bool);

impl PdfGuard {
    fn enter() -> Self {
        PdfGuard(IN_PDF_GUARD.with(|g| g.replace(true)))
    }
}

impl Drop for PdfGuard {
    fn drop(&mut self) {
        IN_PDF_GUARD.with(|g| g.set(self.0));
    }
}

/// The document's text, tidied (runs of blank lines collapsed).
pub fn extract(path: &Path) -> Result<String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let text = match ext.as_str() {
        "md" | "markdown" | "txt" => plain(path)?,
        "docx" => docx(path)?,
        "pptx" => pptx(path)?,
        "pdf" => pdf(path)?,
        _ => bail!("{} isn't a supported document (PDF, DOCX, PPTX, MD, TXT)", path.display()),
    };
    Ok(tidy(&text))
}

fn check_size(path: &Path, max: u64, what: &str) -> Result<()> {
    let len = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?.len();
    if len > max {
        bail!("{} is too large to read ({} MB; the limit for {what} is {} MB)", path.display(), len / 1_000_000, max / 1_000_000);
    }
    Ok(())
}

fn plain(path: &Path) -> Result<String> {
    check_size(path, MAX_TEXT_BYTES, "text files")?;
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// pdf-extract panics on many malformed files, so the parse is caught and reported.
fn pdf(path: &Path) -> Result<String> {
    check_size(path, MAX_PDF_BYTES, "PDFs")?;
    let _guard = PdfGuard::enter();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdf_extract::extract_text(path))) {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => bail!("couldn't read this PDF ({e})"),
        Err(_) => bail!("couldn't read this PDF (it may be damaged or use an unsupported feature)"),
    }
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

/// One XML part, refusing to inflate more than `MAX_XML_BYTES` (zip bombs). `budget` is
/// what the whole archive may still inflate.
fn zip_entry(archive: &mut zip::ZipArchive<std::fs::File>, name: &str, budget: &mut u64) -> Result<String> {
    let f = archive.by_name(name)?;
    let limit = MAX_XML_BYTES.min(*budget);
    let mut bytes = Vec::new();
    f.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("{name} is too large to read (this document looks damaged or unsafe)");
    }
    *budget -= bytes.len() as u64;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Text inside `<{tag}>…</{tag}>` runs, with a newline at each `</{para}>`. One pass over
/// the markup: every tag is looked at once.
fn xml_text(xml: &str, tag: &str, para: &str) -> String {
    let close = format!("</{tag}>");
    let mut out = String::new();
    let mut i = 0;
    while let Some(rel) = xml[i..].find('<') {
        let t = i + rel;
        let Some(gt) = xml[t..].find('>') else { break };
        let end = t + gt + 1;
        let inner = &xml[t + 1..end - 1];
        if let Some(closing) = inner.strip_prefix('/') {
            if closing.trim() == para {
                out.push('\n');
            }
        } else if !inner.ends_with('/') && inner.split(|c: char| c.is_whitespace() || c == '/').next() == Some(tag) {
            // A run: its text goes to the closing tag, which is skipped in one step.
            let Some(len) = xml[end..].find(&close) else { break };
            out.push_str(&unescape(&xml[end..end + len]));
            i = end + len + close.len();
            continue;
        }
        i = end;
    }
    out
}

fn docx(path: &Path) -> Result<String> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut z = zip::ZipArchive::new(f).context("not a Word document")?;
    let mut budget = MAX_ARCHIVE_XML_BYTES;
    let xml = zip_entry(&mut z, "word/document.xml", &mut budget)?;
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
    let mut budget = MAX_ARCHIVE_XML_BYTES;
    for (n, name) in slides {
        let xml = zip_entry(&mut z, &name, &mut budget)?;
        out.push_str(&format!("Slide {n}\n{}\n\n", xml_text(&xml, "a:t", "a:p")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_text_handles_many_runs_and_lookalike_tags() {
        let mut xml = String::from("<w:p>");
        for _ in 0..20_000 {
            xml.push_str("<w:r><w:t>ab</w:t></w:r><w:tab/><w:t/>");
        }
        xml.push_str("</w:p><w:p><w:t>end</w:t></w:p>");
        let t = xml_text(&xml, "w:t", "w:p");
        assert_eq!(t.len(), 20_000 * 2 + 1 + 3 + 1);
        assert!(t.ends_with("end\n"));
    }

    #[test]
    fn broken_pdfs_error_instead_of_panicking() {
        let dir = std::env::temp_dir().join(format!("studio-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.pdf");
        std::fs::write(&p, b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 99 0 R >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF").unwrap();
        let e = extract(&p).unwrap_err().to_string();
        assert!(e.contains("couldn't read this PDF"), "{e}");
        assert!(!in_pdf_guard());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_xml_is_refused() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("studio-bomb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bomb.docx");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        w.start_file("word/document.xml", opts).unwrap();
        let chunk = vec![b' '; 1 << 20];
        for _ in 0..52 {
            w.write_all(&chunk).unwrap();
        }
        w.finish().unwrap();
        let e = extract(&p).unwrap_err().to_string();
        assert!(e.contains("too large"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
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
