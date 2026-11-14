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
    pub fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
        // Length-delimit fields so ("ab","c") and ("a","bc") differ.
        self.0 ^= bytes.len() as u64;
        self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
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
    h.write(b"pipeline-3");
    h.write(extra.as_bytes());
    h.write(theme_json.as_bytes());
    h.write(format!("{}x{}@{}", output.width, output.height, output.fps).as_bytes());
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    for dep in dependencies(script, root) {
        let rel = dep.strip_prefix(&root_c).unwrap_or(&dep);
        h.write(rel.to_string_lossy().replace('\\', "/").as_bytes());
        h.write(&std::fs::read(&dep).unwrap_or_default());
    }
    h.finish()
}

/// Where a scene's cached render lives inside its video folder.
pub fn scene_video(video_dir: &Path, script: &Path, key: u64) -> PathBuf {
    let stem = script.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "scene".into());
    video_dir.join(".cache").join("scenes").join(format!("{stem}-{key:016x}.mp4"))
}

/// Remove cached renders of `script` other than `keep` (old versions of the scene).
pub fn prune(video_dir: &Path, script: &Path, keep: &Path) {
    let Some(stem) = script.file_stem().map(|s| s.to_string_lossy().into_owned()) else { return };
    let dir = video_dir.join(".cache").join("scenes");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let ours = name
            .strip_prefix(&format!("{stem}-"))
            .is_some_and(|rest| rest.len() == 16 + 4 || rest.ends_with("-partial.mp4"));
        if ours && p != keep {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_relative_imports() {
        let src = "import { Scene } from \"manim\";\nimport { kit } from '../lib/kit';\nexport { x } from \"./x\";";
        assert_eq!(relative_imports(src), vec!["../lib/kit".to_string(), "./x".to_string()]);
    }

    #[test]
    fn key_changes_with_imports() {
        let dir = std::env::temp_dir().join(format!("studio-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        let scene = dir.join("scenes/a.ts");
        std::fs::write(&scene, "import { k } from \"../lib/kit\";\n").unwrap();
        std::fs::write(dir.join("lib/kit.ts"), "export const k = 1;").unwrap();
        let out = Output::new(1280, 720, 30.0);
        let a = scene_key(&scene, &dir, "{}", out);
        assert_eq!(a, scene_key(&scene, &dir, "{}", out));
        std::fs::write(dir.join("lib/kit.ts"), "export const k = 2;").unwrap();
        let b = scene_key(&scene, &dir, "{}", out);
        assert_ne!(a, b);
        assert_ne!(b, scene_key(&scene, &dir, "{\"x\":1}", out));
        assert_ne!(b, scene_key(&scene, &dir, "{}", Output::new(1920, 1080, 30.0)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
