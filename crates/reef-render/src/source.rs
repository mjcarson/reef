//! Where a plan's docs are read from: a book's files on disk, a repo, or pages kept in memory
//! or in a store.

use crate::markdown::headings_of;
use reef_model::spec::frontmatter_is_yaml;
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
}
