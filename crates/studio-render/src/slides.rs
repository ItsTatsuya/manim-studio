//! Lecture slides from a finished video: one slide per scene, which is that scene's key
//! frame exactly as the video has it, written as a PDF and/or a PowerPoint file.
//!
//! Both writers are plain byte/XML writers with no dependencies: the frames are JPEGs
//! cut out of the cached scene videos with ffmpeg, which PDF and PowerPoint both embed
//! as they are.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::export::quiet;

/// Which slide files to write next to the video.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Formats {
    pub pdf: bool,
    pub pptx: bool,
}

impl Formats {
    pub const NONE: Formats = Formats { pdf: false, pptx: false };

    pub fn any(self) -> bool {
        self.pdf || self.pptx
    }
}

/// One slide: the scene's key frame (a JPEG file), edge to edge, and nothing else. The
/// title is only the picture's alternative text for screen readers; it is not drawn.
#[derive(Clone, Debug)]
pub struct Slide {
    pub title: String,
    pub image: PathBuf,
}

/// Cut one frame out of a scene video as a high-quality JPEG. `at` is clamped inside the
/// clip (a seek past the end can finish without writing a frame).
pub fn extract_frame(ffmpeg: &Path, video: &Path, at: f64, duration: f64, out: &Path) -> Result<()> {
    let at = at.clamp(0.0, (duration - 0.1).max(0.0));
    let _ = std::fs::remove_file(out);
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
        .arg(format!("{at:.3}"))
        .arg("-i")
        .arg(video)
        .args(["-frames:v", "1", "-q:v", "2"])
        .arg(out)
        .stdin(std::process::Stdio::null());
    let o = quiet(&mut cmd).output().context("starting ffmpeg")?;
    if !o.status.success() || !out.is_file() {
        // A clip shorter than the asked time: take its first frame instead.
        if at > 0.0 {
            return extract_frame(ffmpeg, video, 0.0, 0.0, out);
        }
        bail!("ffmpeg wrote no frame: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(())
}

/// Write an engine frame (BGRA) as a JPEG, for the assistant to look at.
pub fn frame_to_jpeg(ffmpeg: &Path, frame: &crate::Frame, out: &Path) -> Result<()> {
    use std::io::Write;
    use std::process::Stdio;
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "bgra", "-s"])
        .arg(format!("{}x{}", frame.width, frame.height))
        .args(["-i", "-", "-frames:v", "1", "-q:v", "3"])
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = quiet(&mut cmd).spawn().context("starting ffmpeg")?;
    // Feed it from a thread: a full pipe must not wait on the read of the other.
    let mut stdin = child.stdin.take().context("ffmpeg stdin")?;
    let bytes = frame.bgra.clone();
    let feeder = std::thread::spawn(move || stdin.write_all(&bytes));
    let o = child.wait_with_output().context("running ffmpeg")?;
    let _ = feeder.join();
    if !o.status.success() || !out.is_file() {
        bail!("ffmpeg wrote no picture: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(())
}

/// Width and height of a JPEG, from its start-of-frame marker.
pub fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        if marker == 0xFF || marker == 0x00 || (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
            i += if marker == 0xFF { 1 } else { 2 };
            continue;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        // SOF0..SOF15 except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let h = u16::from_be_bytes([*data.get(i + 5)?, *data.get(i + 6)?]) as u32;
            let w = u16::from_be_bytes([*data.get(i + 7)?, *data.get(i + 8)?]) as u32;
            return (w > 0 && h > 0).then_some((w, h));
        }
        i += 2 + len;
    }
    None
}

// ---------------------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------------------

/// A page the shape of the frame, 960 pt on its long side (a 16:9 slide is 13.333 in wide).
fn page_size((w, h): (u32, u32)) -> (f64, f64) {
    let (w, h) = (w as f64, h as f64);
    if w >= h { (960.0, 960.0 * h / w) } else { (960.0 * w / h, 960.0) }
}

