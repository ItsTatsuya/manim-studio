//! Per-scene render cache keys.
//!
//! A scene's key covers everything that changes its pixels: the engine revision, the
//! script and every relative file it imports (recursively), the house style and the
//! output settings. Assets referenced by path (images, sounds) are hashed when the
//! script names them in a string literal that resolves to a file in the project.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::Output;

/// 64-bit FNV-1a. Stable across Rust versions and platforms (unlike `DefaultHasher`).
#[derive(Clone, Copy)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn mix(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn mix_len(&mut self, len: u64) {
        // Length-delimit fields so ("ab","c") and ("a","bc") differ.
        self.0 ^= len;
        self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.mix(bytes);
        self.mix_len(bytes.len() as u64);
    }

    /// Same as `write(&std::fs::read(path))`, but streamed so a big asset is never held in
    /// memory. An unreadable file counts as empty.
    pub fn write_file(&mut self, path: &Path) {
        use std::io::Read;
        let Ok(mut f) = std::fs::File::open(path) else { return self.write(&[]) };
        let mut buf = vec![0u8; 1 << 16];
        let mut total = 0u64;
        loop {
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.mix(&buf[..n]);
                    total += n as u64;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        self.mix_len(total);
    }

    pub fn finish(self) -> u64 {
        self.0
    }
}

/// Relative module specifiers (`"./x"`, `'../lib/kit'`) imported by a script.
fn relative_imports(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let l = line.trim_start();
        if !(l.starts_with("import") || l.starts_with("export") || l.contains("from ")) {
            continue;
        }
        for quote in ['"', '\''] {
            let mut rest = l;
            while let Some(i) = rest.find(quote) {
                let tail = &rest[i + 1..];
                let Some(j) = tail.find(quote) else { break };
                let spec = &tail[..j];
                if spec.starts_with("./") || spec.starts_with("../") {
                    out.push(spec.to_string());
                }
                rest = &tail[j + 1..];
            }
        }
    }
    out
}

/// Every string literal that names an existing file under `root` (assets).
fn asset_refs(src: &str, base: &Path, root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for quote in ['"', '\''] {
        let mut rest = src;
        while let Some(i) = rest.find(quote) {
            let tail = &rest[i + 1..];
            let Some(j) = tail.find(['\n', quote]) else { break };
            let lit = &tail[..j];
            rest = &tail[j + 1..];
            if lit.is_empty() || lit.len() > 200 || !lit.contains('.') || lit.contains("://") {
                continue;
            }
            for cand in [root.join(lit), base.join(lit)] {
                if cand.is_file() && cand.starts_with(root) {
                    out.push(cand);
                    break;
                }
            }
        }
    }
    out
}

fn resolve(base: &Path, spec: &str) -> Option<PathBuf> {
    let p = base.join(spec);
    let mut cands = vec![p.clone()];
    for ext in ["ts", "js", "mjs"] {
        let mut s = p.clone().into_os_string();
        s.push(format!(".{ext}"));
        cands.push(PathBuf::from(s));
    }
    cands.push(p.join("index.ts"));
    cands.into_iter().find(|c| c.is_file())
}

/// The script and every file it depends on, in a stable order.
pub fn dependencies(script: &Path, root: &Path) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![script.to_path_buf()];
    while let Some(p) = stack.pop() {
        let p = p.canonicalize().unwrap_or(p);
        if !seen.insert(p.clone()) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&p) else { continue };
        let base = p.parent().unwrap_or(Path::new(".")).to_path_buf();
        for spec in relative_imports(&src) {
            if let Some(f) = resolve(&base, &spec) {
                stack.push(f);
            }
        }
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        for a in asset_refs(&src, &base, &root) {
            seen.insert(a.canonicalize().unwrap_or(a));
        }
    }
    seen.into_iter().collect()
}

/// The cache key of one scene render.
pub fn scene_key(script: &Path, root: &Path, theme_json: &str, output: Output) -> u64 {
    scene_key_with(script, root, theme_json, output, "")
}

