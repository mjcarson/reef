//! Where a plan's docs are read from: a book's files on disk, a repo, or pages kept in memory
//! or in a store.

use crate::markdown::headings_of;
use reef_model::spec::frontmatter_is_yaml;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The headings of a doc, as `(id, text, level)`, with the ids mdbook gives them
pub type Headings = Vec<(String, String, u8)>;

/// A source of the docs a plan's nodes link to, each named by a `/`-separated path from the
/// source's root
pub trait DocSource {
    /// Read a doc's markdown as written, frontmatter and all, or `None` if there is no such doc
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from this source's root
    fn read(&self, path: &str) -> Option<String>;

    /// The headings of a doc, with the ids mdbook gives them, or `None` if there is no such doc
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from this source's root
    fn headings(&self, path: &str) -> Option<Headings> {
        // read the doc and find the headings in its body
        self.read(path).map(|text| headings_of(doc_body(&text)))
    }

    /// Where a doc at a path is looked for, as check's messages name it, or why this source
    /// can't hold a doc there
    ///
    /// A book's docs are under its `src`, so that is where a source says it looks unless it says
    /// otherwise.
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from this source's root
    fn place(&self, path: &str) -> Result<String, String> {
        Ok(format!("src/{path}"))
    }
}

impl<T: DocSource + ?Sized> DocSource for &T {
    /// Read a doc from the source this refers to
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the source's root
    fn read(&self, path: &str) -> Option<String> {
        (**self).read(path)
    }

    /// The headings of a doc in the source this refers to
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the source's root
    fn headings(&self, path: &str) -> Option<Headings> {
        (**self).headings(path)
    }

    /// Where the source this refers to looks for a doc
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the source's root
    fn place(&self, path: &str) -> Result<String, String> {
        (**self).place(path)
    }
}

impl<T: DocSource + ?Sized> DocSource for Box<T> {
    /// Read a doc from the boxed source
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the source's root
    fn read(&self, path: &str) -> Option<String> {
        (**self).read(path)
    }

    /// The headings of a doc in the boxed source
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the source's root
    fn headings(&self, path: &str) -> Option<Headings> {
        (**self).headings(path)
    }

    /// Where the boxed source looks for a doc
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the source's root
    fn place(&self, path: &str) -> Result<String, String> {
        (**self).place(path)
    }
}

/// A doc's markdown without the YAML frontmatter a node file opens with
///
/// # Arguments
///
/// * `text` - The doc as written
pub fn doc_body(text: &str) -> &str {
    frontmatter_is_yaml(text).map_or(text, |fm| fm.body)
}

/// Docs read from files under a directory, such as a book's `src`
#[derive(Debug, Clone)]
pub struct FsSource {
    /// The directory every doc's path starts from
    root: PathBuf,
}

impl FsSource {
    /// Read docs from the files under a directory
    ///
    /// # Arguments
    ///
    /// * `root` - The directory every doc's path starts from
    pub fn new<P: Into<PathBuf>>(root: P) -> Self {
        FsSource { root: root.into() }
    }

    /// The directory every doc's path starts from
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl DocSource for FsSource {
    /// Read a doc from its file under the root
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the root
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(path)).ok()
    }
}

/// Docs read from a repo's working tree, which no link may leave
///
/// A path with a `..` in it, or an absolute one, is no doc of the repo's, and neither is a file a
/// symlink in the repo leads out of it to. Check names a doc by its path from the repo's root.
#[derive(Debug, Clone)]
pub struct RepoSource {
    /// The repo's root, canonical, so a path read under it can be told to stay there
    root: PathBuf,
}

impl RepoSource {
    /// Read docs from a repo's working tree
    ///
    /// # Arguments
    ///
    /// * `root` - The repo's root
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_render::{DocSource, RepoSource};
    ///
    /// let docs = RepoSource::new("/src/reef");
    /// assert!(docs.read("../secrets.md").is_none());
    /// assert_eq!(docs.place("../secrets.md").unwrap_err(), "leaves the repo");
    /// ```
    pub fn new<P: Into<PathBuf>>(root: P) -> Self {
        // a root that can't be resolved is kept as given, and reads nothing
        let root = root.into();
        let root = root.canonicalize().unwrap_or(root);
        RepoSource { root }
    }

