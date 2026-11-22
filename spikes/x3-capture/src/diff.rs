//! A captured diff: files, hunks and lines, the same model whichever reader produced it
//!
//! Both git's CLI and gix produce this, so the two can be compared field by field. Paths and
//! blob ids come from the raw half of git's output, never from parsing a patch's headers, so a
//! quoted or tab-suffixed path in the patch can't confuse it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What happened to one file between the base and the head
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileStatus {
    /// The file is new in the head
    Added,
    /// The file is gone from the head
    Deleted,
    /// The file is in both, with new contents or a new mode
    Modified,
    /// The file moved, and its contents may have changed with it
    Renamed,
}

/// The kind of one line in a hunk
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineKind {
    /// In both the base and the head, shown around a change
    Context,
    /// Only in the head
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
    /// Its 1-based number in the head, if it is there
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
    /// Where the hunk starts in the base, 0 when the base side is empty
    pub old_start: u32,
    /// How many base lines it spans
    pub old_lines: u32,
    /// Where the hunk starts in the head, 0 when the head side is empty
    pub new_start: u32,
    /// How many head lines it spans
    pub new_lines: u32,
    /// Its lines, in order
    pub lines: Vec<Line>,
}

/// One file's changes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    /// What happened to the file
    pub status: FileStatus,
    /// Its path in the base, if it was there
    pub old_path: Option<String>,
    /// Its path in the head, if it is there
    pub new_path: Option<String>,
    /// Its mode in the base, such as 100644
    pub old_mode: Option<String>,
    /// Its mode in the head
    pub new_mode: Option<String>,
    /// Its blob in the base
    pub old_id: Option<String>,
    /// Its blob in the head
    pub new_id: Option<String>,
    /// Whether git took either side to be binary, in which case it has no hunks
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
    /// Its hunks, in order
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    /// The path the file is shown under: its head path, or its base path once deleted
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or_default()
    }
}

/// The blobs a path held on each side, which is what says whether a diff is the right one
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sides {
    /// The blob it held in the base, if any
    pub old: Option<String>,
    /// The blob it holds in the head, if any
    pub new: Option<String>,
}

/// A whole captured diff
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    /// Every changed file, in git's order
    pub files: Vec<FileDiff>,
}

impl Diff {
    /// The blobs each touched path held on each side
    ///
    /// A rename touches two paths: its source loses its blob and its destination gains one.
    /// Two diffs with the same map changed the same files to the same contents, whatever their
    /// hunks look like.
    pub fn sides(&self) -> BTreeMap<String, Sides> {
        let mut sides: BTreeMap<String, Sides> = BTreeMap::new();
        for file in &self.files {
            // the base side, under the path it had there
            if let Some(path) = &file.old_path {
                sides.entry(path.clone()).or_default().old = file.old_id.clone();
            }
            // the head side, under the path it has now
            if let Some(path) = &file.new_path {
                sides.entry(path.clone()).or_default().new = file.new_id.clone();
            }
        }
        sides
    }

    /// The same diff with its files sorted by path, to compare readers that list them apart
    ///
    /// git lists files in its tree order and gix in its own, so two readers agree when their
    /// sorted diffs are equal.
    pub fn sorted(&self) -> Diff {
        let mut files = self.files.clone();
        files.sort_by(|a, b| {
            a.path()
                .cmp(b.path())
                .then_with(|| a.old_path.cmp(&b.old_path))
        });
        Diff { files }
    }

    /// Counts the lines added and removed across every file
    pub fn counts(&self) -> (usize, usize) {
        let mut added = 0;
        let mut removed = 0;
        for line in self
            .files
            .iter()
            .flat_map(|file| &file.hunks)
            .flat_map(|hunk| &hunk.lines)
        {
            // context lines count for neither
            match line.kind {
                LineKind::Added => added += 1,
                LineKind::Removed => removed += 1,
                LineKind::Context => {}
            }
        }
        (added, removed)
    }
}

/// The all-zero id git writes for a side that isn't there
const NULL_ID: &str = "0000000000000000000000000000000000000000";

