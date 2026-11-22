//! `reef import`: a book's plan read from its files and written into a central plan as one
//! revision
//!
//! ```text
//! reef import [BOOK] [--replace] [--project NAME] [--plan NAME]
//! ```
//!
//! - **The book is checked first.** Its node files are read and checked as `mdbook-reef check`
//!   checks them, and a book with any problem imports nothing, each problem said at its file and
//!   line.
//! - **Each node a row.** A group's inline members are lifted out of it, each a node of its own
//!   with its `group` filled in, and the plan is written in the order the book was: each file's
//!   node, then its members, depth first. A file's body goes on its own node, and a member's
//!   `body:` stays in its spec, where it was written.
//! - **The book's pages are the plan's.** Every markdown file in the book's `src` that isn't a
//!   node file is kept with the plan as a page, at its path from `src`: the book's
//!   `design/app.md` is the plan's page `design/app.md`, so the book can go once it is imported.
//! - **Links lead where they did.** A stored node's links are read from its repo's root, over
//!   which the plan's pages lie, so each link written in a node, in its doc links or in any of
//!   its markdown, is rewritten to lead to the same page, file and heading from there. A page
//!   keeps its links to other pages as written, since they keep their places, and its links to
//!   any other file lead to the repo's from where the page now is. A link to a repo file that a
//!   page at the same path would hide is refused.
//! - **A link into a node's file names the node.** A link a node's or a page's markdown writes
//!   into one of the book's node files becomes `node:<ID>`, naming the node that heads the file,
//!   with the heading it named, so it leads to the node once the book is gone, and an export
//!   writes it back to wherever the node's file is. A doc link reads a file's section, so it
//!   leads to the repo's file as any other does.
//! - **A plan with nodes or pages is replaced only when asked.** The import is a whole-plan
//!   write, which leaves out every node and page the book doesn't have, so a plan that has any
//!   takes `--replace`.
//!
//! A book that names its own doc sections is refused, since a central plan takes its briefs from
//! the default sections.

use reef_api::{
    NodeWrite, PLAN_PAGES_MAX_BYTES, PageWrite, PlanWrite, Project, Rewrite, check_page_path, lift,
};
use reef_model::PlanName;
use reef_model::spec::{frontmatter_is_yaml, parse_node_file};
use reef_render::book::{BookNode, Config, Plan, markdown_files, read_nodes};
use reef_render::links::{LinkError, rewrite_links, rewrite_spec};
use reef_render::paths::Target;
use reef_render::{Sections, paths};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cli::{PlanPick, picked, plan_in};
use crate::client::Client;

/// A book on disk: where it is, and its `[preprocessor.reef]`
#[derive(Debug, Clone)]
pub struct Book {
    /// The book's directory, which holds its `book.toml`
    pub dir: PathBuf,
    /// The directory its chapters and nodes are written in
    pub src: PathBuf,
    /// Its `[preprocessor.reef]`
    pub config: Config,
}

/// Opens the book in a directory, reading where its `src` is and its `[preprocessor.reef]`
///
/// # Arguments
///
/// * `dir` - The book's directory, which holds its `book.toml`
pub fn open_book(dir: &Path) -> anyhow::Result<Book> {
    // the book's config, which a reef book's has a `[preprocessor.reef]` in
    let book_toml = dir.join("book.toml");
    let text = std::fs::read_to_string(&book_toml)
        .map_err(|error| anyhow::anyhow!("reading {}: {error}", book_toml.display()))?;
    let table: toml::Table = toml::from_str(&text)
        .map_err(|error| anyhow::anyhow!("reading {}: {error}", book_toml.display()))?;
    let Some(reef) = table
        .get("preprocessor")
        .and_then(|preprocessors| preprocessors.get("reef"))
    else {
        anyhow::bail!(
            "{} has no `[preprocessor.reef]`, so it holds no reef plan",
            book_toml.display()
        );
    };
    let config: Config = reef.clone().try_into().map_err(|error| {
        anyhow::anyhow!(
            "reading {}'s [preprocessor.reef]: {error}",
            book_toml.display()
        )
    })?;
    // its `src`, which mdbook takes to be `src` unless `[book]` says
    let src = table
        .get("book")
        .and_then(|book| book.get("src"))
        .and_then(|src| src.as_str())
        .unwrap_or("src");
    // both found as they are on disk, so a path within the repo can be taken from them
    let dir = dir
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("finding {}: {error}", dir.display()))?;
    let src = dir
        .join(src)
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("finding the book's src `{src}`: {error}"))?;
    Ok(Book { dir, src, config })
}

