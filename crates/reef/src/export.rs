//! `reef export`: a plan written out as a book of group files with inline nodes, new or over
//! the book it lives in
//!
//! ```text
//! reef export <DIR> [--replace] [--project NAME] [--plan NAME]
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
//!   same file and heading from where the book's file is, in the project's repo. A node's link
//!   to a page leads to it from the node's file, which a new book writes at the root of `src`,
//!   where the plan's root is, so there it is left as written.
//! - **Over the book it lives in, with `--replace`.** The directory must hold a reef book, and
//!   its layout is kept. A node that heads a file of the book keeps that file. A node with no
//!   body of its own is listed under its group, wherever the group is written, so a node put
//!   from the terminal lands in its group's file. Any other node starts a file beside its
//!   group's. The summary keeps its lines and parts, dropping those of files the plan no longer
//!   has and listing a new file after the last line of its kind, and the book's `book.toml`, its
//!   viewer and its other files are left alone. A markdown file of the book the plan no longer
//!   has is removed. Only a file whose text changes is written, so an export over a book it just
//!   wrote writes nothing.
//!
//! A new book's viewer isn't written, since mdbook fails on assets it can't find:
//! `mdbook-reef install` puts it in.

use reef_api::{StoredNode, StoredPage};
use reef_model::NodeSpec;
use reef_model::spec::{first_heading, frontmatter_is_yaml};
use reef_render::book::{markdown_files, read_nodes};
use reef_render::doc_body;
use reef_render::links::{LinkError, rewrite_links, rewrite_spec};
use reef_render::paths::{self, Target};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::Path;

use crate::cli::{PlanPick, picked, plan_in};
use crate::client::Client;
use crate::import::{default_sections, link_root, open_book, prefix};

/// A node of the plan, placed in a file of the book
struct Placed {
    /// Its spec, its links leading from its file, without its members yet
    spec: NodeSpec,
    /// Its body, its links leading from its file
    body: String,
    /// Where its members are among the nodes placed, in the order they are listed
    members: Vec<usize>,
}

/// Where each of a plan's nodes is written in a book
struct Placing {
    /// The nodes that head a file, by their places in the written order, in the order their
    /// files are listed
    heads: Vec<usize>,
    /// Each head's file, from the book's `src`, in the order of `heads`
    paths: Vec<String>,
    /// The members each node lists inline, by their places in the written order
    members: Vec<Vec<usize>>,
    /// The file each node is written in, by its place in `heads`
    file_of: Vec<usize>,
}

/// The layout of a book a plan is written back over: the file each of its nodes heads, and the
/// other files it holds
struct Layout {
    /// The file each node that heads one is written in, from the book's `src`, by its id
    heads: HashMap<String, String>,
    /// Each node file's title, as its summary may list it: its node's title, else its id
    titles: HashMap<String, String>,
    /// Every markdown file of the book but its summary, from its `src`
    markdown: BTreeSet<String>,
    /// The book's summary, if it has one
    summary: Option<String>,
    /// The directories under `src` the book's nodes are read from, or none for all of it
    roots: Vec<String>,
}

impl Layout {
    /// Reads the layout of the book whose `src` this is
    ///
    /// A file that can't be read as a node, a node heading two files, and a markdown file that
    /// can't be read are refused, since the export would take any of them for a page the plan
    /// doesn't have and remove it.
    ///
    /// # Arguments
    ///
    /// * `src` - The book's `src`
    /// * `roots` - The directories under `src` its nodes are read from, or none for all of it
    fn read(src: &Path, roots: &[String]) -> anyhow::Result<Layout> {
        // the book's node files, each of which must read as one
        let (files, diags) = read_nodes(src, roots);
        if !diags.is_empty() {
            let lines: Vec<String> = diags.iter().map(|diag| format!("  {diag}")).collect();
            anyhow::bail!(
                "the book's files can't all be read as nodes, so which is a page isn't known:\n{}",
                lines.join("\n")
            );
        }
        // each node heading one file
        let mut heads = HashMap::with_capacity(files.len());
        let mut titles = HashMap::with_capacity(files.len());
        for file in &files {
            if let Some(other) = heads.insert(file.spec.id.clone(), file.rel.clone()) {
                anyhow::bail!(
                    "`{}` heads both {other} and {} in the book, so which file is its isn't known",
                    file.spec.id,
                    file.rel
                );
            }
            titles.insert(file.rel.clone(), node_title(&file.spec).to_string());
        }
        // and every other markdown file readable, so none is taken for a page by mistake
        let markdown: BTreeSet<String> = markdown_files(src, &[]).into_iter().collect();
        for rel in &markdown {
            if !titles.contains_key(rel) && std::fs::read_to_string(src.join(rel)).is_err() {
                anyhow::bail!(
                    "{rel} can't be read as markdown, so the export would take it for a page the \
                     plan doesn't have and remove it: move it out of the book's src"
                );
            }
        }
        Ok(Layout {
            heads,
            titles,
            markdown,
            summary: std::fs::read_to_string(src.join("SUMMARY.md")).ok(),
            roots: roots.to_vec(),
        })
    }
}

