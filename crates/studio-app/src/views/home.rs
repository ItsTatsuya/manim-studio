use crate::app::{Workspace, click, display, faint, stage_image, strong, with_value};
use crate::model::{HOUSE_STYLES, format_short};
use gpui::{
    ColorExt,
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder, px,
};
use studio_ui::components::{Button, Segment, Segmented, caps, panel};
use studio_ui::theme::{ActiveTheme, radius, text};
use studio_ui::{Icon, IconName};

impl Workspace {
    pub fn render_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let viewport_h = f32::from(window.viewport_size().height);
        let viewport_w = f32::from(window.viewport_size().width);
        let t = cx.theme().clone();

        let composer = div()
            .w_full()
            .max_w(px(680.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .pt(px(18.))
            .pb(px(12.))
            .pl(px(20.))
            .pr(px(14.))
            .rounded(radius::COMPOSER)
            .bg(t.surface_2)
            .border_1()
            .border_color(t.line_strong)
            .shadow(t.pop_shadow())
            .cursor_text()
            .on_mouse_down(gpui::MouseButton::Left, crate::app::focus_on_click(&self.home_input))
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| this.add_documents(paths.paths().to_vec(), cx)))
            .child(div().pr(px(6.)).child(self.home_input.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .children(self.attachments.iter().enumerate().map(|(ai, path)| {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        div().child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(7.))
                                .h(px(28.))
                                .pl(px(10.))
                                .pr(px(4.))
                                .rounded(radius::CONTROL)
                                .bg(t.surface_3)
                                .text_color(t.fg_2)
                                .text_size(text::SMALL)
                                .child(Icon::new(IconName::Doc).small())
                                .child(strong(name))
                                .child(
                                    div()
                                        .id(("remove-attachment", ai)).transitions(studio_ui::motion::hovers)
                                        .size(px(20.))
                                        .rounded(radius::THUMB)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .bg(t.surface.opacity(0.))
                                        .hover_style(|s| s.bg(t.surface))
                                        .tooltip(studio_ui::components::Tooltip::text("Remove"))
                                        .on_click(click(cx, move |this, _, cx| {
                                            if ai < this.attachments.len() {
                                                this.attachments.remove(ai);
                                            }
                                            cx.notify();
                                        }))
                                        .child(Icon::new(IconName::X).size(px(12.)).color(t.fg_3)),
                                ),
                        )
                    }))
                    .flex_wrap()
                    .when(self.attachments.is_empty(), |d| d.hidden()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    // How the video is made: picked once, remembered for the next one.
                    .child(self.plan_options(viewport_h, viewport_w, cx))
                    .child(div().flex_1())
                    .child(Button::icon("attach", IconName::Clip).tooltip("Attach notes, PDFs or slides").on_click(click(cx, |this, _, cx| {
                        this.pick_documents(cx)
                    })))
                    .child(
                        Button::icon("home-send", IconName::Up)
                            .primary()
                            .tooltip("Plan the video")
                            .disabled(self.home_input.read(cx).text().trim().is_empty() && self.attachments.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| this.start_from_home_click(window, cx))),
                    ),
            );

        let mut all: Vec<_> = self
            .projects
            .iter()
            .enumerate()
            .flat_map(|(pi, p)| p.videos.iter().enumerate().map(move |(vi, v)| (pi, vi, v)))
            .collect();
        all.sort_by_key(|v| std::cmp::Reverse(v.2.updated));
        let recent: Vec<_> = all
            .into_iter()
            .take(4)
            .map(|(pi, vi, v)| (pi, vi, v.title.clone(), v.cover_still(), v.cover(), v.seconds(), v.updated_label()))
            .collect();
        let no_recent = recent.is_empty();
        let recent_cards = recent.into_iter().map(|(pi, vi, title, still, art, secs, updated)| {
            div()
                .id(("recent", pi * 100 + vi))
                .group("recent-card")
                .w(gpui::relative(0.25))
                .flex_none()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .cursor_pointer()
                .on_click(click(cx, move |this, _, cx| this.open_video(pi, vi, cx)))
                .child(
                    div()
                        .w_full()
                        .aspect_ratio(16. / 9.)
                        .rounded(radius::CONTROL)
                        .overflow_hidden()
                        .border_1()
                        .border_color(t.line)
                        .group_hover_style("recent-card", |s| s.border_color(t.line_strong))
                        .mb(px(6.))
                        .child(match still {
                            Some(crate::model::Still(img)) => crate::app::still_image(img).into_any_element(),
                            None => stage_image(art).into_any_element(),
                        }),
                )
                .child(strong(title).text_size(text::SMALL).truncate())
                .child(faint(format!("{} · {updated}", format_short(secs)), cx).text_size(text::CAPS))
        });

        panel(cx)
            .size_full()
            .px(px(48.))
            .pt(px(40.))
            .pb(px(32.))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(22.))
                    .child(display("What should students learn today?"))
                    .child(composer)
                    .child(faint(
                        "Type an idea, drop in notes, or paste a full script. Studio shows you a storyboard before writing anything.",
                        cx,
                    ))
                    .when_some(self.projects.get(self.home_target()).map(|p| p.name.clone()), |d, folder| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(text::SMALL)
                                .text_color(t.fg_3)
                                .child(Icon::new(IconName::Folder).small().color(t.fg_3))
                                .child(format!("Saves to {folder}")),
                        )
                    }),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(760.))
                    .mx_auto()
                    .pt(px(24.))
                    .border_t_1()
                    .border_color(t.line)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .mb(px(12.))
                            .child(caps("Recent", cx)),
                    )
                    .when(no_recent, |d| d.child(faint("Videos you make show up here.", cx)))
                    .child(div().flex().gap(px(16.)).pr(px(48.)).children(recent_cards)),
            )
            .into_any_element()
    }

    /// One "Options" button under the prompt that sums up how the next video is made,
    /// and opens a panel with every choice.
    fn plan_options(&self, viewport_h: f32, viewport_w: f32, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::model::{APPROACHES, LEVELS, length_label};
        const KEY: &str = "plan-options";
        let p = &self.prefs;
        let mut summary = vec![
            HOUSE_STYLES[p.house_style.min(HOUSE_STYLES.len() - 1)].name.to_string(),
            length_label(p.video_seconds),
            APPROACHES[p.approach.min(APPROACHES.len() - 1)].name.to_string(),
        ];
        if p.level > 0 {
            summary.push(LEVELS[p.level.min(LEVELS.len() - 1)].name.to_string());
        }
        let open = self.menu_open(KEY);
        let trigger = Button::new("plan-options-btn", summary.join(" · "))
            .small()
            .ghost()
            .selected(open)
            .with_icon(IconName::Sliders)
            .icon_right(IconName::ChevronDown)
            .when(!open, |b| b.tooltip("Look, length and how the video explains"))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_menu(KEY, !open, cx)));
        // Open below the button, or above it when the window is too short for that.
        const PANEL_H: f32 = 380.;
        const PANEL_W: f32 = 580.;
        const PREVIEW_W: f32 = 260.;
        let at = self.options_anchor.get();
        let (top, bottom) = (f32::from(at.origin.y), f32::from(at.origin.y + at.size.height));
        let upward = bottom + PANEL_H > viewport_h - 8. && top - PANEL_H > 8.;
        let anchor = self.options_anchor.clone();
        div()
            .relative()
            .flex_none()
            .child(gpui::canvas(move |b, _, _| anchor.set(b), |_, _, _, _| {}).absolute().size_full())
            .child(trigger)
            .when(open, |d| {
                // The look preview goes beside the panel, on whichever side has room.
                let preview_left = f32::from(at.origin.x) + PANEL_W + PREVIEW_W + 16. > viewport_w;
                let panel = self
                    .plan_panel(preview_left, cx)
                    .map(|p| if upward { p.mb(px(6.)) } else { p.mt(px(6.)) })
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| this.toggle_menu(KEY, false, cx)));
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .when(upward, |d| d.top_0())
                        .when(!upward, |d| d.bottom_0())
                        .child(gpui::deferred(
                            gpui::anchored()
                                .anchor(if upward { gpui::Anchor::BottomLeft } else { gpui::Anchor::TopLeft })
                                .snap_to_window_with_margin(px(8.))
                                .child(studio_ui::motion::pop(panel, "plan-panel-pop", upward)),
                        )),
                )
            })
    }

    fn plan_panel(&self, preview_left: bool, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        use crate::model::{APPROACHES, ENDINGS, LENGTHS, LEVELS, PACES, length_label};
        let t = cx.theme().clone();
        let p = &self.prefs;
        let secs = p.video_seconds;
        let look = p.house_style.min(HOUSE_STYLES.len() - 1);
        let approach = p.approach.min(APPROACHES.len() - 1);
        let level = p.level.min(LEVELS.len() - 1);
        let pace = p.pace.min(PACES.len() - 1);
        let ending = p.ending.min(ENDINGS.len() - 1);
        let (fg_2, fg_3, line) = (t.fg_2, t.fg_3, t.line);

        // A settings-style row: label on the left, the control and what the current
        // choice means on the right.
        let row = move |label: &'static str, control: AnyElement, note: String, last: bool| {
            div()
                .flex()
                .items_start()
                .gap(px(12.))
                .px(px(12.))
                .py(px(6.))
                .when(!last, |d| d.border_b_1().border_color(line))
                .child(div().w(px(60.)).flex_none().pt(px(7.)).text_size(text::BODY).text_color(fg_2).child(label))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(control)
                        .child(div().pl(px(2.)).text_size(text::CAPS).text_color(fg_3).child(note)),
                )
        };
        let seg = |id: &'static str,
                   labels: Vec<String>,
                   selected: usize,
                   pick: fn(&mut Workspace, usize, &mut Context<Workspace>),
                   cx: &mut Context<Self>| {
            div()
                .flex()
                .child(
                    Segmented::new(id, labels.into_iter().map(Segment::new).collect(), selected).on_select(with_value(
                        cx,
                        move |this, i: usize, _, cx| {
                            pick(this, i, cx);
                            this.save_prefs();
                            cx.notify();
                        },
                    )),
                )
                .into_any_element()
        };

        let look_ctl = div()
            .flex()
            .child(
                Segmented::new("opt-look", HOUSE_STYLES.iter().map(|s| Segment::new(s.name)).collect(), look)
                    .on_select(with_value(cx, |this, i: usize, _, cx| {
                        this.prefs.house_style = i;
                        this.save_prefs();
                        this.restyle(cx);
                    }))
                    .on_item_hover(with_value(cx, |this, i: usize, _, cx| {
                        if this.look_hover != Some(i) {
                            this.look_hover = Some(i);
                            this.hover_style(Some(i), cx);
                            cx.notify();
                        }
                    })),
            )
            .into_any_element();
        // What a look looks like, while its name is hovered.
        let preview = self.look_hover.filter(|i| *i < HOUSE_STYLES.len()).map(|i| {
            let s = &HOUSE_STYLES[i];
            // `pop` makes its element relative, so the card sits in an absolute holder.
            div()
                .absolute()
                .top_0()
                .when(!preview_left, |d| d.left(gpui::relative(1.)).ml(px(8.)))
                .when(preview_left, |d| d.right(gpui::relative(1.)).mr(px(8.)))
                .w(px(260.))
                .child(studio_ui::motion::pop(
                div()
                    .w_full()
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .rounded(radius::PANEL)
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.line_strong)
                    .shadow(t.pop_shadow())
                    .child(
                        div()
                            .w_full()
                            .aspect_ratio(16. / 9.)
                            .rounded(radius::THUMB)
                            .overflow_hidden()
                            .child(crate::app::style_image(self, i, s.art)),
                    )
                    .child(
                        div()
                            .px(px(4.))
                            .pb(px(2.))
                            .flex()
                            .flex_col()
                            .child(div().text_size(text::BODY).text_color(t.fg).child(s.name))
                            .child(div().text_size(text::SMALL).text_color(t.fg_3).child(s.blurb)),
                    ),
                "look-preview",
                false,
            ))
        });
        // A typed length that isn't a preset selects no preset.
        let preset = LENGTHS.iter().position(|s| *s == secs).unwrap_or(usize::MAX);
        let length_ctl = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(seg(
                "opt-length",
                LENGTHS.iter().map(|s| length_label(*s)).collect(),
                preset,
                |this, i, cx| {
                    this.prefs.video_seconds = LENGTHS[i];
                    this.length_input.update(cx, |input, cx| input.clear(cx));
                },
                cx,
            ))
            .child(
                div()
                    .w(px(84.))
                    .h(px(32.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(radius::CARD)
                    .bg(t.surface_2)
                    .border_1()
                    .border_color(if preset == usize::MAX { t.fg_3 } else { t.line })
                    .text_size(text::BODY)
                    .child(self.length_input.clone()),
            )
            .into_any_element();
        let length_note = if preset == usize::MAX {
            format!("{} typed. Type another, like 3:30, 7 min or 90s.", length_label(secs))
        } else {
            "Or type a length, like 3:30, 7 min or 90s.".to_string()
        };
        let approach_ctl = seg(
            "opt-approach",
            APPROACHES.iter().map(|c| c.short.to_string()).collect(),
            approach,
            |this, i, _| this.prefs.approach = i,
            cx,
        );
        let level_ctl = seg(
            "opt-level",
            LEVELS.iter().map(|c| c.short.to_string()).collect(),
            level,
            |this, i, _| this.prefs.level = i,
            cx,
        );
        let pace_ctl = seg(
            "opt-pace",
            PACES.iter().map(|(c, _)| c.name.to_string()).collect(),
            pace,
            |this, i, _| this.prefs.pace = i,
            cx,
        );
        let ending_ctl = seg(
            "opt-ending",
            ENDINGS.iter().map(|c| c.short.to_string()).collect(),
            ending,
            |this, i, _| this.prefs.ending = i,
            cx,
        );

        div()
            .id("plan-panel")
            .relative()
            .occlude()
            // Moving anywhere else in the panel hides the look preview.
            .on_mouse_move(cx.listener(|this, _, _, cx| this.clear_look_hover(cx)))
            .w(px(580.))
            .py(px(2.))
            .flex()
            .flex_col()
            .rounded(radius::PANEL)
            // The panel colour, not `raised`: the selected pills are `raised`, so they
            // need a darker ground to stand out.
            .bg(t.surface)
            .border_1()
            .border_color(t.line_strong)
            .shadow(t.pop_shadow())
            .cursor_default()
            .child(row("Look", look_ctl, HOUSE_STYLES[look].blurb.to_string(), false))
            .child(row("Length", length_ctl, length_note, false))
            .child(row("Explains", approach_ctl, format!("{}: {}", APPROACHES[approach].name, APPROACHES[approach].blurb.to_lowercase()), false))
            .child(row("For", level_ctl, format!("{}: {}", LEVELS[level].name, LEVELS[level].blurb.to_lowercase()), false))
            .child(row("Pace", pace_ctl, PACES[pace].0.blurb.to_string(), false))
            .child(row("Ending", ending_ctl, format!("{}: {}", ENDINGS[ending].name, ENDINGS[ending].blurb.to_lowercase()), true))
            .children(preview)
    }

    pub fn clear_look_hover(&mut self, cx: &mut Context<Self>) {
        if self.look_hover.take().is_some() {
            self.hover_style(None, cx);
            cx.notify();
        }
    }

    fn start_from_home_click(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.home_input.read(cx).text().to_string();
        self.start_plan(text, cx);
    }
}