/// Refuses a book that names its own doc sections, since a central plan briefs its nodes from
/// the default sections, which would not be the book's
///
/// # Arguments
///
/// * `config` - The book's `[preprocessor.reef]`
/// * `book` - The book's directory as it was given, for the message
pub(crate) fn default_sections(config: &Config, book: &Path) -> anyhow::Result<()> {
    if config.sections != Sections::default() {
        anyhow::bail!(
            "{} names its own doc sections in `[preprocessor.reef]`, which a central plan doesn't \
             keep, so its nodes would be briefed from other sections than the book's",
            book.display()
        );
    }
    Ok(())
}

/// The root a project's stored links lead from, for a directory in its repo: the repo's own
/// root, or the root of the linked worktree the directory is in
///
/// # Arguments
///
/// * `client` - The service's client
/// * `project` - The project the directory is to be in
/// * `dir` - The directory, canonical
pub(crate) async fn link_root(
    client: &Client,
    project: &Project,
    dir: &Path,
) -> anyhow::Result<PathBuf> {
    // the project's repo as it is on disk, as the directory was found
    let repo = project
        .repo
        .canonicalize()
        .unwrap_or_else(|_| project.repo.clone());
    // the working tree that holds the directory, if it is in git at all
    let tree = dir.ancestors().find(|dir| dir.join(".git").exists());
    match tree {
        // in the repo's own working tree, or in a project outside git
        Some(tree) if tree == repo => Ok(repo),
        None if dir.starts_with(&repo) => Ok(repo),
        // in a linked worktree, which must be the project's
        Some(tree) => {
            let found = client.find_project(tree).await?;
            if found.is_some_and(|found| found.name == project.name) {
                return Ok(tree.to_path_buf());
            }
            anyhow::bail!(
                "{} is not in project `{}`'s repo, {}",
                dir.display(),
                project.name,
                project.repo.display()
            )
        }
        None => anyhow::bail!(
            "{} is not in project `{}`'s repo, {}",
            dir.display(),
            project.name,
            project.repo.display()
        ),
    }
}

/// A directory's path from a root that holds it, with `/` separators
///
/// # Arguments
///
/// * `root` - The root, canonical
/// * `dir` - The directory, canonical
pub(crate) fn prefix(root: &Path, dir: &Path) -> anyhow::Result<String> {
    // the directory must lie under the root its links lead from
    let rel = dir
        .strip_prefix(root)
        .map_err(|_| anyhow::anyhow!("{} is not under {}", dir.display(), root.display()))?;
    // as a link writes it, whatever the platform's separator
    let parts: Vec<String> = rel
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .collect();
    Ok(parts.join("/"))
}

/// Where a book's links lead once it is imported: to the plan's pages, which were the book's,
/// to the nodes its node files hold, and to the repo's other files, which the pages lie over
#[derive(Debug, Clone)]
pub struct BookLinks {
    /// The book's `src`, from the repo's root
    prefix: String,
    /// The book's pages, by their paths from its `src`, which are their paths in the plan
    pages: BTreeSet<String>,
    /// The id of the node heading each of the book's node files, by the file's path from its
    /// `src`
    nodes: HashMap<String, String>,
}

impl BookLinks {
    /// The links of a book whose `src` lies at a path in its repo, and which holds these pages
    ///
    /// # Arguments
    ///
    /// * `prefix` - The book's `src`, from the repo's root
    /// * `pages` - The book's pages, by their paths from its `src`
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::import::BookLinks;
    ///
    /// let links = BookLinks::new("plan/src", ["design/app.md".to_string()]);
    /// // a node's link to a page leads to the plan's page, and any other to the repo's file
    /// assert_eq!(links.lead("milestones/m6.md", false, "../design/app.md#x"), Ok(Some("design/app.md#x".into())));
    /// assert_eq!(links.lead("milestones/m6.md", false, "../../../docs/x.md"), Ok(Some("docs/x.md".into())));
    /// // a page's link to another page is left as written
    /// assert_eq!(links.lead("design/app.md", true, "app.md#rows"), Ok(None));
    /// ```
    pub fn new<P: Into<String>, I: IntoIterator<Item = String>>(prefix: P, pages: I) -> Self {
        BookLinks {
            prefix: prefix.into(),
            pages: pages.into_iter().collect(),
            nodes: HashMap::new(),
        }
    }

