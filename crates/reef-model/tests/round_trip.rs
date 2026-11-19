//! Every node written in reef's books prints as YAML and reads back as the same spec.

use reef_model::NodeSpec;
use reef_model::spec::parse_node_file;
use std::path::{Path, PathBuf};

/// Every markdown file under a directory
///
/// # Arguments
///
/// * `dir` - The directory to look in
fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    // walk the directory and everything under it
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_book_node_round_trips() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut specs = 0;
    for book in ["example-book/src", "plan/src"] {
        for path in markdown_files(&root.join(book)) {
            let text = std::fs::read_to_string(&path).unwrap();
            let Some((spec, _)) = parse_node_file(&text).unwrap() else { continue };
            // what is printed reads back as the same spec
            let yaml = spec.to_yaml();
            let back = NodeSpec::from_yaml(&yaml).unwrap_or_else(|e| panic!("{}: {e:?}\n{yaml}", path.display()));
            assert_eq!(back, spec, "{}:\n{yaml}", path.display());
            // and printing it again gives the same text
            assert_eq!(back.to_yaml(), yaml, "{}", path.display());
            specs += 1 + spec.nodes.len();
        }
    }
    // both books were found and have nodes
    assert!(specs > 250, "only {specs} specs");
}
