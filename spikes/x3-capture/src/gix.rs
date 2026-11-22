//! Every step of a capture but the snapshot, through gix, to weigh against git's CLI
//!
//! - **The diff**: a tree-to-tree diff with renames at 50%, then each changed blob is diffed
//!   with imara's Myers and git's slider heuristics, cut into hunks with three lines of
//!   context. Hunk starts follow git's convention: an empty side starts at the line before
//!   it, 0 at a file's top.
//! - **The carry**: a three-way merge of trees, labelled as `git merge-tree` labels it.
//! - **merge-base and ancestry**, from gix's revision walk.
//!
//! gix 0.90 can't stage a worktree, so the snapshot stays `git add -A` and `git write-tree`.
//! Everything meets at tree ids in the one object store, so gix reads the tree git wrote.

use gix::diff::blob::unified_diff::{ConsumeHunk, ContextSize, DiffLineKind, HunkHeader};
use gix::diff::blob::{Algorithm, InternedInput, UnifiedDiff, diff_with_slider_heuristics};
use gix::object::tree::diff::ChangeDetached;
use gix::objs::tree::EntryMode;
use std::path::Path;
use tracing::instrument;

use crate::diff::{Diff, FileDiff, FileStatus, Hunk, Line, LineKind};
use crate::git::Merged;

/// How many bytes git looks through for a NUL to call a blob binary
const BINARY_PROBE: usize = 8000;

/// Gathers the hunks UnifiedDiff hands over into the captured model
#[derive(Default)]
struct Hunks {
    /// The hunks so far
    hunks: Vec<Hunk>,
}

impl ConsumeHunk for Hunks {
    type Out = Vec<Hunk>;

    /// Numbers one hunk's lines on each side and keeps it
    ///
    /// # Arguments
    ///
    /// * `header` - Where the hunk starts and how long it is on each side
    /// * `lines` - Its lines, each with its kind and its bytes, newline included
    fn consume_hunk(
        &mut self,
        header: HunkHeader,
        lines: &[(DiffLineKind, &[u8])],
    ) -> std::io::Result<()> {
        // git starts an empty side at the line before it
        let start = |start: u32, len: u32| if len == 0 { start - 1 } else { start };
        let old_start = start(header.before_hunk_start, header.before_hunk_len);
        let new_start = start(header.after_hunk_start, header.after_hunk_len);
        // number each line from where its sides start
        let mut old_no = old_start.max(1);
        let mut new_no = new_start.max(1);
        let mut out = Vec::with_capacity(lines.len());
        for (kind, bytes) in lines {
            let kind = match kind {
                DiffLineKind::Context => LineKind::Context,
                DiffLineKind::Add => LineKind::Added,
                DiffLineKind::Remove => LineKind::Removed,
            };
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
            // a token keeps its newline, and the last line of a file may have none
            let no_newline = !bytes.ends_with(b"\n");
            let text = bytes.strip_suffix(b"\n").unwrap_or(bytes);
            out.push(Line {
                kind,
                old,
                new,
                text: String::from_utf8_lossy(text).into_owned(),
                no_newline,
            });
        }
        self.hunks.push(Hunk {
            old_start,
            old_lines: header.before_hunk_len,
            new_start,
            new_lines: header.after_hunk_len,
            lines: out,
        });
        Ok(())
    }

    /// Hands back every hunk
    fn finish(self) -> Self::Out {
        self.hunks
    }
}

/// A side of a changed entry: its mode and its blob
struct Side {
    /// The entry's mode
    mode: EntryMode,
    /// The blob it held
    id: gix::ObjectId,
}

/// The mode git writes for an entry, such as 100644
///
/// # Arguments
///
/// * `mode` - The entry's mode
fn octal(mode: EntryMode) -> String {
    mode.kind().as_octal_str().to_string()
}

/// Whether git would call a blob binary: a NUL in its first 8000 bytes
///
/// # Arguments
///
/// * `data` - The blob's contents
fn is_binary(data: &[u8]) -> bool {
    data[..data.len().min(BINARY_PROBE)].contains(&0)
}