    /// These links, with a link into each of the book's node files naming the node heading it
    ///
    /// # Arguments
    ///
    /// * `nodes` - Each node file's path from the book's `src`, and its node's id
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::import::BookLinks;
    ///
    /// let nodes = [("milestones/m6.md".to_string(), "M6".to_string())];
    /// let links = BookLinks::new("plan/src", ["design/app.md".to_string()]).with_nodes(nodes);
    /// // a page's link into the node's file names the node, and a node's own heading too
    /// assert_eq!(links.lead("design/app.md", true, "../milestones/m6.md"), Ok(Some("node:M6".into())));
    /// assert_eq!(links.lead("milestones/m6.md", false, "#rows"), Ok(Some("node:M6#rows".into())));
    /// // and a node's doc link leads to the file, whose section it reads
    /// assert_eq!(links.lead_doc("milestones/m5.md", "m6.md"), Ok(Some("plan/src/milestones/m6.md".into())));
    /// ```
    pub fn with_nodes<I: IntoIterator<Item = (String, String)>>(mut self, nodes: I) -> Self {
        self.nodes.extend(nodes);
        self
    }

    /// The book's pages, by their paths from its `src`
    pub fn pages(&self) -> impl Iterator<Item = &str> {
        self.pages.iter().map(String::as_str)
    }

    /// Whether a path in the plan names a page: the page's markdown, or the page it is served as
    ///
    /// # Arguments
    ///
    /// * `path` - The path, from the plan's root
    fn names_page(&self, path: &str) -> bool {
        self.pages.contains(path)
            || (path.ends_with(".html")
                && paths::md_of(path).iter().any(|md| self.pages.contains(md)))
    }

    /// The id of the node whose file a path from the repo's root names, as its markdown or as
    /// the page it is served as, if it names one of the book's node files
    ///
    /// # Arguments
    ///
    /// * `path` - The path, from the repo's root
    fn node_of(&self, path: &str) -> Option<&str> {
        // only a path in the book's `src` can be one of its node files
        let rel = match self.prefix.is_empty() {
            true => path,
            false => path.strip_prefix(&format!("{}/", self.prefix))?,
        };
        // the file's markdown, or the page mdbook writes for it
        let found = |md: &str| self.nodes.get(md).map(String::as_str);
        found(rel).or_else(|| {
            rel.ends_with(".html")
                .then(|| paths::md_of(rel).iter().find_map(|md| found(md)))
                .flatten()
        })
    }

    /// The path a path from the repo's root takes in the plan, if it names one of the book's
    /// pages
    ///
    /// # Arguments
    ///
    /// * `path` - The path, from the repo's root
    fn page_of(&self, path: &str) -> Option<String> {
        // only a path in the book's `src` can be one of its pages
        let rel = match self.prefix.is_empty() {
            true => path,
            false => path.strip_prefix(&format!("{}/", self.prefix))?,
        };
        self.names_page(rel).then(|| rel.to_string())
    }

    /// A link written in the markdown of one of the book's files, as it leads from where the
    /// file is kept once imported, or `None` if it is left as written
    ///
    /// A node's links lead from the repo's root. A page's links lead from its own path, so its
    /// links to other pages are left as written. A link into a node's file names the node, with
    /// the heading it named. A link that leads to a repo file a page at the same path would hide
    /// is refused, saying why.
    ///
    /// # Arguments
    ///
    /// * `rel` - The file's path from the book's `src`
    /// * `page` - Whether the file is a page, rather than a node's
    /// * `link` - The link as written
    pub fn lead(&self, rel: &str, page: bool, link: &str) -> Result<Option<String>, String> {
        self.lead_to(rel, page, link, true)
    }

    /// A doc link written in a node of one of the book's node files, as it leads from the
    /// repo's root once imported, or `None` if it is left as written
    ///
    /// A doc link reads a section of the file it names, so one into a node's file leads to
    /// that file, as a link to any other file does.
    ///
    /// # Arguments
    ///
    /// * `rel` - The node file's path from the book's `src`
    /// * `link` - The doc link as written
    pub fn lead_doc(&self, rel: &str, link: &str) -> Result<Option<String>, String> {
        self.lead_to(rel, false, link, false)
    }