/// The files an export over a book writes and removes
struct Kept {
    /// Each file to write, from the book's `src`, with its text, the summary last
    files: Vec<(String, String)>,
    /// The book's markdown files the plan no longer has, from its `src`
    removed: Vec<String>,
    /// The files written that the book didn't have, from its `src`
    added: Vec<String>,
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

    /// A link written in a page, as it leads from where the book's file is, or `None` if it is
    /// left as written
    ///
    /// # Arguments
    ///
    /// * `from` - The page's path in the plan
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

    /// A link written in a node, as it leads from the book's file the node is written in, or
    /// `None` if it is left as written
    ///
    /// # Arguments
    ///
    /// * `file` - The node's file, from the book's `src`
    /// * `src_dir` - The book's `src`, from the repo's root
    /// * `link` - The link as written, from the repo's root
    fn from_node(&self, file: &str, src_dir: &str, link: &str) -> Option<String> {
        // only a link to a file of the repo's leads elsewhere from the book
        let Target::Internal { path, anchor } = paths::resolve("", link) else {
            return None;
        };
        let dir = paths::parent(file);
        // a link to a page leads to it from the file's directory, as written at the root
        if self.named(&path) {
            return match dir.is_empty() {
                true => None,
                false => paths::moved("", dir, link),
            };
        }
        // a link to a heading of the node's own file names the heading alone
        if let Some(anchor) = anchor
            && path == paths::join(src_dir, file)
        {
            return Some(format!("#{anchor}"));
        }
        // and any other leads to the repo's file from where the book's file is
        paths::moved("", &paths::join(src_dir, dir), link)
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
    // each node placed in a numbered file at the root of `src`
    let known = Pages::new(pages);
    let placing = chain_placing(nodes);
    // a page at the same path as a node's file would be written over
    for (head, name) in placing.heads.iter().zip(&placing.paths) {
        if known.paths.contains(name.as_str()) {
            anyhow::bail!(
                "page `{name}` has the name the file of node `{}` is written to: move the page",
                nodes[*head].spec.id
            );
        }
    }
    // the node files, then each page at its path, their links leading from where they are
    let node_files = render_nodes(nodes, &placing, &known, to_dir)?;
    let page_files = render_pages(pages, &known, to_dir)?;
    // and the summary listing each, the node files first
    let summary = fresh_summary(&placing, nodes, pages);
    let mut out: Vec<(String, String)> = node_files
        .into_iter()
        .chain(page_files)
        .map(|(path, text)| (format!("src/{path}"), text))
        .collect();
    out.push(("src/SUMMARY.md".to_string(), summary));
    Ok(out)
}

/// Places each node in a new book by the written order: a node with no body of its own,
/// whose group is on the current file's open chain, is listed under it, and any other starts
/// a file, numbered so the files sort in the written order
///
/// # Arguments
///
/// * `nodes` - The plan's nodes, in the order it was written in
fn chain_placing(nodes: &[StoredNode]) -> Placing {
    let mut members = vec![Vec::new(); nodes.len()];
    let mut file_of = Vec::with_capacity(nodes.len());
    // the nodes that start a file, and the chain of nodes open in the current one
    let mut heads: Vec<usize> = Vec::new();
    let mut chain: Vec<usize> = Vec::new();
    for (at, node) in nodes.iter().enumerate() {
        // a node with no body of its own is listed under its group, if its file holds it
        let under = match (&node.spec.group, node.body.is_empty()) {
            (Some(group), true) => chain
                .iter()
                .rposition(|open| nodes[*open].spec.id == *group),
            _ => None,
        };
        match under {
            Some(depth) => {
                // the chain is cut back to the group, which lists it after its members so far
                chain.truncate(depth + 1);
                members[chain[depth]].push(at);
            }
            None => {
                // else it starts a file of its own, keeping the group it names
                heads.push(at);
                chain.clear();
            }
        }
        // either way it is written in the current file
        file_of.push(heads.len() - 1);
        chain.push(at);
    }
    // each file numbered so the files sort in the order the plan was written in
    let width = heads.len().to_string().len().max(3);
    let paths = heads
        .iter()
        .enumerate()
        .map(|(i, head)| format!("{:0width$}-{}.md", i + 1, slug(&nodes[*head].spec.id)))
        .collect();
    Placing {
        heads,
        paths,
        members,
        file_of,
    }
}

/// Places each node in the book it is written back over: a node that heads a file of the
/// book keeps it, a node with no body of its own is listed under its group wherever that is
/// written, and any other starts a file beside its group's
///
/// # Arguments
///
/// * `nodes` - The plan's nodes, in the order it was written in
/// * `pages` - The plan's pages, whose paths a new file can't take
/// * `layout` - The book's layout
fn kept_placing(nodes: &[StoredNode], pages: &[StoredPage], layout: &Layout) -> Placing {
    // each node's place in the written order by its id, and each group's members in order
    let at_of: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(at, node)| (node.spec.id.as_str(), at))
        .collect();
    let mut children: HashMap<&str, Vec<usize>> = HashMap::new();
    for (at, node) in nodes.iter().enumerate() {
        if let Some(group) = &node.spec.group {
            children.entry(group.as_str()).or_default().push(at);
        }
    }
    // the names a new file can't take, as a file system that ignores case compares them
    let mut taken: HashSet<String> = layout
        .markdown
        .iter()
        .map(String::as_str)
        .chain(pages.iter().map(|page| page.path.as_str()))
        .chain(["SUMMARY.md"])
        .map(str::to_lowercase)
        .collect();
    // each node that heads a file of the book keeps it
    let mut heads = Vec::new();
    let mut head_paths = Vec::new();
    let mut file_of: Vec<Option<usize>> = vec![None; nodes.len()];
    for (at, node) in nodes.iter().enumerate() {
        if let Some(path) = layout.heads.get(&node.spec.id) {
            file_of[at] = Some(heads.len());
            heads.push(at);
            head_paths.push(path.clone());
        }
    }
    let mut members = vec![Vec::new(); nodes.len()];
    let mut queue: VecDeque<usize> = heads.iter().copied().collect();
    loop {
        // each node with no body of its own goes under its group, wherever that is written,
        // after the members before it in the written order
        while let Some(parent) = queue.pop_front() {
            for child in children
                .get(nodes[parent].spec.id.as_str())
                .into_iter()
                .flatten()
            {
                if file_of[*child].is_none() && nodes[*child].body.is_empty() {
                    file_of[*child] = file_of[parent];
                    members[parent].push(*child);
                    queue.push_back(*child);
                }
            }
        }
        // the first node left, in the written order, starts a file of its own
        let Some(at) = (0..nodes.len()).find(|at| file_of[*at].is_none()) else {
            break;
        };
        let group = nodes[at].spec.group.as_deref();
        // beside the last file before it of a node in its group
        let beside = heads
            .iter()
            .zip(&head_paths)
            .filter(|(head, _)| **head < at && nodes[**head].spec.group.as_deref() == group)
            .max_by_key(|(head, _)| **head)
            .map(|(_, path)| paths::parent(path).to_string());
        // else in its group's file's directory, else where the book's nodes are read from
        let in_group = || {
            group
                .and_then(|group| at_of.get(group))
                .and_then(|group| file_of[*group])
                .map(|file| paths::parent(&head_paths[file]).to_string())
        };
        let dir = beside
            .or_else(in_group)
            .or_else(|| layout.roots.first().cloned())
            .unwrap_or_default();
        let path = free_name(&dir, &nodes[at].spec.id, &taken);
        taken.insert(path.to_lowercase());
        file_of[at] = Some(heads.len());
        heads.push(at);
        head_paths.push(path);
        queue.push_back(at);
    }
    // the files listed in the order their nodes were written in
    let mut by_order: Vec<usize> = (0..heads.len()).collect();
    by_order.sort_by_key(|file| heads[*file]);
    let mut renumbered = vec![0; heads.len()];
    for (new, old) in by_order.iter().enumerate() {
        renumbered[*old] = new;
    }
    Placing {
        heads: by_order.iter().map(|file| heads[*file]).collect(),
        paths: by_order
            .iter()
            .map(|file| head_paths[*file].clone())
            .collect(),
        members,
        file_of: file_of
            .into_iter()
            .map(|file| renumbered[file.expect("every node is placed")])
            .collect(),
    }
}

