//! Projects and videos on disk.
//!
//! ```text
//! <data>/projects/<project>/project.json      { "name": "Grade 8 Geometry" }
//! <data>/projects/<project>/<video>/video.json title, grade, status, storyboard, settings, session
//!                                   chat.json  the assistant conversation as shown
//!                                   scenes/    one TypeScript scene per file, in order
//!                                   lib/ assets/ .cache/ exports/
//! ```

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use studio_agent::prompt::Storyboard;
use studio_ui::IconName;

use crate::model::{Msg, Project, Provider, Scene, SceneEngine, Video, VideoSettings, VideoStatus};

/// `STUDIO_HOME=<dir>` keeps settings and data in one folder instead of the user's
/// profile (for tests and screenshots that must not touch real projects).
pub fn studio_home() -> Option<PathBuf> {
    std::env::var_os("STUDIO_HOME").map(PathBuf::from)
}

pub fn data_dir() -> Option<PathBuf> {
    if let Some(home) = studio_home() {
        return Some(home.join("data"));
    }
    directories::ProjectDirs::from("app", "ManimStudio", "Manim Studio").map(|d| d.data_dir().to_path_buf())
}

pub fn projects_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("projects"))
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct ProjectFile {
    name: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    no_watermark: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct VideoFile {
    title: String,
    grade: String,
    status: String,
    updated: u64,
    source: Option<String>,
    #[serde(flatten)]
    settings: VideoSettings,
    storyboard: Option<Storyboard>,
    /// "claude" / "codex"
    provider: Option<String>,
    session: Option<String>,
    request: Option<String>,
    /// Per-scene teacher notes, by scene index.
    notes: Vec<Option<String>>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChatRecord {
    User { text: String },
    Assistant { text: String },
    Activity { ok: bool, text: String, meta: String },
    Problem { provider: String, raw: String },
    Usage { text: String },
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "untitled".into() } else { out }
}

/// A folder name under `parent` that doesn't exist yet.
fn unique(parent: &Path, base: &str) -> PathBuf {
    let mut p = parent.join(base);
    let mut n = 2;
    while p.exists() {
        p = parent.join(format!("{base}-{n}"));
        n += 1;
    }
    p
}

fn humanize(stem: &str) -> String {
    let words = stem.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-').replace('-', " ");
    let mut c = words.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_else(|| stem.to_string())
}

/// Scenes from the storyboard paired with the scene files on disk, in order.
pub fn scenes_for(dir: &Path, storyboard: Option<&Storyboard>, notes: &[Option<String>]) -> Vec<Scene> {
    let files = studio_render::sample::scene_files(dir);
    let beats = storyboard.map(|s| s.scenes.as_slice()).unwrap_or(&[]);
    let n = files.len().max(beats.len());
    (0..n)
        .map(|i| {
            let mut s = match beats.get(i) {
                Some(b) => Scene::from_beat(i, b),
                None => {
                    let stem = files[i].file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    Scene::from_beat(i, &studio_agent::prompt::Beat { title: humanize(&stem), say: String::new(), visual: String::new(), seconds: 10 })
                }
            };
            s.engine = SceneEngine { file: files.get(i).cloned(), ..Default::default() };
            s.note = notes.get(i).cloned().flatten();
            s
        })
        .collect()
}

/// Load one video folder (after moving it, for example).
pub fn load_video_dir(dir: &Path) -> Option<Video> {
    load_video(dir)
}

/// Write `bytes` to `path` through a sibling temp file, so a crash or a second instance
/// never leaves half a file behind.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Backups kept per file.
const BACKUPS_KEPT: usize = 5;

/// `chat.json` at `secs` -> `chat.json.1700000000.bak`.
fn backup_name(file: &str, secs: u64) -> String {
    format!("{file}.{secs}.bak")
}

/// The timestamp in a backup's name (`file.<secs>.bak`), if `name` is a backup of `file`.
fn backup_secs(file: &str, name: &str) -> Option<u64> {
    name.strip_prefix(file)?.strip_prefix('.')?.strip_suffix(".bak")?.parse().ok()
}

/// Of the sibling file names, the backups of `file` to delete so only the newest `keep` stay.
fn stale_backups(file: &str, names: &[String], keep: usize) -> Vec<String> {
    let mut found: Vec<(u64, &String)> = names.iter().filter_map(|n| Some((backup_secs(file, n)?, n))).collect();
    found.sort();
    let drop = found.len().saturating_sub(keep);
    found.into_iter().take(drop).map(|(_, n)| n.clone()).collect()
}

/// Keep a timestamped copy of `path` (`<name>.<unix-secs>.bak`), or move it aside with
/// `move_it`, so a later save (which writes over the file) doesn't destroy what the teacher
/// had. At most [`BACKUPS_KEPT`] backups of a file stay; a copy identical to the newest
/// backup isn't made again.
fn backup(path: &Path, move_it: bool) {
    let Some(file) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return };
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    let newest = names.iter().filter_map(|n| Some((backup_secs(&file, n)?, n))).max().map(|(_, n)| dir.join(n));
    let same_as_newest = newest.is_some_and(|n| matches!((std::fs::read(&n), std::fs::read(path)), (Ok(a), Ok(b)) if a == b));
    // Never write over an existing backup (two backups in one second, or a clock that
    // went backwards): step the stamp until the name is free.
    let mut secs = now();
    while dir.join(backup_name(&file, secs)).exists() {
        secs += 1;
    }
    let name = backup_name(&file, secs);
    let bak = dir.join(&name);
    if same_as_newest {
        if move_it {
            let _ = std::fs::remove_file(path);
        }
    } else if move_it {
        if std::fs::rename(path, &bak).is_err() {
            let _ = std::fs::copy(path, &bak);
        }
    } else {
        let _ = std::fs::copy(path, &bak);
    }
    if bak.exists() && !names.contains(&name) {
        names.push(name.clone());
    }
    // The backup just made is never pruned, even if its stamp sorts oldest.
    for old in stale_backups(&file, &names, BACKUPS_KEPT) {
        if old != name {
            let _ = std::fs::remove_file(dir.join(old));
        }
    }
}