    /// A link written in one of the book's files, as it leads from where the file is kept once
    /// imported, or `None` if it is left as written
    ///
    /// # Arguments
    ///
    /// * `rel` - The file's path from the book's `src`
    /// * `page` - Whether the file is a page, rather than a node's
    /// * `link` - The link as written
    /// * `to_nodes` - Whether a link into a node's file names the node
    fn lead_to(
        &self,
        rel: &str,
        page: bool,
        link: &str,
        to_nodes: bool,
    ) -> Result<Option<String>, String> {
        // where the link leads in the repo, as the book reads it
        let from = paths::join(&self.prefix, rel);
        let Target::Internal { path, anchor } = paths::resolve(&from, link) else {
            return Ok(None);
        };
        // a link from the root to the root itself names no file
        if path.is_empty() {
            return Ok(None);
        }
        // a link to one of the book's pages leads to the plan's page at the same path
        if let Some(target) = self.page_of(&path) {
            // which a page reaches as it did, the pages keeping their places
            if page {
                return Ok(None);
            }
            return Ok(Some(match anchor {
                Some(anchor) => format!("{target}#{anchor}"),
                None => target,
            }));
        }
        // a link into a node's file names its node, wherever the file goes, unless its id can't
        // be named so
        if to_nodes
            && let Some(id) = self.node_of(&path)
            && let Some(named) = paths::node_href(id, anchor.as_deref())
        {
            return Ok(Some(named));
        }
        // any other file is the repo's, which a page at its path would hide
        if self.names_page(&path) {
            return Err(format!(
                "the link `{link}` leads to the repo's `{path}`, which the book's page at that \
                 path would hide once imported"
            ));
        }
        // so it leads there from the file's new home: the root for a node, the page's own place
        let home = match page {
            true => paths::parent(rel),
            false => "",
        };
        Ok(paths::moved(&from, home, link))
    }

    /// The function a file's links are rewritten with, keeping each refusal it meets
    ///
    /// # Arguments
    ///
    /// * `rel` - The file's path from the book's `src`
    /// * `page` - Whether the file is a page, rather than a node's
    /// * `to_nodes` - Whether a link into a node's file names the node, as a markdown link does
    /// * `refused` - Where each link refused is kept, with why
    fn map<'a>(
        &'a self,
        rel: &'a str,
        page: bool,
        to_nodes: bool,
        refused: &'a RefCell<Vec<String>>,
    ) -> impl Fn(&str) -> Option<String> + 'a {
        move |link| match self.lead_to(rel, page, link, to_nodes) {
            Ok(led) => led,
            // a refused link is left as written, and said once the file is rewritten
            Err(why) => {
                refused.borrow_mut().push(why);
                None
            }
        }
    }
}

/// The paths of a book's pages: every markdown file in its `src` that isn't a node file, from
/// its `src`
///
/// A page a link can't name, or the plan can't keep, is refused at its file.
///
/// # Arguments
///
/// * `src` - The book's `src`
/// * `nodes` - The book's node files
pub fn pages_of(src: &Path, nodes: &[BookNode]) -> anyhow::Result<Vec<String>> {
    // every markdown file but the summary, wherever it is, less those that are nodes
    let nodes: BTreeSet<&str> = nodes.iter().map(|node| node.rel.as_str()).collect();
    let pages: Vec<String> = markdown_files(src, &[])
        .into_iter()
        .filter(|rel| !nodes.contains(rel.as_str()))
        .collect();
    // each at a path a page can have
    for rel in &pages {
        check_page_path(rel).map_err(|why| anyhow::anyhow!("{rel}: {why}"))?;
    }
    Ok(pages)
}

