//! git's CLI as the oracle for reef's gix steps: a diff and a merge of trees, as git takes them
//!
//! Both of reef's suites include this file by path, the git module's own and the service's CLI
//! cases, so the one parser checks every diff reef keeps. Paths and blob ids come from the raw
//! half of `git diff --raw -z -p`, never from a patch's headers, so a quoted path can't confuse
//! it. Each command runs from its directory alone, with no one's config.

// each suite that includes this uses only the part of the oracle it checks against
#![allow(dead_code)]

use reef_api::{FileDiff, FileStatus, Hunk, Line, LineKind};
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// How `git diff` is asked for a diff gix can match: renames at 50%, Myers with the indent
/// heuristic, three lines of context, and every file's record before its patch
const DIFF_ARGS: &[&str] = &[
    "diff",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--find-renames=50%",
    "--diff-algorithm=myers",
    "--indent-heuristic",
    "--unified=3",
    "--raw",
    "-z",
    "-p",
    "--no-abbrev",
    "--full-index",
];

/// The all-zero id git writes for a side that isn't there
const NULL_ID: &str = "0000000000000000000000000000000000000000";

/// The mode git writes for a side that isn't there
const NULL_MODE: &str = "000000";

/// Runs git in a directory with no one's config, returning what it wrote and how it exited
///
/// # Arguments
///
/// * `dir` - The directory to run in
/// * `args` - Git's arguments
fn git(dir: &Path, args: &[&str]) -> Output {
    // from the directory alone, in the C locale, with neither the user's nor the system's config
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
    ] {
        command.env_remove(var);
    }
    command.stdin(Stdio::null()).output().expect("git runs")
}

