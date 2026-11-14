use studio_ui::motion;
use crate::app::{Mode, RightTab, Workspace, click, faint, num, scene_image, stage_image, still_image, strong, sub, with_value};
use crate::model::{Issue, Msg, format_short, format_time};
use gpui::{
    ColorExt,
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use studio_ui::components::{
    Button, Chip, Dot, ProgressBar, Segment, Segmented, Tone, caps, floating,
    panel,
};
use studio_ui::theme::{self, ActiveTheme, FONT_MONO, num_features, radius, space, text};
use studio_ui::{Icon, IconName};

impl Workspace {
    /// Studio and Code need scenes; a video still being planned has none.
    fn render_no_scenes(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let planning = self.busy(self.current);
        panel(cx)
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .child(Icon::new(IconName::Board).color(t.fg_3))
            .child(strong(if planning { "The assistant is writing the storyboard" } else { "No scenes yet" }))
            .child(faint("Scenes appear here once the storyboard is approved and built.", cx))
            .child(Button::new("go-storyboard", "Open storyboard").small().on_click(click(cx, |this, _, cx| {
                this.mode = Mode::Storyboard;
                cx.notify();
            })))
            .into_any_element()
    }

    pub fn render_studio(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.video().scenes.is_empty() {
            return self.render_no_scenes(cx);
        }
        let compact = theme::is_compact(window);
        let right_w = if compact { px(320.) } else { space::RIGHT_PANEL };
        div()
            .size_full()
            .flex()
            .gap(space::GUTTER)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(space::GUTTER)
                    .child(self.render_stage(compact, cx))
                    .child(self.render_scene_strip(cx)),
            )
            .child(self.render_right_panel(right_w, cx))
            .into_any_element()
    }

    fn render_stage(&mut self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let video = self.video();
        let scene_ix = self.scene.min(video.scenes.len() - 1);
        let scene = &video.scenes[scene_ix];
        let total = video.length();
        let art = scene.art;
        let live = video.has_scripts() && scene.engine.file.is_some();
        let issue = scene.engine.issue.clone();
        let error = if live { self.stage.error.clone() } else { None };
        let picture: AnyElement = match (&self.stage.frame, live) {
            (Some(img), true) => still_image(img.clone()).into_any_element(),
            (None, true) => div().size_full().bg(t.stage).into_any_element(),
            _ => stage_image(art).into_any_element(),
        };
        // The previous scene dissolving into this one.
        let fading = match (&self.stage.fade_from, self.stage.fade_opacity()) {
            (Some((img, _)), Some(o)) if live => Some(div().absolute().top_0().left_0().size_full().opacity(o).child(still_image(img.clone()))),
            _ => None,
        };
        let picture = div().relative().size_full().child(picture).children(fading).into_any_element();
        let volume = self.volume_control(compact, cx).into_any_element();
        let tool = |id: &'static str, icon: IconName, tip: &'static str, on: bool, flip: fn(&mut Workspace)| {
            Button::icon(id, icon).ghost().selected(on).tooltip(tip).on_click(click(cx, move |this, _, cx| {
                flip(this);
                cx.notify();
            }))
        };
        let caption = self.stage_captions.then(|| scene.say.trim().to_string()).filter(|s| !s.is_empty());
        let local = (self.playhead - video.scene_start(scene_ix)).max(0.);
        let scene_len = scene.length().max(0.01);

        panel(cx).flex_1().child(
            div()
                .flex_1()
                .min_h_0()
                .m(px(10.))
                .rounded(radius::CARD)
                .bg(t.stage)
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .px(px(28.))
                .pt(px(28.))
                .pb(px(60.))
                .child(
                    div()
                        .w_full()
                        .max_h_full()
                        .aspect_ratio(16. / 9.)
                        .rounded(radius::INNER)
                        .overflow_hidden()
                        .border_1()
                        .border_color(t.line)
                        .relative()
                        .child(picture)
                        // Where the engine keeps text: x ±6.5 of 7.1, y ±3.6 of 4 units.
                        .when(self.stage_safe, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left(gpui::relative(0.0423))
                                    .right(gpui::relative(0.0423))
                                    .top(gpui::relative(0.05))
                                    .bottom(gpui::relative(0.05))
                                    .border_1()
                                    .border_color(gpui::hsla(0.55, 0.9, 0.6, 0.7)),
                            )
                        })
                        .when_some(caption, |d, text| {
                            d.child(
                                div().absolute().left_0().right_0().bottom(gpui::relative(0.06)).flex().justify_center().child(
                                    div()
                                        .max_w(gpui::relative(0.8))
                                        .px(px(10.))
                                        .py(px(4.))
                                        .rounded(radius::INNER)
                                        .bg(gpui::hsla(0., 0., 0., 0.72))
                                        .text_color(gpui::white())
                                        .text_size(text::SMALL)
                                        .text_center()
                                        .child(text),
                                ),
                            )
                        }),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(12.))
                        .top(px(12.))
                        .flex()
                        .gap(px(6.))
                        .when_some(issue.filter(|_| error.is_none()), |d, issue| match issue {
                            Issue::Layout { count, first } => d.child(
                                div().id("layout-issues").child(
                                    Chip::new(if count == 1 { "1 layout issue".to_string() } else { format!("{count} layout issues") })
                                        .icon(IconName::Info),
                                )
                                .tooltip(studio_ui::components::tooltip::Tooltip::text(first)),
                            ),
                            Issue::Error { .. } => d,
                        }),
                )
                .when_some(error, |d, error| {
                    let line = studio_render::error_line(&error);
                    let short = studio_render::short_error(&error);
                    d.child(
                        div().absolute().top(px(12.)).left_0().right_0().flex().justify_center().child(
                            floating(cx)
                                .max_w(px(560.))
                                .items_start()
                                .gap(px(10.))
                                .px(px(12.))
                                .py(px(10.))
                                .rounded(radius::PANEL)
                                .child(div().pt(px(2.)).child(Icon::new(IconName::Info).small().color(t.err)))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.))
                                        .child(strong(match line {
                                            Some(l) => format!("Scene {} stopped at line {l}", scene_ix + 1),
                                            None => format!("Scene {} could not run", scene_ix + 1),
                                        }))
                                        .child(div().text_size(text::SMALL).text_color(t.fg_2).font_family(FONT_MONO).child(short)),
                                )
                                .child(Button::new("stage-view-code", "View code").small().on_click(click(cx, |this, _, cx| {
                                    this.mode = Mode::Code;
                                    cx.notify();
                                }))),
                        ),
                    )
                })
                .child(
                    div().absolute().bottom(px(12.)).left_0().right_0().flex().justify_center().child(
                        floating(cx)
                            .whitespace_nowrap()
                            .child(
                                Button::icon("play", if self.playing { IconName::Pause } else { IconName::Play })
                                    .tooltip(if self.playing { "Pause  Space" } else { "Play  Space" })
                                    .on_click(click(cx, |this, _, cx| this.toggle_play(cx))),
                            )
                            .child(
                                div()
                                    .px(px(10.))
                                    .flex()
                                    .gap(px(4.))
                                    .font_features(num_features())
                                    .child(strong(format_time(self.playhead)))
                                    .child(div().text_color(t.fg_3).child(format!("/ {}", format_time(total)))),
                            )
                            .when(live && !compact, |d| {
                                let bounds = self.scrub_bounds.clone();
                                d.child(
                                    div()
                                        .id("scrub")
                                        .relative()
                                        .w(px(140.))
                                        .h(px(20.))
                                        .mr(px(10.))
                                        .flex()
                                        .items_center()
                                        .cursor_pointer()
                                        .child(
                                            gpui::canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                                                .absolute()
                                                .size_full(),
                                        )
                                        .child(div().w_full().child(ProgressBar::new(local / scene_len).tone(Tone::Neutral).height(3.)))
                                        .on_mouse_down(
                                            gpui::MouseButton::Left,
                                            cx.listener(|this, ev: &gpui::MouseDownEvent, _, cx| {
                                                this.scrubbing = true;
                                                this.playing = false;
                                                this.scrub_to(ev.position.x, cx);
                                            }),
                                        )
                                        .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _, cx| {
                                            if this.scrubbing && ev.pressed_button == Some(gpui::MouseButton::Left) {
                                                this.scrub_to(ev.position.x, cx);
                                            } else {
                                                this.scrubbing = false;
                                            }
                                        }))
                                        .on_mouse_up(gpui::MouseButton::Left, cx.listener(|this, _, _, _| this.scrubbing = false))
                                        .tooltip(studio_ui::components::Tooltip::text("Drag to scrub this scene")),
                                )
                            })
                            .child(div().mx(px(6.)).w(px(1.)).h(px(18.)).bg(t.line_strong))
                            .child(volume)
                            // View toggles live with the playback controls, off the picture.
                            .child(div().mx(px(6.)).w(px(1.)).h(px(18.)).bg(t.line_strong))
                            .child(tool("tool-safe", IconName::Grid, "Safe area for text  G", self.stage_safe, |w| w.stage_safe = !w.stage_safe))
                            .child(tool("tool-cc", IconName::Captions, "Show narration as captions  C", self.stage_captions, |w| {
                                w.stage_captions = !w.stage_captions
                            }))
                            .child(tool("tool-loop", IconName::Loop, "Loop this scene  L", self.stage_loop, |w| w.stage_loop = !w.stage_loop)),
                    ),
                )

        )
    }

    /// Mute button and volume slider for the narration preview.
    fn volume_control(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let level = self.stage_volume();
        let icon = if level <= 0.01 {
            IconName::VolumeOff
        } else if level < 0.5 {
            IconName::VolumeLow
        } else {
            IconName::Volume
        };
        let bounds = self.volume_bounds.clone();
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                Button::icon("volume-mute", icon)
                    .ghost()
                    .tooltip(if level <= 0.01 { "Unmute narration  M" } else { "Mute narration  M" })
                    .on_click(click(cx, |this, _, cx| this.toggle_mute(cx))),
            )
            .when(!compact, |d| {
                d.child(
                    div()
                        .id("volume-slider")
                        .relative()
                        .w(px(64.))
                        .h(px(20.))
                        .mr(px(4.))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .child(gpui::canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().size_full())
                        .child(div().w_full().child(ProgressBar::new(level).tone(Tone::Neutral).height(3.)))
                        // The knob shows where the level is and that it can be dragged.
                        .child(
                            div()
                                .absolute()
                                .top(px(5.))
                                .left(px(64. * level - 5.).max(px(0.)).min(px(54.)))
                                .size(px(10.))
                                .rounded_full()
                                .bg(t.fg)
                                .border_1()
                                .border_color(t.line_strong),
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, ev: &gpui::MouseDownEvent, _, cx| {
                                this.volume_dragging = true;
                                this.volume_to(ev.position.x, cx);
                            }),
                        )
                        .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _, cx| {
                            if this.volume_dragging && ev.pressed_button == Some(gpui::MouseButton::Left) {
                                this.volume_to(ev.position.x, cx);
                            } else if this.volume_dragging {
                                this.volume_dragging = false;
                                this.save_prefs();
                            }
                        }))
                        .on_mouse_up(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, _| {
                                this.volume_dragging = false;
                                this.save_prefs();
                            }),
                        )
                        .tooltip(studio_ui::components::Tooltip::text(format!("Narration volume {}%", (level * 100.).round()))),
                )
            })
    }

    fn render_scene_strip(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let video = self.video();
        let rendering = self.render_progress.is_some();
        let cards: Vec<_> = video
            .scenes
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let on = i == self.scene;
                div()
                    .id(("scene", i)).transitions(studio_ui::motion::hovers)
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(6.))
                    .rounded(radius::CARD)
                    .border_1()
                    .when(on, |d| d.bg(t.surface_3).border_color(t.fg_3))
                    .when(!on, |d| {
                        d.bg(t.surface_2)
                            .border_color(t.line)
                            .cursor_pointer()
                            .hover_style(|s| s.border_color(t.line_strong))
                    })
                    .on_click(click(cx, move |this, _, cx| this.seek_scene(i, cx)))
                    .child(
                        div()
                            .w_full()
                            .aspect_ratio(16. / 9.)
                            .rounded(radius::THUMB)
                            .overflow_hidden()
                            .child(scene_image(s, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(2.))
                            .text_size(text::SMALL)
                            .when(rendering, |d| d.child(Dot::new(Tone::Signal)))
                            .when(!rendering && matches!(s.engine.issue, Some(Issue::Error { .. })), |d| d.child(Dot::new(Tone::Err)))
                            .child(strong(s.title.clone()).min_w_0().truncate())
                            .child(div().flex_1())
                            .child(num(format_short(s.length().round() as u32)).text_color(t.fg_3).text_size(text::CAPS)),
                    )
            })
            .collect();
        let count = video.scenes.len();
        let total = format_short(video.seconds());
        let has_scripts = video.has_scripts();
        panel(cx).flex_none().child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(12.))
                        .pt(px(10.))
                        .pb(px(8.))
                        .child(strong("Scenes"))
                        .child(num(format!("{count} · {total}")).text_color(t.fg_3))
                        .child(div().flex_1())
                        .when(has_scripts, |d| {
                            let (style, output) = self.video_setting_menus(("strip-style", "strip-output"), true, cx);
                            d.child(style).child(output).child(self.watermark_menu("strip-watermark", true, cx))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .px(px(10.))
                        .pb(px(10.))
                        .children(cards)
                        .child(
                            div()
                                .id("add-scene").transitions(studio_ui::motion::hovers)
                                .flex_none()
                                .w(px(60.))
                                .rounded(radius::CARD)
                                .border_1()
                                .border_color(t.line_strong)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .bg(t.surface_2.opacity(0.))
                                .hover_style(|s| s.bg(t.surface_2))
                                .tooltip(studio_ui::components::Tooltip::text("Ask the assistant for another scene"))
                                .on_click(cx.listener(|this, _, window, cx| this.ask_for_new_scene(window, cx)))
                                .child(Icon::new(IconName::Plus).color(t.fg_3)),
                        ),
                ),
        )
    }

    fn render_right_panel(&mut self, width: gpui::Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let selected = match self.right_tab {
            RightTab::Assistant => 0,
            RightTab::Narration => 1,
            RightTab::Assets => 2,
        };
        let body: AnyElement = match self.right_tab {
            RightTab::Assistant => self.render_assistant(width < px(360.), cx),
            RightTab::Narration => self.render_narration(cx),
            RightTab::Assets => self.render_assets(cx),
        };
        panel(cx)
            .w(width)
            .flex_none()
            .child(
                div().p(px(8.)).border_b_1().border_color(t.line).child(
                    Segmented::new(
                        "right-tabs",
                        vec![
                            Segment::new("Assistant").icon(IconName::Spark),
                            Segment::new("Narration").icon(IconName::Mic),
                            Segment::new("Assets").icon(IconName::Image),
                        ],
                        selected,
                    )
                    .fill()
                    .on_select(with_value(cx, |this, i: usize, _, cx| {
                        this.right_tab = [RightTab::Assistant, RightTab::Narration, RightTab::Assets][i];
                        cx.notify();
                    })),
                ),
            )
            .child(motion::enter(
                div().flex_1().min_h_0().min_w_0().w_full().flex().flex_col().child(body),
                ("right-body", selected),
                motion::BASE,
                6.,
            ))
    }

    fn render_assistant(&mut self, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let progress = self.render_progress;
        let job_label = self
            .jobs
            .iter()
            .find(|j| j.video == self.current && j.status.active())
            .map(|j| match &j.status {
                crate::model::JobStatus::Rendering { scene, of } => format!("Rendering scene {scene} of {of}"),
                crate::model::JobStatus::Exporting => "Joining scenes".to_string(),
                crate::model::JobStatus::Waiting(n) => format!("Waiting in queue · #{n}"),
                _ => "Rendering".to_string(),
            })
            .unwrap_or_else(|| "Rendering".to_string());
        let show_usage = self.prefs.show_usage;
        let chat = self.video().chat.clone();
        let busy = self.busy(self.current);
        let empty = chat.is_empty();
        let has_scripts = self.video().has_scripts();
        let latest_problem = Workspace::last_problem(self.video()).map(|(i, ..)| i);
        let items = chat.iter().enumerate().filter(|(_, m)| !m.is_work_detail()).map(|(i, m)| -> (usize, AnyElement) {
            (i, match m {
                Msg::User(s) => div()
                    .flex()
                    .justify_end()
                    .child(
                        div()
                            .max_w(gpui::relative(0.88))
                            .bg(t.surface_3)
                            .px(px(12.))
                            .py(px(9.))
                            .rounded(radius::PANEL)
                            .rounded_br(px(4.))
                            .child(s.clone()),
                    )
                    .into_any_element(),
                Msg::Assistant(s) => div().line_height(px(20.)).child(s.clone()).into_any_element(),
                Msg::Activity { icon, ok, text: label, meta } => activity_row(*icon, *ok, label.clone(), meta.clone(), cx),
                Msg::Problem { provider, raw } => self.problem_card(i, *provider, raw, latest_problem == Some(i), cx),
                Msg::Working(w) => working_line(w, cx),
                Msg::Rendering => match progress {
                    Some(p) => div()
                        .id(("rendering", i))
                        .flex()
                        .flex_col()
                        .gap(px(7.))
                        .px(px(10.))
                        .py(px(8.))
                        .rounded(radius::CONTROL)
                        .bg(t.surface_2)
                        .border_1()
                        .border_color(t.line)
                        .text_size(text::SMALL)
                        .text_color(t.fg_2)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(Dot::new(Tone::Signal))
                                .child(job_label.clone())
                                .child(div().flex_1())
                                .child(num(format!("{}%", (p * 100.) as u32))),
                        )
                        .child(ProgressBar::new(p).animated(("render-bar", i)))
                        .into_any_element(),
                    None => div().into_any_element(),
                },
                Msg::Usage(_) if !show_usage => div().into_any_element(),
                Msg::Usage(s) => faint(s.clone(), cx).text_size(text::CAPS).into_any_element(),
            })
        });
        let provider = self.default_provider;
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("chat")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.chat_scroll)
                    .px(px(14.))
                    .pt(px(14.))
                    .pb(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .children(items.map(|(i, el)| motion::enter(div().child(el), ("chat-msg", i), motion::BASE, 6.)))
                    .when(empty, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .pt(px(8.))
                                .child(strong(if has_scripts { "Ask for a change" } else { "Nothing built yet" }))
                                .child(faint(
                                    if has_scripts {
                                        "Pick a scene below, then describe what to change: “slow this down”, “label the axes”, “make the question harder”. Only that scene is sent, so turns stay small."
                                    } else {
                                        "Approve the storyboard to build the scenes."
                                    },
                                    cx,
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .m(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .pt(px(10.))
                    .pb(px(8.))
                    .pl(px(12.))
                    .pr(px(10.))
                    .rounded(radius::PANEL)
                    .bg(t.surface_2)
                    .border_1()
                    .border_color(t.line_strong)
                    .cursor_text()
                    .on_mouse_down(gpui::MouseButton::Left, crate::app::focus_on_click(&self.chat_input))
                    .child(self.chat_input.clone())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                // Shrinks and clips first, so the send button always stays visible.
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .flex()
                                    .items_center()
                                    .gap(px(2.))
                                    .child(
                                        Button::icon("chat-attach", IconName::Clip)
                                            .small()
                                            .tooltip("Add images, SVGs or sounds")
                                            .on_click(click(cx, |this, _, cx| this.pick_assets(cx))),
                                    )
                                    .child(self.tier_menu("model", self.prefs.build_tier, provider, true, !compact, cx, |p, v| p.build_tier = v))
                                    .child(self.reasoning_menu("reasoning", self.prefs.build_reasoning, true, cx, |p, v| p.build_reasoning = v)),
                            )
                            .child(if busy {
                                Button::icon("chat-stop", IconName::Stop)
                                    .small()
                                    .tooltip("Stop the assistant")
                                    .on_click(cx.listener(|this, _, _, cx| this.stop_assistant(this.current, cx)))
                                    .into_any_element()
                            } else {
                                Button::icon("chat-send", IconName::Up)
                                    .primary()
                                    .small()
                                    .tooltip("Send")
                                    .disabled(self.chat_input.read(cx).text().trim().is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let text = this.chat_input.read(cx).text().trim().to_string();
                                        if !text.is_empty() {
                                            this.chat_input.update(cx, |i, cx| i.clear(cx));
                                            this.send_chat(text, cx);
                                        }
                                    }))
                                    .into_any_element()
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_narration(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let video = self.video();
        let editing = self.editing_line;
        let lines: Vec<(String, String, Option<f32>)> = video
            .scenes
            .iter()
            .enumerate()
            .map(|(i, s)| (format_short(video.scene_start(i).round() as u32), s.say.clone(), s.engine.voice.as_ref().map(|v| v.seconds)))
            .collect();
        let voice = if self.prefs.narration { self.voice_label() } else { "Narration off".to_string() };
        let speed = crate::engine::SPEEDS[self.prefs.speed.min(3)].0;
        let captions = crate::engine::CAPTIONS[self.prefs.captions.min(2)];
        div()
            .id("narration")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(Chip::new(format!("{voice} · {speed}")).icon(IconName::Mic))
                    .child(Chip::new(captions).icon(IconName::Captions))
                    .child(div().flex_1())
                    .child(Button::icon("voice-settings", IconName::Gear).small().tooltip("Voice & captions").on_click(click(cx, |this, _, cx| {
                        this.settings_page = crate::app::SettingsPage::Voice;
                        this.go(crate::app::Screen::Settings, cx);
                    }))),
            )
            .children(lines.into_iter().enumerate().map(|(i, (ts, line, secs))| {
                let on = i == self.scene;
                let row = div()
                    .id(("narr", i)).transitions(studio_ui::motion::hovers)
                    .w_full()
                    .min_w_0()
                    .flex()
                    .gap(px(12.))
                    .p(px(10.))
                    .rounded(radius::CONTROL)
                    .border_1()
                    .when(on, |d| d.bg(t.surface_2).border_color(t.line_strong))
                    .when(!on, |d| d.bg(t.surface_2.opacity(0.)).border_color(t.line_strong.opacity(0.)).cursor_pointer().hover_style(|s| s.bg(t.surface_2)))
                    .child(
                        div()
                            .w(px(34.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(num(ts).text_size(text::SMALL).text_color(t.fg_3))
                            .when_some(secs, |d, s| d.child(num(format!("{s:.1}s")).text_size(text::CAPS).text_color(t.fg_3))),
                    );
                if editing == Some(i) {
                    row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .px(px(8.))
                                    .py(px(6.))
                                    .rounded(radius::CONTROL)
                                    .bg(t.surface)
                                    .border_1()
                                    .border_color(t.line_strong)
                                    .child(self.narration_input.clone()),
                            )
                            .child(faint("Enter saves and re-speaks the line · Esc cancels", cx).text_size(text::CAPS)),
                    )
                } else {
                    row.on_click(cx.listener(move |this, _, window, cx| {
                        this.seek_scene(i, cx);
                        this.edit_narration_line(i, window, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .line_height(px(20.))
                            .text_color(if on { t.fg } else { t.fg_2 })
                            .child(if line.is_empty() { "(no narration)".to_string() } else { line }),
                    )
                }
            }))
            .child(faint("Click a line to edit it. Each scene lasts at least as long as its narration.", cx).mt(px(4.)))
            .into_any_element()
    }

    fn render_assets(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let files = self.video().dir.as_deref().map(crate::store::assets).unwrap_or_default();
        let empty = files.is_empty();
        div()
            .flex_1()
            .min_h_0()
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .when(empty, |d| d.child(faint("No assets yet. Images, SVGs and sounds in the video's assets folder appear here.", cx)))
            .children(files.into_iter().enumerate().map(|(i, (name, size))| {
                let lower = name.to_lowercase();
                let icon = if [".wav", ".mp3", ".ogg", ".flac"].iter().any(|e| lower.ends_with(e)) {
                    IconName::Music
                } else if [".png", ".jpg", ".jpeg", ".svg", ".gif", ".webp"].iter().any(|e| lower.ends_with(e)) {
                    IconName::Image
                } else {
                    IconName::Doc
                };
                div()
                    .id(("asset", i))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .h(px(40.))
                    .rounded(radius::CONTROL)
                    .bg(t.surface_2)
                    .border_1()
                    .border_color(t.line)
                    .child(Icon::new(icon).color(t.fg_2))
                    .child(div().flex_1().min_w_0().truncate().font_family(FONT_MONO).text_size(text::SMALL).child(name))
                    .child(num(size).text_size(text::CAPS).text_color(t.fg_3))
            }))
            .child(
                div()
                    .id("asset-drop").transitions(studio_ui::motion::hovers)
                    .mt(px(8.))
                    .h(px(96.))
                    .rounded(radius::PANEL)
                    .border_1()
                    .border_color(t.line_strong)
                    .cursor_pointer()
                    .bg(t.surface_2.opacity(0.))
                    .hover_style(|s| s.bg(t.surface_2))
                    .on_click(click(cx, |this, _, cx| this.pick_assets(cx)))
                    .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| this.add_assets(paths.paths().to_vec(), cx)))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .child(Icon::new(IconName::Plus).color(t.fg_3))
                    .child(faint("Drop images, SVGs or sounds, or click to choose", cx)),
            )
            .into_any_element()
    }

    pub fn render_code(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.video().scenes.is_empty() {
            return self.render_no_scenes(cx);
        }
        let t = cx.theme().clone();
        let video = self.video();
        let scenes: Vec<_> = video.scenes.iter().map(|s| (s.title.clone(), s.engine.issue.clone())).collect();
        let scene = &video.scenes[self.scene.min(video.scenes.len() - 1)];
        let file = scene.engine.file.clone();
        let rel = match (&file, &video.dir) {
            (Some(f), Some(d)) => f.strip_prefix(d).unwrap_or(f).to_string_lossy().replace('\\', "/"),
            (Some(f), None) => f.to_string_lossy().into_owned(),
            (None, _) => "No script yet".into(),
        };
        let issue = scene.engine.issue.clone();
        let checked = scene.engine.checked;
        let source = file.as_deref().map(|f| self.code_text(f)).unwrap_or_default();
        let bad_line = match &issue {
            Some(Issue::Error { line, .. }) => *line,
            _ => None,
        };
        let status = match (&issue, checked) {
            (Some(Issue::Error { line: Some(l), .. }), _) => Chip::new(format!("Error · line {l}")).tone(Tone::Err),
            (Some(Issue::Error { .. }), _) => Chip::new("Error").tone(Tone::Err),
            (Some(Issue::Layout { count, .. }), _) => {
                Chip::new(if *count == 1 { "1 layout issue".to_string() } else { format!("{count} layout issues") }).icon(IconName::Info)
            }
            (None, true) => Chip::new("Check passed").tone(Tone::Ok).icon(IconName::Check),
            (None, false) => Chip::new("Checking…"),
        };
        let message = match &issue {
            Some(Issue::Error { message, .. }) => Some((true, message.clone())),
            Some(Issue::Layout { first, .. }) => Some((false, first.clone())),
            None => None,
        };
        let lines = source.lines().enumerate().map(|(i, l)| {
            let bad = bad_line == Some(i as u32 + 1);
            div()
                .flex()
                .gap(px(16.))
                .when(bad, |d| d.bg(t.err_soft))
                .child(div().w(px(28.)).flex_none().text_color(if bad { t.err } else { t.fg_3 }).child(format!("{:>3}", i + 1)))
                .child(div().h(px(20.)).whitespace_nowrap().text_color(t.fg).child(l.replace('\t', "    ")))
        });
        div()
            .size_full()
            .flex()
            .gap(space::GUTTER)
            .child(
                panel(cx)
                    .w(px(220.))
                    .flex_none()
                    .p(px(6.))
                    .gap(px(2.))
                    .child(div().px(px(8.)).pt(px(8.)).pb(px(6.)).child(caps("Scenes", cx)))
                    .children(scenes.into_iter().enumerate().map(|(i, (s, issue))| {
                        let row = studio_ui::components::ListRow::new(("code-scene", i), format!("{:02} {}", i + 1, s))
                            .icon(IconName::Script)
                            .selected(i == self.scene)
                            .on_click(click(cx, move |this, _, cx| this.seek_scene(i, cx)));
                        match issue {
                            Some(Issue::Error { .. }) => row.meta("error"),
                            _ => row,
                        }
                    })),
            )
            .child(
                panel(cx)
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(14.))
                            .h(px(42.))
                            .border_b_1()
                            .border_color(t.line)
                            .child(div().font_family(FONT_MONO).text_size(text::SMALL).child(rel))
                            .child(status)
                            .child(div().flex_1())
                            .child(sub("Saved edits reload the preview", cx).text_size(text::SMALL))
                            .when_some(file, |d, f| {
                                d.child(Button::new("open-in-editor", "Open in editor").small().ghost().with_icon(IconName::External).on_click(
                                    move |_, _, _| crate::engine::open_file(&f),
                                ))
                            }),
                    )
                    .when_some(message, |d, (err, m)| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .px(px(14.))
                                .py(px(8.))
                                .border_b_1()
                                .border_color(t.line)
                                .bg(if err { t.err_soft } else { t.surface_2 })
                                .text_size(text::SMALL)
                                .child(Icon::new(IconName::Info).small().color(if err { t.err } else { t.fg_2 }))
                                .child(div().flex_1().min_w_0().font_family(FONT_MONO).text_color(if err { t.err } else { t.fg_2 }).child(m)),
                        )
                    })
                    .child(
                        div()
                            .id("code")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p(px(14.))
                            .font_family(FONT_MONO)
                            .text_size(text::SMALL)
                            .line_height(px(20.))
                            .children(lines),
                    ),
            )
            .into_any_element()
    }
}

