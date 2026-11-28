//! A plan's own pages: markdown docs kept with the plan, which its nodes link to as they link
//! to its repo's files
//!
//! A page is named by a `/`-separated path, in the same space as the repo's files: a plan's
//! pages lie over its repo's working tree, and a page hides the repo's file at its path. A
//! page's links lead from its own path, as a book's do from its file.
//!
//! Each edit names the version its writer read, or `None` for a new page, so an edit made
//! against a page that has moved on is refused as stale, an answer and not a failure. An edit
//! that would add a problem to the plan is refused with check's messages. Either refusal writes
//! nothing.

use crate::node::Refusal;
use reef_model::Diagnostic;
use serde::{Deserialize, Serialize};

/// The most text a plan's pages may hold together, in bytes
///
/// A plan's pages are read as one partition, which must fit in one of shoal's frames (64 MiB
/// by default), so the cap leaves room for the rows' keys.
pub const PLAN_PAGES_MAX_BYTES: usize = 32 << 20;

/// The largest body a whole-plan write or a page edit may send, in bytes
///
/// It holds a plan's nodes beside pages at their cap, which axum's default of 2 MiB does not:
/// the example book's pages alone are 2.4 MB.
pub const WRITE_BODY_MAX: usize = 48 << 20;

/// A page as a writer gives it to the store
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageWrite {
    /// Its path, which links name it by
    pub path: String,
    /// Its markdown as written
    pub text: String,
}

impl PageWrite {
    /// A page at a path
    ///
    /// # Arguments
    ///
    /// * `path` - Its path, which links name it by
    /// * `text` - Its markdown as written
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::PageWrite;
    ///
    /// let page = PageWrite::new("design/app.md", "# The app\n");
    /// assert_eq!(page.path, "design/app.md");
    /// ```
    pub fn new<P: Into<String>, T: Into<String>>(path: P, text: T) -> Self {
        PageWrite {
            path: path.into(),
            text: text.into(),
        }
    }
}

/// A page as a reader got it from the store
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPage {
    /// Its path, which links name it by
    pub path: String,
    /// Its markdown as written
    pub text: String,
    /// The version it was read at, which an edit of it names
    pub version: u64,
}

impl From<StoredPage> for PageWrite {
    /// The page as it would be written again
    ///
    /// # Arguments
    ///
    /// * `page` - The page as read
    fn from(page: StoredPage) -> Self {
        PageWrite {
            path: page.path,
            text: page.text,
        }
    }
}

/// A plan's pages as a reader got them
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageListing {
    /// The revision of the plan's pages that was current
    pub revision: u64,
    /// Its pages, sorted by path
    pub pages: Vec<StoredPage>,
}

/// One change to a plan's pages
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageEdit {
    /// Writes a page, new or changed
    Put {
        /// The page as it is to be written
        page: PageWrite,
        /// The version its writer read, or `None` for a new page
        read: Option<u64>,
    },
    /// Takes a page out of the plan
    Remove {
        /// The page's path
        path: String,
        /// The version its writer read
        read: u64,
    },
}

impl PageEdit {
    /// The path of the page this edit writes or takes out
    pub fn path(&self) -> &str {
        match self {
            PageEdit::Put { page, .. } => &page.path,
            PageEdit::Remove { path, .. } => path,
        }
    }
}

/// What came of a page edit
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageEdited {
    /// The edit was written
    Applied {
        /// The page's new version, or `None` once it is taken out
        version: Option<u64>,
    },
    /// The page was other than its writer read it, so nothing was written
    Stale {
        /// The page's path
        path: String,
        /// How the page differs from what its writer read
        refusal: Refusal,
    },
    /// The check found problems the edit would add, so nothing was written
    Refused(Vec<Diagnostic>),
}

