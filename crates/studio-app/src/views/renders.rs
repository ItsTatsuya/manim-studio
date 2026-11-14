use crate::app::{Workspace, faint, num, stage_image, still_image, strong};
use crate::model::{JobStatus, Still};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder, px,
};
use studio_ui::components::{Button, Dot, ProgressBar, Tone, panel};
use studio_ui::theme::{self, ActiveTheme, radius, text};
use studio_ui::{Icon, IconName};

impl Workspace {
    pub fn render_renders(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let compact = theme::is_compact(window);
        let filter = self.queue_filter;
        let visible = |s: &JobStatus| match filter {
            1 => s.active(),
            2 => matches!(s, JobStatus::Done(_)),
            3 => matches!(s, JobStatus::Failed(_) | JobStatus::Cancelled),
            _ => true,
        };
        let col = |w: f32| div().w(px(w)).flex_none().flex().items_center();
        let header = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .h(px(38.))
            .border_b_1()
            .border_color(t.line)
            .text_size(text::SMALL)
            .text_color(t.fg_3)
            .child(col(72.))
            .child(div().flex_1().child("Video"))
            .when(!compact, |d| d.child(col(96.).child("Output")))
            .child(col(240.).child("Status"))
            .when(!compact, |d| d.child(col(60.).child("Time")))
            .child(col(30.));

        let rows: Vec<_> = self
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, j)| visible(&j.status))
            .map(|(i, j)| {
                let active = matches!(j.status, JobStatus::Rendering { .. } | JobStatus::Exporting);
                let status: AnyElement = match &j.status {
                    JobStatus::Rendering { scene, of } => div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .w(px(190.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .text_color(t.fg)
                                .child(Dot::new(Tone::Signal))
                                .child(format!("Rendering scene {scene} of {of}"))
                                .child(div().flex_1())
                                .child(num(format!("{}%", (j.progress * 100.) as u32)).text_size(text::SMALL).text_color(t.fg_3)),
                        )
                        .child(ProgressBar::new(j.progress))
                        .into_any_element(),
                    JobStatus::Failed(reason) => div()
                        .id(("job-failed", i))
                        .min_w_0()
                        .child(Self::render_chip_for(&JobStatus::Failed(short_reason(reason))))
                        .tooltip(studio_ui::components::Tooltip::text(reason.clone()))
                        .into_any_element(),
                    other => Self::render_chip_for(other).into_any_element(),
                };
                let detail = match &j.status {
                    JobStatus::Done(_) if j.cached > 0 => format!("{} · {} new, {} cached", j.project, j.rendered, j.cached),
                    _ => j.project.clone(),
                };
                let picture: AnyElement = match &j.thumb {
                    Some(Still(img)) => still_image(img.clone()).into_any_element(),
                    None => stage_image(j.art).into_any_element(),
                };
                let id = j.id;
                let action = match (&j.status, &j.path) {
                    (JobStatus::Done(_), Some(path)) => {
                        let path = path.clone();
                        Button::icon(("job-show", i), IconName::Folder)
                            .tooltip("Show in folder")
                            .on_click(move |_, _, _| crate::engine::reveal(&path))
                            .into_any_element()
                    }
                    (s, _) if s.active() => Button::icon(("job-cancel", i), IconName::X)
                        .tooltip("Cancel")
                        .on_click(cx.listener(move |this, _, _, cx| this.cancel_job(id, cx)))
                        .into_any_element(),
                    _ => {
                        let video = j.video;
                        Button::icon(("job-retry", i), IconName::Refresh)
                            .tooltip("Render again")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_video(video.0, video.1, cx);
                                this.start_render(cx);
                            }))
                            .into_any_element()
                    }
                };
                div()
                    .id(("job", i))
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .px(px(14.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(t.line)
                    .when(active, |d| d.bg(t.surface_2))
                    .hover_style(|s| s.bg(t.surface_2))
                    .child(col(72.).aspect_ratio(16. / 9.).rounded(radius::INNER).overflow_hidden().child(picture))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(strong(j.title.clone()).truncate())
                            .child(faint(detail, cx).truncate()),
                    )
                    .when(!compact, |d| d.child(col(96.).child(num(j.output.clone()).text_color(t.fg_2))))
                    .child(col(240.).child(status))
                    .when(!compact, |d| d.child(col(60.).child(num(j.time.clone()).text_color(t.fg_2))))
                    .child(col(30.).child(action))
            })
            .collect();

        let rendering = self.jobs.iter().filter(|j| matches!(j.status, JobStatus::Rendering { .. } | JobStatus::Exporting)).count();
        let waiting = self.jobs.iter().filter(|j| matches!(j.status, JobStatus::Waiting(_))).count();
        let empty = rows.is_empty();
        let engine = match (&self.engine.backend_info, self.engine.ffmpeg) {
            (_, false) => match &self.ffmpeg_dl {
                crate::engine::NaturalVoice::Downloading(f) => format!("Engine: setting up ffmpeg ({:.0}%)", f * 100.),
                _ => "Engine: ffmpeg is missing · download it in Settings → Rendering".to_string(),
            },
            (Some(info), true) => format!("Engine: {info}"),
            (None, true) => "Engine: ready".to_string(),
        };

        panel(cx)
            .size_full()
            .child(header)
            .child(
                div()
                    .id("jobs")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .size_full()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(px(8.))
                                .py(px(80.))
                                .child(Icon::new(IconName::Film).color(t.fg_3))
                                .child(strong(if self.jobs.is_empty() { "No renders yet" } else { "Nothing here" }))
                                .child(faint("Press Render in a video. Scenes you haven't changed are reused from the cache.", cx)),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .px(px(14.))
                    .h(px(38.))
                    .border_t_1()
                    .border_color(t.line)
                    .text_size(text::SMALL)
                    .text_color(t.fg_3)
                    .child(num(format!("{rendering} rendering · {waiting} waiting")))
                    .when(self.queue_paused, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_color(t.signal)
                                .child(Icon::new(IconName::Pause).small().color(t.signal))
                                .child("Queue paused"),
                        )
                    })
                    .child(div().flex_1())
                    .child(div().min_w_0().truncate().px(px(6.)).rounded(radius::CHIP).child(engine)),
            )
            .into_any_element()
    }
}

/// Failure reasons can be long engine messages; the chip shows the start, the tooltip all.
fn short_reason(reason: &str) -> String {
    let mut s: String = reason.chars().take(34).collect();
    if reason.chars().count() > 34 {
        s.push('…');
    }
    s
}