/// The one line shown while the assistant works: what it is doing, for how long,
/// and a bar that never sits still, so it is clear nothing is stuck.
fn working_line(w: &crate::model::Working, cx: &Context<Workspace>) -> AnyElement {
    let t = cx.theme();
    let secs = w.since.elapsed().as_secs();
    let bar = match w.progress {
        Some((done, of)) if of > 0 && done > 0 => ProgressBar::new(done as f32 / of as f32).animated("working-bar"),
        _ => ProgressBar::indeterminate("working-sweep"),
    };
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(px(12.))
        .py(px(10.))
        .rounded(radius::CONTROL)
        .bg(t.surface_2)
        .border_1()
        .border_color(t.line)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(studio_ui::components::Pulse::new("working-pulse"))
                .child(div().flex_1().min_w_0().truncate().text_color(t.fg).child(format!("{}…", w.label)))
                .when_some(w.progress.filter(|(_, of)| *of > 0), |d, (done, of)| {
                    d.child(num(format!("{done} of {of}")).text_size(text::SMALL).text_color(t.fg_3))
                })
                .child(num(format!("{}:{:02}", secs / 60, secs % 60)).text_size(text::SMALL).text_color(t.fg_3)),
        )
        .child(bar.height(2.))
        .into_any_element()
}

fn activity_row(icon: IconName, ok: bool, label: String, meta: String, cx: &Context<Workspace>) -> AnyElement {
    let t = cx.theme();
    let (path, rest) = match label.split_once(' ') {
        Some((verb, rest)) if rest.contains('/') => (Some(rest.to_string()), verb.to_string()),
        _ => (None, label.clone()),
    };
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .px(px(10.))
        .py(px(7.))
        .rounded(radius::CONTROL)
        .bg(t.surface_2)
        .border_1()
        .border_color(t.line)
        .text_size(text::SMALL)
        .text_color(t.fg_2)
        .child(div().pt(px(3.)).child(Icon::new(icon).small().color(if ok { t.ok } else { t.fg_2 })))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(rest)
                .when_some(path, |d, p| d.child(div().font_family(FONT_MONO).text_color(t.fg).child(p))),
        )
        .child(num(meta).text_color(t.fg_3).font_weight(FontWeight::NORMAL))
        .into_any_element()
}

