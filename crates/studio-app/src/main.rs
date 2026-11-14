// Release builds on Windows run without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod assistant;
mod audio;
mod crash;
mod attach;
mod engine;
mod manage;
mod model;
mod palette;
mod prefs;
mod shortcuts;
mod store;
mod update;
mod views;

fn main() {
    // `manim-studio render-worker ...`: render one scene for the queue, then exit.
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some(studio_render::worker::ARG) {
        std::process::exit(studio_render::worker::main(args));
    }
    crash::install();
    // A verified update staged last session is swapped in before anything opens.
    if update::apply_pending_at_start() {
        return;
    }
    gpui_platform::application()
        .with_assets(studio_ui::assets::Assets)
        .run(|cx| {
            studio_ui::init(cx).expect("failed to load bundled fonts");
            app::init(cx);
            app::open_main_window(cx);
            cx.activate(true);
        });
}
