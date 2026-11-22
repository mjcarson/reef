//! `reef export`: a plan written out as a book of group files with inline nodes
//!
//! ```text
//! reef export <DIR> [--project NAME] [--plan NAME]
//! ```
//!
//! - **A new book.** The directory must be new or empty. It gets a `book.toml`, a
//!   `src/SUMMARY.md` that lists each file, and a file for each run of the plan's nodes that one
//!   group's file can hold.
//! - **The nesting rebuilt from the written order.** A node with no body of its own, whose group
//!   is on the current file's open chain, is listed inline under that group. Any other node
//!   starts a file, keeping its `group:` and its body. The files are numbered so they sort in
//!   the plan's written order, so importing the book gives the plan back, each node at its place.
//! - **Links lead from each file.** A stored node's links lead from the repo's root, so each is
//!   rewritten to lead to the same file and heading from the book's `src`, which must be in the
//!   project's repo.
//!
//! The book's viewer isn't written, since mdbook fails on assets it can't find:
//! `mdbook-reef install` puts it in.

use reef_api::StoredNode;
use reef_model::NodeSpec;
use reef_render::links::{LinkError, rewrite_links, rewrite_spec};
use reef_render::paths;
use std::io::Write;
use std::path::Path;

use crate::cli::{PlanPick, picked, plan_in};
use crate::client::Client;
use crate::import::{link_root, prefix};

/// A node of the plan, placed in a file of the book
struct Placed {
    /// Its spec, its links leading from the book's `src`, without its members yet
    spec: NodeSpec,
    /// Its body, its links leading from the book's `src`
    body: String,
    /// Where its members are among the nodes placed, in the order they are listed
    members: Vec<usize>,
}

/// The files a plan's nodes make in a book: each file's path under the book's directory and
/// its text
///
/// # Arguments
///
/// * `nodes` - The plan's nodes, in the order it was written in
/// * `to_dir` - The book's `src`, from the repo's root, which the links are to lead from
///
/// # Examples
///
/// ```
/// use reef::export::files;
/// use reef_api::StoredNode;
/// use reef_model::NodeSpec;
///
/// let node = |id: &str, group: Option<&str>, body: &str, order: u64| StoredNode {
///     spec: NodeSpec { id: id.into(), group: group.map(Into::into), ..NodeSpec::default() },
///     body: body.into(),
///     order,
///     version: 1,
/// };
/// // a group with a body, and a member with none, which is listed inline in its file
/// let nodes = [node("G", None, "# G\n", 0), node("m", Some("G"), "", 1)];
/// let written = files(&nodes, "book/src").unwrap();
/// assert_eq!(written[0].0, "src/001-g.md");
/// assert_eq!(written[0].1, "---\nid: G\nnodes:\n- id: m\n---\n# G\n");
/// ```
pub fn files(nodes: &[StoredNode], to_dir: &str) -> anyhow::Result<Vec<(String, String)>> {
    // each link leads from the book's `src`, where every file is written
    let map = |link: &str| paths::moved("", to_dir, link);
    let mut placed: Vec<Placed> = Vec::with_capacity(nodes.len());
    // the nodes that start a file, and the chain of nodes open in the current one
    let mut heads: Vec<usize> = Vec::new();
    let mut chain: Vec<usize> = Vec::new();
    for node in nodes {
        let mut spec = node.spec.clone();
        rewrite_spec(&mut spec, &map)?;
        let body = rewrite_links(&node.body, &map).map_err(|error| LinkError {
            at: format!("`{}` body", spec.id),
            ..error
        })?;
        // a node with no body of its own is listed under its group, if its file holds it
        let under = match (&spec.group, body.is_empty()) {
            (Some(group), true) => chain
                .iter()
                .rposition(|open| placed[*open].spec.id == *group),
            _ => None,
        };
        let at = placed.len();
        match under {
            Some(depth) => {
                // the chain is cut back to the group, which lists it after its members so far
                chain.truncate(depth + 1);
                placed[chain[depth]].members.push(at);
                // and the import that reads it back gives it that group again
                spec.group = None;
            }
            None => {
                // else it starts a file of its own, keeping the group it names
                heads.push(at);
                chain.clear();
            }
        }
        chain.push(at);
        placed.push(Placed {
            spec,
            body,
            members: Vec::new(),
        });
    }
    // each file numbered so the files sort in the order the plan was written in
    let width = heads.len().to_string().len().max(3);
    let mut out = Vec::with_capacity(heads.len() + 1);
    let mut summary = String::from("# Summary\n\n");
    for (i, head) in heads.iter().enumerate() {
        let name = format!("{:0width$}-{}.md", i + 1, slug(&placed[*head].spec.id));
        let spec = nested(&placed, *head);
        // listed in the summary by its title, else its id
        let title = spec.title.as_deref().unwrap_or(&spec.id);
        summary.push_str(&format!("- [{}]({name})\n", title.replace(']', "\\]")));
        let text = format!("---\n{}---\n{}", spec.to_yaml(), placed[*head].body);
        out.push((format!("src/{name}"), text));
    }
    out.push(("src/SUMMARY.md".to_string(), summary));
    Ok(out)
}

