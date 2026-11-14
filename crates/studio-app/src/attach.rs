//! Files the teacher brings in: documents for planning (Home), copied into the video's
//! `sources/` folder next to their extracted text (`<name>.md`), and assets for scenes
//! (Studio), copied into `assets/`. A video folder stays self-contained.

use std::path::{Path, PathBuf};

use gpui::{AsyncApp, Context, PathPromptOptions, WeakEntity};
use studio_ui::IconName;

use crate::app::Workspace;
use crate::model::Msg;

const ASSET_EXTS: &[&str] = &["png", "jpg", "jpeg", "svg", "wav", "mp3", "ogg", "flac"];

fn is_asset(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ASSET_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Copy `src` into `dir`, renaming on a clash. Returns the new path.
pub fn copy_into(src: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let mut dest = dir.join(&name);
    if dest.exists() && std::fs::read(&dest).ok() != std::fs::read(src).ok() {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (name.clone(), String::new()),
        };
        let mut n = 2;
        while dest.exists() {
            dest = dir.join(format!("{stem}-{n}{ext}"));
            n += 1;
        }
    }
    std::fs::copy(src, &dest)?;
    Ok(dest)
}

impl Workspace {
    pub(crate) fn pick(&mut self, prompt: &'static str, cx: &mut Context<Self>, then: fn(&mut Workspace, Vec<PathBuf>, &mut Context<Workspace>)) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some(prompt.into()) });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update(cx, |this, cx| then(this, paths, cx)).ok();
            }
        })
        .detach();
    }

    pub fn pick_documents(&mut self, cx: &mut Context<Self>) {
        self.pick("Attach notes, PDFs or slides", cx, |this, paths, cx| this.add_documents(paths, cx));
    }

    /// Attach documents to the next video (Home).
    pub fn add_documents(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut skipped = Vec::new();
        for p in paths {
            if studio_docs::supported(&p) {
                if !self.attachments.contains(&p) {
                    self.attachments.push(p);
                }
            } else {
                skipped.push(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            }
        }
        if !skipped.is_empty() {
            self.notice = Some(format!("Skipped {}: Studio reads PDF, Word, PowerPoint, Markdown and text files.", skipped.join(", ")));
        }
        cx.notify();
    }

    pub fn pick_assets(&mut self, cx: &mut Context<Self>) {
        self.pick("Add images, SVGs or sounds", cx, |this, paths, cx| this.add_assets(paths, cx));
    }

    /// Copy images, SVGs and sounds into the current video's assets folder.
    pub fn add_assets(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let video = self.current;
        let Some(dir) = self.video().dir.clone() else { return };
        let mut added = Vec::new();
        let mut skipped = Vec::new();
        for p in paths {
            if !is_asset(&p) {
                skipped.push(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                continue;
            }
            match copy_into(&p, &dir.join("assets")) {
                Ok(dest) => added.push(dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                Err(e) => self.notice = Some(format!("Couldn't add {}: {e}", p.display())),
            }
        }
        if !skipped.is_empty() {
            self.notice = Some(format!("Skipped {}: assets can be PNG, JPEG, SVG or audio files.", skipped.join(", ")));
        }
        if let Some(chat) = self.chat_of(video) {
            for name in &added {
                chat.push(Msg::Activity { icon: IconName::Image, ok: true, text: format!("Added assets/{name}"), meta: String::new() });
            }
        }
        if !added.is_empty() {
            self.persist(video);
        }
        cx.notify();
    }

    /// Asset paths of a video, relative to its folder ("assets/pizza.svg").
    pub fn asset_paths(&self, video: (usize, usize)) -> Vec<String> {
        let Some(dir) = self.projects.get(video.0).and_then(|p| p.videos.get(video.1)).and_then(|v| v.dir.clone()) else {
            return vec![];
        };
        crate::store::assets(&dir).into_iter().map(|(n, _)| format!("assets/{n}")).collect()
    }
}
