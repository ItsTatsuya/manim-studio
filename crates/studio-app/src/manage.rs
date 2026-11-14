//! Folder (project) and video management from the sidebar: new folder, new video in a
//! folder, rename, move, show in the file manager, delete.
//!
//! Videos are addressed by (project, video) indices all over the workspace, so the
//! changes that shift indices (delete, move) wait until no assistant turn or render
//! is in flight, then remap every stored index in one place.

use crate::app::{Screen, Workspace};
use crate::store;
use gpui::{
    Context, Focusable, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px,
};
use std::time::{Duration, Instant};
use studio_ui::theme::{ActiveTheme, radius, text};
use studio_ui::{Icon, IconName};

/// A video being dragged in the sidebar (onto a folder to move it there).
#[derive(Clone, Debug)]
pub struct DragVideo {
    pub at: (usize, usize),
    pub title: SharedString,
}

/// A folder being dragged in the sidebar (onto another folder to take its place).
#[derive(Clone, Debug)]
pub struct DragFolder {
    pub pi: usize,
    pub name: SharedString,
}

/// What follows the pointer while dragging a sidebar row.
pub struct DragPreview {
    icon: IconName,
    label: SharedString,
}

impl DragPreview {
    pub fn new(icon: IconName, label: SharedString) -> Self {
        Self { icon, label }
    }
}

impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .max_w(px(220.))
            .h(px(28.))
            .px(px(10.))
            .rounded(radius::CONTROL)
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .shadow(t.pop_shadow())
            .text_size(text::BODY)
            .text_color(t.fg)
            .child(Icon::new(self.icon).small().color(t.fg_2))
            .child(div().min_w_0().truncate().child(self.label.clone()))
    }
}

/// A sidebar row a menu or rename belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowTarget {
    Project(usize),
    Video(usize, usize),
}

/// A destructive action waiting for the teacher to confirm.
#[derive(Clone, Debug)]
pub struct Confirm {
    pub target: RowTarget,
    pub title: String,
    pub body: String,
    pub action: &'static str,
}

impl Workspace {
    // ------------------------------------------------------------------ menus

    pub fn toggle_row_menu(&mut self, target: RowTarget, open: bool, cx: &mut Context<Self>) {
        if open {
            // The trigger's own click follows the mouse-down that closed the menu.
            let just_closed = matches!(self.row_menu_dismissed, Some((t, at)) if t == target && at.elapsed() < Duration::from_millis(400));
            self.row_menu_dismissed = None;
            if just_closed {
                return;
            }
            self.open_menu = None;
            self.row_menu = Some(target);
        } else if self.row_menu == Some(target) {
            self.row_menu = None;
            self.row_menu_dismissed = Some((target, Instant::now()));
        }
        cx.notify();
    }

    // ------------------------------------------------------------------ create