/// A placed node's spec, with each of its members listed in it in turn
///
/// # Arguments
///
/// * `placed` - The nodes placed
/// * `at` - Where the node is among them
fn nested(placed: &[Placed], at: usize) -> NodeSpec {
    let mut spec = placed[at].spec.clone();
    spec.nodes = placed[at]
        .members
        .iter()
        .map(|member| nested(placed, *member))
        .collect();
    spec
}

/// A file name's stem for a node: its id lowercased, each run of anything but ASCII letters
/// and digits a `-`
///
/// # Arguments
///
/// * `id` - The node's id
fn slug(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for c in id.chars() {
        // letters and digits kept, everything else one `-` a run
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    // with no `-` at either end, and a name for an id with nothing to keep
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "node".to_string()
    } else {
        trimmed.to_string()
    }
}

/// A book's `book.toml`, titled by the plan it holds
///
/// # Arguments
///
/// * `title` - The book's title
fn book_toml(title: &str) -> String {
    // the title as a TOML string, quoted and escaped
    let title = toml::Value::String(title.to_string());
    format!("[book]\ntitle = {title}\nsrc = \"src\"\n\n[preprocessor.reef]\n")
}

/// Exports a plan to a new book in a directory of its project's repo
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which a relative path to the book starts from
/// * `dir` - The directory to write the book in, new or empty
/// * `pick` - The project and plan named, if they were
/// * `out` - Where to print what was exported
pub async fn export<W: Write>(
    client: &Client,
    cwd: &Path,
    dir: &Path,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // a directory that holds anything is left as it is
    let book = cwd.join(dir);
    let existed = book.exists();
    if existed && std::fs::read_dir(&book)?.next().is_some() {
        anyhow::bail!("{} isn't empty: export writes a new book", dir.display());
    }
    std::fs::create_dir_all(&book)?;
    // the book, written, or the directory made for it taken away again
    let written = write_book(client, &book, dir, pick, out).await;
    if written.is_err() && !existed {
        let _ = std::fs::remove_dir_all(&book);
    }
    written
}

/// Writes a plan's book into a directory that is there and empty
///
/// # Arguments
///
/// * `client` - The service's client
/// * `book` - The directory to write the book in
/// * `dir` - The directory as it was given, for messages
/// * `pick` - The project and plan named, if they were
/// * `out` - Where to print what was exported
async fn write_book<W: Write>(
    client: &Client,
    book: &Path,
    dir: &Path,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the plan, in the project the directory is in, else the one named
    let book = book.canonicalize()?;
    let project = picked(client, &book, &pick.project).await?;
    let root = link_root(client, &project, &book).await?;
    let to_dir = paths::join(&prefix(&root, &book)?, "src");
    let plan = plan_in(client, project, pick.plan.as_deref()).await?;
    // its nodes, in the order it was written in, as the book's files
    let listing = client.list(&plan).await?;
    let written = files(&listing.nodes, &to_dir)?;
    std::fs::create_dir_all(book.join("src"))?;
    for (path, text) in &written {
        std::fs::write(book.join(path), text)?;
    }
    std::fs::write(book.join("book.toml"), book_toml(&plan.to_string()))?;
    // the summary is no node's file
    writeln!(
        out,
        "exported {} of `{plan}` to {} in {}: `mdbook-reef install {}` puts in the viewer that \
         draws it",
        counted(listing.nodes.len(), "node"),
        dir.display(),
        counted(written.len() - 1, "file"),
        dir.display()
    )?;
    Ok(())
}