/// Diffs one file's two sides into hunks, or none when either is binary
///
/// # Arguments
///
/// * `repo` - The repo the blobs are in
/// * `old` - The base side, if any
/// * `new` - The head side, if any
fn hunks(
    repo: &gix::Repository,
    old: Option<&Side>,
    new: Option<&Side>,
) -> anyhow::Result<(bool, Vec<Hunk>)> {
    // a missing side is empty
    let read = |side: Option<&Side>| -> anyhow::Result<Vec<u8>> {
        match side {
            Some(side) => Ok(repo.find_blob(side.id)?.detach().data),
            None => Ok(Vec::new()),
        }
    };
    let before = read(old)?;
    let after = read(new)?;
    // git shows a binary file as a line saying it changed, and no hunks
    if is_binary(&before) || is_binary(&after) {
        return Ok((true, Vec::new()));
    }
    // identical contents, as a mode change or a pure rename leaves, have no hunks
    if before == after {
        return Ok((false, Vec::new()));
    }
    // a line diff, with git's slider heuristics, cut into hunks with three lines around each
    let input = InternedInput::new(before.as_slice(), after.as_slice());
    let diff = diff_with_slider_heuristics(Algorithm::Myers, &input);
    let hunks =
        UnifiedDiff::new(&diff, &input, Hunks::default(), ContextSize::symmetrical(3)).consume()?;
    Ok((false, hunks))
}

/// Turns one of gix's tree changes into a captured file, if it is a file at all
///
/// # Arguments
///
/// * `repo` - The repo the blobs are in
/// * `change` - The change to turn
fn file(repo: &gix::Repository, change: ChangeDetached) -> anyhow::Result<Option<FileDiff>> {
    // the change's kind, paths and sides
    let (status, old_path, new_path, old, new) = match change {
        ChangeDetached::Addition {
            location,
            entry_mode,
            id,
            ..
        } => (
            FileStatus::Added,
            None,
            Some(location.to_string()),
            None,
            Some(Side {
                mode: entry_mode,
                id,
            }),
        ),
        ChangeDetached::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => (
            FileStatus::Deleted,
            Some(location.to_string()),
            None,
            Some(Side {
                mode: entry_mode,
                id,
            }),
            None,
        ),
        ChangeDetached::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => (
            FileStatus::Modified,
            Some(location.to_string()),
            Some(location.to_string()),
            Some(Side {
                mode: previous_entry_mode,
                id: previous_id,
            }),
            Some(Side {
                mode: entry_mode,
                id,
            }),
        ),
        ChangeDetached::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            location,
            ..
        } => (
            FileStatus::Renamed,
            Some(source_location.to_string()),
            Some(location.to_string()),
            Some(Side {
                mode: source_entry_mode,
                id: source_id,
            }),
            Some(Side {
                mode: entry_mode,
                id,
            }),
        ),
    };
    // trees are walked into and submodules left alone, as git's diff of files does
    let is_file = |side: &Option<Side>| {
        side.as_ref()
            .is_none_or(|side| side.mode.is_blob_or_symlink())
    };
    if !is_file(&old) || !is_file(&new) {
        return Ok(None);
    }
    let (binary, hunks) = hunks(repo, old.as_ref(), new.as_ref())?;
    Ok(Some(FileDiff {
        status,
        old_path,
        new_path,
        old_mode: old.as_ref().map(|side| octal(side.mode)),
        new_mode: new.as_ref().map(|side| octal(side.mode)),
        old_id: old.as_ref().map(|side| side.id.to_string()),
        new_id: new.as_ref().map(|side| side.id.to_string()),
        binary,
        hunks,
    }))
}

/// Opens the repo a checkout is in
///
/// # Arguments
///
/// * `dir` - The checkout, or any directory in it
pub fn open(dir: &Path) -> anyhow::Result<gix::Repository> {
    Ok(gix::discover(dir)?)
}

/// The diff between two trees or commits, through gix, in an open repo
///
/// # Arguments
///
/// * `repo` - The repo both are in
/// * `base` - The full id of the tree or commit the diff starts from
/// * `head` - The full id of the tree or commit it ends at
#[instrument(name = "gix::diff", skip(repo), err(Debug))]
pub fn diff(repo: &gix::Repository, base: &str, head: &str) -> anyhow::Result<Diff> {
    // either side may be a commit, which is peeled to its tree
    let tree = |id: &str| -> anyhow::Result<gix::Tree<'_>> {
        let id = gix::ObjectId::from_hex(id.as_bytes())?;
        Ok(repo.find_object(id)?.peel_to_tree()?)
    };
    let base = tree(base)?;
    let head = tree(head)?;
    // paths tracked in full, and renames found as git finds them by default
    let mut options = gix::diff::Options::default();
    options
        .track_path()
        .track_rewrites(Some(gix::diff::Rewrites {
            copies: None,
            percentage: Some(0.5),
            limit: 1000,
            track_empty: false,
        }));
    let changes = repo.diff_tree_to_tree(&base, &head, options)?;
    // each change that is a file, in the order git would list it
    let mut files = Vec::with_capacity(changes.len());
    for change in changes {
        if let Some(file) = file(repo, change)? {
            files.push(file);
        }
    }
    files.sort_by(|a, b| a.path().cmp(b.path()));
    Ok(Diff { files })
}