    /// Add a folder at the end of the list and start naming it.
    pub fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = store::fresh_project_name(&self.projects);
        let Some(p) = store::new_project(&name) else {
            self.notice = Some("Couldn't create the folder.".into());
            cx.notify();
            return;
        };
        if !self.prefs.folder_order.is_empty() {
            self.prefs.folder_order.push(p.dir.clone());
            self.save_prefs();
        }
        self.projects.push(p);
        let pi = self.projects.len() - 1;
        self.start_rename(RowTarget::Project(pi), window, cx);
    }

    /// Open the composer with new videos going into `pi`.
    pub fn new_video_in(&mut self, pi: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.home_project = Some(pi);
        if let Some(p) = self.projects.get_mut(pi) {
            p.expanded = true;
        }
        self.row_menu = None;
        self.go(Screen::Home, cx);
        let handle = self.home_input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// The folder a new video from Home goes into.
    pub fn home_target(&self) -> usize {
        self.home_project
            .filter(|p| *p < self.projects.len())
            .unwrap_or(self.current.0)
            .min(self.projects.len().saturating_sub(1))
    }

    // ------------------------------------------------------------------ rename

    pub fn start_rename(&mut self, target: RowTarget, window: &mut Window, cx: &mut Context<Self>) {
        let name = match target {
            RowTarget::Project(pi) => self.projects.get(pi).map(|p| p.name.clone()),
            RowTarget::Video(pi, vi) => self.projects.get(pi).and_then(|p| p.videos.get(vi)).map(|v| v.title.clone()),
        };
        let Some(name) = name else { return };
        let (RowTarget::Project(pi) | RowTarget::Video(pi, _)) = target;
        self.projects[pi].expanded = true;
        self.row_menu = None;
        self.rename = Some(target);
        self.rename_input.update(cx, |i, cx| i.set_text_selected(name, cx));
        let handle = self.rename_input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Save the rename in progress (Enter or clicking away). Empty names are ignored.
    pub fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.rename.take() else { return };
        let name = self.rename_input.read(cx).text().trim().to_string();
        if !name.is_empty() {
            match target {
                RowTarget::Project(pi) => {
                    if let Some(p) = self.projects.get_mut(pi)
                        && p.name != name
                    {
                        store::rename_project(p, &name);
                    }
                }
                RowTarget::Video(pi, vi) => {
                    if let Some(v) = self.projects.get_mut(pi).and_then(|p| p.videos.get_mut(vi))
                        && v.title != name
                    {
                        store::rename_video(v, &name);
                    }
                }
            }
        }
        cx.notify();
    }

    pub fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            cx.notify();
        }
    }

    // ------------------------------------------------------------------ delete / move

    /// Why indices can't shift right now, if they can't.
    fn structure_locked(&self) -> Option<&'static str> {
        if !self.running.is_empty() {
            Some("Wait for the assistant to finish before deleting or moving videos.")
        } else if self.jobs.iter().any(|j| j.status.active()) {
            Some("Wait for renders to finish (or cancel them in Renders) before deleting or moving videos.")
        } else {
            None
        }
    }

    pub fn request_delete(&mut self, target: RowTarget, cx: &mut Context<Self>) {
        self.row_menu = None;
        if let Some(why) = self.structure_locked() {
            self.notice = Some(why.into());
            cx.notify();
            return;
        }
        let confirm = match target {
            RowTarget::Video(pi, vi) => {
                let Some(v) = self.projects.get(pi).and_then(|p| p.videos.get(vi)) else { return };
                Confirm {
                    target,
                    title: format!("Delete “{}”?", v.title),
                    body: "The video, its scenes, narration and exports move to the Recycle Bin.".into(),
                    action: "Delete video",
                }
            }
            RowTarget::Project(pi) => {
                let Some(p) = self.projects.get(pi) else { return };
                let n = p.videos.len();
                Confirm {
                    target,
                    title: format!("Delete the folder “{}”?", p.name),
                    body: match n {
                        0 => "The folder is empty. It moves to the Recycle Bin.".into(),
                        1 => "Its video moves to the Recycle Bin with it.".into(),
                        n => format!("All {n} videos in it move to the Recycle Bin with it."),
                    },
                    action: "Delete folder",
                }
            }
        };
        self.confirm = Some(confirm);
        cx.notify();
    }

    pub fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.confirm.take() else { return };
        if let Some(why) = self.structure_locked() {
            self.notice = Some(why.into());
            cx.notify();
            return;
        }
        match c.target {
            RowTarget::Video(pi, vi) => {
                let Some(dir) = self.projects.get(pi).and_then(|p| p.videos.get(vi)).map(|v| v.dir.clone()) else { return };
                if let Some(dir) = dir
                    && let Err(e) = store::trash_dir(&dir)
                {
                    self.notice = Some(format!("Couldn't delete the video: {e}"));
                    cx.notify();
                    return;
                }
                self.projects[pi].videos.remove(vi);
                self.remap(|(p, v)| match (p == pi, v.cmp(&vi)) {
                    (true, std::cmp::Ordering::Equal) => None,
                    (true, std::cmp::Ordering::Greater) => Some((p, v - 1)),
                    _ => Some((p, v)),
                }, cx);
            }
            RowTarget::Project(pi) => {
                let Some(dir) = self.projects.get(pi).map(|p| p.dir.clone()) else { return };
                if let Err(e) = store::trash_dir(&dir) {
                    self.notice = Some(format!("Couldn't delete the folder: {e}"));
                    cx.notify();
                    return;
                }
                self.projects.remove(pi);
                if !self.prefs.folder_order.is_empty() {
                    self.prefs.folder_order.retain(|d| *d != dir);
                    self.save_prefs();
                }
                self.home_project = match self.home_project {
                    Some(h) if h == pi => None,
                    Some(h) if h > pi => Some(h - 1),
                    h => h,
                };
                self.remap(|(p, v)| match p.cmp(&pi) {
                    std::cmp::Ordering::Equal => None,
                    std::cmp::Ordering::Greater => Some((p - 1, v)),
                    std::cmp::Ordering::Less => Some((p, v)),
                }, cx);
            }
        }
        cx.notify();
    }

    pub fn move_video(&mut self, (pi, vi): (usize, usize), to: usize, cx: &mut Context<Self>) {
        self.row_menu = None;
        if pi == to || to >= self.projects.len() {
            return;
        }
        if let Some(why) = self.structure_locked() {
            self.notice = Some(why.into());
            cx.notify();
            return;
        }
        let Some(dir) = self.projects.get(pi).and_then(|p| p.videos.get(vi)).and_then(|v| v.dir.clone()) else { return };
        let moved = store::move_video(&dir, &self.projects[to]).ok().and_then(|d| store::load_video_dir(&d));
        let Some(video) = moved else {
            self.notice = Some("Couldn't move the video.".into());
            cx.notify();
            return;
        };
        self.projects[pi].videos.remove(vi);
        self.projects[to].videos.push(video);
        self.projects[to].expanded = true;
        let new = (to, self.projects[to].videos.len() - 1);
        let was_current = self.current == (pi, vi);
        self.remap(|(p, v)| {
            if (p, v) == (pi, vi) {
                Some(new)
            } else if p == pi && v > vi {
                Some((p, v - 1))
            } else {
                Some((p, v))
            }
        }, cx);
        if was_current {
            // Its files have new paths.
            self.prefs.last_video = self.video().dir.clone();
            self.save_prefs();
            self.stage.open = None;
            self.sync_preview();
        }
        self.refresh_thumbs_for(new);
        self.refresh_voice(new, cx);
        cx.notify();
    }

    /// Move folder `from` to position `to` (where the folder it was dropped on sits).
    pub fn reorder_project(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let n = self.projects.len();
        if from == to || from >= n || to >= n {
            return;
        }
        if let Some(why) = self.structure_locked() {
            self.notice = Some(why.replace("deleting or moving videos", "reordering folders"));
            cx.notify();
            return;
        }
        let p = self.projects.remove(from);
        self.projects.insert(to, p);
        let map = move |p: usize| {
            if p == from {
                to
            } else if from < to && p > from && p <= to {
                p - 1
            } else if to < from && p >= to && p < from {
                p + 1
            } else {
                p
            }
        };
        self.home_project = self.home_project.map(map);
        self.row_menu = None;
        self.remap(|(p, v)| Some((map(p), v)), cx);
        self.prefs.folder_order = self.projects.iter().map(|p| p.dir.clone()).collect();
        self.save_prefs();
        cx.notify();
    }

    /// Apply an index change to everything that stores (project, video) pairs.
    /// `None` means the video is gone.
    fn remap(&mut self, f: impl Fn((usize, usize)) -> Option<(usize, usize)>, cx: &mut Context<Self>) {
        // Results still in flight were asked for at the old positions.
        self.thumb_gen = self.thumb_gen.wrapping_add(1);
        self.stage.open = None;
        self.jobs.retain_mut(|j| match f(j.video) {
            Some(v) => {
                j.video = v;
                true
            }
            None => false,
        });
        self.rename = None;
        match f(self.current) {
            Some(cur) => self.current = cur,
            None => {
                // The open video was deleted: land on a neighbour (or nothing, if it was
                // the last one), then the composer.
                let fallback = self
                    .projects
                    .iter()
                    .enumerate()
                    .find_map(|(p, pr)| (!pr.videos.is_empty()).then_some((p, 0)));
                self.stage.open = None;
                self.stage.frame = None;
                self.playing = false;
                if let Some(a) = &mut self.audio {
                    a.stop();
                }
                match fallback {
                    Some((p, v)) => self.open_video(p, v, cx),
                    None => {
                        self.current = (0, 0);
                        self.scene = 0;
                        self.playhead = 0.;
                        self.prefs.last_video = None;
                        self.save_prefs();
                    }
                }
                self.go(Screen::Home, cx);
            }
        }
        // Reopen the stage and redo thumbnails under the new positions.
        self.sync_preview();
        self.refresh_thumbs();
    }

    /// Show a folder or video in Explorer / Finder.
    pub fn reveal_row(&mut self, target: RowTarget, cx: &mut Context<Self>) {
        self.row_menu = None;
        let dir = match target {
            RowTarget::Project(pi) => self.projects.get(pi).map(|p| p.dir.clone()),
            RowTarget::Video(pi, vi) => self.projects.get(pi).and_then(|p| p.videos.get(vi)).and_then(|v| v.dir.clone()),
        };
        if let Some(d) = dir {
            crate::engine::reveal(&d);
        }
        cx.notify();
    }
}