/// A count of things, the word for them singular for one
///
/// # Arguments
///
/// * `count` - How many there are
/// * `word` - The word for one of them
fn counted(count: usize, word: &str) -> String {
    match count {
        1 => format!("1 {word}"),
        count => format!("{count} {word}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::{files, slug};
    use reef_api::StoredNode;
    use reef_model::NodeSpec;

    /// A stored node, as a listing gives it
    ///
    /// # Arguments
    ///
    /// * `yaml` - Its spec
    /// * `body` - Its row's body
    /// * `order` - Where it falls in the written order
    fn stored(yaml: &str, body: &str, order: u64) -> StoredNode {
        StoredNode {
            spec: NodeSpec::from_yaml(yaml).unwrap(),
            body: body.to_string(),
            order,
            version: 1,
        }
    }

    /// Each file's node and the ids listed in it, depth first
    ///
    /// # Arguments
    ///
    /// * `written` - The files written
    fn shape(written: &[(String, String)]) -> Vec<(String, Vec<String>)> {
        /// A spec's id, then each of its members' in turn
        ///
        /// # Arguments
        ///
        /// * `spec` - The spec
        /// * `out` - The ids so far
        fn ids(spec: &NodeSpec, out: &mut Vec<String>) {
            out.push(spec.id.clone());
            spec.nodes.iter().for_each(|member| ids(member, out));
        }
        written
            .iter()
            .filter(|(path, _)| !path.ends_with("SUMMARY.md"))
            .map(|(path, text)| {
                let (spec, _) = reef_model::spec::parse_node_file(text).unwrap().unwrap();
                let mut out = Vec::new();
                ids(&spec, &mut out);
                (path.clone(), out)
            })
            .collect()
    }

    /// A member after a file node goes back under its group, a chain is cut back to an outer
    /// group, and a node whose group comes later, or that has a body, starts a file of its own
    #[test]
    fn nesting_rebuilt_from_the_written_order() {
        let nodes = [
            // a node with a body whose group is written after it
            stored("id: a\ngroup: F\n", "# A\n", 0),
            stored("id: F\n", "", 1),
            stored("id: A\ngroup: F\n", "", 2),
            stored("id: a1\ngroup: A\n", "", 3),
            stored("id: a1x\ngroup: a1\n", "", 4),
            // back out to F, past A and a1
            stored("id: m\ngroup: F\n", "", 5),
            // a member of A, which m has closed
            stored("id: a2\ngroup: A\n", "", 6),
        ];
        let written = files(&nodes, "out/src").unwrap();
        let shape = shape(&written);
        let expected: Vec<(String, Vec<String>)> = vec![
            ("src/001-a.md".into(), vec!["a".into()]),
            (
                "src/002-f.md".into(),
                ["F", "A", "a1", "a1x", "m"].map(String::from).to_vec(),
            ),
            ("src/003-a2.md".into(), vec!["a2".into()]),
        ];
        assert_eq!(shape, expected);
        // a2 keeps its group, which a member listed inline leaves to the node that lists it
        assert!(written[2].1.contains("group: A\n"), "{}", written[2].1);
        assert!(!written[1].1.contains("group:"), "{}", written[1].1);
        // and the summary lists each file
        let summary = &written.last().unwrap().1;
        assert_eq!(
            summary,
            "# Summary\n\n- [a](001-a.md)\n- [F](002-f.md)\n- [a2](003-a2.md)\n"
        );
    }

    /// Bodies keep their every space, and links lead from the book's `src`
    #[test]
    fn bodies_kept_and_links_moved() {
        let body = "  # Indented\n\nSee [d](docs/d.md#x).\n\n\n";
        let member = "id: m\ngroup: G\ndocs: [docs/d.md]\nbody: \"  lead\\n\\ntrail\\n\\n\\n\"\n";
        let nodes = [stored("id: G\n", body, 0), stored(member, "", 1)];
        let written = files(&nodes, "out/src").unwrap();
        let (spec, fm) = reef_model::spec::parse_node_file(&written[0].1)
            .unwrap()
            .unwrap();
        assert_eq!(fm.body, "  # Indented\n\nSee [d](../../docs/d.md#x).\n\n\n");
        assert_eq!(spec.nodes[0].body.as_deref(), Some("  lead\n\ntrail\n\n\n"));
        assert_eq!(spec.nodes[0].docs, ["../../docs/d.md"]);
    }

    /// A file's name keeps an id's letters and digits, lowercased
    #[test]
    fn slugs() {
        assert_eq!(slug("Item 346"), "item-346");
        assert_eq!(slug("C26"), "c26");
        assert_eq!(slug("--"), "node");
        assert_eq!(slug("a/b.c"), "a-b-c");
    }
}