    /// The repo's root, canonical when it could be resolved
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file a doc's path names, if it stays in the repo once symlinks are followed
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the repo's root
    fn file(&self, path: &str) -> Option<PathBuf> {
        // a path that climbs out, or starts from somewhere else, is refused as written
        if leaves(path) {
            return None;
        }
        // and so is one a symlink takes out of the repo
        let file = self.root.join(path).canonicalize().ok()?;
        file.starts_with(&self.root).then_some(file)
    }
}

impl DocSource for RepoSource {
    /// Read a doc from its file in the repo, unless its path leaves the repo
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the repo's root
    fn read(&self, path: &str) -> Option<String> {
        // only a file that stays in the repo is read
        let file = self.file(path)?;
        std::fs::read_to_string(file).ok()
    }

    /// A doc's path from the repo's root, or a refusal of one that leaves the repo as written
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the repo's root
    fn place(&self, path: &str) -> Result<String, String> {
        // a symlink out is found when it is read, and so is just not found
        match leaves(path) {
            true => Err("leaves the repo".to_string()),
            false => Ok(path.to_string()),
        }
    }
}

/// Whether a `/`-separated path leaves the directory it starts from as written: it is absolute,
/// or one of its parts is `..`
///
/// # Arguments
///
/// * `path` - The path to judge
fn leaves(path: &str) -> bool {
    path.starts_with('/') || path.split('/').any(|part| part == "..")
}

/// Docs kept in memory by path, such as pages loaded from a store or written by a test
#[derive(Debug, Clone, Default)]
pub struct MemSource {
    /// Each doc's markdown as written, by its path
    docs: HashMap<String, String>,
}

impl MemSource {
    /// An empty source of docs
    pub fn new() -> Self {
        MemSource::default()
    }

    /// Add a doc, or replace the one at the same path
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path
    /// * `text` - The doc's markdown as written
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_render::{DocSource, MemSource};
    ///
    /// let mut docs = MemSource::new();
    /// docs.insert("design/a.md", "# A\n\n## Context\n\nWhy.\n");
    /// assert_eq!(docs.headings("design/a.md").unwrap()[1].0, "context");
    /// ```
    pub fn insert<P: Into<String>, T: Into<String>>(&mut self, path: P, text: T) {
        self.docs.insert(path.into(), text.into());
    }

    /// This source with a doc added, or replacing the one at the same path
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path
    /// * `text` - The doc's markdown as written
    pub fn with<P: Into<String>, T: Into<String>>(mut self, path: P, text: T) -> Self {
        self.insert(path, text);
        self
    }
}

impl DocSource for MemSource {
    /// Read a doc kept at a path
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path
    fn read(&self, path: &str) -> Option<String> {
        self.docs.get(path).cloned()
    }
}

impl<P: Into<String>, T: Into<String>> FromIterator<(P, T)> for MemSource {
    /// Keep each doc by its path
    ///
    /// # Arguments
    ///
    /// * `iter` - The docs, each a path and its markdown
    fn from_iter<I: IntoIterator<Item = (P, T)>>(iter: I) -> Self {
        // add each doc in turn, a later one replacing an earlier at the same path
        let mut docs = MemSource::new();
        for (path, text) in iter {
            docs.insert(path, text);
        }
        docs
    }
}

/// Docs read from one source first and another under it, such as a plan's pages over its
/// repo's working tree
///
/// A doc the source over holds hides the one at the same path under it. Paths are named as the
/// source under names them, so check's messages, and the paths it refuses, are the same with
/// or without the layer over it.
#[derive(Debug, Clone)]
pub struct Layered<A, B> {
    /// The source read first, whose docs hide those at the same paths under it
    over: A,
    /// The source read when the one over it holds no doc at a path, which names every path
    under: B,
}

impl<A: DocSource, B: DocSource> Layered<A, B> {
    /// Read docs from one source, then from another under it
    ///
    /// # Arguments
    ///
    /// * `over` - The source read first
    /// * `under` - The source read when the one over it holds no doc at a path
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_render::{DocSource, Layered, MemSource};
    ///
    /// let pages = MemSource::new().with("design.md", "# Page\n");
    /// let repo = MemSource::new().with("design.md", "# Repo\n").with("other.md", "# Other\n");
    /// let docs = Layered::new(pages, repo);
    /// assert_eq!(docs.read("design.md").as_deref(), Some("# Page\n"));
    /// assert_eq!(docs.read("other.md").as_deref(), Some("# Other\n"));
    /// ```
    pub fn new(over: A, under: B) -> Self {
        Layered { over, under }
    }
}