/// A new node file's path in a directory, named for its node, that no file takes
///
/// # Arguments
///
/// * `dir` - The directory, from the book's `src`
/// * `id` - The node's id
/// * `taken` - The paths taken, lowercased
fn free_name(dir: &str, id: &str, taken: &HashSet<String>) -> String {
    // its id's slug, then numbered from 2 until a name is free
    let stem = slug(id);
    (1..)
        .map(|n| match n {
            1 => paths::join(dir, &format!("{stem}.md")),
            n => paths::join(dir, &format!("{stem}-{n}.md")),
        })
        .find(|path| !taken.contains(&path.to_lowercase()))
        .expect("some numbered name is free")
}

/// Each node file's path and text: its node with its members listed inline, each link leading
/// from where the file is
///
/// # Arguments
///
/// * `nodes` - The plan's nodes, in the order it was written in
/// * `placing` - Where each node is written
/// * `known` - The plan's pages
/// * `src_dir` - The book's `src`, from the repo's root
fn render_nodes(
    nodes: &[StoredNode],
    placing: &Placing,
    known: &Pages,
    src_dir: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    let mut placed: Vec<Placed> = Vec::with_capacity(nodes.len());
    for (at, node) in nodes.iter().enumerate() {
        // each link leads from the file the node is written in
        let file = &placing.paths[placing.file_of[at]];
        let map = |link: &str| known.from_node(file, src_dir, link);
        let mut spec = node.spec.clone();
        rewrite_spec(&mut spec, &map)?;
        let body = rewrite_links(&node.body, &map).map_err(|error| LinkError {
            at: format!("`{}` body", spec.id),
            ..error
        })?;
        // a member listed inline is given its group again by the import that reads it back
        if placing.heads[placing.file_of[at]] != at {
            spec.group = None;
        }
        placed.push(Placed {
            spec,
            body,
            members: placing.members[at].clone(),
        });
    }
    // each file its node's frontmatter, its members nested in it, then its body
    Ok(placing
        .heads
        .iter()
        .zip(&placing.paths)
        .map(|(head, path)| {
            let spec = nested(&placed, *head);
            let text = format!("---\n{}---\n{}", spec.to_yaml(), placed[*head].body);
            (path.clone(), text)
        })
        .collect())
}

