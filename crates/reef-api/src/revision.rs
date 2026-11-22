//! A node's captured changes: revisions of files, hunks and lines, each line anchored
//!
//! `reef node capture` reads a node's diff from its own checkout, from its start carried forward
//! to its worktree, and sends every changed file with its hunks. The service keeps each capture
//! as a [`Revision`], numbered from 1 for its node, and never reads git again, so a squash, a
//! rebase or a deleted branch loses nothing it shows.
//!
//! - **A revision lists its files without their hunks**, as [`FileSummary`]s, so a node's
//!   revisions can be listed whatever their size. A file's hunks are read one file at a time, as
//!   a [`FileDiff`], when it is opened.
//! - **Each line has an anchor** a comment can name: its revision, its file and its number on the
//!   side it is on ([`Anchor`]).
//! - **A file too large to keep whole** is kept without its hunks and says so, as a binary file
//!   is kept without them.

use reef_model::NodeName;
use serde::{Deserialize, Serialize};

use crate::work::CaptureShas;

/// What happened to one file between a node's base and its worktree
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileStatus {
    /// The file is new in the worktree
    Added,
    /// The file is gone from the worktree
    Deleted,
    /// The file is on both sides, with new contents or a new mode
    Modified,
    /// The file moved, and its contents may have changed with it
    Renamed,
}

/// The kind of one line in a hunk
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineKind {
    /// On both sides, shown around a change
    Context,
    /// Only in the worktree
    Added,
    /// Only in the base
    Removed,
}

/// One line of a hunk, numbered on each side it is on
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// Whether the line was added, removed or is context
    pub kind: LineKind,
    /// Its 1-based number in the base, if it is there
    pub old: Option<u32>,
    /// Its 1-based number in the worktree, if it is there
    pub new: Option<u32>,
    /// Its text, without the line's newline
    pub text: String,
    /// Whether the line ends without a newline, as a file's last line can
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_newline: bool,
}

/// One hunk: a run of changed lines with their context
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    /// Where the hunk starts in the base, the line before it when the base side is empty
    pub old_start: u32,
    /// How many base lines it spans
    pub old_lines: u32,
    /// Where the hunk starts in the worktree, the line before it when that side is empty
    pub new_start: u32,
    /// How many worktree lines it spans
    pub new_lines: u32,
    /// Its lines, in order
    pub lines: Vec<Line>,
}

/// One file's changes, its hunks and all
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    /// What happened to the file
    pub status: FileStatus,
    /// Its path in the base, if it was there
    pub old_path: Option<String>,
    /// Its path in the worktree, if it is there
    pub new_path: Option<String>,
    /// Its mode in the base, such as 100644
    pub old_mode: Option<String>,
    /// Its mode in the worktree
    pub new_mode: Option<String>,
    /// Its blob in the base
    pub old_id: Option<String>,
    /// Its blob in the worktree
    pub new_id: Option<String>,
    /// Whether either side is binary, as git takes it, in which case it has no hunks
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
    /// Whether its hunks were too large to keep, in which case it is kept without them
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub too_large: bool,
    /// Its hunks, in order
    #[serde(default)]
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    /// The path the file is shown under: its worktree path, or its base path once deleted
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::revision::{FileDiff, FileStatus};
    ///
    /// let gone = FileDiff {
    ///     status: FileStatus::Deleted,
    ///     old_path: Some("old.rs".to_string()),
    ///     new_path: None,
    ///     old_mode: Some("100644".to_string()),
    ///     new_mode: None,
    ///     old_id: None,
    ///     new_id: None,
    ///     binary: false,
    ///     too_large: false,
    ///     hunks: Vec::new(),
    /// };
    /// assert_eq!(gone.path(), "old.rs");
    /// ```
    pub fn path(&self) -> &str {
        // a file that is still there is shown where it is now
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or_default()
    }

    /// Counts the lines added and removed across the file's hunks
    pub fn counts(&self) -> (u32, u32) {
        let mut added = 0;
        let mut removed = 0;
        for line in self.hunks.iter().flat_map(|hunk| &hunk.lines) {
            // context lines count for neither
            match line.kind {
                LineKind::Added => added += 1,
                LineKind::Removed => removed += 1,
                LineKind::Context => {}
            }
        }
        (added, removed)
    }

    /// The file as a revision lists it: everything but its hunks, and how many lines changed
    pub fn summary(&self) -> FileSummary {
        // the counts come from the hunks, which the summary leaves out
        let (added, removed) = self.counts();
        FileSummary {
            status: self.status,
            old_path: self.old_path.clone(),
            new_path: self.new_path.clone(),
            old_mode: self.old_mode.clone(),
            new_mode: self.new_mode.clone(),
            old_id: self.old_id.clone(),
            new_id: self.new_id.clone(),
            binary: self.binary,
            too_large: self.too_large,
            added,
            removed,
        }
    }
}

