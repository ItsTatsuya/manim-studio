//! Rebuild `resources/icon.ico` from the logo (`studio-ui/assets/brand/logo.svg`):
//! `cargo run -p studio-app --example make_icon`

use resvg::{tiny_skia, usvg};

fn main() -> anyhow::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let svg = std::fs::read(root.join("../studio-ui/assets/brand/logo.svg"))?;
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default())?;
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let mut pngs = Vec::new();
    for s in sizes {
        let mut pixmap = tiny_skia::Pixmap::new(s, s).expect("non-zero size");
        let k = s as f32 / tree.size().width();
        resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pixmap.as_mut());
        pngs.push((s, pixmap.encode_png()?));
    }
    // ICO: header, one directory entry per image, then the PNGs.
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(pngs.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * pngs.len() as u32;
    for (s, png) in &pngs {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in &pngs {
        ico.extend_from_slice(png);
    }
    let out = root.join("resources/icon.ico");
    std::fs::write(&out, ico)?;
    std::fs::write(root.join("resources/icon-256.png"), &pngs.last().expect("sizes").1)?;
    println!("wrote {}", out.display());
    Ok(())
}