/// The mode git writes for a side that isn't there
const NULL_MODE: &str = "000000";

/// One record of `git diff --raw -z`, before its patch is joined to it
struct RawEntry {
    /// The base side's mode
    old_mode: String,
    /// The head side's mode
    new_mode: String,
    /// The base side's blob
    old_id: String,
    /// The head side's blob
    new_id: String,
    /// The status letter, such as M, or R with its score
    status: String,
    /// The first path: the only one, or a rename's source
    path: String,
    /// A rename's or copy's destination
    path2: Option<String>,
}

/// Reads a NUL-terminated field, moving past it
///
/// # Arguments
///
/// * `out` - The raw output
/// * `at` - Where the field starts, moved past its NUL
fn field(out: &[u8], at: &mut usize) -> anyhow::Result<String> {
    // find the field's terminating NUL
    let len = out[*at..]
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| anyhow::anyhow!("git's raw output ended mid-field"))?;
    let text = String::from_utf8_lossy(&out[*at..*at + len]).into_owned();
    *at += len + 1;
    Ok(text)
}

/// Parses the raw records at the front of `git diff --raw -z -p`, returning where the patch starts
///
/// # Arguments
///
/// * `out` - The command's whole output
fn parse_raw(out: &[u8]) -> anyhow::Result<(Vec<RawEntry>, usize)> {
    let mut entries = Vec::new();
    let mut at = 0;
    // every record starts with a colon; the patch, or nothing, follows the last
    while out.get(at) == Some(&b':') {
        // `:old_mode new_mode old_id new_id status`, then its paths
        let meta = field(out, &mut at)?;
        let parts: Vec<&str> = meta[1..].split(' ').collect();
        let [old_mode, new_mode, old_id, new_id, status] = parts[..] else {
            anyhow::bail!("an unexpected raw record: {meta}");
        };
        let path = field(out, &mut at)?;
        // a rename or a copy names its destination too
        let path2 = if status.starts_with('R') || status.starts_with('C') {
            Some(field(out, &mut at)?)
        } else {
            None
        };
        entries.push(RawEntry {
            old_mode: old_mode.to_owned(),
            new_mode: new_mode.to_owned(),
            old_id: old_id.to_owned(),
            new_id: new_id.to_owned(),
            status: status.to_owned(),
            path,
            path2,
        });
    }
    // with -z, a NUL rather than a newline sets the patch apart from the records
    if out.get(at) == Some(&0) {
        at += 1;
    }
    Ok((entries, at))
}

/// Parses a hunk header such as `@@ -2,7 +2,7 @@ fn name`
///
/// # Arguments
///
/// * `line` - The header line
fn parse_header(line: &str) -> anyhow::Result<(u32, u32, u32, u32)> {
    // one side's `start[,len]`, the length 1 when left out
    let side = |text: &str| -> anyhow::Result<(u32, u32)> {
        let mut parts = text.splitn(2, ',');
        let start = parts.next().unwrap_or_default().parse()?;
        let len = match parts.next() {
            Some(len) => len.parse()?,
            None => 1,
        };
        Ok((start, len))
    };
    // `@@ -old +new @@`, with anything after the second `@@` ignored
    let mut words = line.split(' ');
    let (Some("@@"), Some(old), Some(new)) = (words.next(), words.next(), words.next()) else {
        anyhow::bail!("an unexpected hunk header: {line}");
    };
    let (old_start, old_lines) = side(old.trim_start_matches('-'))?;
    let (new_start, new_lines) = side(new.trim_start_matches('+'))?;
    Ok((old_start, old_lines, new_start, new_lines))
}

