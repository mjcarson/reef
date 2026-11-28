//! Where a plan's docs are read from: a book's files on disk, a repo, or pages kept in memory
//! or in a store.

use crate::markdown::headings_of;
use reef_model::spec::frontmatter_is_yaml;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// The headings of a doc, as `(id, text, level)`, with the ids mdbook gives them
pub type Headings = Vec<(String, String, u8)>;

/// One entry of a folder in a source of docs: a file or a folder, by its name
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    /// Its name within the folder
    pub name: String,
    /// Whether it is a folder
    pub folder: bool,
}

impl Entry {
    /// A file of a folder, by its name
    ///
    /// # Arguments
    ///
    /// * `name` - Its name within the folder
    pub fn file<N: Into<String>>(name: N) -> Self {
        Entry {
            name: name.into(),
            folder: false,
        }
    }

    /// A folder of a folder, by its name
    ///
    /// # Arguments
    ///
    /// * `name` - Its name within the folder
    pub fn folder<N: Into<String>>(name: N) -> Self {
        Entry {
            name: name.into(),
            folder: true,
        }
    }
}

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

    /// The files and folders of a folder, by name, its dotfiles left out, or none if it isn't
    /// one of this source's
    ///
    /// A source that can't list its folders lists none, so a book read from it is empty.
    ///
    /// # Arguments
    ///
    /// * `dir` - The folder's path from this source's root, empty for the root itself
    fn list(&self, dir: &str) -> Vec<Entry> {
        // list nothing by default since a source that can't list its folders has no book
        let _ = dir;
        Vec::new()
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

    /// The files and folders of a folder of the source this refers to
    ///
    /// # Arguments
    ///
    /// * `dir` - The folder's path from the source's root
    fn list(&self, dir: &str) -> Vec<Entry> {
        (**self).list(dir)
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

    /// The files and folders of a folder of the boxed source
    ///
    /// # Arguments
    ///
    /// * `dir` - The folder's path from the source's root
    fn list(&self, dir: &str) -> Vec<Entry> {
        (**self).list(dir)
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

    /// The files and folders of a folder under the root, by name
    ///
    /// # Arguments
    ///
    /// * `dir` - The folder's path from the root
    fn list(&self, dir: &str) -> Vec<Entry> {
        // list every entry in the folder, keeping each symlink as whatever it leads to
        listed(&self.root.join(dir), |_| true)
    }
}

/// The files and folders of a directory on disk, by name, its dotfiles left out, each symlink
/// taken as what it leads to and kept only if `keep` keeps where it leads
///
/// # Arguments
///
/// * `dir` - The directory
/// * `keep` - Whether an entry that leads to a path, symlinks followed, is listed
fn listed<F: Fn(&Path) -> bool>(dir: &Path, keep: F) -> Vec<Entry> {
    // open the directory, listing nothing if it can't be read
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    // collect each readable entry the directory holds
    let mut entries = Vec::new();
    for entry in read.flatten() {
        // skip dotfiles, as a book's walk does
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        // follow any symlinks to where the entry leads, skipping one that leads nowhere
        let Ok(target) = entry.path().canonicalize() else {
            continue;
        };
        // skip an entry whose target the caller doesn't keep
        if !keep(&target) {
            continue;
        }
        // list the target as a folder or a file, and skip anything else such as a socket
        if target.is_dir() {
            entries.push(Entry::folder(name));
        } else if target.is_file() {
            entries.push(Entry::file(name));
        }
    }
    // sort the entries by name since `read_dir` returns them in no set order
    entries.sort();
    entries
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

    /// The files and folders of a folder of the repo, by name, each kept only if it stays in
    /// the repo once its symlinks are followed
    ///
    /// # Arguments
    ///
    /// * `dir` - The folder's path from the repo's root
    fn list(&self, dir: &str) -> Vec<Entry> {
        // list nothing for a folder whose path climbs out of the repo as written
        if leaves(dir) {
            return Vec::new();
        }
        // resolve the folder's symlinks and list nothing if that takes it out of the repo
        let Some(folder) = self
            .root
            .join(dir)
            .canonicalize()
            .ok()
            .filter(|folder| folder.starts_with(&self.root))
        else {
            return Vec::new();
        };
        // list the folder's entries, keeping only those that lead somewhere inside the repo
        listed(&folder, |target| target.starts_with(&self.root))
    }
}

/// Whether a `/`-separated path leaves the directory it starts from as written: it is absolute,
/// or one of its parts is `..`
///
/// # Arguments
///
/// * `path` - The path to judge
pub fn leaves(path: &str) -> bool {
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

    /// Each doc kept, by its path, with its markdown as written, in no order
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_render::MemSource;
    ///
    /// let docs = MemSource::new().with("a.md", "# A\n");
    /// assert_eq!(docs.iter().collect::<Vec<_>>(), [("a.md", "# A\n")]);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.docs
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
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

    /// The files and folders of a folder, told from the paths of the docs kept under it: a doc
    /// `a/b/c.md` lists `b` as a folder of `a`
    ///
    /// # Arguments
    ///
    /// * `dir` - The folder's path, empty for the root
    fn list(&self, dir: &str) -> Vec<Entry> {
        // build the prefix every doc path under this folder starts with, empty for the root
        let prefix = match dir.trim_end_matches('/') {
            "" => String::new(),
            dir => format!("{dir}/"),
        };
        // track each name once, sorted, with whether any doc puts it as a folder
        let mut entries: BTreeMap<String, bool> = BTreeMap::new();
        for path in self.docs.keys() {
            // skip docs that aren't under this folder
            let Some(rest) = path.strip_prefix(&prefix) else {
                continue;
            };
            // take the path's next part, which is a folder if more parts follow it
            let (name, folder) = match rest.split_once('/') {
                Some((name, _)) => (name, true),
                None => (rest, false),
            };
            // skip an empty part, such as from a doubled `/`, and any dotfile
            if name.is_empty() || name.starts_with('.') {
                continue;
            }
            // mark the name as a folder if any doc under it says it is one
            *entries.entry(name.to_string()).or_default() |= folder;
        }
        // turn each name into an entry in name order
        entries
            .into_iter()
            .map(|(name, folder)| Entry { name, folder })
            .collect()
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

    /// The files and folders of a folder in either source, by name, the source over's entry
    /// taking a name both hold
    ///
    /// # Arguments
    ///
    /// * `dir` - The folder's path
    fn list(&self, dir: &str) -> Vec<Entry> {
        // list the source under first, keyed by name so each name is kept once
        let mut entries: BTreeMap<String, bool> = self
            .under
            .list(dir)
            .into_iter()
            .map(|entry| (entry.name, entry.folder))
            .collect();
        // add the source over's entries, replacing any of the same name from the source under
        entries.extend(
            self.over
                .list(dir)
                .into_iter()
                .map(|entry| (entry.name, entry.folder)),
        );
        // turn each name back into an entry in name order
        entries
            .into_iter()
            .map(|(name, folder)| Entry { name, folder })
            .collect()
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
    /// Each folder listed so far, with what it held
    listed: RefCell<HashMap<String, Vec<Entry>>>,
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
            listed: RefCell::new(HashMap::new()),
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

    /// The files and folders of a folder as they were the first time it was listed
    ///
    /// # Arguments
    ///
    /// * `dir` - The folder's path
    fn list(&self, dir: &str) -> Vec<Entry> {
        // answer a folder listed before with what it held then
        if let Some(entries) = self.listed.borrow().get(dir) {
            return entries.clone();
        }
        // otherwise list the folder now and keep its entries for the next ask
        let entries = self.source.list(dir);
        self.listed
            .borrow_mut()
            .insert(dir.to_string(), entries.clone());
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_left_out_of_headings() {
        let docs: MemSource = [("n.md", "---\nid: n\n---\n# N\n\n## Why\n")]
            .into_iter()
            .collect();
        // a node file's frontmatter is not part of its doc
        assert_eq!(doc_body(&docs.read("n.md").unwrap()), "# N\n\n## Why\n");
        let heads = docs.headings("n.md").unwrap();
        assert_eq!(
            heads.iter().map(|h| h.0.as_str()).collect::<Vec<_>>(),
            ["n", "why"]
        );
        // a missing doc has no headings
        assert!(docs.headings("gone.md").is_none());
        // a boxed or borrowed source reads the same
        /// The headings of `n.md` in any source
        fn heads_of(source: impl DocSource) -> Option<Headings> {
            source.headings("n.md")
        }
        assert_eq!(
            heads_of(Box::new(docs.clone()) as Box<dyn DocSource>),
            Some(heads.clone())
        );
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
        assert_eq!(
            docs.read("docs/in.md").as_deref(),
            Some("# In\n\n## Rows\n")
        );
        assert_eq!(docs.place("docs/in.md").as_deref(), Ok("docs/in.md"));
        // one beside it is not, however its path climbs out, nor is one a symlink leads out to
        for path in [
            "../outside.md",
            "docs/../../outside.md",
            "/etc/hostname",
            "docs/out.md",
        ] {
            assert!(docs.read(path).is_none(), "{path} was read");
        }
        // check refuses a link that leaves the repo as written, and finds no doc behind a symlink out
        let renderer = Renderer::new(&docs, Sections::default());
        assert_eq!(
            renderer.doc_link("", "../outside.md#rows").unwrap_err(),
            "doc `../outside.md#rows` leaves the repo"
        );
        assert_eq!(
            renderer
                .doc_link("docs/in.md", "../../outside.png")
                .unwrap_err(),
            "doc `../../outside.png` leaves the repo"
        );
        assert_eq!(
            renderer.doc_link("", "docs/out.md").unwrap_err(),
            "doc `docs/out.md` not found (looked for docs/out.md)"
        );
        assert_eq!(
            renderer.doc_link("", "docs/in.md#rows").unwrap().href,
            "reef-root:/docs/in.html#rows"
        );
        // add a folder, a dotfile, and a symlink to the folder from inside the repo
        std::fs::create_dir_all(repo.join("docs/sub")).unwrap();
        std::fs::write(repo.join("docs/.hidden.md"), "# Hidden\n").unwrap();
        std::os::unix::fs::symlink(repo.join("docs/sub"), repo.join("docs/linked")).unwrap();
        // check the folder lists its doc, its folder, and the symlink as a folder, but never
        // the symlink out of the repo or the dotfile
        assert_eq!(
            docs.list("docs"),
            [
                Entry::file("in.md"),
                Entry::folder("linked"),
                Entry::folder("sub")
            ]
        );
        // check the root lists its one folder
        assert_eq!(docs.list(""), [Entry::folder("docs")]);
        // check a path that climbs out, a file, and a missing folder all list nothing
        for dir in ["..", "docs/../..", "/etc", "docs/in.md", "gone"] {
            assert!(docs.list(dir).is_empty(), "{dir} was listed");
        }
        // add a symlink at the root that leads out of the repo
        std::os::unix::fs::symlink(&dir, repo.join("up")).unwrap();
        // check that listing through the symlink finds nothing and the root leaves it out
        assert!(docs.list("up").is_empty());
        assert_eq!(docs.list(""), [Entry::folder("docs")]);
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
        let pages = MemSource::new()
            .with("design/a.md", "# Page\n\n## Pages\n")
            .with("c.md", "# C\n");
        let docs = Layered::new(&pages, RepoSource::new(&dir));
        // a page hides the repo's doc at its path, headings and all
        assert_eq!(
            docs.read("design/a.md").as_deref(),
            Some("# Page\n\n## Pages\n")
        );
        assert_eq!(docs.headings("design/a.md").unwrap()[1].0, "pages");
        // a path no page holds is read from the repo, and one neither holds is no doc
        assert_eq!(docs.headings("design/b.md").unwrap()[1].0, "columns");
        assert_eq!(docs.read("c.md").as_deref(), Some("# C\n"));
        assert!(docs.read("gone.md").is_none());
        // paths are named as the repo names them, and a climb out is refused even over a page
        assert_eq!(docs.place("c.md").as_deref(), Ok("c.md"));
        assert_eq!(docs.place("../c.md").unwrap_err(), "leaves the repo");
        let renderer = Renderer::new(&docs, Sections::default());
        assert_eq!(
            renderer.doc_link("", "gone.md").unwrap_err(),
            "doc `gone.md` not found (looked for gone.md)"
        );
        assert_eq!(
            renderer.doc_link("", "design/a.md#rows").unwrap_err(),
            "doc `design/a.md#rows`: design/a.md has no heading with the id `rows`"
        );
        assert_eq!(
            renderer.doc_link("", "design/a.md#pages").unwrap().href,
            "reef-root:/design/a.html#pages"
        );
        // lay pages over the repo that share a file's name, add a folder, and turn a file into one
        let pages = MemSource::new()
            .with("design/a.md", "# Page\n")
            .with("design/new/n.md", "# N\n")
            .with("design/b.md/odd.md", "# Odd\n");
        let docs = Layered::new(&pages, RepoSource::new(&dir));
        // check the folder lists each name from both sources once, the pages' kind winning
        assert_eq!(
            docs.list("design"),
            [
                Entry::file("a.md"),
                Entry::folder("b.md"),
                Entry::folder("new")
            ]
        );
        // check the root lists the one folder both sources share
        assert_eq!(docs.list(""), [Entry::folder("design")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Docs kept in memory list each folder from the paths under it, and a cached source lists
    /// each folder once
    #[test]
    fn memory_lists_folders_from_paths() {
        // keep docs at a few depths, a dotfile among them
        let docs = MemSource::new()
            .with("a/b/c.md", "# C\n")
            .with("a/d.md", "# D\n")
            .with("a/.e.md", "# E\n")
            .with("top.md", "# Top\n");
        // check a deeper doc lists its next part as a folder and the dotfile is left out
        assert_eq!(docs.list("a"), [Entry::folder("b"), Entry::file("d.md")]);
        // check a trailing `/` lists the same folder
        assert_eq!(docs.list("a/"), docs.list("a"));
        // check the root and a nested folder list their own entries
        assert_eq!(docs.list(""), [Entry::folder("a"), Entry::file("top.md")]);
        assert_eq!(docs.list("a/b"), [Entry::file("c.md")]);
        // check a folder no doc is under lists nothing
        assert!(docs.list("gone").is_empty());
        // check a cached source lists the folder as its source did, the second time from its cache
        let cached = Cached::new(docs.clone());
        assert_eq!(cached.list("a"), docs.list("a"));
        assert_eq!(cached.list("a"), docs.list("a"));
        // define a source that only reads, leaving `list` to the trait's default
        struct Unlisted;
        impl DocSource for Unlisted {
            /// Read nothing
            ///
            /// # Arguments
            ///
            /// * `_path` - The doc's path
            fn read(&self, _path: &str) -> Option<String> {
                None
            }
        }
        // check that a source that can't list folders lists nothing
        assert!(Unlisted.list("").is_empty());
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
        let source = Counting {
            docs: MemSource::new().with("a.md", "# A\n\n## Why\n"),
            reads: Default::default(),
        };
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
