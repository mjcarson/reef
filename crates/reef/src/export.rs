//! `reef export`: a plan written out as a book of group files with inline nodes
//!
//! ```text
//! reef export <DIR> [--project NAME] [--plan NAME]
//! ```
//!
//! - **A new book.** The directory must be new or empty. It gets a `book.toml`, a
//!   `src/SUMMARY.md` that lists each file, a file for each run of the plan's nodes that one
//!   group's file can hold, and a file for each of the plan's pages.
//! - **The nesting rebuilt from the written order.** A node with no body of its own, whose group
//!   is on the current file's open chain, is listed inline under that group. Any other node
//!   starts a file, keeping its `group:` and its body. The files are numbered so they sort in
//!   the plan's written order, so importing the book gives the plan back, each node at its place.
//! - **Pages where they were.** Each page is written at its path under the book's `src`, and
//!   listed in the summary after the node files, under `# Pages`, so its links to other pages
//!   lead where they did, and a ```` ```reef ```` fence's `page` can name it.
//! - **Links lead from each file.** A stored node's links lead from the repo's root, and a
//!   page's from its own path, so each link to a file of the repo is rewritten to lead to the
//!   same file and heading from where the book's file is, in the project's repo. A link to a
//!   page is left as written: node files sit at the root of `src`, where the plan's root is.
//!
//! The book's viewer isn't written, since mdbook fails on assets it can't find:
//! `mdbook-reef install` puts it in.

use reef_api::{StoredNode, StoredPage};
use reef_model::NodeSpec;
use reef_model::spec::{first_heading, frontmatter_is_yaml};
use reef_render::doc_body;
use reef_render::links::{LinkError, rewrite_links, rewrite_spec};
use reef_render::paths::{self, Target};
use std::collections::HashSet;
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

/// The plan's pages, for telling a link to one from a link to a file of the repo
struct Pages<'a> {
    /// Each page's path
    paths: HashSet<&'a str>,
}

impl<'a> Pages<'a> {
    /// The paths of a plan's pages
    ///
    /// # Arguments
    ///
    /// * `pages` - The plan's pages
    fn new(pages: &'a [StoredPage]) -> Self {
        Pages {
            paths: pages.iter().map(|page| page.path.as_str()).collect(),
        }
    }

    /// Whether a path names a page: the page's markdown, or the page it is served as
    ///
    /// # Arguments
    ///
    /// * `path` - The path, from the plan's root
    fn named(&self, path: &str) -> bool {
        self.paths.contains(path)
            || (path.ends_with(".html")
                && paths::md_of(path)
                    .iter()
                    .any(|md| self.paths.contains(md.as_str())))
    }

    /// A link written in a node or a page, as it leads from where the book's file is, or `None`
    /// if it is left as written
    ///
    /// # Arguments
    ///
    /// * `from` - Where the link is read from in the plan: `""` for a node, a page's path
    /// * `to_dir` - The directory of the book's file, from the repo's root
    /// * `link` - The link as written
    fn lead(&self, from: &str, to_dir: &str, link: &str) -> Option<String> {
        // a link to a page reaches it as it did, the pages keeping their places under `src`
        if let Target::Internal { path, .. } = paths::resolve(from, link)
            && self.named(&path)
        {
            return None;
        }
        // any other leads to the repo's file from where the book's file is
        paths::moved(from, to_dir, link)
    }
}

