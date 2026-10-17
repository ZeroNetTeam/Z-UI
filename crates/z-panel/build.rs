//! The dashboard is built by `npm run build` into `dashboard/dist` and
//! embedded into the binary. A Rust-only build (CI's clippy, a contributor
//! without Node) must still compile, so a missing `dist` gets a placeholder
//! page saying how to build the real one.

use std::path::Path;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dashboard/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(&dist).expect("creating dashboard/dist");
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Z-UI</title></head>\
             <body><p>The dashboard was not built into this binary. Run \
             <code>npm --prefix dashboard ci &amp;&amp; npm --prefix dashboard run build</code> \
             and build again.</p></body></html>",
        )
        .expect("writing the placeholder dashboard");
    }
}
