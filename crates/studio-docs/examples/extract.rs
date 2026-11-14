//! `cargo run -p studio-docs --example extract -- <file>`: print what Studio reads from a document.
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: extract <file>");
    let text = studio_docs::extract(std::path::Path::new(&path))?;
    println!("{} characters", text.len());
    println!("{}", studio_docs::truncate(&text, 600));
    Ok(())
}