/// Latin-1 text for the document title; anything the standard fonts cannot show is dropped.
fn winansi(s: &str) -> String {
    s.chars()
        .filter(|c| (' '..='~').contains(c) || ('\u{00A1}'..='\u{00FF}').contains(c))
        .collect()
}

fn pdf_escape(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 2);
    for c in s.chars() {
        let b = c as u32 as u8;
        match b {
            b'(' | b')' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            _ => out.push(b),
        }
    }
    out
}

/// A PDF with one page per slide, each the frame filling its page.
pub fn write_pdf(title: &str, slides: &[Slide], out: &Path) -> Result<()> {
    if slides.is_empty() {
        bail!("no slides");
    }
    let mut objects: Vec<Vec<u8>> = Vec::new();
    // 1 catalog, 2 pages, 3 info; then per slide: page, content, image.
    let first_page = 4;
    let kids: Vec<String> = (0..slides.len()).map(|i| format!("{} 0 R", first_page + i * 3)).collect();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.push(format!("<< /Type /Pages /Count {} /Kids [{}] >>", slides.len(), kids.join(" ")).into_bytes());
    let mut info = b"<< /Producer (Manim Studio) /Title (".to_vec();
    info.extend(pdf_escape(&winansi(title)));
    info.extend(b") >>");
    objects.push(info);
    for (i, slide) in slides.iter().enumerate() {
        let jpeg = std::fs::read(&slide.image).with_context(|| format!("reading {}", slide.image.display()))?;
        let (iw, ih) = jpeg_size(&jpeg).with_context(|| format!("{} is not a JPEG", slide.image.display()))?;
        let (pw, ph) = page_size((iw, ih));
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im Do Q\n").into_bytes();
        let page = first_page + i * 3;
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im {} 0 R >> >> /Contents {} 0 R >>",
                page + 2,
                page + 1
            )
            .into_bytes(),
        );
        let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        stream.extend(&content);
        stream.extend(b"endstream");
        objects.push(stream);
        let mut img = format!(
            "<< /Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceRGB /BitsPerComponent 8 \
             /Filter /DCTDecode /Length {} >>\nstream\n",
            jpeg.len()
        )
        .into_bytes();
        img.extend(&jpeg);
        img.extend(b"\nendstream");
        objects.push(img);
    }
    let mut file: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (n, body) in objects.iter().enumerate() {
        offsets.push(file.len());
        file.extend(format!("{} 0 obj\n", n + 1).bytes());
        file.extend(body);
        file.extend(b"\nendobj\n");
    }
    let xref = file.len();
    file.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for o in offsets {
        file.extend(format!("{o:010} 00000 n \n").bytes());
    }
    file.extend(format!("trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    std::fs::write(out, file).with_context(|| format!("writing {}", out.display()))
}

// ---------------------------------------------------------------------------------------
// PowerPoint
// ---------------------------------------------------------------------------------------

/// A slide the shape of the frame, 12 192 000 EMU (13.333 in) on its long side.
fn slide_emu((w, h): (u32, u32)) -> (i64, i64) {
    let long = 12_192_000.0;
    let (w, h) = (w as f64, h as f64);
    if w >= h { (long as i64, (long * h / w).round() as i64) } else { ((long * w / h).round() as i64, long as i64) }
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            // Control characters are not allowed in XML 1.0.
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn slide_xml(slide: &Slide, (cx, cy): (i64, i64)) -> String {
    format!(
        "{XML}<p:sld xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
<p:pic><p:nvPicPr><p:cNvPr id=\"2\" name=\"Frame\" descr=\"{descr}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr>\
<p:blipFill><a:blip r:embed=\"rId2\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
<p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",
        descr = xml_escape(&slide.title),
    )
}

fn master_xml() -> String {
    format!(
        "{XML}<p:sldMaster xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"000000\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
</p:spTree></p:cSld>\
<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst>\
<p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz=\"2800\"/></a:lvl1pPr></p:titleStyle>\
<p:bodyStyle><a:lvl1pPr><a:defRPr sz=\"1600\"/></a:lvl1pPr></p:bodyStyle>\
<p:otherStyle><a:lvl1pPr><a:defRPr sz=\"1600\"/></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"
    )
}

fn layout_xml() -> String {
    format!(
        "{XML}<p:sldLayout xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\"><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
    )
}

fn theme_xml() -> String {
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    let line = "<a:ln w=\"9525\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln>";
    format!(
        "{XML}<a:theme xmlns:a=\"{NS_A}\" name=\"Manim Studio\"><a:themeElements>\
<a:clrScheme name=\"Manim Studio\"><a:dk1><a:srgbClr val=\"17171C\"/></a:dk1><a:lt1><a:srgbClr val=\"FFFFFF\"/></a:lt1><a:dk2><a:srgbClr val=\"474752\"/></a:dk2><a:lt2><a:srgbClr val=\"F2F2F4\"/></a:lt2>\
<a:accent1><a:srgbClr val=\"E8A33D\"/></a:accent1><a:accent2><a:srgbClr val=\"3D7BE8\"/></a:accent2><a:accent3><a:srgbClr val=\"3DA86B\"/></a:accent3><a:accent4><a:srgbClr val=\"D6504B\"/></a:accent4><a:accent5><a:srgbClr val=\"8A5CD6\"/></a:accent5><a:accent6><a:srgbClr val=\"6B6B76\"/></a:accent6>\
<a:hlink><a:srgbClr val=\"3D7BE8\"/></a:hlink><a:folHlink><a:srgbClr val=\"8A5CD6\"/></a:folHlink></a:clrScheme>\
<a:fontScheme name=\"Manim Studio\"><a:majorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme>\
<a:fmtScheme name=\"Manim Studio\"><a:fillStyleLst>{fill}{fill}{fill}</a:fillStyleLst><a:lnStyleLst>{line}{line}{line}</a:lnStyleLst>\
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>\
<a:bgFillStyleLst>{fill}{fill}{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"
    )
}

fn rels(items: &[(&str, &str, String)]) -> String {
    let mut s = format!("{XML}<Relationships xmlns=\"{PKG_REL}\">");
    for (id, kind, target) in items {
        s.push_str(&format!("<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{target}\"/>"));
    }
    s.push_str("</Relationships>");
    s
}

/// A PowerPoint file (16:9) with one slide per scene.
pub fn write_pptx(title: &str, slides: &[Slide], out: &Path) -> Result<()> {
    if slides.is_empty() {
        bail!("no slides");
    }
    let mut zip = Zip::default();
    // Every slide is the shape of the first frame (all of a video's frames share one).
    let first = std::fs::read(&slides[0].image).with_context(|| format!("reading {}", slides[0].image.display()))?;
    let size = slide_emu(jpeg_size(&first).with_context(|| format!("{} is not a JPEG", slides[0].image.display()))?);
    let ct_slides: String = (1..=slides.len())
        .map(|n| format!("<Override PartName=\"/ppt/slides/slide{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"))
        .collect();
    zip.add(
        "[Content_Types].xml",
        format!(
            "{XML}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>{ct_slides}</Types>"
        )
        .into_bytes(),
    );
    zip.add(
        "_rels/.rels",
        rels(&[
            ("rId1", "officeDocument", "ppt/presentation.xml".into()),
            ("rId2", "extended-properties", "docProps/app.xml".into()),
        ])
        .replace(
            "</Relationships>",
            "<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>",
        )
        .into_bytes(),
    );
    zip.add(
        "docProps/core.xml",
        format!(
            "{XML}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
<dc:title>{}</dc:title><dc:creator>Manim Studio</dc:creator></cp:coreProperties>",
            xml_escape(title)
        )
        .into_bytes(),
    );
    zip.add(
        "docProps/app.xml",
        format!(
            "{XML}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Manim Studio</Application><Slides>{}</Slides></Properties>",
            slides.len()
        )
        .into_bytes(),
    );
    let ids: String = (0..slides.len()).map(|i| format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 256 + i, i + 2)).collect();
    zip.add(
        "ppt/presentation.xml",
        format!(
            "{XML}<p:presentation xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" saveSubsetFonts=\"1\">\
<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst>\
<p:sldSz cx=\"{}\" cy=\"{}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>",
            size.0, size.1
        )
        .into_bytes(),
    );
    let mut pres_rels = vec![("rId1".to_string(), "slideMaster", "slideMasters/slideMaster1.xml".to_string())];
    for i in 0..slides.len() {
        pres_rels.push((format!("rId{}", i + 2), "slide", format!("slides/slide{}.xml", i + 1)));
    }
    pres_rels.push((format!("rId{}", slides.len() + 2), "theme", "theme/theme1.xml".into()));
    let pres_rels: Vec<(&str, &str, String)> = pres_rels.iter().map(|(a, b, c)| (a.as_str(), *b, c.clone())).collect();
    zip.add("ppt/_rels/presentation.xml.rels", rels(&pres_rels).into_bytes());
    zip.add("ppt/slideMasters/slideMaster1.xml", master_xml().into_bytes());
    zip.add(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml".into()), ("rId2", "theme", "../theme/theme1.xml".into())]).into_bytes(),
    );
    zip.add("ppt/slideLayouts/slideLayout1.xml", layout_xml().into_bytes());
    zip.add(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml".into())]).into_bytes(),
    );
    zip.add("ppt/theme/theme1.xml", theme_xml().into_bytes());
    for (i, slide) in slides.iter().enumerate() {
        let n = i + 1;
        let jpeg = std::fs::read(&slide.image).with_context(|| format!("reading {}", slide.image.display()))?;
        zip.add(&format!("ppt/slides/slide{n}.xml"), slide_xml(slide, size).into_bytes());
        zip.add(
            &format!("ppt/slides/_rels/slide{n}.xml.rels"),
            rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml".into()), ("rId2", "image", format!("../media/image{n}.jpg"))]).into_bytes(),
        );
        zip.add(&format!("ppt/media/image{n}.jpg"), jpeg);
    }
    std::fs::write(out, zip.finish()).with_context(|| format!("writing {}", out.display()))
}

// ---------------------------------------------------------------------------------------
// A stored (uncompressed) ZIP: enough for an Office package, whose big parts are JPEGs.
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct Zip {
    data: Vec<u8>,
    entries: Vec<(String, u32, u32, u32)>,
}

impl Zip {
    fn add(&mut self, name: &str, bytes: Vec<u8>) {
        let mut crc = flate2::Crc::new();
        crc.update(&bytes);
        let (crc, size, offset) = (crc.sum(), bytes.len() as u32, self.data.len() as u32);
        let d = &mut self.data;
        d.extend(0x0403_4b50u32.to_le_bytes());
        // Version 2.0, UTF-8 names, stored, DOS date 1980-01-01.
        d.extend([20, 0, 0x00, 0x08, 0, 0, 0, 0, 0x21, 0]);
        d.extend(crc.to_le_bytes());
        d.extend(size.to_le_bytes());
        d.extend(size.to_le_bytes());
        d.extend((name.len() as u16).to_le_bytes());
        d.extend(0u16.to_le_bytes());
        d.extend(name.as_bytes());
        d.extend(bytes);
        self.entries.push((name.to_string(), crc, size, offset));
    }

    fn finish(mut self) -> Vec<u8> {
        let start = self.data.len() as u32;
        for (name, crc, size, offset) in &self.entries {
            let d = &mut self.data;
            d.extend(0x0201_4b50u32.to_le_bytes());
            d.extend([20, 0, 20, 0, 0x00, 0x08, 0, 0, 0, 0, 0x21, 0]);
            d.extend(crc.to_le_bytes());
            d.extend(size.to_le_bytes());
            d.extend(size.to_le_bytes());
            d.extend((name.len() as u16).to_le_bytes());
            d.extend([0u8; 12]);
            d.extend(offset.to_le_bytes());
            d.extend(name.as_bytes());
        }
        let end = self.data.len() as u32;
        let n = self.entries.len() as u16;
        let d = &mut self.data;
        d.extend(0x0605_4b50u32.to_le_bytes());
        d.extend([0u8; 4]);
        d.extend(n.to_le_bytes());
        d.extend(n.to_le_bytes());
        d.extend((end - start).to_le_bytes());
        d.extend(start.to_le_bytes());
        d.extend(0u16.to_le_bytes());
        self.data
    }
}

/// The slide files sit next to the video, under its name (`Title.pdf`, `Title.pptx`).
pub fn paths(video: &Path) -> (PathBuf, PathBuf) {
    (video.with_extension("pdf"), video.with_extension("pptx"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2×1 baseline JPEG header is enough for the size reader.
    fn tiny_jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8];
        j.extend(h.to_be_bytes());
        j.extend(w.to_be_bytes());
        j.extend([1, 1, 0x11, 0]);
        j.extend([0xFF, 0xD9]);
        j
    }

    #[test]
    fn jpeg_size_reads_the_frame_header() {
        assert_eq!(jpeg_size(&tiny_jpeg(1920, 1080)), Some((1920, 1080)));
        assert_eq!(jpeg_size(b"not a jpeg"), None);
    }

    #[test]
    fn pages_take_the_shape_of_the_frame() {
        assert_eq!(page_size((1920, 1080)), (960.0, 540.0));
        let (w, h) = page_size((1080, 1920));
        assert_eq!(h, 960.0);
        assert!((w - 540.0).abs() < 1e-9);
        assert_eq!(slide_emu((1920, 1080)), (12_192_000, 6_858_000));
    }

    /// The cross-reference table must point at the objects, or readers rebuild or refuse.
    #[test]
    fn pdf_cross_references_point_at_their_objects() {
        let dir = std::env::temp_dir().join(format!("studio-slides-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("a.jpg");
        std::fs::write(&img, tiny_jpeg(1280, 720)).unwrap();
        let slides = vec![Slide { title: "Area".into(), image: img }; 2];
        let out = dir.join("deck.pdf");
        write_pdf("Deck", &slides, &out).unwrap();
        let pdf = std::fs::read(&out).unwrap();
        let rfind = |needle: &[u8]| pdf.windows(needle.len()).rposition(|w| w == needle).unwrap();
        let xref = rfind(b"
xref
") + 1;
        let tail = String::from_utf8_lossy(&pdf[xref..]).into_owned();
        for (n, line) in tail.lines().skip(3).take_while(|l| l.ends_with(" n ")).enumerate() {
            let at: usize = line[..10].parse().unwrap();
            assert!(pdf[at..].starts_with(format!("{} 0 obj", n + 1).as_bytes()), "object {} is not at {at}", n + 1);
        }
        let start: usize = tail.lines().skip_while(|l| *l != "startxref").nth(1).unwrap().parse().unwrap();
        assert_eq!(start, xref);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zip_ends_with_a_directory_naming_every_part() {
        let mut z = Zip::default();
        z.add("a.txt", b"hello".to_vec());
        z.add("dir/b.txt", b"world".to_vec());
        let bytes = z.finish();
        let eocd = bytes.len() - 22;
        assert_eq!(&bytes[eocd..eocd + 4], 0x0605_4b50u32.to_le_bytes());
        assert_eq!(u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]), 2);
        let dir_at = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
        assert_eq!(&bytes[dir_at..dir_at + 4], 0x0201_4b50u32.to_le_bytes());
    }
}
