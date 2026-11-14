//! Title bar, window controls and sidebar.

use crate::app::{Mode, Screen, SettingsPage, Workspace, click, drag_area, faint, with_value};
use crate::manage::{DragFolder, DragPreview, DragVideo, RowTarget};
use gpui::{
    AppContext, ColorExt,
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, WindowControlArea, div, prelude::FluentBuilder, px,
};
use studio_ui::components::{Button, Chip, Dropdown, ListRow, MenuItem, Segment, Segmented, caps};
use studio_ui::theme::{self, ActiveTheme, radius, space, text};
use studio_ui::{Icon, IconName, motion};

impl Workspace {
    pub fn render_titlebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let compact = theme::is_compact(window);
        let mac = cfg!(target_os = "macos");

        let mut left = div().flex().flex_none().items_center().gap(px(10.)).min_w_0();
        if mac {
            left = left.child(div().w(px(64.)));
        } else {
            left = left.pl(px(14.));
        }

        if self.screen != Screen::Onboarding {
            left = left.child(gpui::img(studio_ui::assets::LOGO).size(px(24.)).flex_none());
            if !compact && self.screen != Screen::Settings {
                left = left.child(
                    Button::icon("toggle-sidebar", IconName::Sidebar)
                        .tooltip(if cfg!(target_os = "macos") { "Toggle sidebar  ⌘B" } else { "Toggle sidebar  Ctrl+B" })
                        .on_click(click(cx, |this, _, cx| {
                            this.prefs.sidebar_collapsed = !this.prefs.sidebar_collapsed;
                            this.save_prefs();
                            cx.notify();
                        })),
                );
            }
            if self.screen == Screen::Video {
                left = left.child(self.mode_switch(compact, cx));
            }
        }