impl<A: DocSource, B: DocSource> DocSource for Layered<A, B> {
    /// Read a doc from the source over, else from the one under it
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path
    fn read(&self, path: &str) -> Option<String> {
        self.over.read(path).or_else(|| self.under.read(path))
    }

    /// The headings of the doc the source over holds at a path, else of the one under it
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path
    fn headings(&self, path: &str) -> Option<Headings> {
        self.over
            .headings(path)
            .or_else(|| self.under.headings(path))
    }

    /// Where the source under looks for a doc, or why it can't hold one there
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path
    fn place(&self, path: &str) -> Result<String, String> {
        self.under.place(path)
    }
}

/// A source whose docs are each read once, however often they are asked for
///
/// Two renderers over one cached source, such as a write's plan as it stands and as the write
/// would leave it, read each doc from the same moment.
#[derive(Debug)]
pub struct Cached<S> {
    /// The source each doc is first read from
    source: S,
    /// Each path read so far, with the doc found there, if any
    read: RefCell<HashMap<String, Option<String>>>,
}

impl<S: DocSource> Cached<S> {
    /// Keep each doc read from a source
    ///
    /// # Arguments
    ///
    /// * `source` - The source each doc is first read from
    pub fn new(source: S) -> Self {
        Cached {
            source,
            read: RefCell::new(HashMap::new()),
        }
    }
}

impl<S: DocSource> DocSource for Cached<S> {
    /// Read a doc as it was the first time it was asked for
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path
    fn read(&self, path: &str) -> Option<String> {
        // a path read before answers as it did then
        if let Some(doc) = self.read.borrow().get(path) {
            return doc.clone();
        }
        // otherwise read it now, and keep what was found
        let doc = self.source.read(path);
        self.read.borrow_mut().insert(path.to_string(), doc.clone());
        doc
    }

