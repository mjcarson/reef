//! The app's wasm client never holds shoal, the service or reef-view
//!
//! gxhash can't target wasm, and the service is the only code that opens shoal, so the client's
//! tree, with `web` on for wasm32, names neither. The canvas is Dioxus's own, drawn from
//! reef-draw, so reef-view's viewer isn't linked in either.

use std::path::Path;
use std::process::Command;

/// The crates whose names say they belong to shoal, glommio, the service or reef-view
///
/// # Arguments
///
/// * `tree` - `cargo tree`'s output, one crate a line with no prefix
fn forbidden(tree: &str) -> Vec<&str> {
    tree.lines()
        .filter(|line| {
            // each line is `name vX.Y.Z (source)`, sometimes marked `(*)`
            let name = line.split_whitespace().next().unwrap_or("");
            name.starts_with("shoal")
                || name.starts_with("glommio")
                || name == "reef-service"
                || name == "reef-view"
        })
        .collect()
}

/// `cargo tree` of the wasm client names no shoal, glommio, reef-service or reef-view crate
#[test]
fn the_wasm_client_has_neither_shoal_nor_the_service() {
    // central's workspace manifest, one directory up from this crate
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../Cargo.toml");
    // every crate the client builds with, as dx builds it, from the lockfile alone
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "reef-app",
            "--features",
            "web",
            "--target",
            "wasm32-unknown-unknown",
            "-e",
            "normal,build",
            "--prefix",
            "none",
            "--frozen",
        ])
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .expect("cargo runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // the tree was read, and holds the client's own renderer and what the canvas draws from
    assert!(
        stdout.lines().any(|line| line.starts_with("dioxus-web ")),
        "{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line.starts_with("reef-draw ")),
        "{stdout}"
    );
    // and none of shoal's, glommio's, the service's or reef-view's
    assert_eq!(forbidden(&stdout), Vec::<&str>::new());
}
