//! reef stays a small, stable client: neither shoal nor Dioxus is in its tree
//!
//! Only `reef-service`, `reef-app` and `reef-shoal` may depend on shoal, and the CLI must never
//! depend on the app's wire format, so `cargo tree` of this crate shows neither.

use std::path::Path;
use std::process::Command;

/// The crates whose names say they belong to shoal or to Dioxus
///
/// # Arguments
///
/// * `tree` - `cargo tree`'s output, one crate a line with no prefix
fn forbidden(tree: &str) -> Vec<&str> {
    tree.lines()
        .filter(|line| {
            // each line is `name vX.Y.Z (source)`, sometimes marked `(*)`
            let name = line.split_whitespace().next().unwrap_or("");
            ["shoal", "dioxus", "glommio"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })
        .collect()
}

/// `cargo tree -p reef` names no shoal, Dioxus or glommio crate
#[test]
fn tree_has_neither_dioxus_nor_shoal() {
    // the root workspace's manifest, two directories up from this crate
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    // every crate the binary builds with, normal and build dependencies, from the lockfile alone
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "reef",
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
    // the tree was read, and holds the client's own dependencies
    assert!(
        stdout.lines().any(|line| line.starts_with("reqwest ")),
        "{stdout}"
    );
    // and none of shoal's or Dioxus's
    assert_eq!(forbidden(&stdout), Vec::<&str>::new());
}

/// The filter finds a forbidden crate wherever it sits in the tree
#[test]
fn forbidden_crates_are_found() {
    // a tree with each kind of crate in it, among allowed ones
    let tree = "reef v0.1.0\nshoal v0.1.0 (/src/shoal)\ndioxus-core v0.7.10\nreqwest v0.12.28\n";
    assert_eq!(
        forbidden(tree),
        vec!["shoal v0.1.0 (/src/shoal)", "dioxus-core v0.7.10"]
    );
}