    /// Where the source looks for a doc
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path
    fn place(&self, path: &str) -> Result<String, String> {
        self.source.place(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_left_out_of_headings() {
        let docs: MemSource = [("n.md", "---\nid: n\n---\n# N\n\n## Why\n")].into_iter().collect();
        // a node file's frontmatter is not part of its doc
        assert_eq!(doc_body(&docs.read("n.md").unwrap()), "# N\n\n## Why\n");
        let heads = docs.headings("n.md").unwrap();
        assert_eq!(heads.iter().map(|h| h.0.as_str()).collect::<Vec<_>>(), ["n", "why"]);
        // a missing doc has no headings
        assert!(docs.headings("gone.md").is_none());
        // a boxed or borrowed source reads the same
        /// The headings of `n.md` in any source
        fn heads_of(source: impl DocSource) -> Option<Headings> {
            source.headings("n.md")
        }
        assert_eq!(heads_of(Box::new(docs.clone()) as Box<dyn DocSource>), Some(heads.clone()));
        assert_eq!(heads_of(&docs), Some(heads));
    }

    #[test]
    fn files_under_a_root() {
        let dir = std::env::temp_dir().join(format!("reef-fs-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("a/b.md"), "# B\n").unwrap();
        let docs = FsSource::new(&dir);
        // a doc is read by its path from the root
        assert_eq!(docs.read("a/b.md").as_deref(), Some("# B\n"));
        assert!(docs.read("a/c.md").is_none());
        assert_eq!(docs.root(), dir.as_path());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A repo's source reads no doc outside the repo, and check refuses a link that leaves it
    #[test]
    fn link_leaving_the_repo_is_refused() {
        use crate::docs::{Renderer, Sections};
        // a repo with a doc in it, a doc beside it, and a symlink in it to the one beside it
        let dir = std::env::temp_dir().join(format!("reef-repo-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::write(repo.join("docs/in.md"), "# In\n\n## Rows\n").unwrap();
        std::fs::write(dir.join("outside.md"), "# Outside\n\n## Rows\n").unwrap();
        std::os::unix::fs::symlink(dir.join("outside.md"), repo.join("docs/out.md")).unwrap();
        let docs = RepoSource::new(&repo);
        // a doc in the repo is read, and named by its path from the root
        assert_eq!(docs.read("docs/in.md").as_deref(), Some("# In\n\n## Rows\n"));
        assert_eq!(docs.place("docs/in.md").as_deref(), Ok("docs/in.md"));
        // one beside it is not, however its path climbs out, nor is one a symlink leads out to
        for path in ["../outside.md", "docs/../../outside.md", "/etc/hostname", "docs/out.md"] {
            assert!(docs.read(path).is_none(), "{path} was read");
        }
        // check refuses a link that leaves the repo as written, and finds no doc behind a symlink out
        let renderer = Renderer::new(&docs, Sections::default());
        assert_eq!(renderer.doc_link("", "../outside.md#rows").unwrap_err(), "doc `../outside.md#rows` leaves the repo");
        assert_eq!(renderer.doc_link("docs/in.md", "../../outside.png").unwrap_err(), "doc `../../outside.png` leaves the repo");
        assert_eq!(renderer.doc_link("", "docs/out.md").unwrap_err(), "doc `docs/out.md` not found (looked for docs/out.md)");
        assert_eq!(renderer.doc_link("", "docs/in.md#rows").unwrap().href, "reef-root:/docs/in.html#rows");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A page hides the repo's doc at its path, a path it doesn't hold falls through, and paths
    /// are named and refused by the source under
    #[test]
    fn layered_reads_over_first_and_places_from_under() {
        use crate::docs::{Renderer, Sections};
        let dir = std::env::temp_dir().join(format!("reef-layered-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("design")).unwrap();
        std::fs::write(dir.join("design/a.md"), "# Repo\n\n## Rows\n").unwrap();
        std::fs::write(dir.join("design/b.md"), "# B\n\n## Columns\n").unwrap();
        let pages = MemSource::new().with("design/a.md", "# Page\n\n## Pages\n").with("c.md", "# C\n");
        let docs = Layered::new(&pages, RepoSource::new(&dir));
        // a page hides the repo's doc at its path, headings and all
        assert_eq!(docs.read("design/a.md").as_deref(), Some("# Page\n\n## Pages\n"));
        assert_eq!(docs.headings("design/a.md").unwrap()[1].0, "pages");
        // a path no page holds is read from the repo, and one neither holds is no doc
        assert_eq!(docs.headings("design/b.md").unwrap()[1].0, "columns");
        assert_eq!(docs.read("c.md").as_deref(), Some("# C\n"));
        assert!(docs.read("gone.md").is_none());
        // paths are named as the repo names them, and a climb out is refused even over a page
        assert_eq!(docs.place("c.md").as_deref(), Ok("c.md"));
        assert_eq!(docs.place("../c.md").unwrap_err(), "leaves the repo");
        let renderer = Renderer::new(&docs, Sections::default());
        assert_eq!(renderer.doc_link("", "gone.md").unwrap_err(), "doc `gone.md` not found (looked for gone.md)");
        assert_eq!(renderer.doc_link("", "design/a.md#rows").unwrap_err(), "doc `design/a.md#rows`: design/a.md has no heading with the id `rows`");
        assert_eq!(renderer.doc_link("", "design/a.md#pages").unwrap().href, "reef-root:/design/a.html#pages");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A cached source reads each doc once, a missing one included, and names paths as its
    /// source does
    #[test]
    fn cached_reads_each_doc_once() {
        /// A source that counts its reads
        struct Counting {
            /// The docs it holds
            docs: MemSource,
            /// How many reads it has answered
            reads: std::cell::Cell<usize>,
        }
        impl DocSource for Counting {
            /// Read a doc, counting the read
            ///
            /// # Arguments
            ///
            /// * `path` - The doc's path
            fn read(&self, path: &str) -> Option<String> {
                self.reads.set(self.reads.get() + 1);
                self.docs.read(path)
            }

            /// Name a path as a repo would
            ///
            /// # Arguments
            ///
            /// * `path` - The doc's path
            fn place(&self, path: &str) -> Result<String, String> {
                Ok(path.to_string())
            }
        }
        let source = Counting { docs: MemSource::new().with("a.md", "# A\n\n## Why\n"), reads: Default::default() };
        let docs = Cached::new(&source);
        // each path is read from the source once, found or not
        for _ in 0..3 {
            assert_eq!(docs.read("a.md").as_deref(), Some("# A\n\n## Why\n"));
            assert!(docs.read("gone.md").is_none());
            assert_eq!(docs.headings("a.md").unwrap()[1].0, "why");
        }
        assert_eq!(source.reads.get(), 2);
        // and named as the source names it
        assert_eq!(docs.place("a.md").as_deref(), Ok("a.md"));
    }
}
