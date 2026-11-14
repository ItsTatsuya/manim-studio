//! Stills of the style showcase in every house style, for visual review:
//! `cargo run -p studio-render --release --example showcase -- <out dir>`

use studio_render::preview::{OpenScene, load, thumbnail_time};
use studio_render::{Output, showcase, style};

fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "showcase".into()));
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../studio-ui/assets");
    studio_render::fonts::register_dir(&assets.join("fonts"));
    studio_render::fonts::register_dir(&assets.join("lesson-fonts"));
    let dir = out.join("src");
    let path = match std::env::var("SHOWCASE_SCRIPT") { Ok(p) => std::path::PathBuf::from(p), Err(_) => showcase::install(&dir)? };
    let output = Output::new(960, 540, 30.0);
    for (i, s) in style::STYLES.iter().enumerate() {
        let req = OpenScene { path: path.clone(), root: dir.clone(), theme_json: style::json(i).into(), scene: None, globals: vec![], narration: None, narration_lines: vec![] };
        let (host, _, t) = load(&req, output)?;
        for (k, time) in [thumbnail_time(&t), 1.0].iter().enumerate() {
            let file = out.join(format!("{}-{k}.png", s.id));
            studio_render::manim::output::save_png(&file, output.width, output.height, &host.frame_at(*time)?)?;
        }
        println!("{} {:.2}s", s.id, t.duration);
        for p in &t.problems {
            println!("  problem: {} at {:.2}s", p.message, p.time);
        }
    }
    Ok(())
}
