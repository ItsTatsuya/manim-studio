//! Write PNG stills of the sample lesson for visual review:
//! `cargo run -p studio-render --release --example snapshots -- <out dir> [style index] [times...]`

use studio_render::preview::{OpenScene, load, thumbnail_time};
use studio_render::{Output, sample, style};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = std::path::PathBuf::from(args.first().cloned().unwrap_or_else(|| "snapshots".into()));
    let style_ix: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let fractions: Vec<f64> = args.iter().skip(2).filter_map(|s| s.parse().ok()).collect();
    // The app's bundled fonts, so stills match the app rather than this machine's fonts.
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../studio-ui/assets");
    studio_render::fonts::register_dir(&assets.join("fonts"));
    studio_render::fonts::register_dir(&assets.join("lesson-fonts"));
    let dir = out.join("video");
    let scenes = sample::install(&dir)?;
    let output = Output::new(960, 540, 30.0);
    for path in scenes {
        let req = OpenScene { path: path.clone(), root: dir.clone(), theme_json: style::json(style_ix).into(), scene: None, globals: vec![], narration: None, narration_lines: vec![] };
        let (host, name, t) = load(&req, output)?;
        let mut times = vec![thumbnail_time(&t)];
        times.extend(fractions.iter().map(|f| f * t.duration));
        for (k, time) in times.iter().enumerate() {
            let rgba = host.frame_at(*time)?;
            let file = out.join(format!("{name}-{k}.png"));
            studio_render::manim::output::save_png(&file, output.width, output.height, &rgba)?;
            println!("{} {:.2}s/{:.2}s  {}", name, time, t.duration, file.display());
        }
        for p in &t.problems {
            println!("  problem: {} at {:.2}s", p.message, p.time);
        }
    }
    Ok(())
}