/// The cache key with extra inputs (e.g. the narration file's name, itself a content hash).
pub fn scene_key_with(script: &Path, root: &Path, theme_json: &str, output: Output, extra: &str) -> u64 {
    let mut h = Fnv::default();
    h.write(crate::ENGINE_REV.as_bytes());
    // Studio's own scene pipeline (narration timing); bump when it changes renders.
    h.write(b"pipeline-4");
    h.write(extra.as_bytes());
    h.write(theme_json.as_bytes());
    // The encoder is not keyed on purpose (see `fonts_fingerprint`); the bundled fonts are,
    // by the caller, through `extra`.
    h.write(format!("{}x{}@{}", output.width, output.height, output.fps).as_bytes());
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    for dep in dependencies(script, root) {
        let rel = dep.strip_prefix(&root_c).unwrap_or(&dep);
        h.write(rel.to_string_lossy().replace('\\', "/").as_bytes());
        h.write_file(&dep);
    }
    h.finish()
}

/// Where a scene's cached render lives inside its video folder.
pub fn scene_video(video_dir: &Path, script: &Path, key: u64) -> PathBuf {
    let stem = script.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "scene".into());
    video_dir.join(".cache").join("scenes").join(format!("{stem}-{key:016x}.mp4"))
}

/// A fingerprint of the bundled fonts' names and sizes (not their contents): text renders
/// with them, so changing a font changes the pixels. Part of the scene key via `extra`.
///
/// The encoder choice (`Encoder`, hardware or x264) is deliberately *not* part of the
/// key: the video is equivalent whichever encoder wrote it, so switching it keeps the cache.
pub fn fonts_fingerprint(dir: Option<&Path>) -> String {
    let Some(dir) = dir else { return String::new() };
    let mut fonts: Vec<(String, u64)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok().filter(|m| m.is_file())?;
            Some((e.file_name().to_string_lossy().into_owned(), m.len()))
        })
        .collect();
    fonts.sort();
    fonts.iter().map(|(n, s)| format!("{n}:{s}")).collect::<Vec<_>>().join(",")
}

/// The sidecar files that sit next to a cached scene video (`.seconds`, `.starts`, and
/// the `.output` label `prune` uses to keep one render per output preset).
const SIDECARS: [&str; 3] = ["seconds", "starts", "output"];

/// Record which output preset a cached scene video was rendered for.
pub fn write_output_label(video: &Path, output: Output) {
    let _ = std::fs::write(video.with_extension("output"), format!("{}x{}@{}", output.width, output.height, output.fps));
}

/// What a file in the scene cache folder is: `{stem}-{16 hex}.{ext}` (a render or one of
/// its sidecars) or a partial render. Returns the key part for renders and sidecars.
fn cache_entry<'a>(name: &'a str, stem: &str) -> Option<(Option<&'a str>, &'a str)> {
    let rest = name.strip_prefix(stem)?.strip_prefix('-')?;
    // Partial renders: `{stem}-partial.mp4` and the engine's `{stem}-partial.partial.mp4`.
    if matches!(rest, "partial.mp4" | "partial.partial.mp4") {
        return Some((None, rest));
    }
    let (key, ext) = rest.split_once('.')?;
    (key.len() == 16 && key.chars().all(|c| c.is_ascii_hexdigit()) && (ext == "mp4" || SIDECARS.contains(&ext))).then_some((Some(key), ext))
}

