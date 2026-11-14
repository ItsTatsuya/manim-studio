//! `cargo run -p icongen` regenerates `crates/studio-app/resources/` from
//! `crates/studio-ui/assets/brand/logo.svg`.

use anyhow::{Context, Result};
use resvg::{tiny_skia, usvg};
use std::path::Path;

const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let svg = std::fs::read(root.join("crates/studio-ui/assets/brand/logo.svg"))?;
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default())?;
    let out = root.join("crates/studio-app/resources");
    std::fs::create_dir_all(&out)?;

    let mut icon = ico::IconDir::new(ico::ResourceType::Icon);
    for size in SIZES {
        let mut pixmap = tiny_skia::Pixmap::new(size, size).context("pixmap")?;
        let scale = size as f32 / tree.size().width();
        resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        // tiny-skia stores premultiplied RGBA; ICO wants straight alpha.
        let rgba: Vec<u8> = pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        let image = ico::IconImage::from_rgba_data(size, size, rgba);
        icon.add_entry(ico::IconDirEntry::encode(&image)?);
        if size == 256 {
            pixmap.save_png(out.join("icon-256.png"))?;
        }
    }
    icon.write(std::fs::File::create(out.join("icon.ico"))?)?;
    println!("wrote {}", out.display());
    Ok(())
}