/// Each page's path and text, in path order, its links to other pages as written and its
/// others leading from where it is written
///
/// # Arguments
///
/// * `pages` - The plan's pages
/// * `known` - The plan's pages, for telling a link to one
/// * `src_dir` - The book's `src`, from the repo's root
fn render_pages(
    pages: &[StoredPage],
    known: &Pages,
    src_dir: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    let mut sorted: Vec<&StoredPage> = pages.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut out = Vec::with_capacity(sorted.len());
    for page in sorted {
        // its links lead from where it is written, and the frontmatter holds none
        let to = paths::join(src_dir, paths::parent(&page.path));
        let map = |link: &str| known.lead(&page.path, &to, link);
        let at = frontmatter_is_yaml(&page.text).map_or(0, |fm| page.text.len() - fm.body.len());
        let (head, body) = page.text.split_at(at);
        let body = rewrite_links(body, &map).map_err(|error| LinkError {
            at: format!("page `{}`", page.path),
            ..error
        })?;
        out.push((page.path.clone(), format!("{head}{body}")));
    }
    Ok(out)
}

/// The title a node file is listed by: its node's title, else its id
///
/// # Arguments
///
/// * `spec` - The file's node
fn node_title(spec: &NodeSpec) -> &str {
    spec.title.as_deref().unwrap_or(&spec.id)
}

/// The title a page is listed by: its first heading, else its file's name
///
/// # Arguments
///
/// * `page` - The page
fn page_title(page: &StoredPage) -> String {
    let stem = page.path.rsplit('/').next().unwrap_or(&page.path);
    first_heading(doc_body(&page.text)).unwrap_or_else(|| stem.trim_end_matches(".md").to_string())
}

/// A summary's line listing a file by a title
///
/// # Arguments
///
/// * `indent` - What the line starts with before the link, such as `- `
/// * `title` - The title it is listed by
/// * `path` - The file, from the book's `src`
fn summary_line(indent: &str, title: &str, path: &str) -> String {
    format!("{indent}[{}]({path})\n", title.replace(']', "\\]"))
}

/// A new book's summary: each node file in the order the files are listed, then each page
/// under `# Pages`
///
/// # Arguments
///
/// * `placing` - Where each node is written
/// * `nodes` - The plan's nodes, in the order it was written in
/// * `pages` - The plan's pages
fn fresh_summary(placing: &Placing, nodes: &[StoredNode], pages: &[StoredPage]) -> String {
    let mut summary = String::from("# Summary\n\n");
    // each node file by its node's title
    for (head, path) in placing.heads.iter().zip(&placing.paths) {
        summary.push_str(&summary_line("- ", node_title(&nodes[*head].spec), path));
    }
    // then each page by its first heading, in path order
    let mut sorted: Vec<&StoredPage> = pages.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    if !sorted.is_empty() {
        summary.push_str("\n# Pages\n\n");
    }
    for page in sorted {
        summary.push_str(&summary_line("- ", &page_title(page), &page.path));
    }
    summary
}

/// The file a summary's line lists, from the book's `src`, if it lists one
///
/// # Arguments
///
/// * `line` - The line
fn listed(line: &str) -> Option<String> {
    // the first link's destination, as markdown reads it
    let found: RefCell<Option<String>> = RefCell::new(None);
    let note = |dest: &str| -> Option<String> {
        found.borrow_mut().get_or_insert_with(|| dest.to_string());
        None
    };
    rewrite_links(line.trim_start(), &note).ok()?;
    let dest = found.into_inner()?;
    // a draft chapter names no file
    if dest.trim().is_empty() {
        return None;
    }
    match paths::resolve("SUMMARY.md", &dest) {
        Target::Internal { path, .. } if !path.is_empty() => Some(path),
        _ => None,
    }
}

/// What a book's summary is to list once a plan is written over it
struct Listing<'a> {
    /// Each node file written, from the book's `src`, with the title it is listed by
    nodes: HashMap<&'a str, &'a str>,
    /// Each page written, from the book's `src`
    pages: HashSet<&'a str>,
    /// The files no longer written, whose lines are dropped
    removed: HashSet<&'a str>,
    /// The node files new to the book, in the order they are listed, with their titles
    new_nodes: Vec<(&'a str, &'a str)>,
    /// The pages new to the book, in path order, with their titles
    new_pages: Vec<(&'a str, String)>,
}

