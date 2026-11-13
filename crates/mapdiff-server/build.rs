//! Tells the crate whether the page has been built, so that it can be put
//! inside the binary. Without it the crate still builds, and the server
//! says how to build the page.

use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(embedded_ui)");
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/dist");
    println!("cargo::rerun-if-changed={}", dist.display());
    if dist.join("index.html").is_file() {
        println!("cargo::rustc-cfg=embedded_ui");
    }
}