/// Parses a full id
///
/// # Arguments
///
/// * `id` - The id, in hex
fn id(id: &str) -> anyhow::Result<gix::ObjectId> {
    Ok(gix::ObjectId::from_hex(id.trim().as_bytes())?)
}

/// The commit HEAD is at, if any
///
/// # Arguments
///
/// * `repo` - The checkout's repo
pub fn head(repo: &gix::Repository) -> anyhow::Result<Option<String>> {
    // an unborn HEAD has no commit to name
    match repo.head_id() {
        Ok(head) => Ok(Some(head.detach().to_string())),
        Err(_) => Ok(None),
    }
}

/// The commit a revision such as a branch's name resolves to, if it resolves
///
/// # Arguments
///
/// * `repo` - The repo it is in
/// * `rev` - The revision, such as `main`
pub fn commit(repo: &gix::Repository, rev: &str) -> anyhow::Result<Option<String>> {
    // a name that resolves to nothing is an answer, not a failure
    let Ok(found) = repo.rev_parse_single(rev) else {
        return Ok(None);
    };
    Ok(Some(found.object()?.peel_to_commit()?.id.to_string()))
}

/// The tree a commit or tree holds
///
/// # Arguments
///
/// * `repo` - The repo it is in
/// * `rev` - The full id of the commit or tree
pub fn tree(repo: &gix::Repository, rev: &str) -> anyhow::Result<String> {
    Ok(repo.find_object(id(rev)?)?.peel_to_tree()?.id.to_string())
}

/// The best common ancestor of two commits, if they have one
///
/// # Arguments
///
/// * `repo` - The repo both are in
/// * `a` - One commit's full id
/// * `b` - The other's
pub fn merge_base(repo: &gix::Repository, a: &str, b: &str) -> anyhow::Result<Option<String>> {
    Ok(repo
        .merge_base(id(a)?, id(b)?)?
        .map(|base| base.detach().to_string()))
}

/// Whether `ancestor` is `head` or one of its ancestors
///
/// An ancestor of the head is the one best common ancestor of the two, so this asks whether
/// their merge-base is the ancestor itself.
///
/// # Arguments
///
/// * `repo` - The repo both are in
/// * `ancestor` - The commit that may be an ancestor
/// * `head` - The commit whose history is searched
pub fn is_ancestor(repo: &gix::Repository, ancestor: &str, head: &str) -> anyhow::Result<bool> {
    let ancestor = id(ancestor)?;
    Ok(repo
        .merge_base(ancestor, id(head)?)?
        .is_some_and(|base| base.detach() == ancestor))
}

/// Merges two trees against a base tree, as `git merge-tree --write-tree --merge-base` does
///
/// Conflicted files hold markers labelled with each side's tree id and no ancestor label, as
/// git's do, so a conflicted merge writes the same tree.
///
/// # Arguments
///
/// * `repo` - The repo the trees are in
/// * `base` - The tree both sides started from
/// * `ours` - One side
/// * `theirs` - The other side
#[instrument(name = "gix::merge_trees", skip(repo), err(Debug))]
pub fn merge_trees(
    repo: &gix::Repository,
    base: &str,
    ours: &str,
    theirs: &str,
) -> anyhow::Result<Merged> {
    // label each side with its tree id, as git labels a merge of bare trees
    let labels = gix::merge::blob::builtin_driver::text::Labels {
        ancestor: None,
        current: Some(ours.into()),
        other: Some(theirs.into()),
    };
    let options = repo.tree_merge_options()?;
    let mut outcome = repo.merge_trees(id(base)?, id(ours)?, id(theirs)?, labels, options)?;
    let tree = outcome.tree.write()?.detach().to_string();
    // the files that still hold markers, or whose tree change couldn't be resolved
    let how = gix::merge::tree::TreatAsUnresolved::default();
    let mut conflicts: Vec<String> = outcome
        .conflicts
        .iter()
        .filter(|conflict| conflict.is_unresolved(how))
        .flat_map(|conflict| [conflict.ours.location(), conflict.theirs.location()])
        .map(|path| path.to_string())
        .collect();
    conflicts.sort();
    conflicts.dedup();
    Ok(Merged { tree, conflicts })
}
