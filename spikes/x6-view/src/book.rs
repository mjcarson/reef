//! A book's plan as the viewer's payload, read the way mdbook-reef reads it
//!
//! mdbook-reef is a binary alone, so its walk is copied here, by way of
//! `spikes/x2-rows/src/book.rs`, from `crates/mdbook-reef/src/plan.rs`. Its doc links start
//! with reef-render's `ROOT`, which is replaced with the path the server serves the book's
//! rendered pages from, so the viewer's *Read here* fetches them wherever the app's route is.

use reef_model::spec::parse_node_file;
use reef_model::{Graph, Origin, Payload, Record, Status, nodes_from_records};
use reef_render::{FsSource, ROOT, Renderer, Sections, paths};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A book beside the spike in this checkout
pub struct Book {
    /// The book's directory, holding `book.toml`
    pub dir: PathBuf,
    /// The path the server serves the book's rendered pages from, with a trailing slash
    pub served_at: &'static str,
}

impl Book {
    /// The example book, served at `/book/example/`
    pub fn example() -> Self {
        // two levels up from the spike is reef's root
        Book {
            dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../example-book"),
            served_at: "/book/example/",
        }
    }

    /// This plan's own book, served at `/book/plan/`
    pub fn plan() -> Self {
        // two levels up from the spike is reef's root
        Book {
            dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan"),
            served_at: "/book/plan/",
        }
    }

    /// The directory `mdbook build` renders the book's pages into
    pub fn rendered(&self) -> PathBuf {
        self.dir.join("book")
    }

    /// Reads the book's plan into the viewer's payload, its links under `served_at`
    pub fn payload(&self) -> anyhow::Result<Payload> {
        let src = self.dir.join("src");
        // read every node file and build them into one checked graph
        let (mut graph, mut diags) = Graph::build(nodes_from_records(records(&src)?));
        // check their doc links and fill in their brief forms from their docs
        let renderer = Renderer::new(FsSource::new(&src), Sections::default());
        diags.extend(renderer.prepare(&mut graph));
        // a problem doesn't stop the plan drawing, as in mdbook-reef's build
        for diag in &diags {
            eprintln!("x6: {diag}");
        }
        // a node written in a chapter links to the chapter's page
        let chapters = chapters(&src)?;
        let page_of = |node: &reef_model::Node| {
            node.origin
                .file()
                .filter(|file| chapters.contains(*file))
                .map(|file| format!("{ROOT}{}", paths::html_of(file)))
        };
        let payload = renderer.payload(&graph, page_of);
        // point its links at the served pages, as mdbook-reef points them at the page's prefix
        let json = serde_json::to_string(&payload)?.replace(ROOT, self.served_at);
        Ok(serde_json::from_str(&json)?)
    }
}

/// Marks the first work item that is still to do as done, as a second revision of a plan would
///
/// Returns the id of the node it marked, if any was to do.
///
/// # Arguments
///
/// * `payload` - The plan to change
pub fn finish_one(payload: &mut Payload) -> Option<String> {
    // a group is a node another names as its parent; only work items take a status
    let groups: HashSet<String> = payload
        .nodes
        .iter()
        .filter_map(|node| node.parent.clone())
        .collect();
    let node = payload.nodes.iter_mut().find(|node| {
        !groups.contains(&node.id) && matches!(node.status, None | Some(Status::Todo))
    })?;
    // mark it done, as a put of a newer revision would
    node.status = Some(Status::Done);
    Some(node.id.clone())
}

/// The book's chapter files, relative to `src`, as its `SUMMARY.md` links them
///
/// # Arguments
///
/// * `src` - The book's `src` directory
fn chapters(src: &Path) -> anyhow::Result<HashSet<String>> {
    let summary = std::fs::read_to_string(src.join("SUMMARY.md"))?;
    // every `(path.md)` link target is a chapter
    let chapters = summary
        .split("](")
        .skip(1)
        .filter_map(|rest| rest.split_once(')'))
        .map(|(link, _)| link.trim_start_matches("./").to_string())
        .filter(|link| link.ends_with(".md"))
        .collect();
    Ok(chapters)
}

/// Every markdown file under a book's `src`, relative to it and sorted, as mdbook-reef finds them
///
/// # Arguments
///
/// * `src` - The book's `src` directory
fn markdown_files(src: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![src.to_path_buf()];
    // walk the whole of `src`, since neither book names roots
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // dotfiles are never nodes
            if name.starts_with('.') {
                continue;
            }
            // descend into directories, and keep every markdown page but the summary
            if path.is_dir() {
                stack.push(path);
            } else if name.ends_with(".md")
                && name != "SUMMARY.md"
                && let Ok(rel) = path.strip_prefix(src)
            {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    // the order files sort in is the order the plan was written in
    out.sort();
    out
}

/// Reads every node file of a book into a record, in the order its files sort in
///
/// # Arguments
///
/// * `src` - The book's `src` directory
fn records(src: &Path) -> anyhow::Result<Vec<Record>> {
    let mut records = Vec::new();
    for (order, rel) in markdown_files(src).into_iter().enumerate() {
        // a page with no frontmatter is not a node
        let text = std::fs::read_to_string(src.join(&rel))?;
        let Some((spec, fm)) =
            parse_node_file(&text).map_err(|error| anyhow::anyhow!("{rel}: {}", error.message))?
        else {
            continue;
        };
        // keep the spec, the page after its frontmatter, and where it was written
        records.push(Record {
            spec,
            body: fm.body.to_string(),
            order: order as u64,
            origin: Origin::File {
                source: rel,
                line: 1,
            },
        });
    }
    Ok(records)
}