/// The diff git's CLI shows between two trees or commits, its files sorted by path
///
/// # Arguments
///
/// * `dir` - A worktree of the repo both are in
/// * `base` - The tree or commit the diff starts from
/// * `head` - The tree or commit it ends at
pub fn git_diff(dir: &Path, base: &str, head: &str) -> Vec<FileDiff> {
    // one command gives every file's record and its patch
    let mut args = DIFF_ARGS.to_vec();
    args.extend([base, head]);
    let output = git(dir, &args);
    assert!(
        output.status.success(),
        "git diff in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut files = parse(&output.stdout);
    files.sort_by(|a, b| a.path().cmp(b.path()));
    files
}

/// Merges two trees against a base tree as `git merge-tree --write-tree` does: the tree it wrote
/// and the paths that conflicted, sorted
///
/// # Arguments
///
/// * `dir` - A worktree of the repo the trees are in
/// * `base` - The tree both sides started from
/// * `ours` - One side
/// * `theirs` - The other side
pub fn git_merge_tree(dir: &Path, base: &str, ours: &str, theirs: &str) -> (String, Vec<String>) {
    // `--merge-base` lets all three be trees; exit 1 means it conflicted, still with a tree
    let merge_base = format!("--merge-base={base}");
    let args = [
        "merge-tree",
        "--write-tree",
        "--no-messages",
        "--name-only",
        merge_base.as_str(),
        ours,
        theirs,
    ];
    let output = git(dir, &args);
    let conflicted = match output.status.code() {
        Some(0) => false,
        Some(1) => true,
        _ => panic!(
            "git merge-tree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    };
    // the tree is the first line, and the conflicted files follow it, a line each
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    let tree = lines.next().unwrap_or_default().trim().to_string();
    let mut conflicts: Vec<String> = match conflicted {
        true => lines
            .take_while(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
        false => Vec::new(),
    };
    conflicts.sort();
    conflicts.dedup();
    (tree, conflicts)
}

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
    /// A rename's destination
    path2: Option<String>,
}

/// Reads a NUL-terminated field, moving past it
///
/// # Arguments
///
/// * `out` - The raw output
/// * `at` - Where the field starts, moved past its NUL
fn field(out: &[u8], at: &mut usize) -> String {
    // the field runs to its terminating NUL
    let len = out[*at..]
        .iter()
        .position(|byte| *byte == 0)
        .expect("git's raw output ended mid-field");
    let text = String::from_utf8_lossy(&out[*at..*at + len]).into_owned();
    *at += len + 1;
    text
}

/// Parses the raw records at the front of `git diff --raw -z -p`, returning where the patch starts
///
/// # Arguments
///
/// * `out` - The command's whole output
fn parse_raw(out: &[u8]) -> (Vec<RawEntry>, usize) {
    let mut entries = Vec::new();
    let mut at = 0;
    // every record starts with a colon; the patch, or nothing, follows the last
    while out.get(at) == Some(&b':') {
        // `:old_mode new_mode old_id new_id status`, then its paths
        let meta = field(out, &mut at);
        let parts: Vec<&str> = meta[1..].split(' ').collect();
        let [old_mode, new_mode, old_id, new_id, status] = parts[..] else {
            panic!("an unexpected raw record: {meta}");
        };
        let path = field(out, &mut at);
        // a rename or a copy names its destination too
        let path2 =
            (status.starts_with('R') || status.starts_with('C')).then(|| field(out, &mut at));
        entries.push(RawEntry {
            old_mode: old_mode.to_string(),
            new_mode: new_mode.to_string(),
            old_id: old_id.to_string(),
            new_id: new_id.to_string(),
            status: status.to_string(),
            path,
            path2,
        });
    }
    // with -z, a NUL rather than a newline sets the patch apart from the records
    if out.get(at) == Some(&0) {
        at += 1;
    }
    (entries, at)
}

/// Parses a hunk header such as `@@ -2,7 +2,7 @@ fn name`
///
/// # Arguments
///
/// * `line` - The header line
fn parse_header(line: &str) -> (u32, u32, u32, u32) {
    // one side's `start[,len]`, the length 1 when left out
    let side = |text: &str| -> (u32, u32) {
        let mut parts = text.splitn(2, ',');
        let start = parts.next().unwrap_or_default().parse().expect("a start");
        let len = parts.next().map_or(1, |len| len.parse().expect("a length"));
        (start, len)
    };
    // `@@ -old +new @@`, with anything after the second `@@` ignored
    let mut words = line.split(' ');
    let (Some("@@"), Some(old), Some(new)) = (words.next(), words.next(), words.next()) else {
        panic!("an unexpected hunk header: {line}");
    };
    let (old_start, old_lines) = side(old.trim_start_matches('-'));
    let (new_start, new_lines) = side(new.trim_start_matches('+'));
    (old_start, old_lines, new_start, new_lines)
}

/// Parses one file's section of a patch into its hunks, and whether git called it binary
///
/// # Arguments
///
/// * `section` - The section's lines, from its `diff --git` line on
fn parse_section(section: &[&str]) -> (bool, Vec<Hunk>) {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut binary = false;
    // the next line numbers on each side, inside the current hunk
    let mut old_no = 0;
    let mut new_no = 0;
    for line in section {
        // a new hunk starts its numbering from its header
        if line.starts_with("@@ ") {
            let (old_start, old_lines, new_start, new_lines) = parse_header(line);
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
            text: text.to_string(),
            no_newline: false,
        });
    }
    (binary, hunks)
}

/// Parses the output of `git diff --raw -z -p --no-abbrev` into its files
///
/// The raw records name each file's paths, modes and blobs. The patch that follows has one
/// `diff --git` section per record, in the same order, which gives each file its hunks.
///
/// # Arguments
///
/// * `out` - The command's whole output
fn parse(out: &[u8]) -> Vec<FileDiff> {
    // the records first, then the patch after them
    let (entries, patch_at) = parse_raw(out);
    let patch = String::from_utf8_lossy(&out[patch_at..]);
    let lines: Vec<&str> = patch.split('\n').collect();
    // cut the patch into its files' sections
    let mut starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("diff --git "))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        starts.len(),
        entries.len(),
        "git's patch and its raw output list different files"
    );
    starts.push(lines.len());
    // join each record to its section
    let mut files = Vec::with_capacity(entries.len());
    for (index, entry) in entries.into_iter().enumerate() {
        let (binary, hunks) = parse_section(&lines[starts[index]..starts[index + 1]]);
        // a side that isn't there has a null mode and id
        let side = |mode: String, id: String| match mode == NULL_MODE {
            true => (None, None),
            false => (Some(mode), Some(id).filter(|id| id != NULL_ID)),
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
            _ => panic!("an unexpected status: {}", entry.status),
        };
        files.push(FileDiff {
            status,
            old_path,
            new_path,
            old_mode,
            new_mode,
            old_id,
            new_id,
            binary,
            too_large: false,
            hunks,
        });
    }
    files
}
