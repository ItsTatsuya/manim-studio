fn main() {
    println!("cargo:rerun-if-changed=resources/icon.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("resources/icon.ico")
            .set("ProductName", "Manim Studio")
            .set("FileDescription", "Manim Studio");
        res.compile().expect("failed to embed Windows resources");
    }
}