/// Remove cached renders of `script` that `keep` replaces: old versions of the scene for
/// the same output preset, their sidecars, and partial leftovers. Renders for *other*
/// output presets are kept (one per stem and preset, so flipping between 720p and 1080p
/// doesn't re-render everything); a render with no `.output` label counts as old.
pub fn prune(video_dir: &Path, script: &Path, keep: &Path) {
    let Some(stem) = script.file_stem().map(|s| s.to_string_lossy().into_owned()) else { return };
    let dir = video_dir.join(".cache").join("scenes");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let keep_name = keep.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let keep_key = cache_entry(&keep_name, &stem).and_then(|(k, _)| k).map(str::to_string);
    let label = |key: &str| std::fs::read_to_string(dir.join(format!("{stem}-{key}.output"))).ok();
    let keep_label = keep_key.as_deref().and_then(label);
    let names: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    for name in &names {
        let Some((key, _)) = cache_entry(name, &stem) else { continue };
        let remove = match key {
            None => true,
            Some(k) if Some(k) == keep_key.as_deref() => false,
            // A different preset's render survives while its video is still there.
            Some(k) => {
                let has_video = names.iter().any(|n| *n == format!("{stem}-{k}.mp4"));
                !(has_video && keep_label.is_some() && label(k).is_some_and(|l| Some(&l) != keep_label.as_ref()))
            }
        };
        if remove {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_changes_with_imports() {
        let dir = std::env::temp_dir().join(format!("studio-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        let scene = dir.join("scenes/a.ts");
        // Both quote styles and re-exports count as imports.
        std::fs::write(&scene, "import { Scene } from \"manim\";\nimport { k } from '../lib/kit';\nexport { x } from \"./x\";\n").unwrap();
        std::fs::write(dir.join("lib/kit.ts"), "export const k = 1;").unwrap();
        std::fs::write(dir.join("scenes/x.ts"), "export const x = 1;").unwrap();
        let out = Output::new(1280, 720, 30.0);
        let a = scene_key(&scene, &dir, "{}", out);
        assert_eq!(a, scene_key(&scene, &dir, "{}", out));
        std::fs::write(dir.join("lib/kit.ts"), "export const k = 2;").unwrap();
        let after_kit = scene_key(&scene, &dir, "{}", out);
        assert_ne!(a, after_kit);
        std::fs::write(dir.join("scenes/x.ts"), "export const x = 2;").unwrap();
        let b = scene_key(&scene, &dir, "{}", out);
        assert_ne!(after_kit, b);
        assert_ne!(b, scene_key(&scene, &dir, "{\"x\":1}", out));
        assert_ne!(b, scene_key(&scene, &dir, "{}", Output::new(1920, 1080, 30.0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_keeps_one_render_per_output_preset() {
        let dir = std::env::temp_dir().join(format!("studio-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let scenes = dir.join(".cache/scenes");
        std::fs::create_dir_all(&scenes).unwrap();
        let script = dir.join("scenes/01-a.ts");
        let (hd, sd) = (Output::new(1920, 1080, 30.0), Output::new(1280, 720, 30.0));
        let touch = |name: &str| std::fs::write(scenes.join(name), "x").unwrap();
        let video = |key: &str, o: Output| {
            touch(&format!("01-a-{key}.mp4"));
            touch(&format!("01-a-{key}.seconds"));
            touch(&format!("01-a-{key}.starts"));
            write_output_label(&scenes.join(format!("01-a-{key}.mp4")), o);
        };
        video("0000000000000001", hd);
        video("0000000000000002", sd);
        video("0000000000000003", hd);
        touch("01-a-partial.mp4");
        touch("01-a-partial.partial.mp4");
        touch("01-a-0000000000000009.seconds");
        touch("01-ab-0000000000000001.mp4");
        prune(&dir, &script, &scenes.join("01-a-0000000000000003.mp4"));
        let mut left: Vec<String> = std::fs::read_dir(&scenes).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(
            left,
            [
                "01-a-0000000000000002.mp4", "01-a-0000000000000002.output", "01-a-0000000000000002.seconds", "01-a-0000000000000002.starts",
                "01-a-0000000000000003.mp4", "01-a-0000000000000003.output", "01-a-0000000000000003.seconds", "01-a-0000000000000003.starts",
                "01-ab-0000000000000001.mp4",
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