/// The rows a book's node files make: each file's node, then each of its members lifted out
/// of it, numbered in the order the book was written in, their links leading from the repo's
/// root
///
/// # Arguments
///
/// * `files` - The book's node files, in the order they sort in
/// * `links` - Where the book's links lead once it is imported
/// * `plan` - The plan they are to be written to, whose names each id must make
///
/// # Examples
///
/// ```
/// use reef::import::{BookLinks, writes};
/// use reef_model::PlanName;
/// use reef_render::book::read_nodes;
///
/// # fn main() -> anyhow::Result<()> {
/// let src = std::env::temp_dir().join(format!("reef-writes-doc-{}", std::process::id()));
/// std::fs::create_dir_all(src.join("plan"))?;
/// std::fs::write(src.join("plan/g.md"), "---\nid: g\ndocs: [../d.md]\nnodes: [{ id: m }]\n---\nThe group.\n")?;
/// let (files, _) = read_nodes(&src, &[]);
/// let links = BookLinks::new("book/src", []);
/// let rows = writes(files, &links, &PlanName::new("reef", "central")?)?;
/// assert_eq!(rows[0].spec.docs, ["book/src/d.md"]);
/// assert_eq!((rows[1].spec.id.as_str(), rows[1].order), ("m", 1));
/// # std::fs::remove_dir_all(&src)?;
/// # Ok(())
/// # }
/// ```
pub fn writes(
    files: Vec<BookNode>,
    links: &BookLinks,
    plan: &PlanName,
) -> anyhow::Result<Vec<NodeWrite>> {
    let mut rows = Vec::new();
    for file in files {
        // each link in the file leads where it did, from the repo's root
        let rel = &file.rel;
        let refused = RefCell::new(Vec::new());
        let map = links.map(rel, false, true, &refused);
        let doc_map = links.map(rel, false, false, &refused);
        let mut body = rewrite_links(&file.body, &map).map_err(|error| {
            let at = format!("`{}` body", file.spec.id);
            anyhow::anyhow!("{rel}: {}", LinkError { at, ..error })
        })?;
        // the file's node, then each member lifted out of it, depth first
        let specs = lift(file.spec);
        let mut lifted = Vec::with_capacity(specs.len());
        for mut spec in specs {
            // its doc links lead to the files whose sections they read, and its markdown's to
            // the nodes of any node files they lead into
            let docs = std::mem::take(&mut spec.docs);
            rewrite_spec(&mut spec, &map).map_err(|error| anyhow::anyhow!("{rel}: {error}"))?;
            spec.docs = docs
                .into_iter()
                .map(|link| doc_map(&link).unwrap_or(link))
                .collect();
            lifted.push(spec);
        }
        // a link a page would hide refuses the file
        if let Some(why) = refused.borrow().first() {
            anyhow::bail!("{rel}: {why}");
        }
        for spec in lifted {
            // an id must name a node in the plan, which a book's ids needn't
            plan.node(spec.id.as_str()).map_err(|error| {
                anyhow::anyhow!("{rel}: `{}` can't be a node of `{plan}`: {error}", spec.id)
            })?;
            // the file's body is its own node's, and a member's is in its spec if it has one
            let order = rows.len() as u64;
            rows.push(NodeWrite {
                spec,
                body: std::mem::take(&mut body),
                order,
            });
        }
    }
    Ok(rows)
}

/// The pages a book's other markdown files make, each at its path from the book's `src`, its
/// links leading where they did from there
///
/// Only a page's markdown after its frontmatter holds links, so the frontmatter is kept as
/// written. A page whose frontmatter makes it a node is refused, and so are pages holding more
/// than a plan keeps.
///
/// # Arguments
///
/// * `src` - The book's `src`
/// * `links` - Where the book's links lead once it is imported, which names its pages
pub fn page_writes(src: &Path, links: &BookLinks) -> anyhow::Result<Vec<PageWrite>> {
    let mut pages = Vec::new();
    let mut bytes = 0;
    for rel in links.pages() {
        let text = std::fs::read_to_string(src.join(rel))
            .map_err(|error| anyhow::anyhow!("reading {rel}: {error}"))?;
        // a page exported again must import as a page, not a node
        if !matches!(parse_node_file(&text), Ok(None)) {
            anyhow::bail!(
                "{rel} has a node's frontmatter, an `id`, but is outside the book's `roots`, so \
                 it would be kept as a page that a book reads as a node"
            );
        }
        // the markdown after the frontmatter, whose links lead where they did
        let at = frontmatter_is_yaml(&text).map_or(0, |fm| text.len() - fm.body.len());
        let (head, body) = text.split_at(at);
        let refused = RefCell::new(Vec::new());
        let body = rewrite_links(body, &links.map(rel, true, true, &refused)).map_err(|error| {
            let at = "body".to_string();
            anyhow::anyhow!("{rel}: {}", LinkError { at, ..error })
        })?;
        if let Some(why) = refused.borrow().first() {
            anyhow::bail!("{rel}: {why}");
        }
        bytes += rel.len() + head.len() + body.len();
        pages.push(PageWrite::new(rel, format!("{head}{body}")));
    }
    // all of them must fit in what a plan keeps
    if bytes > PLAN_PAGES_MAX_BYTES {
        anyhow::bail!(
            "the book's pages hold {bytes} bytes, more than the {PLAN_PAGES_MAX_BYTES} a plan keeps"
        );
    }
    Ok(pages)
}

