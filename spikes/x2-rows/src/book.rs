//! The example book's plan, read the way mdbook-reef reads it, and flattened into one spec a node
//!
//! mdbook-reef is a binary alone, so its walk is copied here from
//! `crates/mdbook-reef/src/plan.rs`. A book keeps most nodes inline in a group's `nodes:` list,
//! while the store keeps a row a node, so each member is lifted out into a spec of its own.

use reef_model::spec::parse_node_file;
use reef_model::{Diagnostic, Graph, NodeSpec, Origin, Record, nodes_from_records};
use std::path::{Path, PathBuf};

/// One node as the store keeps it: a spec with no members, its body and its written order
#[derive(Debug, Clone)]
pub struct Flat {
    /// The node's spec, its `group` filled in if it was listed inline, and no `nodes`
    pub spec: NodeSpec,
    /// A file node's page; empty for a node listed inline, whose body stays in its spec
    pub body: String,
    /// Where it falls in the order the plan was written in
    pub order: u64,
}

/// The example book's `src`, beside the spike in this checkout
pub fn example_book() -> PathBuf {
    // two levels up from the spike is reef's root
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../example-book/src")
}

/// Every markdown file under a book's `src`, relative to it and sorted, as mdbook-reef finds them
///
/// # Arguments
///
/// * `src` - The book's `src` directory
fn markdown_files(src: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![src.to_path_buf()];
    // walk the whole of `src`, since the example book names no roots
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
pub fn records(src: &Path) -> anyhow::Result<Vec<Record>> {
    let mut records = Vec::new();
    for (order, rel) in markdown_files(src).into_iter().enumerate() {
        // a page with no frontmatter is not a node
        let text = std::fs::read_to_string(src.join(&rel))?;
        let Some((spec, fm)) = parse_node_file(&text)
            .map_err(|error| anyhow::anyhow!("{rel}: {}", error.message))?
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

/// Builds the graph a book's records make, the way mdbook-reef does before rendering
///
/// # Arguments
///
/// * `records` - The book's records
pub fn graph(records: Vec<Record>) -> (Graph, Vec<Diagnostic>) {
    // the nodes in written order, members after their groups, checked as one graph
    Graph::build(nodes_from_records(records))
}

/// Lifts every inline member out of its group, so each node is a spec of its own
///
/// A member names the group that listed it, as `Node::from_spec` would have, and the order is
/// depth first: a group, then its members, then the next record. The nodes these build are the
/// nodes the records build, except that none is inline.
///
/// # Arguments
///
/// * `records` - The book's records, in any order
pub fn flatten(mut records: Vec<Record>) -> Vec<Flat> {
    // put the records in written order, ties as given, as `nodes_from_records` does
    records.sort_by_key(|record| record.order);
    let mut out = Vec::new();
    for record in records {
        lift(record.spec, record.body, &mut out);
    }
    out
}

/// Pushes a spec, then each of its members after it
///
/// # Arguments
///
/// * `spec` - The spec to push, members and all
/// * `body` - Its body: a file node's page, or empty for a member
/// * `out` - The flattened specs so far
fn lift(mut spec: NodeSpec, body: String, out: &mut Vec<Flat>) {
    // take its members out of it, so the store holds it alone
    let members = std::mem::take(&mut spec.nodes);
    let id = spec.id.clone();
    let order = out.len() as u64;
    out.push(Flat { spec, body, order });
    for mut member in members {
        // a member belongs to the node that lists it unless it names a group
        if member.group.is_none() {
            member.group = Some(id.clone());
        }
        // its body stays in its spec, as it was written
        lift(member, String::new(), out);
    }
}