/// The files a plan's nodes and pages make in a book: each file's path under the book's
/// directory and its text
///
/// # Arguments
///
/// * `nodes` - The plan's nodes, in the order it was written in
/// * `pages` - The plan's pages
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
/// let written = files(&nodes, &[], "book/src").unwrap();
/// assert_eq!(written[0].0, "src/001-g.md");
/// assert_eq!(written[0].1, "---\nid: G\nnodes:\n- id: m\n---\n# G\n");
/// ```
pub fn files(
    nodes: &[StoredNode],
    pages: &[StoredPage],
    to_dir: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    // each node's link leads from the book's `src`, where every node's file is written
    let known = Pages::new(pages);
    let map = |link: &str| known.lead("", to_dir, link);
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
    let mut out = Vec::with_capacity(heads.len() + pages.len() + 1);
    let mut summary = String::from("# Summary\n\n");
    for (i, head) in heads.iter().enumerate() {
        let name = format!("{:0width$}-{}.md", i + 1, slug(&placed[*head].spec.id));
        // a page at the same path would be written over
        if known.paths.contains(name.as_str()) {
            anyhow::bail!(
                "page `{name}` has the name the file of node `{}` is written to: move the page",
                placed[*head].spec.id
            );
        }
        let spec = nested(&placed, *head);
        // listed in the summary by its title, else its id
        let title = spec.title.as_deref().unwrap_or(&spec.id);
        summary.push_str(&format!("- [{}]({name})\n", title.replace(']', "\\]")));
        let text = format!("---\n{}---\n{}", spec.to_yaml(), placed[*head].body);
        out.push((format!("src/{name}"), text));
    }
    // then each page at its path, listed after the nodes by its first heading
    let mut sorted: Vec<&StoredPage> = pages.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    if !sorted.is_empty() {
        summary.push_str("\n# Pages\n\n");
    }
    for page in sorted {
        // its links lead from where it is written, and the frontmatter holds none
        let to = paths::join(to_dir, paths::parent(&page.path));
        let map = |link: &str| known.lead(&page.path, &to, link);
        let at = frontmatter_is_yaml(&page.text).map_or(0, |fm| page.text.len() - fm.body.len());
        let (head, body) = page.text.split_at(at);
        let body = rewrite_links(body, &map).map_err(|error| LinkError {
            at: format!("page `{}`", page.path),
            ..error
        })?;
        // titled by its first heading, else its file's name
        let stem = page.path.rsplit('/').next().unwrap_or(&page.path);
        let title = first_heading(doc_body(&page.text))
            .unwrap_or_else(|| stem.trim_end_matches(".md").to_string());
        summary.push_str(&format!(
            "- [{}]({})\n",
            title.replace(']', "\\]"),
            page.path
        ));
        out.push((format!("src/{}", page.path), format!("{head}{body}")));
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
    // its nodes, in the order it was written in, and its pages, as the book's files
    let listing = client.list(&plan).await?;
    let pages = client.pages(&plan).await?.pages;
    let written = files(&listing.nodes, &pages, &to_dir)?;
    for (path, text) in &written {
        // a page's directories are made as it is written
        let path = book.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)?;
    }
    std::fs::write(book.join("book.toml"), book_toml(&plan.to_string()))?;
    // the summary is no node's or page's file
    let what = match pages.len() {
        0 => counted(listing.nodes.len(), "node"),
        count => format!(
            "{} and {}",
            counted(listing.nodes.len(), "node"),
            counted(count, "page")
        ),
    };
    writeln!(
        out,
        "exported {what} of `{plan}` to {} in {}: `mdbook-reef install {}` puts in the viewer \
         that draws it",
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
    use reef_api::{StoredNode, StoredPage};
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
        let written = files(&nodes, &[], "out/src").unwrap();
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
        let written = files(&nodes, &[], "out/src").unwrap();
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

    /// A page, as a listing gives it
    ///
    /// # Arguments
    ///
    /// * `path` - Its path
    /// * `text` - Its markdown
    fn page(path: &str, text: &str) -> StoredPage {
        StoredPage {
            path: path.to_string(),
            text: text.to_string(),
            version: 1,
        }
    }

    /// Pages are written at their paths under `src`, listed after the nodes by their first
    /// headings, their links to pages left as written and their other links moved
    #[test]
    fn pages_written_under_src_and_listed() {
        let nodes = [stored(
            "id: G\ndocs: [design/app.md#rows, docs/x.md]\n",
            "[plan](plan.md)\n",
            0,
        )];
        let pages = [
            page(
                "plan.md",
                "# The plan, full width\n\n[app](design/app.html)\n",
            ),
            page(
                "design/app.md",
                "---\ntitle: kept\n---\n## Rows\n\n[plan](../plan.md) [x](../docs/x.md#y) [r](#rows)\n",
            ),
            page("notes/untitled.md", "No heading.\n"),
        ];
        let written = files(&nodes, &pages, "out/src").unwrap();
        let paths: Vec<&str> = written.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "src/001-g.md",
                "src/design/app.md",
                "src/notes/untitled.md",
                "src/plan.md",
                "src/SUMMARY.md"
            ]
        );
        // a node's links to pages are left as written, and to the repo moved
        let (spec, fm) = reef_model::spec::parse_node_file(&written[0].1)
            .unwrap()
            .unwrap();
        assert_eq!(spec.docs, ["design/app.md#rows", "../../docs/x.md"]);
        assert_eq!(fm.body, "[plan](plan.md)\n");
        // a page keeps its frontmatter and its links to pages, and leads to the repo from where it is
        assert_eq!(
            written[1].1,
            "---\ntitle: kept\n---\n## Rows\n\n[plan](../plan.md) [x](../../../docs/x.md#y) [r](#rows)\n"
        );
        assert_eq!(written[3].1, pages[0].text);
        // the summary lists the pages after the nodes, by their first headings or names
        assert_eq!(
            written[4].1,
            "# Summary\n\n- [G](001-g.md)\n\n# Pages\n\n- [Rows](design/app.md)\n- [untitled](notes/untitled.md)\n- [The plan, full width](plan.md)\n"
        );
        // a page that a node's file would be written over is refused
        let error = files(&nodes, &[page("001-g.md", "# X\n")], "out/src")
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("page `001-g.md` has the name"), "{error}");
    }
}
