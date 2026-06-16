// Ensure web/dist/index.html exists at compile time so rust-embed (which embeds
// web/dist into the binary) never fails a fresh `cargo build`/`cargo test` before
// the frontend has been built. The release pipeline builds the real frontend
// first; this only drops a placeholder page otherwise, so the backend still
// compiles and runs (serving a "frontend not built" page) on a clean checkout.

use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let dist = Path::new(&manifest).join("../../web/dist");
    let index = dist.join("index.html");
    if !index.exists() {
        let _ = std::fs::create_dir_all(&dist);
        let _ = std::fs::write(
            &index,
            "<!doctype html><meta charset=utf-8><title>Fluid</title>\
             <body style=\"font-family:sans-serif;padding:2rem;background:#0b0e14;color:#e6edf3\">\
             <h1>Fluid 前端尚未构建</h1>\
             <p>运行 <code>npm --prefix web install &amp;&amp; npm --prefix web run build</code> 后重新构建后端。</p>",
        );
    }
    println!("cargo:rerun-if-changed=../../web/dist");
}
