// Ensure web/dist/index.html exists for fresh debug builds/tests by creating a
// visible placeholder when the frontend has not been built. Release builds are
// stricter: they reject a missing/placeholder SPA so rust-embed can never package
// a distributable binary without the real Vite frontend.

use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let dist = Path::new(&manifest).join("../../web/dist");
    let index = dist.join("index.html");
    let profile = std::env::var("PROFILE").unwrap_or_default();
    println!("cargo:rerun-if-changed=../../web/dist");

    if !index.exists() {
        if profile == "release" {
            panic!(
                "release build requires the real frontend; run `npm --prefix web ci && npm --prefix web run build` first"
            );
        }
        let _ = std::fs::create_dir_all(&dist);
        let _ = std::fs::write(
            &index,
            "<!doctype html><meta charset=utf-8><title>Fluid</title>\
             <body style=\"font-family:sans-serif;padding:2rem;background:#0b0e14;color:#e6edf3\">\
             <h1>Fluid 前端尚未构建</h1>\
             <p>运行 <code>npm --prefix web install &amp;&amp; npm --prefix web run build</code> 后重新构建后端。</p>",
        );
    }

    if profile == "release" {
        let html = std::fs::read_to_string(&index).unwrap_or_else(|error| {
            panic!("cannot read release frontend {}: {error}", index.display())
        });
        let has_app_shell = html.contains("id=\"app\"");
        let has_built_script = html.contains("<script type=\"module\"")
            && html.contains("src=\"/assets/")
            && dist.join("assets").is_dir();
        assert!(
            has_app_shell && has_built_script,
            "release frontend is a placeholder or incomplete; rebuild web/dist before packaging"
        );
    }
}