/// Keep a copy of a file we couldn't read (see [`backup`]).
pub fn back_up_unreadable(path: &Path) {
    backup(path, false);
}

/// Keep a copy of a file that is about to be written by something that may not understand it.
pub fn back_up_copy(path: &Path) {
    backup(path, false);
}

/// Move a file we can't read at all out of the way (see [`backup`]).
pub fn move_aside(path: &Path) {
    backup(path, true);
}

fn load_video(dir: &Path) -> Option<Video> {
    let bytes = std::fs::read(dir.join("video.json")).ok()?;
    let f: VideoFile = match serde_json::from_slice(&bytes) {
        Ok(f) => f,
        Err(_) => {
            back_up_unreadable(&dir.join("video.json"));
            return None;
        }
    };
    let title = if f.title.is_empty() { humanize(&dir.file_name()?.to_string_lossy()) } else { f.title.clone() };
    let scenes = scenes_for(dir, f.storyboard.as_ref(), &f.notes);
    let session = match (f.provider.as_deref().and_then(Provider::from_key), f.session) {
        (Some(p), Some(s)) => Some((p, s)),
        _ => None,
    };
    let updated = if f.updated > 0 {
        f.updated
    } else {
        std::fs::metadata(dir.join("video.json"))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
    };
    Some(Video {
        title,
        status: VideoStatus::from_key(&f.status),
        updated,
        scenes,
        source: f.source,
        grade: f.grade,
        dir: Some(dir.to_path_buf()),
        settings: f.settings,
        storyboard: f.storyboard,
        session,
        chat: load_chat(dir),
        busy: None,
        request: f.request,
        resume_next: false,
        voice_gen: 0,
    })
}

