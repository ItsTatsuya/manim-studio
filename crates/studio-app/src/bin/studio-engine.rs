//! Engine worker for Manim Studio's render queue (see `studio_render::worker`).

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(studio_render::worker::ARG) {
        args.remove(0);
    }
    std::process::exit(studio_render::worker::main(args));
}
