//! Tells the crate where the built page is, so that it can be put inside
//! the binary. In the repository that is `ui/dist`; a copy of this crate
//! from crates.io carries it in `page/`. Without either the crate still
//! builds, and the server says how to build the page.

use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(embedded_ui)");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let places = [root.join("page"), root.join("../../ui/dist")];
    match places
        .iter()
        .find(|place| place.join("index.html").is_file())
    {
        Some(page) => {
            println!("cargo::rerun-if-changed={}", page.display());
            println!("cargo::rustc-env=MAPDIFF_PAGE_DIR={}", page.display());
            println!("cargo::rustc-cfg=embedded_ui");
        }
        None => {
            for place in &places {
                println!("cargo::rerun-if-changed={}", place.display());
            }
        }
    }
}