/// Write `video.json` for a video (best effort).
pub fn save_video(v: &Video) {
    let Some(dir) = &v.dir else { return };
    let f = VideoFile {
        title: v.title.clone(),
        grade: v.grade.clone(),
        status: v.status.key().into(),
        updated: v.updated,
        source: v.source.clone(),
        settings: v.settings,
        storyboard: v.storyboard.clone(),
        provider: v.session.as_ref().map(|(p, _)| p.key().to_string()),
        session: v.session.as_ref().map(|(_, s)| s.clone()),
        request: v.request.clone(),
        notes: v.scenes.iter().map(|s| s.note.clone()).collect(),
    };
    if let Ok(json) = serde_json::to_vec_pretty(&f) {
        let _ = write_atomic(&dir.join("video.json"), &json);
    }
}

/// Chat records from a `chat.json`, one by one: a record this version doesn't know
/// (written by a newer one) is skipped instead of discarding the whole conversation.
/// Returns the records and how many were skipped, or `None` when the file isn't a JSON list.
fn parse_chat(bytes: &[u8]) -> Option<(Vec<ChatRecord>, usize)> {
    let values: Vec<serde_json::Value> = serde_json::from_slice(bytes).ok()?;
    let total = values.len();
    let records: Vec<ChatRecord> = values.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect();
    let skipped = total - records.len();
    Some((records, skipped))
}

fn load_chat(dir: &Path) -> Vec<Msg> {
    let path = dir.join("chat.json");
    let Ok(bytes) = std::fs::read(&path) else { return vec![] };
    let records = match parse_chat(&bytes) {
        Some((records, skipped)) => {
            if skipped > 0 {
                // The next save writes only what this version understands.
                back_up_unreadable(&path);
            }
            records
        }
        None => {
            // Not readable at all: move it aside rather than let the next save replace it.
            move_aside(&path);
            return vec![];
        }
    };
    records
        .into_iter()
        .map(|r| match r {
            ChatRecord::User { text } => Msg::User(text),
            ChatRecord::Assistant { text } => Msg::Assistant(text),
            // Failures saved before they were explained become explained ones.
            ChatRecord::Activity { ok: false, text, .. } if text.starts_with("Claude Code stopped: ") || text.starts_with("Codex stopped: ") => {
                let (cli, raw) = text.split_once(" stopped: ").unwrap_or_default();
                Msg::Problem { provider: if cli == "Codex" { Provider::OpenAi } else { Provider::Claude }, raw: raw.to_string() }
            }
            ChatRecord::Activity { ok, text, meta } => {
                let icon = if text.starts_with("Edited") || text.starts_with("Wrote") {
                    IconName::Script
                } else if ok {
                    IconName::Check
                } else {
                    IconName::Info
                };
                Msg::Activity { icon, ok, text, meta }
            }
            ChatRecord::Problem { provider, raw } => Msg::Problem { provider: Provider::from_key(&provider).unwrap_or(Provider::Claude), raw },
            ChatRecord::Usage { text } => Msg::Usage(text),
        })
        .filter(|m| !m.is_work_detail())
        .collect()
}

/// Save the conversation (live rows like "Rendering" are not kept).
pub fn save_chat(v: &Video) {
    let Some(dir) = &v.dir else { return };
    let records: Vec<ChatRecord> = v
        .chat
        .iter()
        .filter_map(|m| match m {
            Msg::User(t) => Some(ChatRecord::User { text: t.clone() }),
            Msg::Assistant(t) => Some(ChatRecord::Assistant { text: t.clone() }),
            Msg::Activity { ok, text, meta, .. } => Some(ChatRecord::Activity { ok: *ok, text: text.clone(), meta: meta.clone() }),
            Msg::Problem { provider, raw } => Some(ChatRecord::Problem { provider: provider.key().into(), raw: raw.clone() }),
            Msg::Usage(t) => Some(ChatRecord::Usage { text: t.clone() }),
            Msg::Working(_) | Msg::Rendering => None,
        })
        .collect();
    if let Ok(json) = serde_json::to_vec_pretty(&records) {
        let _ = write_atomic(&dir.join("chat.json"), &json);
    }
}