        let mut right = div().flex().flex_none().items_center().gap(px(6.));
        match self.screen {
            Screen::Onboarding => {
                right = right.child(faint(format!("Step {} of {}", self.onboarding_step + 1, crate::views::onboarding::STEPS), cx).mr(px(8.)));
            }
            Screen::Home => {
                right = right.child(self.usage_meter(compact, cx));
            }
            Screen::Video if self.mode == Mode::Storyboard => {
                let busy = self.busy(self.current);
                let v = self.video();
                let built = v.has_scripts();
                let has_plan = v.storyboard.is_some() && !v.scenes.is_empty();
                let chip = match (&v.busy, built) {
                    (Some(crate::model::Busy::Planning), _) => Chip::new("Writing storyboard…").tone(studio_ui::components::Tone::Signal),
                    (Some(crate::model::Busy::Revising), _) => Chip::new("Revising…").tone(studio_ui::components::Tone::Signal),
                    (Some(_), _) => Chip::new("Building…").tone(studio_ui::components::Tone::Signal),
                    (None, true) => Chip::new(format!("Built · {} scenes", v.scenes.len())).tone(studio_ui::components::Tone::Ok),
                    (None, false) => Chip::new("Draft · not built yet"),
                };
                right = right.child(chip);
                right = if built {
                    right.child(Button::new("tb-open-studio", "Open studio").primary().icon_right(IconName::ChevronRight).on_click(click(cx, |this, _, cx| {
                        this.mode = Mode::Studio;
                        this.sync_preview();
                        cx.notify();
                    })))
                } else {
                    right.child(
                        Button::new("tb-approve", "Approve & build")
                            .primary()
                            .disabled(busy || !has_plan)
                            .icon_right(IconName::ChevronRight)
                            .on_click(click(cx, |this, _, cx| this.build_video(cx))),
                    )
                };
            }
            Screen::Video => {
                right = right
                    .child(self.usage_meter(compact, cx))
                    .child({
                        let b = Button::new("tb-render", "Render")
                            .primary()
                            .with_icon(IconName::Play)
                            .on_click(click(cx, |this, _, cx| this.start_render(cx)));
                        if compact { b } else { b.kbd(if cfg!(target_os = "macos") { "⌘ ↵" } else { "Ctrl ↵" }) }
                    });
            }
            Screen::Renders => {
                right = right
                    .child(
                        Segmented::new(
                            "queue-filter",
                            ["All", "Queue", "Finished", "Failed"].map(Segment::new).into_iter().collect(),
                            self.queue_filter,
                        )
                        .on_select(crate::app::with_value(cx, |this, i: usize, _, cx| {
                            this.queue_filter = i;
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new("tb-pause", if self.queue_paused { "Start queue" } else { "Pause queue" })
                            .with_icon(if self.queue_paused { IconName::Play } else { IconName::Pause })
                            .on_click(click(cx, |this, _, cx| this.set_queue_paused(!this.queue_paused, cx))),
                    );
            }
            Screen::Settings => {}
        }

        div()
            .id("titlebar")
            .flex()
            .flex_none()
            .items_center()
            .h(space::TITLEBAR)
            .w_full()
            .child(left)
            .child(drag_area("titlebar-drag"))
            .child(right.pr(px(10.)))
            .when(!mac, |d| d.child(self.window_controls(window, cx)))
            .into_any_element()
    }

    fn window_controls(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let maximized = window.is_maximized();
        let button = |id: &'static str, icon: IconName, area: WindowControlArea, close: bool| {
            div()
                .id(id)
                .transitions(studio_ui::motion::hovers)
                .w(px(46.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.fg_2)
                .bg(if close { t.window_close_hover } else { t.surface_2 }.opacity(0.))
                .window_control_area(area)
                .hover_style(move |s| {
                    if close {
                        s.bg(t.window_close_hover).text_color(gpui::white())
                    } else {
                        s.bg(t.surface_2).text_color(t.fg)
                    }
                })
                .child(Icon::new(icon).size(px(16.)).color(t.fg_2))
        };
        div()
            .flex()
            .flex_none()
            .h_full()
            .child(
                button("win-min", IconName::WinMin, WindowControlArea::Min, false)
                    .on_click(|_, window, _| window.minimize_window()),
            )
            .child(
                button(
                    "win-max",
                    IconName::WinMax,
                    WindowControlArea::Max,
                    false,
                )
                .when(maximized, |d| d.opacity(0.9))
                .on_click(|_, window, _| window.zoom_window()),
            )
            .child(
                button("win-close", IconName::WinClose, WindowControlArea::Close, true)
                    .on_click(|_, window, _| window.remove_window()),
            )
    }

    /// The "⋯" menu of a folder row.
    fn project_menu(&self, pi: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let target = RowTarget::Project(pi);
        let items = vec![
            MenuItem::new("New video here").icon(IconName::Plus),
            MenuItem::new("Rename").icon(IconName::Pencil),
            MenuItem::new(crate::engine::REVEAL_LABEL).icon(IconName::External),
            MenuItem::new("Delete folder").icon(IconName::Trash).danger().divider(),
        ];
        Dropdown::new(
            SharedString::from(format!("project-menu-{pi}")),
            Button::icon(("project-more", pi), IconName::Dots).small().ghost().tooltip("Folder options"),
            items,
        )
        .width(200.)
        .open(self.row_menu == Some(target))
        .on_toggle(with_value(cx, move |this, open: bool, _, cx| this.toggle_row_menu(target, open, cx)))
        .on_select(with_value(cx, move |this, i: usize, window, cx| match i {
            0 => this.new_video_in(pi, window, cx),
            1 => this.start_rename(target, window, cx),
            2 => this.reveal_row(target, cx),
            _ => this.request_delete(target, cx),
        }))
    }

    /// The "⋯" menu of a video row.
    fn video_menu(&self, pi: usize, vi: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let target = RowTarget::Video(pi, vi);
        let others: Vec<(usize, String)> =
            self.projects.iter().enumerate().filter(|(i, _)| *i != pi).map(|(i, p)| (i, p.name.clone())).collect();
        let mut items = vec![MenuItem::new("Rename").icon(IconName::Pencil), MenuItem::new(crate::engine::REVEAL_LABEL).icon(IconName::External)];
        let first_move = items.len() + 1;
        if !others.is_empty() {
            items.push(MenuItem::heading("Move to").divider());
            items.extend(others.iter().map(|(_, name)| MenuItem::new(name.clone()).icon(IconName::Move)));
        }
        let delete_ix = items.len();
        items.push(MenuItem::new("Delete").icon(IconName::Trash).danger().divider());
        let targets: Vec<usize> = others.iter().map(|(i, _)| *i).collect();
        Dropdown::new(
            SharedString::from(format!("video-menu-{pi}-{vi}")),
            Button::icon((SharedString::from(format!("video-more-{pi}")), vi), IconName::Dots).small().ghost().tooltip("Video options"),
            items,
        )
        .width(210.)
        .open(self.row_menu == Some(target))
        .on_toggle(with_value(cx, move |this, open: bool, _, cx| this.toggle_row_menu(target, open, cx)))
        .on_select(with_value(cx, move |this, i: usize, window, cx| {
            if i == 0 {
                this.start_rename(target, window, cx)
            } else if i == 1 {
                this.reveal_row(target, cx)
            } else if i == delete_ix {
                this.request_delete(target, cx)
            } else if let Some(to) = i.checked_sub(first_move).and_then(|k| targets.get(k)) {
                this.move_video((pi, vi), *to, cx)
            }
        }))
    }

    /// The inline name field shown in a row being renamed.
    fn rename_field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        div()
            .h(px(24.))
            .px(px(6.))
            .flex()
            .items_center()
            .rounded(radius::SEGMENT)
            .bg(t.surface)
            .border_1()
            .border_color(t.ring)
            .child(self.rename_input.clone())
    }

    pub fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let on_video = self.screen == Screen::Video;
        let query = self.search_input.read(cx).text().trim().to_lowercase();
        let searching = !query.is_empty();
        let mut list = div().flex().flex_col().gap(px(2.));
        let mut matches = 0;
        for pi in 0..self.projects.len() {
            let project = &self.projects[pi];
            let hits: Vec<usize> = project
                .videos
                .iter()
                .enumerate()
                .filter(|(_, v)| query.is_empty() || v.title.to_lowercase().contains(&query) || project.name.to_lowercase().contains(&query))
                .map(|(i, _)| i)
                .collect();
            // Empty folders still show (so a new folder can be filled), except in search results.
            if hits.is_empty() && searching {
                continue;
            }
            matches += hits.len();
            // Searching shows every match, so collapsed projects open.
            let expanded = project.expanded || searching;
            let target = RowTarget::Project(pi);
            let mut row = ListRow::new(("project", pi), project.name.clone())
                .icon(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                .on_click(click(cx, move |this, _, cx| {
                    this.projects[pi].expanded = !this.projects[pi].expanded;
                    cx.notify();
                }))
                .on_right_click(cx.listener(move |this, _: &gpui::MouseDownEvent, _, cx| this.toggle_row_menu(target, true, cx)))
                .decorate({
                    // Drag a folder onto another to reorder; drop a video here to move it in.
                    let drop_bg = t.ring.opacity(0.26);
                    let fg = t.fg;
                    let name: SharedString = project.name.clone().into();
                    let on_video = cx.listener(move |this, d: &DragVideo, _, cx| this.move_video(d.at, pi, cx));
                    let on_folder = cx.listener(move |this, d: &DragFolder, _, cx| this.reorder_project(d.pi, pi, cx));
                    move |d| {
                        d.on_drag(DragFolder { pi, name: name.clone() }, |d, _, _, cx| {
                            cx.new(|_| DragPreview::new(IconName::Folder, d.name.clone()))
                        })
                        .can_drop(move |v, _, _| {
                            v.downcast_ref::<DragVideo>().is_some_and(|d| d.at.0 != pi)
                                || v.downcast_ref::<DragFolder>().is_some_and(|d| d.pi != pi)
                        })
                        .on_drag_over::<DragVideo>(move |s, _, _, _| s.bg(drop_bg).text_color(fg))
                        .on_drag_over::<DragFolder>(move |s, _, _, _| s.bg(drop_bg).text_color(fg))
                        .on_drop(on_video)
                        .on_drop(on_folder)
                    }
                });
            if self.rename == Some(target) {
                row = row.editor(self.rename_field(cx));
            } else {
                row = row.actions(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            Button::icon(("project-add", pi), IconName::Plus)
                                .small()
                                .ghost()
                                .tooltip("New video in this folder")
                                .on_click(cx.listener(move |this, _, window, cx| this.new_video_in(pi, window, cx))),
                        )
                        .child(self.project_menu(pi, cx)),
                    self.row_menu == Some(target),
                );
            }
            list = list.child(row);
            if expanded {
                let mut kids = div().flex().flex_col().gap(px(2.));
                for vi in hits.iter().copied() {
                    let video = &self.projects[pi].videos[vi];
                    let target = RowTarget::Video(pi, vi);
                    let mut row = ListRow::new((SharedString::from(format!("video-{pi}")), vi), video.title.clone())
                        .indent()
                        .leading(Self::status_dot(video.status))
                        .meta(video.updated_label())
                        .selected(on_video && self.current == (pi, vi))
                        .on_click(click(cx, move |this, _, cx| this.open_video(pi, vi, cx)))
                        .on_right_click(cx.listener(move |this, _: &gpui::MouseDownEvent, _, cx| this.toggle_row_menu(target, true, cx)))
                        .decorate({
                            // Drag a video onto any row of another folder to move it there.
                            let drop_bg = t.ring.opacity(0.26);
                            let title: SharedString = video.title.clone().into();
                            let on_video = cx.listener(move |this, d: &DragVideo, _, cx| this.move_video(d.at, pi, cx));
                            move |d| {
                                d.on_drag(DragVideo { at: (pi, vi), title: title.clone() }, |d, _, _, cx| {
                                    cx.new(|_| DragPreview::new(IconName::Film, d.title.clone()))
                                })
                                .can_drop(move |v, _, _| v.downcast_ref::<DragVideo>().is_some_and(|d| d.at.0 != pi))
                                .on_drag_over::<DragVideo>(move |s, _, _, _| s.bg(drop_bg))
                                .on_drop(on_video)
                            }
                        });
                    if self.rename == Some(target) {
                        row = row.editor(self.rename_field(cx));
                    } else {
                        row = row.actions(self.video_menu(pi, vi, cx), self.row_menu == Some(target));
                    }
                    kids = kids.child(row);
                }
                if hits.is_empty() {
                    kids = kids.child(
                        div()
                            .id(("empty-folder", pi))
                            .h(space::ROW)
                            .pl(px(28.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .rounded(radius::CONTROL)
                            .text_size(text::SMALL)
                            .text_color(t.fg_3)
                            .cursor_pointer()
                            .transitions(motion::hovers)
                            .bg(t.surface_2.opacity(0.))
                            .hover_style(|s| s.text_color(t.fg).bg(t.surface_2))
                            .on_click(cx.listener(move |this, _, window, cx| this.new_video_in(pi, window, cx)))
                            .can_drop(move |v, _, _| v.downcast_ref::<DragVideo>().is_some_and(|d| d.at.0 != pi))
                            .on_drag_over::<DragVideo>({
                                let drop_bg = t.ring.opacity(0.26);
                                move |s, _, _, _| s.bg(drop_bg)
                            })
                            .on_drop(cx.listener(move |this, d: &DragVideo, _, cx| this.move_video(d.at, pi, cx)))
                            .child(Icon::new(IconName::Plus).small().color(t.fg_3))
                            .child("Make the first video"),
                    );
                }
                // Opening a folder lets its videos slide in.
                list = list.child(motion::enter(kids, ("folder-kids", pi), motion::FAST, -4.));
            }
        }

        let queue_count = self
            .jobs
            .iter()
            .filter(|j| j.status.active())
            .count();

        div()
            .id("sidebar")
            .w(space::SIDEBAR)
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(4.))
            .pt(px(6.))
            .child(
                div()
                    .id("search")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(30.))
                    .px(px(9.))
                    .mx(px(2.))
                    .rounded(radius::CONTROL)
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.line)
                    .shadow(t.edge())
                    .cursor_text()
                    .on_mouse_down(gpui::MouseButton::Left, crate::app::focus_on_click(&self.search_input))
                    .child(Icon::new(IconName::Search).small().color(t.fg_3))
                    .child(div().flex_1().min_w_0().child(self.search_input.clone()))
                    .when(searching, |d| {
                        d.child(
                            Button::icon("clear-search", IconName::X)
                                .small()
                                .tooltip("Clear search")
                                .on_click(click(cx, |this, _, cx| {
                                    this.search_input.update(cx, |i, cx| i.clear(cx));
                                })),
                        )
                    })
                    .when(!searching, |d| {
                        d.child(studio_ui::components::Kbd::new(if cfg!(target_os = "macos") { "⌘ K" } else { "Ctrl K" }))
                    }),
            )
            .child(
                div().mt(px(6.)).child(
                    ListRow::new("new-video", "New video")
                        .icon(IconName::Plus)
                        .selected(self.screen == Screen::Home)
                        .on_click(click(cx, |this, _, cx| {
                            this.home_project = None;
                            this.go(Screen::Home, cx)
                        })),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .pl(px(8.))
                    .pr(px(3.))
                    .pt(px(10.))
                    .pb(px(2.))
                    .child(div().flex_1().child(caps("Projects", cx)))
                    .child(
                        Button::icon("new-folder", IconName::FolderPlus)
                            .small()
                            .ghost()
                            .tooltip("New folder")
                            .on_click(cx.listener(|this, _, window, cx| this.new_folder(window, cx))),
                    ),
            )
            .child(
                div()
                    .id("project-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list)
                    .when(matches == 0 && !query.is_empty(), |d| {
                        d.child(faint(format!("No videos match “{query}”"), cx).px(px(8.)).py(px(6.)))
                    })
                    // Empty folders show their own "Make the first video" row; this is for no folders at all.
                    .when(query.is_empty() && self.projects.is_empty(), |d| {
                        d.child(faint("No videos yet", cx).px(px(8.)).py(px(6.)))
                    }),
            )
            .child(
                ListRow::new("nav-styles", "House styles")
                    .icon(IconName::Palette)
                    .on_click(click(cx, |this, _, cx| {
                        this.settings_page = SettingsPage::Styles;
                        this.go(Screen::Settings, cx)
                    })),
            )
            .child(
                ListRow::new("nav-renders", "Renders")
                    .icon(IconName::Queue)
                    .when(queue_count > 0, |r| r.meta(queue_count.to_string()))
                    .selected(self.screen == Screen::Renders)
                    .on_click(click(cx, |this, _, cx| this.go(Screen::Renders, cx))),
            )
            .child(
                ListRow::new("nav-settings", "Settings")
                    .icon(IconName::Gear)
                    .on_click(click(cx, |this, _, cx| this.go(Screen::Settings, cx))),
            )
            .child(div().h(px(4.)))
            .into_any_element()
    }

    /// The 48px icon rail shown while the sidebar is hidden (collapsed, or a narrow window):
    /// the same destinations, one click away.
    pub fn render_rail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let queue_count = self.jobs.iter().filter(|j| j.status.active()).count();
        let item = |id: &'static str, icon: IconName, tip: &'static str| Button::icon(id, icon).ghost().tooltip(tip);
        div()
            .id("rail")
            .w(px(40.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .pt(px(4.))
            .pb(px(4.))
            .child(item("rail-new", IconName::Plus, "New video").on_click(click(cx, |this, _, cx| this.go(Screen::Home, cx))))
            .child(item("rail-find", IconName::Search, "Find a command or video").on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx))))
            .child(div().flex_1())
            .child(item("rail-styles", IconName::Palette, "House styles").on_click(click(cx, |this, _, cx| {
                this.settings_page = SettingsPage::Styles;
                this.go(Screen::Settings, cx)
            })))
            .child({
                let b = item("rail-renders", IconName::Queue, "Renders").selected(self.screen == Screen::Renders);
                let b = if queue_count > 0 { b.badge(queue_count.to_string()) } else { b };
                b.on_click(click(cx, |this, _, cx| this.go(Screen::Renders, cx)))
            })
            .child(item("rail-settings", IconName::Gear, "Settings").on_click(click(cx, |this, _, cx| this.go(Screen::Settings, cx))))
            .into_any_element()
    }
}