/// A book's summary, kept as it was but for its files: a line of a file gone dropped, a node
/// file's line retitled if it was listed by its node's old title, each new node file listed
/// after the last node file's line and each new page after the last page's, or under a
/// `# Pages` of its own
///
/// A summary that lists no node file lists no new one either.
///
/// # Arguments
///
/// * `old` - The book's summary
/// * `titles` - Each node file's title in the book, by its path
/// * `listing` - What it is to list
fn kept_summary(old: &str, titles: &HashMap<String, String>, listing: &Listing) -> String {
    let mut lines: Vec<String> = Vec::new();
    // where each kind's last line is, and what it starts with before its link
    let mut last_node: Option<(usize, String)> = None;
    let mut last_page: Option<(usize, String)> = None;
    for line in old.split_inclusive('\n') {
        let Some(path) = listed(line) else {
            lines.push(line.to_string());
            continue;
        };
        // a file gone takes its line with it
        if listing.removed.contains(path.as_str()) {
            continue;
        }
        let mut line = line.to_string();
        // a node file listed by its node's old title is listed by its new one
        if let (Some(was), Some(now)) = (titles.get(&path), listing.nodes.get(path.as_str()))
            && was != now
        {
            let from = format!("[{}](", was.replace(']', "\\]"));
            let to = format!("[{}](", now.replace(']', "\\]"));
            line = line.replacen(&from, &to, 1);
        }
        let indent = line[..line.find('[').unwrap_or(0)].to_string();
        lines.push(line);
        if listing.nodes.contains_key(path.as_str()) {
            last_node = Some((lines.len(), indent));
        } else if listing.pages.contains(path.as_str()) {
            last_page = Some((lines.len(), indent));
        }
    }
    // the new files' lines, each kind after its last line, the later place first
    let mut inserts: Vec<(usize, Vec<String>)> = Vec::new();
    if let Some((after, indent)) = &last_node
        && !listing.new_nodes.is_empty()
    {
        let new = listing.new_nodes.iter();
        let new = new.map(|(path, title)| summary_line(indent, title, path));
        inserts.push((*after, new.collect()));
    }
    let new_pages = listing.new_pages.iter();
    let page_lines = |indent: &str| -> Vec<String> {
        new_pages
            .clone()
            .map(|(path, title)| summary_line(indent, title, path))
            .collect()
    };
    match &last_page {
        Some((after, indent)) if !listing.new_pages.is_empty() => {
            inserts.push((*after, page_lines(indent)));
        }
        // a summary listing no page lists the new ones under a part of their own
        None if !listing.new_pages.is_empty() => {
            let mut part = vec!["\n# Pages\n\n".to_string()];
            part.extend(page_lines("- "));
            inserts.push((lines.len(), part));
        }
        _ => {}
    }
    inserts.sort_by_key(|(after, _)| std::cmp::Reverse(*after));
    for (after, new) in inserts {
        // the line before ends, so the new lines start lines of their own
        if after > 0 && !lines[after - 1].ends_with('\n') {
            lines[after - 1].push('\n');
        }
        lines.splice(after..after, new);
    }
    lines.concat()
}