/// Parses one file's section of a patch into its hunks, and whether git called it binary
///
/// # Arguments
///
/// * `section` - The section's lines, from its `diff --git` line on
fn parse_section(section: &[&str]) -> anyhow::Result<(bool, Vec<Hunk>)> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut binary = false;
    // the next line numbers on each side, inside the current hunk
    let mut old_no = 0;
    let mut new_no = 0;
    for line in section {
        // a new hunk starts its numbering from its header
        if line.starts_with("@@ ") {
            let (old_start, old_lines, new_start, new_lines) = parse_header(line)?;
            hunks.push(Hunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
                lines: Vec::new(),
            });
            old_no = old_start.max(1);
            new_no = new_start.max(1);
            continue;
        }
        // headers come before the first hunk; a binary file has no hunks at all
        let Some(hunk) = hunks.last_mut() else {
            if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
                binary = true;
            }
            continue;
        };
        // a line's kind is its first character
        let (kind, text) = match line.as_bytes().first() {
            Some(b' ') => (LineKind::Context, &line[1..]),
            Some(b'+') => (LineKind::Added, &line[1..]),
            Some(b'-') => (LineKind::Removed, &line[1..]),
            // `\ No newline at end of file` marks the line before it
            Some(b'\\') => {
                if let Some(last) = hunk.lines.last_mut() {
                    last.no_newline = true;
                }
                continue;
            }
            // an empty line only appears at the very end of git's output
            _ => continue,
        };
        // number the line on each side it is on
        let (old, new) = match kind {
            LineKind::Context => (Some(old_no), Some(new_no)),
            LineKind::Added => (None, Some(new_no)),
            LineKind::Removed => (Some(old_no), None),
        };
        if old.is_some() {
            old_no += 1;
        }
        if new.is_some() {
            new_no += 1;
        }
        hunk.lines.push(Line {
            kind,
            old,
            new,
            text: text.to_owned(),
            no_newline: false,
        });
    }
    Ok((binary, hunks))
}

/// Parses the output of `git diff --raw -z -p --no-abbrev` into a diff
///
/// The raw records name each file's paths, modes and blobs. The patch that follows has one
/// `diff --git` section per record, in the same order, which gives each file its hunks.
///
/// # Arguments
///
/// * `out` - The command's whole output
pub fn parse_git(out: &[u8]) -> anyhow::Result<Diff> {
    // the records first, then the patch after them
    let (entries, patch_at) = parse_raw(out)?;
    let patch = String::from_utf8_lossy(&out[patch_at..]);
    let lines: Vec<&str> = patch.split('\n').collect();
    // cut the patch into its files' sections
    let mut starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("diff --git "))
        .map(|(index, _)| index)
        .collect();
    if starts.len() != entries.len() {
        anyhow::bail!(
            "git's patch has {} files but its raw output {}",
            starts.len(),
            entries.len()
        );
    }
    starts.push(lines.len());
    // join each record to its section
    let mut files = Vec::with_capacity(entries.len());
    for (index, entry) in entries.into_iter().enumerate() {
        let (binary, hunks) = parse_section(&lines[starts[index]..starts[index + 1]])?;
        // a side that isn't there has a null mode and id
        let side = |mode: String, id: String| {
            if mode == NULL_MODE {
                (None, None)
            } else {
                (Some(mode), Some(id))
            }
        };
        let (old_mode, old_id) = side(entry.old_mode, entry.old_id);
        let (new_mode, new_id) = side(entry.new_mode, entry.new_id);
        let (status, old_path, new_path) = match entry.status.as_bytes().first() {
            Some(b'A') => (FileStatus::Added, None, Some(entry.path)),
            Some(b'D') => (FileStatus::Deleted, Some(entry.path), None),
            Some(b'R') => (FileStatus::Renamed, Some(entry.path), entry.path2),
            Some(b'M') | Some(b'T') => (
                FileStatus::Modified,
                Some(entry.path.clone()),
                Some(entry.path),
            ),
            _ => anyhow::bail!("an unexpected status: {}", entry.status),
        };
        files.push(FileDiff {
            status,
            old_path,
            new_path,
            old_mode,
            new_mode,
            old_id: old_id.filter(|id| id != NULL_ID),
            new_id: new_id.filter(|id| id != NULL_ID),
            binary,
            hunks,
        });
    }
    Ok(Diff { files })
}