/// Folder name of the bundled sample lesson's project.
const SAMPLE_PROJECT: &str = "grade-8-geometry";

/// The project that holds the bundled sample lesson.
pub fn is_sample_project(p: &Project) -> bool {
    p.dir.file_name().is_some_and(|n| n == SAMPLE_PROJECT)
}

/// Every project and video on disk. The bundled sample lesson is installed first, so
/// there is always something to open.
pub fn load() -> Vec<Project> {
    let Some(root) = projects_dir() else { return vec![] };
    let sample_project = root.join(SAMPLE_PROJECT);
    let sample_video = sample_project.join("area-of-a-circle");
    // Install the sample once. After that only refresh it while it exists, so deleting
    // it (or its folder) sticks.
    let marker = root.join(".sample-installed");
    if !marker.exists() {
        let _ = std::fs::create_dir_all(&sample_project);
        if !sample_project.join("project.json").is_file() {
            let _ = std::fs::write(sample_project.join("project.json"), r#"{ "name": "Grade 8 Geometry" }"#);
        }
        if studio_render::sample::install(&sample_video).is_ok() {
            let _ = std::fs::write(&marker, "");
        }
    } else if sample_video.join("video.json").is_file() {
        let _ = studio_render::sample::install(&sample_video);
    }

    let mut projects: Vec<Project> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|dir| {
            let f: ProjectFile = match std::fs::read(dir.join("project.json")) {
                Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|_| {
                    back_up_unreadable(&dir.join("project.json"));
                    ProjectFile::default()
                }),
                Err(_) => ProjectFile::default(),
            };
            let name = if f.name.is_empty() { humanize(&dir.file_name().unwrap_or_default().to_string_lossy()) } else { f.name };
            let mut videos: Vec<Video> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join("video.json").is_file())
                .filter_map(|p| load_video(&p))
                .collect();
            videos.sort_by_key(|v| std::cmp::Reverse(v.updated));
            Project { name, expanded: false, videos, dir, no_watermark: f.no_watermark }
        })
        .collect();
    projects.sort_by(|a, b| {
        let newest = |p: &Project| p.videos.iter().map(|v| v.updated).max().unwrap_or(0);
        newest(b).cmp(&newest(a))
    });
    if let Some(p) = projects.first_mut() {
        p.expanded = true;
    }
    projects
}

/// Put folders in the order the teacher dragged them into. Folders not in `order`
/// (new since) keep their newest-first place after the ordered ones.
pub fn apply_folder_order(projects: &mut [Project], order: &[PathBuf]) {
    if order.is_empty() {
        return;
    }
    let rank = |p: &Project| order.iter().position(|d| *d == p.dir).unwrap_or(usize::MAX);
    // Stable: unranked folders keep their relative (newest-first) order.
    projects.sort_by_key(rank);
    for (i, p) in projects.iter_mut().enumerate() {
        p.expanded = i == 0;
    }
}

/// Save a project's own settings (`project.json`).
pub fn save_project(p: &Project) {
    let f = ProjectFile { name: p.name.clone(), no_watermark: p.no_watermark };
    if let Ok(b) = serde_json::to_vec_pretty(&f) {
        let _ = write_atomic(&p.dir.join("project.json"), &b);
    }
}

/// Create a project folder.
pub fn new_project(name: &str) -> Option<Project> {
    let root = projects_dir()?;
    let dir = unique(&root, &slug(name));
    std::fs::create_dir_all(&dir).ok()?;
    let _ = std::fs::write(dir.join("project.json"), serde_json::to_vec_pretty(&ProjectFile { name: name.into(), no_watermark: false }).ok()?);
    Some(Project { name: name.into(), expanded: true, videos: vec![], dir, no_watermark: false })
}

