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
//! - **Links lead from the repo's root.** A stored node's links are read from its project's
//!   repo, so each link written in a file, in its doc links or in any of its markdown, is
//!   rewritten to lead to the same file and heading from there.
//! - **A plan with nodes is replaced only when asked.** The import is a whole-plan write, which
//!   leaves out every node the book doesn't have, so a plan that has any takes `--replace`.
//!
//! A book that names its own doc sections is refused, since a central plan takes its briefs from
//! the default sections.

use reef_api::{NodeWrite, PlanWrite, Project, Rewrite, lift};
use reef_model::PlanName;
use reef_render::book::{BookNode, Config, Plan, read_nodes};
use reef_render::links::{LinkError, rewrite_links, rewrite_spec};
use reef_render::{Sections, paths};
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

/// The rows a book's node files make: each file's node, then each of its members lifted out
/// of it, numbered in the order the book was written in, their links leading from the repo's
/// root
///
/// # Arguments
///
/// * `files` - The book's node files, in the order they sort in
/// * `prefix` - The book's `src`, from the repo's root
/// * `plan` - The plan they are to be written to, whose names each id must make
///
/// # Examples
///
/// ```
/// use reef::import::writes;
/// use reef_model::PlanName;
/// use reef_render::book::read_nodes;
///
/// # fn main() -> anyhow::Result<()> {
/// let src = std::env::temp_dir().join(format!("reef-writes-doc-{}", std::process::id()));
/// std::fs::create_dir_all(src.join("plan"))?;
/// std::fs::write(src.join("plan/g.md"), "---\nid: g\ndocs: [../d.md]\nnodes: [{ id: m }]\n---\nThe group.\n")?;
/// let (files, _) = read_nodes(&src, &[]);
/// let rows = writes(files, "book/src", &PlanName::new("reef", "central")?)?;
/// assert_eq!(rows[0].spec.docs, ["book/src/d.md"]);
/// assert_eq!((rows[1].spec.id.as_str(), rows[1].order), ("m", 1));
/// # std::fs::remove_dir_all(&src)?;
/// # Ok(())
/// # }
/// ```
pub fn writes(
    files: Vec<BookNode>,
    prefix: &str,
    plan: &PlanName,
) -> anyhow::Result<Vec<NodeWrite>> {
    let mut rows = Vec::new();
    for file in files {
        // each link in the file leads where it did, from the repo's root
        let from = paths::join(prefix, &file.rel);
        let map = |link: &str| paths::moved(&from, "", link);
        let rel = &file.rel;
        let mut body = rewrite_links(&file.body, &map).map_err(|error| {
            let at = format!("`{}` body", file.spec.id);
            anyhow::anyhow!("{rel}: {}", LinkError { at, ..error })
        })?;
        // the file's node, then each member lifted out of it, depth first
        for mut spec in lift(file.spec) {
            rewrite_spec(&mut spec, &map).map_err(|error| anyhow::anyhow!("{rel}: {error}"))?;
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
    if opened.config.sections != Sections::default() {
        anyhow::bail!(
            "{} names its own doc sections in `[preprocessor.reef]`, which a central plan doesn't \
             keep, so its nodes would be briefed from other sections than the book's",
            book.display()
        );
    }
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
    // the rows the book makes, each id one the plan can name
    let (files, _) = read_nodes(&opened.src, &opened.config.roots);
    let rows = writes(files, &prefix, &plan)?;
    // a plan named is listed if it isn't
    let added = named && client.add_plan(&plan).await?.added;
    // a plan that has nodes loses those the book doesn't have, so is replaced only when asked
    let listing = client.list(&plan).await?;
    if !listing.nodes.is_empty() && !replace {
        anyhow::bail!(
            "`{plan}` has {}, which an import replaces: pass `--replace` to replace them",
            counted(listing.nodes.len(), "node")
        );
    }
    // the whole plan, written as one revision
    let count = rows.len();
    match client.write_plan(&plan, &PlanWrite { nodes: rows }).await? {
        Rewrite::Applied(done) => {
            writeln!(
                out,
                "imported {} from {} into `{plan}` at revision {}",
                counted(count, "node"),
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
    use super::{open_book, prefix, writes};
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
        let rows = writes(files, "b/src", &plan).unwrap();
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
        let error = writes(files, "", &plan).unwrap_err().to_string();
        assert!(
            error.starts_with("a.md: `a/b` can't be a node of `reef/central`"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&src);
    }
}
