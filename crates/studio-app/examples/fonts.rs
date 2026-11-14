//! Font comparison sheet for checking text rendering on each platform.
use gpui::{App, AppContext, Context, IntoElement, ParentElement, Render, Styled, Window, WindowOptions, div, px, rgb, FontWeight};

struct Sheet;

const SAMPLE: &str = "Storyboards and questions. A small model with Auto reasoning is enough. 0123456789";

impl Render for Sheet {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let row = |label: &str, family: &'static str, size: f32, ls: Option<f32>, w: FontWeight| {
            let mut d = div().font_family(family).text_size(px(size)).font_weight(w).child(format!("{label}: {SAMPLE}"));
            if let Some(l) = ls { d = d.letter_spacing(px(l)); }
            d
        };
        div().size_full().bg(rgb(0x1b1b1f)).text_color(rgb(0xeeeeee)).p(px(20.)).flex().flex_col().gap(px(10.))
            .child(row("A Inter 13", "Inter", 13., None, FontWeight::NORMAL))
            .child(row("B Inter 13 +0.2", "Inter", 13., Some(0.2), FontWeight::NORMAL))
            .child(row("C Inter 14", "Inter", 14., None, FontWeight::NORMAL))
            .child(row("D Inter Display 13", "Inter Display", 13., None, FontWeight::NORMAL))
            .child(row("E Segoe UI 13", "Segoe UI", 13., None, FontWeight::NORMAL))
            .child(row("F Segoe UI Variable Text 14", "Segoe UI Variable Text", 14., None, FontWeight::NORMAL))
            .child(row("G Inter 13 Medium", "Inter", 13., None, FontWeight::MEDIUM))
            .child(row("H Segoe UI 13 Semibold", "Segoe UI", 13., None, FontWeight::SEMIBOLD))
    }
}

fn main() {
    gpui_platform::application().with_assets(studio_ui::assets::Assets).run(|cx: &mut App| {
        studio_ui::init(cx).unwrap();
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Sheet)).unwrap();
        cx.activate(true);
    });
}