/// The files a plan makes written back over a book, keeping the book's layout, and the book's
/// files it no longer has
///
/// # Arguments
///
/// * `nodes` - The plan's nodes, in the order it was written in
/// * `pages` - The plan's pages
/// * `layout` - The book's layout
/// * `src_dir` - The book's `src`, from the repo's root
fn kept_files(
    nodes: &[StoredNode],
    pages: &[StoredPage],
    layout: &Layout,
    src_dir: &str,
) -> anyhow::Result<Kept> {
    // a page at the path of a file a node of the plan keeps would be written over
    let ids: HashSet<&str> = nodes.iter().map(|node| node.spec.id.as_str()).collect();
    for page in pages {
        let kept = layout
            .heads
            .iter()
            .find(|(id, path)| **path == page.path && ids.contains(id.as_str()));
        if let Some((id, _)) = kept {
            anyhow::bail!(
                "page `{}` has the path of node `{id}`'s file in the book: move the page, or the \
                 node's file",
                page.path
            );
        }
    }
    // each node placed as the book has it, and each file's text
    let known = Pages::new(pages);
    let placing = kept_placing(nodes, pages, layout);
    let node_files = render_nodes(nodes, &placing, &known, src_dir)?;
    let page_files = render_pages(pages, &known, src_dir)?;
    // the files the book has that the plan no longer does, and those it didn't have
    let written: HashSet<&str> = node_files
        .iter()
        .chain(&page_files)
        .map(|(path, _)| path.as_str())
        .collect();
    let removed: Vec<String> = layout
        .markdown
        .iter()
        .filter(|path| !written.contains(path.as_str()))
        .cloned()
        .collect();
    let added = |files: &[(String, String)]| -> Vec<String> {
        files
            .iter()
            .map(|(path, _)| path.clone())
            .filter(|path| !layout.markdown.contains(path))
            .collect()
    };
    let (added_nodes, added_pages) = (added(&node_files), added(&page_files));
    // the summary kept, listing what is new, else written as a new book's
    let summary = match &layout.summary {
        Some(old) => {
            let titles: HashMap<&str, &str> = placing
                .heads
                .iter()
                .zip(&placing.paths)
                .map(|(head, path)| (path.as_str(), node_title(&nodes[*head].spec)))
                .collect();
            let new_nodes = added_nodes
                .iter()
                .map(|path| (path.as_str(), titles[path.as_str()]))
                .collect();
            let by_path: HashMap<&str, &StoredPage> = pages
                .iter()
                .map(|page| (page.path.as_str(), page))
                .collect();
            let new_pages = added_pages
                .iter()
                .map(|path| (path.as_str(), page_title(by_path[path.as_str()])))
                .collect();
            let listing = Listing {
                nodes: titles,
                pages: page_files.iter().map(|(path, _)| path.as_str()).collect(),
                removed: removed.iter().map(String::as_str).collect(),
                new_nodes,
                new_pages,
            };
            kept_summary(old, &layout.titles, &listing)
        }
        None => fresh_summary(&placing, nodes, pages),
    };
    // the node files and pages, then the summary, which then lists no file not yet written
    let mut files: Vec<(String, String)> = node_files.into_iter().chain(page_files).collect();
    files.push(("SUMMARY.md".to_string(), summary));
    let mut added: Vec<String> = added_nodes.into_iter().chain(added_pages).collect();
    if layout.summary.is_none() {
        added.push("SUMMARY.md".to_string());
    }
    added.sort();
    Ok(Kept {
        files,
        removed,
        added,
    })
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

/// Exports a plan to a book in a directory of its project's repo: a new book, or written over
/// the book there when asked
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which a relative path to the book starts from
/// * `dir` - The directory to write the book in: new or empty, or a reef book to write over
/// * `pick` - The project and plan named, if they were
/// * `replace` - Whether to write over the reef book in the directory, keeping its layout
/// * `out` - Where to print what was exported
pub async fn export<W: Write>(
    client: &Client,
    cwd: &Path,
    dir: &Path,
    pick: &PlanPick,
    replace: bool,
    out: &mut W,
) -> anyhow::Result<()> {
    // over the book in the directory, when asked
    let book = cwd.join(dir);
    if replace {
        return write_over(client, &book, dir, pick, out).await;
    }
    // else a directory that holds anything is left as it is
    let existed = book.exists();
    if existed && std::fs::read_dir(&book)?.next().is_some() {
        anyhow::bail!(
            "{} isn't empty: export writes a new book, or over a reef book with `--replace`",
            dir.display()
        );
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
    let what = nodes_and_pages(listing.nodes.len(), pages.len());
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

/// Writes a plan over the reef book in a directory, keeping the book's layout: each changed
/// file written whole, then each markdown file the plan no longer has removed
///
/// Everything is worked out before anything is written, so a plan that can't be written there
/// leaves the book as it was.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `book` - The book's directory
/// * `dir` - The directory as it was given, for messages
/// * `pick` - The project and plan named, if they were
/// * `out` - Where to print what was exported
async fn write_over<W: Write>(
    client: &Client,
    book: &Path,
    dir: &Path,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // a reef book, briefed from the sections a central plan is
    if !book.join("book.toml").is_file() {
        anyhow::bail!(
            "{} holds no `book.toml`: `--replace` writes over a reef book, and an export without \
             it writes a new one",
            dir.display()
        );
    }
    let opened = open_book(book)?;
    default_sections(&opened.config, dir)?;
    // in the project it is in, else the one named, whose links lead from its repo's root
    let project = picked(client, &opened.dir, &pick.project).await?;
    let root = link_root(client, &project, &opened.src).await?;
    let src_dir = prefix(&root, &opened.src)?;
    let plan = plan_in(client, project, pick.plan.as_deref()).await?;
    // the plan's files, placed as the book has them
    let listing = client.list(&plan).await?;
    let pages = client.pages(&plan).await?.pages;
    let layout = Layout::read(&opened.src, &opened.config.roots)?;
    let kept = kept_files(&listing.nodes, &pages, &layout, &src_dir)?;
    check_targets(&opened.src, &kept.files)?;
    // each file whose text changed written whole, the summary last
    let (mut written, mut unchanged) = (0, 0);
    for (rel, text) in &kept.files {
        let path = opened.src.join(rel);
        if std::fs::read(&path).is_ok_and(|was| was == text.as_bytes()) {
            unchanged += 1;
            continue;
        }
        write_whole(&path, text)?;
        written += 1;
    }
    // then each file the plan no longer has removed, and a directory it leaves empty
    for rel in &kept.removed {
        let path = opened.src.join(rel);
        std::fs::remove_file(&path)?;
        let mut parent = path.parent();
        while let Some(emptied) = parent
            && emptied != opened.src
            && emptied.starts_with(&opened.src)
            && std::fs::read_dir(emptied).is_ok_and(|mut entries| entries.next().is_none())
        {
            std::fs::remove_dir(emptied)?;
            parent = emptied.parent();
        }
    }
    // what was written, then each file added and removed, from the book's directory
    let src = prefix(&opened.dir, &opened.src)?;
    writeln!(
        out,
        "exported {} of `{plan}` over the book in {}: {} written, {unchanged} unchanged, {} removed",
        nodes_and_pages(listing.nodes.len(), pages.len()),
        dir.display(),
        counted(written, "file"),
        kept.removed.len()
    )?;
    for rel in &kept.added {
        writeln!(out, "  added {}", paths::join(&src, rel))?;
    }
    for rel in &kept.removed {
        writeln!(out, "  removed {}", paths::join(&src, rel))?;
    }
    Ok(())
}

/// Refuses files a book's `src` has no place for: a file where a directory is, or a directory
/// where a file is
///
/// # Arguments
///
/// * `src` - The book's `src`
/// * `files` - Each file to write, from `src`, with its text
fn check_targets(src: &Path, files: &[(String, String)]) -> anyhow::Result<()> {
    for (rel, _) in files {
        // a directory where the file goes
        if src.join(rel).is_dir() {
            anyhow::bail!(
                "{rel} is a directory in the book's src, where the export would write a file"
            );
        }
        // or a file where one of its directories goes
        let mut dir = paths::parent(rel);
        while !dir.is_empty() {
            if src.join(dir).is_file() {
                anyhow::bail!(
                    "{dir} is a file in the book's src, where the export would make a directory \
                     for {rel}"
                );
            }
            dir = paths::parent(dir);
        }
    }
    Ok(())
}

/// Writes a file whole: to a dotfile beside it, which no book reads, then renamed into place
///
/// # Arguments
///
/// * `path` - The file
/// * `text` - Its text
fn write_whole(path: &Path, text: &str) -> anyhow::Result<()> {
    // its directory made, as a new file's may need
    let parent = path.parent().expect("a book's file is in a directory");
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .expect("a book's file has a name")
        .to_string_lossy();
    let temp = parent.join(format!(".{name}.reef-export"));
    std::fs::write(&temp, text)?;
    // renamed over the file, or the dotfile taken away again
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(anyhow::anyhow!("writing {}: {error}", path.display()));
    }
    Ok(())
}

/// A count of nodes, and of pages if there are any
///
/// # Arguments
///
/// * `nodes` - How many nodes there are
/// * `pages` - How many pages there are
fn nodes_and_pages(nodes: usize, pages: usize) -> String {
    match pages {
        0 => counted(nodes, "node"),
        pages => format!("{} and {}", counted(nodes, "node"), counted(pages, "page")),
    }
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
    use super::{
        Layout, Listing, Pages, files, free_name, kept_files, kept_placing, kept_summary, slug,
    };
    use reef_api::{StoredNode, StoredPage};
    use reef_model::NodeSpec;
    use std::collections::{BTreeSet, HashMap, HashSet};

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
            .filter_map(|(path, text)| {
                // a page has no node
                let (spec, _) = reef_model::spec::parse_node_file(text).unwrap()?;
                let mut out = Vec::new();
                ids(&spec, &mut out);
                Some((path.clone(), out))
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

    /// A book's layout, as `Layout::read` would find it
    ///
    /// # Arguments
    ///
    /// * `heads` - Each node file's node and path, with its title
    /// * `others` - The book's other markdown files
    /// * `summary` - Its summary, if it has one
    fn layout(heads: &[(&str, &str, &str)], others: &[&str], summary: Option<&str>) -> Layout {
        Layout {
            heads: heads
                .iter()
                .map(|(id, path, _)| (id.to_string(), path.to_string()))
                .collect(),
            titles: heads
                .iter()
                .map(|(_, path, title)| (path.to_string(), title.to_string()))
                .collect(),
            markdown: heads
                .iter()
                .map(|(_, path, _)| *path)
                .chain(others.iter().copied())
                .map(String::from)
                .collect::<BTreeSet<String>>(),
            summary: summary.map(String::from),
            roots: Vec::new(),
        }
    }

    /// A node heading a file of the book keeps it, a member, new or not, goes under its group
    /// after the members before it, a node with a body starts a file beside its group's, a
    /// top-level one beside the top-level files, and its members go under it
    #[test]
    fn kept_layout_places_each_node() {
        let nodes = [
            stored("id: M1\ngroup: root\n", "# M1\n", 0),
            stored("id: a\ngroup: M1\n", "", 1),
            stored("id: M2\ngroup: root\n", "# M2\n", 2),
            stored("id: root\n", "# Root\n", 3),
            stored("id: x\ngroup: root\n", "", 4),
            // put since, after the plan's last
            stored("id: b\ngroup: M1\n", "", 5),
            stored("id: M3\ngroup: root\n", "# M3\n", 6),
            stored("id: c\ngroup: M3\n", "", 7),
            stored("id: T\n", "", 8),
        ];
        let book = layout(
            &[
                ("M1", "milestones/m1.md", "M1"),
                ("M2", "milestones/m2.md", "M2"),
                ("root", "overview.md", "root"),
                ("gone", "milestones/gone.md", "gone"),
            ],
            &["design/app.md"],
            None,
        );
        let placing = kept_placing(&nodes, &[page("t.md", "# T\n")], &book);
        let heads: Vec<(&str, &str)> = placing
            .heads
            .iter()
            .zip(&placing.paths)
            .map(|(head, path)| (nodes[*head].spec.id.as_str(), path.as_str()))
            .collect();
        assert_eq!(
            heads,
            [
                ("M1", "milestones/m1.md"),
                ("M2", "milestones/m2.md"),
                ("root", "overview.md"),
                ("M3", "milestones/m3.md"),
                // beside the file of the top-level root, and past the page at t.md
                ("T", "t-2.md"),
            ]
        );
        // a and b, new since, under M1 in the written order, x under root, c under M3
        assert_eq!(placing.members[0], [1, 5]);
        assert_eq!(placing.members[3], [4]);
        assert_eq!(placing.members[6], [7]);
        // and each written in its group's file
        let file_of = |at: usize| placing.paths[placing.file_of[at]].as_str();
        assert_eq!(file_of(5), "milestones/m1.md");
        assert_eq!(file_of(7), "milestones/m3.md");
    }

    /// A new file takes its node's slug, numbered past any name taken, ignoring case
    #[test]
    fn new_file_names_are_free() {
        let taken: HashSet<String> = ["plan/m3.md", "plan/m3-2.md", "summary.md"]
            .map(String::from)
            .into();
        assert_eq!(free_name("plan", "M3", &taken), "plan/m3-3.md");
        assert_eq!(free_name("", "Summary", &taken), "summary-2.md");
        assert_eq!(free_name("", "C70", &taken), "c70.md");
    }

    /// A node's links lead from its own file: to a page from the file's directory, to a heading
    /// of its own file by the heading alone, and to the repo's other files from where it is
    #[test]
    fn links_lead_from_each_files_directory() {
        let pages = [page("design/app.md", "# App\n")];
        let known = Pages::new(&pages);
        let lead = |file: &str, link: &str| known.from_node(file, "plan/src", link);
        // a page, as written from the root, else from the file's directory
        assert_eq!(lead("overview.md", "design/app.md#rows"), None);
        assert_eq!(
            lead("milestones/m6.md", "design/app.md#rows").as_deref(),
            Some("../design/app.md#rows")
        );
        assert_eq!(
            lead("milestones/m6.md", "design/app.html").as_deref(),
            Some("../design/app.html")
        );
        // a heading of its own file
        assert_eq!(
            lead("milestones/m6.md", "plan/src/milestones/m6.md#x").as_deref(),
            Some("#x")
        );
        // the book's other files, and the repo's
        assert_eq!(
            lead("milestones/m3.md", "plan/src/milestones/m4.md").as_deref(),
            Some("m4.md")
        );
        assert_eq!(
            lead("overview.md", "plan/src/milestones/m10.md").as_deref(),
            Some("milestones/m10.md")
        );
        assert_eq!(
            lead("milestones/m6.md", "README.md#setup").as_deref(),
            Some("../../../README.md#setup")
        );
        // and a URL left as written
        assert_eq!(lead("milestones/m6.md", "https://x.org"), None);
    }

    /// A kept summary keeps its every line and part, retitles a node file listed by its old
    /// title, drops a file gone, and lists new files after the last of their kind
    #[test]
    fn kept_summary_keeps_parts_and_places_new_lines() {
        let old = "# Summary\n\n[Intro](overview.md)\n\n# Milestones\n\n- [M1 Old](m/m1.md)\n  - [Gone](m/gone.md)\n- [M2](m/m2.md)\n\n# Design\n\n- [App](design/app.md)\n- [Draft]()\n";
        let titles: HashMap<String, String> = [
            ("overview.md", "Intro"),
            ("m/m1.md", "M1 Old"),
            ("m/gone.md", "Gone"),
            ("m/m2.md", "M2"),
        ]
        .map(|(path, title)| (path.to_string(), title.to_string()))
        .into();
        let listing = Listing {
            nodes: [
                ("overview.md", "Intro"),
                ("m/m1.md", "M1 New"),
                ("m/m2.md", "M2"),
                ("m/m3.md", "M3"),
            ]
            .into(),
            pages: ["design/app.md", "design/store.md"].into(),
            removed: ["m/gone.md"].into(),
            new_nodes: vec![("m/m3.md", "M3")],
            new_pages: vec![("design/store.md", "The store".to_string())],
        };
        assert_eq!(
            kept_summary(old, &titles, &listing),
            "# Summary\n\n[Intro](overview.md)\n\n# Milestones\n\n- [M1 New](m/m1.md)\n- [M2](m/m2.md)\n- [M3](m/m3.md)\n\n# Design\n\n- [App](design/app.md)\n- [The store](design/store.md)\n- [Draft]()\n"
        );
        // a summary that lists no page lists the new ones under a part of their own, after a
        // last line that didn't end
        let listing = Listing {
            pages: ["design/store.md"].into(),
            new_nodes: Vec::new(),
            ..listing
        };
        assert_eq!(
            kept_summary("# Summary\n\n- [M2](m/m2.md)", &titles, &listing),
            "# Summary\n\n- [M2](m/m2.md)\n\n# Pages\n\n- [The store](design/store.md)\n"
        );
    }

    /// A page at the path of a node's kept file is refused, and a file the plan no longer has
    /// is removed while a new one is added
    #[test]
    fn kept_files_refuse_a_page_over_a_node_file() {
        let nodes = [stored("id: G\n", "# G\n", 0)];
        let book = layout(&[("G", "g.md", "G")], &["old.md"], Some("- [G](g.md)\n"));
        let error = kept_files(&nodes, &[page("g.md", "# G?\n")], &book, "b/src")
            .err()
            .expect("the page is refused")
            .to_string();
        assert_eq!(
            error,
            "page `g.md` has the path of node `G`'s file in the book: move the page, or the node's \
             file"
        );
        let kept = kept_files(&nodes, &[page("new.md", "# New\n")], &book, "b/src").unwrap();
        assert_eq!(kept.removed, ["old.md"]);
        assert_eq!(kept.added, ["new.md"]);
        assert_eq!(
            kept.files.last().unwrap(),
            &(
                "SUMMARY.md".to_string(),
                "- [G](g.md)\n\n# Pages\n\n- [New](new.md)\n".to_string()
            )
        );
    }
}