/// One file as a revision lists it: what happened to it and how many lines, but not its hunks
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSummary {
    /// What happened to the file
    pub status: FileStatus,
    /// Its path in the base, if it was there
    pub old_path: Option<String>,
    /// Its path in the worktree, if it is there
    pub new_path: Option<String>,
    /// Its mode in the base, such as 100644
    pub old_mode: Option<String>,
    /// Its mode in the worktree
    pub new_mode: Option<String>,
    /// Its blob in the base
    pub old_id: Option<String>,
    /// Its blob in the worktree
    pub new_id: Option<String>,
    /// Whether either side is binary, in which case it has no hunks
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
    /// Whether its hunks were too large to keep
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub too_large: bool,
    /// How many lines it added
    pub added: u32,
    /// How many lines it removed
    pub removed: u32,
}

impl FileSummary {
    /// The path the file is shown under: its worktree path, or its base path once deleted
    pub fn path(&self) -> &str {
        // as a file's diff names it
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or_default()
    }
}

/// One capture of a node's changes, as the service keeps it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    /// The node whose changes these are
    pub node: NodeName,
    /// Its number among the node's revisions, from 1, which no restart of the node resets
    pub revision: u32,
    /// When the service stored it, in milliseconds since the epoch
    pub at: u64,
    /// What the capture read: the head, the worktree's tree, the merge-base and the base
    #[serde(flatten)]
    pub shas: CaptureShas,
    /// The files that conflicted carrying the start forward, whose diff may hold how an earlier
    /// node's change was resolved, not only this node's
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// Every changed file, by path, without its hunks
    pub files: Vec<FileSummary>,
}

impl Revision {
    /// Counts the lines added and removed across every file
    pub fn counts(&self) -> (u32, u32) {
        // each file's counts, summed
        self.files.iter().fold((0, 0), |(added, removed), file| {
            (added + file.added, removed + file.removed)
        })
    }
}

/// The side of a diff a line is numbered on
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    /// The base, where a removed line is
    Old,
    /// The worktree, where an added or a context line is
    New,
}

/// Where a line of a node's changes is, for a comment to name: its revision, file and line
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    /// The revision the line is in
    pub revision: u32,
    /// The path its file is shown under
    pub path: String,
    /// The side its number counts on
    pub side: Side,
    /// Its 1-based number on that side
    pub line: u32,
}