/// Whether a path can name a page, and if not why
///
/// A page's path is relative and `/`-separated, and names a markdown file a book could hold:
/// no part of it is empty or starts with a `.`, it holds no `#`, `?`, `\` or control character,
/// it ends in `.md`, and it isn't the root's `SUMMARY.md`, which export writes for a book of its
/// own. A `SUMMARY.md` under a folder may be a page, as a plan's docs may name it (C122). A
/// book's walk keeps every such file, so a page exported into a book imports back.
///
/// # Arguments
///
/// * `path` - The path to judge
///
/// # Examples
///
/// ```
/// use reef_api::check_page_path;
///
/// assert!(check_page_path("design/app.md").is_ok());
/// assert!(check_page_path("../app.md").is_err());
/// ```
pub fn check_page_path(path: &str) -> Result<(), String> {
    // a path names a file from the plan's root
    if path.is_empty() {
        return Err("a page's path is empty".to_string());
    }
    if path.starts_with('/') {
        return Err(format!(
            "`{path}` is absolute, and a page's path is relative to its plan's root"
        ));
    }
    // a link can't name a file whose path holds a link's own marks
    if let Some(mark) = path
        .chars()
        .find(|c| matches!(c, '#' | '?' | '\\') || c.is_control())
    {
        return Err(format!(
            "`{}` holds {mark:?}, which a link can't name a page by",
            path.escape_debug()
        ));
    }
    for part in path.split('/') {
        // every part names a directory or the file, and a book's walk skips dotfiles
        if part.is_empty() {
            return Err(format!("`{path}` has an empty part"));
        }
        if part.starts_with('.') {
            return Err(format!(
                "`{path}` has a part starting with `.`, which a book leaves out"
            ));
        }
    }
    // only markdown is a page, and a book's summary is mdbook's
    if !path.ends_with(".md") {
        return Err(format!(
            "`{path}` isn't markdown: a page's path ends in `.md`"
        ));
    }
    // refuse only the root's summary, since a nested `SUMMARY.md` may be a page
    if path == "SUMMARY.md" {
        return Err(format!(
            "`{path}` is a book's summary, which no page can be"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PageEdit, PageEdited, PageWrite, check_page_path};
    use crate::Refusal;

    /// A page's path names a markdown file a book's walk keeps, and nothing else
    #[test]
    fn page_paths_keep_to_the_rules() {
        // a page at the root or under directories is kept
        for path in [
            "plan.md",
            "design/app.md",
            "a/b/c-d_e.md",
            "README.md",
            "a/SUMMARY.md",
        ] {
            assert_eq!(check_page_path(path), Ok(()), "{path}");
        }
        // anything a book would skip, or a link can't name, is refused with why
        let refused = [
            ("", "a page's path is empty"),
            (
                "/etc/x.md",
                "`/etc/x.md` is absolute, and a page's path is relative to its plan's root",
            ),
            ("a//b.md", "`a//b.md` has an empty part"),
            ("a/", "`a/` has an empty part"),
            (
                "../x.md",
                "`../x.md` has a part starting with `.`, which a book leaves out",
            ),
            (
                "./x.md",
                "`./x.md` has a part starting with `.`, which a book leaves out",
            ),
            (
                "a/.hidden.md",
                "`a/.hidden.md` has a part starting with `.`, which a book leaves out",
            ),
            (
                "a#b.md",
                "`a#b.md` holds '#', which a link can't name a page by",
            ),
            (
                "a?.md",
                "`a?.md` holds '?', which a link can't name a page by",
            ),
            (
                "a\\b.md",
                "`a\\\\b.md` holds '\\\\', which a link can't name a page by",
            ),
            (
                "a\nb.md",
                "`a\\nb.md` holds '\\n', which a link can't name a page by",
            ),
            (
                "design/app.html",
                "`design/app.html` isn't markdown: a page's path ends in `.md`",
            ),
            (
                "SUMMARY.md",
                "`SUMMARY.md` is a book's summary, which no page can be",
            ),
        ];
        for (path, why) in refused {
            assert_eq!(check_page_path(path).unwrap_err(), why, "{path:?}");
        }
    }

    /// Edits and their outcomes are tagged in kebab-case
    #[test]
    fn page_outcomes_are_externally_tagged_in_kebab_case() {
        // a put carries its page and the version read, and a removal its path and version
        let put = PageEdit::Put {
            page: PageWrite::new("design/app.md", "# App\n"),
            read: None,
        };
        let json = serde_json::to_string(&put).unwrap();
        assert_eq!(
            json,
            r##"{"put":{"page":{"path":"design/app.md","text":"# App\n"},"read":null}}"##
        );
        assert_eq!(serde_json::from_str::<PageEdit>(&json).unwrap(), put);
        assert_eq!(put.path(), "design/app.md");
        let remove = PageEdit::Remove {
            path: "plan.md".to_string(),
            read: 3,
        };
        assert_eq!(
            serde_json::to_string(&remove).unwrap(),
            r#"{"remove":{"path":"plan.md","read":3}}"#
        );
        assert_eq!(remove.path(), "plan.md");
        // what came of one is tagged the same way
        let applied = PageEdited::Applied { version: Some(2) };
        assert_eq!(
            serde_json::to_string(&applied).unwrap(),
            r#"{"applied":{"version":2}}"#
        );
        let stale = PageEdited::Stale {
            path: "plan.md".to_string(),
            refusal: Refusal::Moved,
        };
        assert_eq!(
            serde_json::to_string(&stale).unwrap(),
            r#"{"stale":{"path":"plan.md","refusal":"moved"}}"#
        );
        assert_eq!(
            serde_json::to_string(&PageEdited::Refused(vec![])).unwrap(),
            r#"{"refused":[]}"#
        );
    }
}