/// Create an empty video folder in `project` (with the shared scene helpers).
pub fn new_video(project: &Project, title: &str, request: &str) -> Option<Video> {
    let dir = unique(&project.dir, &slug(title));
    std::fs::create_dir_all(dir.join("scenes")).ok()?;
    std::fs::create_dir_all(dir.join("assets")).ok()?;
    std::fs::create_dir_all(dir.join("lib")).ok()?;
    let kit = studio_render::sample::AREA_OF_A_CIRCLE.iter().find(|(p, _)| *p == "lib/kit.ts").map(|(_, b)| *b)?;
    std::fs::write(dir.join("lib").join("kit.ts"), kit).ok()?;
    let v = Video {
        title: title.into(),
        status: VideoStatus::Draft,
        updated: now(),
        scenes: vec![],
        source: None,
        grade: String::new(),
        dir: Some(dir),
        settings: VideoSettings::default(),
        storyboard: None,
        session: None,
        chat: vec![],
        busy: None,
        request: Some(request.into()),
        resume_next: false,
        voice_gen: 0,
    };
    save_video(&v);
    Some(v)
}

/// Files in a video's assets folder: (name, size label).
pub fn assets(dir: &Path) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = std::fs::read_dir(dir.join("assets"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| {
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            let size = if len >= 1 << 20 { format!("{:.1} MB", len as f64 / 1048576.0) } else { format!("{} KB", len.div_ceil(1024)) };
            (e.file_name().to_string_lossy().into_owned(), size)
        })
        .collect();
    v.sort();
    v
}

/// Rename a project (its display name; the folder keeps its name so nothing that
/// points into it breaks).
pub fn rename_project(p: &mut Project, name: &str) {
    p.name = name.to_string();
    save_project(p);
}

/// Rename a video (its title in `video.json`).
pub fn rename_video(v: &mut Video, title: &str) {
    v.title = title.to_string();
    save_video(v);
}

/// Move a video folder into another project. Returns the new folder.
pub fn move_video(dir: &Path, to: &Project) -> std::io::Result<PathBuf> {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into());
    let dest = unique(&to.dir, &name);
    std::fs::rename(dir, &dest)?;
    Ok(dest)
}

/// Send a folder to the Recycle Bin / Trash, so a mistaken delete can be undone there.
pub fn trash_dir(dir: &Path) -> Result<(), String> {
    trash::delete(dir).map_err(|e| e.to_string())
}

/// "New folder", "New folder 2", ... not used by another project yet.
pub fn fresh_project_name(projects: &[Project]) -> String {
    let taken = |n: &str| projects.iter().any(|p| p.name.eq_ignore_ascii_case(n));
    let mut name = "New folder".to_string();
    let mut i = 2;
    while taken(&name) {
        name = format!("New folder {i}");
        i += 1;
    }
    name
}

#[cfg(test)]
mod tests {

    #[test]
    fn backups_rotate_on_disk() {
        let dir = std::env::temp_dir().join(format!("studio-bak-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("chat.json");
        let count = || std::fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".bak")).count();
        for n in 1..=8u64 {
            std::fs::write(dir.join(format!("chat.json.{n}.bak")), n.to_string()).unwrap();
        }
        std::fs::write(&f, "unreadable").unwrap();
        super::back_up_unreadable(&f);
        assert_eq!(count(), 5);
        assert!(f.exists());
        // The same content isn't copied twice.
        super::back_up_unreadable(&f);
        assert_eq!(count(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    #[test]
    fn chat_parse_skips_unknown_records() {
        let json = br#"[
            {"kind":"user","text":"hi"},
            {"kind":"hologram","payload":1},
            {"kind":"assistant","text":"hello"},
            {"kind":"usage"}
        ]"#;
        let (records, skipped) = parse_chat(json).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(skipped, 2);
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("studio-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("chat.json");
        write_atomic(&f, b"one").unwrap();
        write_atomic(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        assert!(!dir.join("chat.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