impl Anchor {
    /// The anchor of one line of a file in a revision
    ///
    /// A removed line is only in the base, so it is anchored there. Every other line is in the
    /// worktree, and anchored where it is now.
    ///
    /// # Arguments
    ///
    /// * `revision` - The revision the line is in
    /// * `path` - The path its file is shown under
    /// * `line` - The line
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::revision::{Anchor, Line, LineKind, Side};
    ///
    /// let added = Line {
    ///     kind: LineKind::Added,
    ///     old: None,
    ///     new: Some(12),
    ///     text: "let x = 1;".to_string(),
    ///     no_newline: false,
    /// };
    /// let anchor = Anchor::of(3, "src/lib.rs", &added).unwrap();
    /// assert_eq!((anchor.side, anchor.line), (Side::New, 12));
    /// ```
    pub fn of(revision: u32, path: &str, line: &Line) -> Option<Anchor> {
        // the side the line is on, and its number there
        let (side, number) = match line.kind {
            LineKind::Removed => (Side::Old, line.old?),
            LineKind::Added | LineKind::Context => (Side::New, line.new?),
        };
        Some(Anchor {
            revision,
            path: path.to_string(),
            side,
            line: number,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Anchor, FileDiff, FileStatus, Hunk, Line, LineKind, Revision, Side};
    use crate::work::CaptureShas;

    /// A line of a kind, numbered on the sides that kind is on
    ///
    /// # Arguments
    ///
    /// * `kind` - The line's kind
    /// * `old` - Its number in the base
    /// * `new` - Its number in the worktree
    fn line(kind: LineKind, old: Option<u32>, new: Option<u32>) -> Line {
        Line {
            kind,
            old,
            new,
            text: "x".to_string(),
            no_newline: false,
        }
    }

    /// A modified file with one hunk: one line removed, two added, one of context
    fn modified() -> FileDiff {
        FileDiff {
            status: FileStatus::Modified,
            old_path: Some("a.rs".to_string()),
            new_path: Some("a.rs".to_string()),
            old_mode: Some("100644".to_string()),
            new_mode: Some("100644".to_string()),
            old_id: Some("a".repeat(40)),
            new_id: Some("b".repeat(40)),
            binary: false,
            too_large: false,
            hunks: vec![Hunk {
                old_start: 1,
                old_lines: 2,
                new_start: 1,
                new_lines: 3,
                lines: vec![
                    line(LineKind::Context, Some(1), Some(1)),
                    line(LineKind::Removed, Some(2), None),
                    line(LineKind::Added, None, Some(2)),
                    line(LineKind::Added, None, Some(3)),
                ],
            }],
        }
    }

    /// A file's summary keeps everything but its hunks, and counts what they changed
    #[test]
    fn summaries_count_what_hunks_changed() {
        let file = modified();
        let summary = file.summary();
        assert_eq!((summary.added, summary.removed), (2, 1));
        assert_eq!(summary.path(), "a.rs");
        assert_eq!(summary.new_id, file.new_id);
        // and a revision sums its files
        let revision = Revision {
            node: "reef/central/C32".parse().unwrap(),
            revision: 1,
            at: 5,
            shas: CaptureShas {
                head: "c".repeat(40),
                tree: "d".repeat(40),
                merge_base: None,
                base: "e".repeat(40),
            },
            conflicts: Vec::new(),
            files: vec![summary.clone(), summary],
        };
        assert_eq!(revision.counts(), (4, 2));
    }

    /// Kinds are written in kebab-case, flags only when set, and a revision's SHAs beside its
    /// number
    #[test]
    fn revisions_read_back_as_written() {
        // a file's flags are left out while they are false
        let file = modified();
        let json = serde_json::to_value(&file).unwrap();
        assert_eq!(json["status"], "modified");
        assert_eq!(json["hunks"][0]["lines"][1]["kind"], "removed");
        assert!(json.get("binary").is_none());
        assert!(json.get("too_large").is_none());
        assert!(json["hunks"][0]["lines"][0].get("no_newline").is_none());
        assert_eq!(serde_json::from_value::<FileDiff>(json).unwrap(), file);
        // a revision's SHAs sit beside its number, and its conflicts only when there are some
        let revision = Revision {
            node: "reef/central/C32".parse().unwrap(),
            revision: 2,
            at: 9,
            shas: CaptureShas {
                head: "c".repeat(40),
                tree: "d".repeat(40),
                merge_base: Some("f".repeat(40)),
                base: "e".repeat(40),
            },
            conflicts: vec!["b.rs".to_string()],
            files: vec![file.summary()],
        };
        let json = serde_json::to_value(&revision).unwrap();
        assert_eq!(json["node"], "reef/central/C32");
        assert_eq!(json["base"], "e".repeat(40));
        assert_eq!(json["conflicts"][0], "b.rs");
        assert_eq!(json["files"][0]["added"], 2);
        assert_eq!(serde_json::from_value::<Revision>(json).unwrap(), revision);
    }

    /// A removed line is anchored in the base, and every other line where it is now
    #[test]
    fn lines_are_anchored_on_their_side() {
        let file = modified();
        let anchors: Vec<(Side, u32)> = file.hunks[0]
            .lines
            .iter()
            .filter_map(|line| Anchor::of(4, file.path(), line))
            .map(|anchor| (anchor.side, anchor.line))
            .collect();
        assert_eq!(
            anchors,
            [
                (Side::New, 1),
                (Side::Old, 2),
                (Side::New, 2),
                (Side::New, 3)
            ]
        );
        // and the anchor names its revision and file
        let first = Anchor::of(4, "a.rs", &file.hunks[0].lines[0]).unwrap();
        assert_eq!((first.revision, first.path.as_str()), (4, "a.rs"));
        // a line missing its side's number has no anchor
        assert!(Anchor::of(4, "a.rs", &line(LineKind::Added, None, None)).is_none());
    }
}