/// Imports a book's plan into a plan of the project its directory is in, as one revision
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which a relative path to the book starts from
/// * `book` - The book's directory, which holds its `book.toml`
/// * `pick` - The project and plan named, if they were
/// * `replace` - Whether to replace the plan's nodes if it has any
/// * `out` - Where to print what was imported
pub async fn import<W: Write>(
    client: &Client,
    cwd: &Path,
    book: &Path,
    pick: &PlanPick,
    replace: bool,
    out: &mut W,
) -> anyhow::Result<()> {
    // the book, which must take its briefs from the sections a central plan does
    let opened = open_book(&cwd.join(book))?;
    default_sections(&opened.config, book)?;
    // and whose nodes are checked as `mdbook-reef check` checks them, before anything is sent
    let checked = Plan::load(&opened.src, &opened.config);
    if !checked.diags.is_empty() {
        anyhow::bail!(
            "{} has {}, so nothing was imported:\n{}",
            book.display(),
            counted(checked.diags.len(), "problem"),
            checked.errors()
        );
    }
    // the project the book is in, else the one named, and the root its links will lead from
    let project = picked(client, &opened.dir, &pick.project).await?;
    let root = link_root(client, &project, &opened.src).await?;
    let prefix = prefix(&root, &opened.src)?;
    // the plan named, else the project's only plan
    let named = pick.plan.is_some();
    let plan = plan_in(client, project, pick.plan.as_deref()).await?;
    // the rows the book makes, each id one the plan can name, and the pages its other files do
    let (files, _) = read_nodes(&opened.src, &opened.config.roots);
    let heads: Vec<(String, String)> = files
        .iter()
        .map(|file| (file.rel.clone(), file.spec.id.clone()))
        .collect();
    let links = BookLinks::new(prefix, pages_of(&opened.src, &files)?).with_nodes(heads);
    let rows = writes(files, &links, &plan)?;
    let pages = page_writes(&opened.src, &links)?;
    // a plan named is listed if it isn't
    let added = named && client.add_plan(&plan).await?.added;
    // a plan that has nodes or pages loses those the book doesn't have, so is replaced only
    // when asked
    let had = (
        client.list(&plan).await?.nodes.len(),
        client.pages(&plan).await?.pages.len(),
    );
    if had != (0, 0) && !replace {
        anyhow::bail!(
            "`{plan}` has {}, which an import replaces: pass `--replace` to replace them",
            nodes_and_pages(had.0, had.1)
        );
    }
    // the whole plan, written as one revision
    let count = nodes_and_pages(rows.len(), pages.len());
    let write = PlanWrite {
        nodes: rows,
        pages: Some(pages),
    };
    match client.write_plan(&plan, &write).await? {
        Rewrite::Applied(done) => {
            writeln!(
                out,
                "imported {count} from {} into `{plan}` at revision {}",
                book.display(),
                done.revision
            )?;
            Ok(())
        }
        // check's own messages, one a line
        Rewrite::Refused(problems) => {
            let lines: Vec<String> = problems
                .iter()
                .map(|problem| format!("  {problem}"))
                .collect();
            // a plan listed for the import is left, with nothing in it
            let left = if added {
                format!("\n`{plan}` was listed for the import, and is left empty")
            } else {
                String::new()
            };
            anyhow::bail!(
                "the import was refused, since it would add these problems to `{plan}`:\n{}{left}",
                lines.join("\n")
            )
        }
    }
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
    use super::{BookLinks, open_book, page_writes, pages_of, prefix, writes};
    use reef_model::PlanName;
    use reef_render::book::read_nodes;
    use std::path::{Path, PathBuf};

    /// A fresh directory for a case, emptied of anything a run before left
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name
    fn scratch(case: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reef-import-{case}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A book's `src` is `src` unless `[book]` says, and a book without `[preprocessor.reef]`
    /// holds no plan
    #[test]
    fn books_opened_by_their_config() {
        let dir = scratch("open");
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        // a src of its own, and roots to look for nodes in
        std::fs::write(
            dir.join("book.toml"),
            "[book]\nsrc = \"pages\"\n\n[preprocessor.reef]\ncommand = \"mdbook-reef\"\nroots = [\"plan\"]\n",
        )
        .unwrap();
        let book = open_book(&dir).unwrap();
        assert!(book.src.ends_with("pages"), "{}", book.src.display());
        assert_eq!(book.config.roots, ["plan"]);
        // no reef preprocessor is no reef plan
        std::fs::write(dir.join("book.toml"), "[book]\ntitle = \"x\"\n").unwrap();
        let error = open_book(&dir).unwrap_err().to_string();
        assert!(error.contains("no `[preprocessor.reef]`"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A directory's prefix from its root, and none for one outside it
    #[test]
    fn prefixes_from_the_root() {
        let root = Path::new("/repo");
        assert_eq!(
            prefix(root, Path::new("/repo/example-book/src")).unwrap(),
            "example-book/src"
        );
        assert_eq!(prefix(root, root).unwrap(), "");
        assert!(prefix(root, Path::new("/elsewhere/src")).is_err());
    }

    /// Each file's node then its members, numbered across the book, the body on the file's node,
    /// a member's in its spec, and every link from the repo's root
    #[test]
    fn rows_in_written_order_with_their_links_moved() {
        let src = scratch("rows");
        std::fs::create_dir_all(src.join("plan")).unwrap();
        std::fs::write(
            src.join("plan/a.md"),
            "---\nid: a\ngroup: g\ndescription: { why: \"See [x](#x).\", what: W }\n---\n# A\n\n## X\n\n[g](g.md)\n",
        )
        .unwrap();
        std::fs::write(
            src.join("plan/g.md"),
            "---\nid: g\nnodes:\n  - id: m\n    body: \"# M\\n\\nSee [a](a.md#x).\\n\"\n    nodes: [{ id: n }]\n---\n",
        )
        .unwrap();
        let (files, diags) = read_nodes(&src, &[]);
        assert!(diags.is_empty(), "{diags:?}");
        let plan = PlanName::new("reef", "central").unwrap();
        let rows = writes(files, &BookLinks::new("b/src", []), &plan).unwrap();
        // in the order the files sort in, each file's members after it, depth first
        let order: Vec<(&str, u64)> = rows
            .iter()
            .map(|row| (row.spec.id.as_str(), row.order))
            .collect();
        assert_eq!(order, [("a", 0), ("g", 1), ("m", 2), ("n", 3)]);
        // the file's body on its node, its links from the root
        assert_eq!(rows[0].body, "# A\n\n## X\n\n[g](b/src/plan/g.md)\n");
        assert_eq!(
            rows[0].spec.description.as_ref().unwrap().why.as_deref(),
            Some("See [x](b/src/plan/a.md#x).")
        );
        // a member's body stays in its spec, and its row's is empty
        assert_eq!(
            rows[2].spec.body.as_deref(),
            Some("# M\n\nSee [a](b/src/plan/a.md#x).\n")
        );
        assert_eq!(rows[2].body, "");
        // each member names its group, and a file node keeps the one it wrote
        let groups: Vec<Option<&str>> = rows.iter().map(|row| row.spec.group.as_deref()).collect();
        assert_eq!(groups, [Some("g"), None, Some("g"), Some("m")]);
        let _ = std::fs::remove_dir_all(&src);
    }

    /// An id a plan can't name is refused at its file
    #[test]
    fn ids_a_plan_cant_name_refused() {
        let src = scratch("ids");
        std::fs::write(src.join("a.md"), "---\nid: \"a/b\"\n---\n").unwrap();
        let (files, _) = read_nodes(&src, &[]);
        let plan = PlanName::new("reef", "central").unwrap();
        let error = writes(files, &BookLinks::new("", []), &plan)
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("a.md: `a/b` can't be a node of `reef/central`"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&src);
    }

    /// A node's link to a page leads to the plan's page, a page's to another is left as
    /// written, a link into a node's file names the node, a doc link leads to the file, and
    /// every other link leads to the repo's file from where the file is kept
    #[test]
    fn page_links_lead_where_they_did() {
        let src = scratch("page-links");
        std::fs::create_dir_all(src.join("design")).unwrap();
        std::fs::create_dir_all(src.join("plan")).unwrap();
        // a node linking a page, a page linking a page, a node and the repo outside the book
        std::fs::write(
            src.join("plan/a.md"),
            "---\nid: a\ndocs: [../design/app.md#rows]\n---\n[app](../design/app.html) [out](../../../docs/x.md)\n",
        )
        .unwrap();
        std::fs::write(
            src.join("design/app.md"),
            "---\ntitle: kept\n---\n# App\n\n[rows](#rows) [plan](../plan.md) [a](../plan/a.md) [out](../../../docs/x.md#y)\n",
        )
        .unwrap();
        std::fs::write(src.join("plan.md"), "# Plan\n\n[app](./design/app.md)\n").unwrap();
        // and a node whose doc link and body lead into another node's file, and its own
        std::fs::write(
            src.join("plan/c.md"),
            "---\nid: c\ndocs: [a.md]\ndescription: { why: \"[own](#h)\" }\n---\n[a](a.md#h) [page](../plan/a.html) [self](c.md)\n",
        )
        .unwrap();
        let (files, diags) = read_nodes(&src, &[]);
        assert!(diags.is_empty(), "{diags:?}");
        let pages = pages_of(&src, &files).unwrap();
        assert_eq!(pages, ["design/app.md", "plan.md"]);
        let heads: Vec<(String, String)> = files
            .iter()
            .map(|file| (file.rel.clone(), file.spec.id.clone()))
            .collect();
        let links = BookLinks::new("book/src", pages).with_nodes(heads);
        let plan = PlanName::new("reef", "central").unwrap();
        // the node's links to the page lead to the plan's page, its others to the repo's files
        let rows = writes(files, &links, &plan).unwrap();
        assert_eq!(rows[0].spec.docs, ["design/app.md#rows"]);
        assert_eq!(rows[0].body, "[app](design/app.html) [out](docs/x.md)\n");
        // the doc link leads to the node's file, and every markdown link into one names its node
        assert_eq!(rows[1].spec.docs, ["book/src/plan/a.md"]);
        assert_eq!(
            rows[1].body,
            "[a](node:a#h) [page](node:a) [self](node:c)\n"
        );
        assert_eq!(
            rows[1]
                .spec
                .description
                .as_ref()
                .and_then(|d| d.why.as_deref()),
            Some("[own](node:c#h)")
        );
        // the pages keep their links to pages, and lead to the rest from where they now are
        let written = page_writes(&src, &links).unwrap();
        assert_eq!(written[0].path, "design/app.md");
        assert_eq!(
            written[0].text,
            "---\ntitle: kept\n---\n# App\n\n[rows](#rows) [plan](../plan.md) [a](node:a) [out](../docs/x.md#y)\n"
        );
        assert_eq!(written[1].text, "# Plan\n\n[app](./design/app.md)\n");
        // a link to a repo file a page would hide is refused, naming the file and the link
        let shadow = BookLinks::new("book/src", ["README.md".to_string()]);
        assert_eq!(
            shadow
                .lead("plan/a.md", false, "../../../README.md")
                .unwrap_err(),
            "the link `../../../README.md` leads to the repo's `README.md`, which the book's \
             page at that path would hide once imported"
        );
        std::fs::write(
            src.join("plan/b.md"),
            "---\nid: b\n---\n[readme](../../../README.md)\n",
        )
        .unwrap();
        let (files, _) = read_nodes(&src, &[]);
        let error = writes(files, &shadow, &plan).unwrap_err().to_string();
        assert!(error.starts_with("plan/b.md: the link"), "{error}");
        let _ = std::fs::remove_dir_all(&src);
    }

    /// A page whose frontmatter makes it a node, or whose path a page can't have, is refused
    #[test]
    fn pages_a_plan_cant_keep_refused() {
        let src = scratch("page-refused");
        std::fs::create_dir_all(src.join("plan")).unwrap();
        std::fs::write(src.join("plan/a.md"), "---\nid: a\n---\n").unwrap();
        std::fs::write(src.join("notes.md"), "---\nid: n\n---\n").unwrap();
        // a node outside the book's roots would be kept as a page a book reads as a node
        let (files, _) = read_nodes(&src, &["plan".to_string()]);
        let links = BookLinks::new("", pages_of(&src, &files).unwrap());
        let error = page_writes(&src, &links).unwrap_err().to_string();
        assert!(
            error.starts_with("notes.md has a node's frontmatter"),
            "{error}"
        );
        // a file a link can't name is no page
        std::fs::remove_file(src.join("notes.md")).unwrap();
        std::fs::write(src.join("a#b.md"), "# A\n").unwrap();
        let (files, _) = read_nodes(&src, &[]);
        let error = pages_of(&src, &files).unwrap_err().to_string();
        assert!(error.starts_with("a#b.md: `a#b.md` holds '#'"), "{error}");
        let _ = std::fs::remove_dir_all(&src);
    }
}
