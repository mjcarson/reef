//! Each node's captured revisions: a row a revision in the node's partition, and a row a file in
//! the revision's
//!
//! - **A revision lists its files, and a file holds its hunks.** A node's revisions are listed by
//!   reading its one partition, whatever their size, and a file's hunks are read alone, when it
//!   is opened.
//! - **Numbered from 1 for each node, and never reset.** A node started again keeps the revisions
//!   it had, since comments will name them, so its next capture is numbered after its latest.
//! - **Files first, then the revision.** A capture writes its files' rows, then its revision's
//!   row `if_absent`, so a revision that reads has every file it lists. A capture killed between
//!   the two leaves file rows no revision lists, which are never served, and the next capture
//!   writes over them under the same number.
//! - **A draft is rewritten in place.** The node's latest revision may be a draft, which a
//!   capture writes over under its number: its files' rows first, then its row over the version
//!   read. A file the new draft no longer lists keeps its row, which is never served. A reader
//!   may meanwhile find a draft's new hunks under its old listing, which a draft, still being
//!   worked on, can afford. A seal of a draft unchanged rewrites its row alone.
//! - **A file too large to keep whole** is kept without its hunks, and says so, since a row must
//!   fit in one of shoal's frames.
//! - **Every row checked against its full key on read**, so a hash collision is caught rather
//!   than served as another node's changes.

use reef_api::{AddCapture, CaptureShas, FileDiff, FileSummary, ResolvingCommit, Revision};
use reef_model::NodeName;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::instrument;

use super::{Store, Unkeepable, bundles};
use crate::backend::{Backend, Expect, Written};
use crate::schema::{RevisionFileRow, RevisionRow};

/// How many bytes of JSON a file's row may hold before its hunks are left out
///
/// A row must fit in one of shoal's frames, 64 MiB by default, with room to spare.
pub const FILE_ROW_MAX: usize = 32 << 20;

/// What a revision row keeps as JSON: everything of a revision but its node and number
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Kept {
    /// When the revision was stored, in milliseconds since the epoch
    at: u64,
    /// What the capture read
    #[serde(flatten)]
    shas: CaptureShas,
    /// The files that conflicted carrying the start forward
    #[serde(default)]
    conflicts: Vec<String>,
    /// Every changed file, without its hunks
    files: Vec<FileSummary>,
    /// The node's own commits the capture read, oldest first
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    commits: Vec<ResolvingCommit>,
    /// Whether it is the node's draft, which a capture may still rewrite
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    draft: bool,
}

/// A node's latest revision, and the version its row was read at, which a draft's rewrite is
/// written over
#[derive(Debug, Clone)]
pub(super) struct Latest {
    /// The revision
    pub revision: Revision,
    /// The version of its row
    pub version: u64,
}

/// What a capture's revision is written as
#[derive(Debug, Clone, Copy)]
pub(super) enum Slot {
    /// A new revision, under a number no row has
    New,
    /// Over the node's draft, its row at the version read
    Over(u64),
}

impl<B: Backend> Store<B> {
    /// Lists a node's revisions, oldest first, each with its files but not their hunks
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Store::revisions", skip_all, err(Debug))]
    pub async fn revisions(&self, node: &NodeName) -> anyhow::Result<Vec<Revision>> {
        // every row of the node's partition, each checked against its full key
        let read = self.revision_rows(node).await?;
        Ok(read.into_iter().map(|latest| latest.revision).collect())
    }

    /// Lists a node's sealed revisions, oldest first, which comments name and follow: every
    /// revision but a draft
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    pub(super) async fn sealed_revisions(&self, node: &NodeName) -> anyhow::Result<Vec<Revision>> {
        // a draft's lines change under it, so no comment goes on them
        let mut revisions = self.revisions(node).await?;
        revisions.retain(|revision| !revision.draft);
        Ok(revisions)
    }

    /// Gets a node's latest revision, and the version its row was read at
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    pub(super) async fn latest_revision(&self, node: &NodeName) -> anyhow::Result<Option<Latest>> {
        // the partition sorts by number, so the last row is the latest
        Ok(self.revision_rows(node).await?.pop())
    }

    /// Reads every revision row of a node's partition, oldest first, with its version
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    async fn revision_rows(&self, node: &NodeName) -> anyhow::Result<Vec<Latest>> {
        // every row, each checked against its full key
        let key = node_key(node);
        let rows = self
            .backend
            .revisions(&node.plan.project, &key, None)
            .await?;
        rows.into_iter()
            .map(|row| {
                let version = row.version;
                let revision = revision(node, row)?;
                Ok(Latest { revision, version })
            })
            .collect()
    }

    /// Gets one of a node's revisions, if it has one of that number
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `number` - The revision's number
    #[instrument(name = "Store::node_revision", skip_all, err(Debug))]
    pub async fn node_revision(
        &self,
        node: &NodeName,
        number: u32,
    ) -> anyhow::Result<Option<Revision>> {
        // the one row under the revision's key
        let key = node_key(node);
        let sort = sort_key(number);
        let rows = self
            .backend
            .revisions(&node.plan.project, &key, Some(sort.as_str()))
            .await?;
        let mut found = None;
        for row in rows {
            let read = revision(node, row)?;
            // a row of another number, as a collision could leave, is none of this one's
            if read.revision == number {
                found = Some(read);
            }
        }
        Ok(found)
    }

    /// Gets one file of a node's revision, its hunks and all, if the revision lists it
    ///
    /// A file row no revision lists, as a capture killed before its revision was written leaves,
    /// is never served.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `number` - The revision's number
    /// * `path` - The path the file is shown under
    #[instrument(name = "Store::revision_file", skip_all, err(Debug))]
    pub async fn revision_file(
        &self,
        node: &NodeName,
        number: u32,
        path: &str,
    ) -> anyhow::Result<Option<FileDiff>> {
        // the revision must list the file
        let Some(listed) = self.node_revision(node, number).await? else {
            return Ok(None);
        };
        if !listed.files.iter().any(|file| file.path() == path) {
            return Ok(None);
        }
        // then its one row, checked against its full key
        let key = node_key(node);
        let rows = self
            .backend
            .revision_files(&node.plan.project, &key, u64::from(number), Some(path))
            .await?;
        let mut found = None;
        for row in rows {
            check_file_key(node, number, &row)?;
            if row.path == path {
                let file: FileDiff = serde_json::from_str(&row.file).map_err(|error| {
                    anyhow::anyhow!(
                        "file `{path}` of revision {number} of `{node}` doesn't read as one: \
                         {error}"
                    )
                })?;
                found = Some(file);
            }
        }
        // a listed file whose row is gone is a store that lost a row, not an answer
        match found {
            Some(file) => Ok(Some(file)),
            None => anyhow::bail!(
                "revision {number} of `{node}` lists `{path}`, but its row is not stored"
            ),
        }
    }

    /// Keeps one capture as a node's revision of a number: its files' rows, then its own
    ///
    /// The caller holds the node's project's work lock, and has found the number free, or the
    /// node's draft under it.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `number` - The revision's number
    /// * `slot` - Whether it is new, or written over the node's draft
    /// * `at` - When it was stored, in milliseconds since the epoch
    /// * `add` - The capture: what it read, its conflicts, its files and whether it is a draft
    pub(super) async fn keep_revision(
        &self,
        node: &NodeName,
        number: u32,
        slot: Slot,
        at: u64,
        add: AddCapture,
    ) -> anyhow::Result<Revision> {
        // each file as its row, and as the revision lists it
        let key = node_key(node);
        let mut rows = Vec::with_capacity(add.files.len());
        let mut listed = Vec::with_capacity(add.files.len());
        for file in add.files {
            let (row, summary) = file_row(node, &key, number, file)?;
            rows.push(row);
            listed.push(summary);
        }
        // the files first, so a revision that reads has every file it lists
        for bundle in bundles(rows, |row| row.path.len() + row.file.len()) {
            self.backend.write_revision_files(bundle).await?;
        }
        // then the revision itself, once
        let kept = Kept {
            at,
            shas: add.shas,
            conflicts: add.conflicts,
            files: listed,
            commits: add.commits,
            draft: add.draft,
        };
        self.write_kept(node, number, slot, kept).await
    }

    /// Seals a node's draft as it is, with the SHAs and commits of the capture that sealed it:
    /// its row alone, since its files are the ones it lists already
    ///
    /// The caller holds the node's project's work lock, and has read the draft as the latest.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `latest` - The node's draft, and the version its row was read at
    /// * `at` - When it was sealed, in milliseconds since the epoch
    /// * `shas` - What the sealing capture read
    /// * `commits` - The node's own commits the sealing capture read
    pub(super) async fn seal_revision(
        &self,
        node: &NodeName,
        latest: Latest,
        at: u64,
        shas: CaptureShas,
        commits: Vec<ResolvingCommit>,
    ) -> anyhow::Result<Revision> {
        // the draft's files and conflicts, which the same base and tree give again
        let Latest { revision, version } = latest;
        let kept = Kept {
            at,
            shas,
            conflicts: revision.conflicts,
            files: revision.files,
            commits,
            draft: false,
        };
        self.write_kept(node, revision.revision, Slot::Over(version), kept)
            .await
    }

    /// Writes a revision's row: under a number no row has, or over a draft's at the version read
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `number` - The revision's number
    /// * `slot` - Whether it is new, or written over the node's draft
    /// * `kept` - The revision but its node and number
    async fn write_kept(
        &self,
        node: &NodeName,
        number: u32,
        slot: Slot,
        kept: Kept,
    ) -> anyhow::Result<Revision> {
        // a new row is the first of its number, and a draft's rewrite the next of its versions
        let (version, expect) = match slot {
            Slot::New => (1, Expect::Absent),
            Slot::Over(read) => (read + 1, Expect::Version(read)),
        };
        let row = RevisionRow {
            project: node.plan.project.clone(),
            node: node_key(node),
            revision: sort_key(number),
            version,
            record: serde_json::to_string(&kept)?,
        };
        match self.backend.write_revision(row, expect).await? {
            Written::Applied => Ok(revised(node, number, kept)),
            // under the lock, only a writer in another process could have written the number
            Written::Refused(refusal) => anyhow::bail!(
                "revision {number} of `{node}` wasn't kept, since another writer stored one under \
                 its number meanwhile: {}",
                refusal.describe()
            ),
        }
    }
}

/// Refuses a capture's files where two are shown under one path, or one under none
///
/// # Arguments
///
/// * `node` - The node captured
/// * `files` - The capture's files
pub(super) fn check_files(node: &NodeName, files: &[FileDiff]) -> anyhow::Result<()> {
    let mut seen = HashSet::with_capacity(files.len());
    for file in files {
        // a file with no path can't be read back by one
        let path = file.path();
        if path.is_empty() {
            let message = format!("a capture of `{node}` names a file with no path");
            return Err(Unkeepable(message).into());
        }
        // and two under one path would leave one of them unreadable
        if !seen.insert(path) {
            let message = format!("a capture of `{node}` names `{path}` twice");
            return Err(Unkeepable(message).into());
        }
    }
    Ok(())
}

/// A node's key within its project, `plan/id`, which names its partition of revisions
///
/// # Arguments
///
/// * `node` - The node's full name
fn node_key(node: &NodeName) -> String {
    format!("{}/{}", node.plan.plan, node.id)
}

/// A revision's sort key: its number padded with zeros, so the partition sorts by number
///
/// # Arguments
///
/// * `number` - The revision's number
fn sort_key(number: u32) -> String {
    format!("{number:010}")
}

/// A file's row in a revision, and its summary for the revision's listing
///
/// A file whose JSON would pass [`FILE_ROW_MAX`] is kept without its hunks, marked too large,
/// its counts still taken from the hunks it had.
///
/// # Arguments
///
/// * `node` - The node's full name
/// * `key` - The node's key within its project
/// * `number` - The revision's number
/// * `file` - The file, its hunks and all
fn file_row(
    node: &NodeName,
    key: &str,
    number: u32,
    mut file: FileDiff,
) -> anyhow::Result<(RevisionFileRow, FileSummary)> {
    // the counts come from every hunk, before any is left out
    let mut summary = file.summary();
    let mut json = serde_json::to_string(&file)?;
    if json.len() > FILE_ROW_MAX {
        file.hunks.clear();
        file.too_large = true;
        summary.too_large = true;
        json = serde_json::to_string(&file)?;
    }
    let row = RevisionFileRow {
        project: node.plan.project.clone(),
        node: key.to_string(),
        revision: u64::from(number),
        path: file.path().to_string(),
        file: json,
    };
    Ok((row, summary))
}

/// A revision as a client reads it, from what was written
///
/// # Arguments
///
/// * `node` - The node's full name
/// * `number` - The revision's number
/// * `kept` - The revision but its node and number
fn revised(node: &NodeName, number: u32, kept: Kept) -> Revision {
    Revision {
        node: node.clone(),
        revision: number,
        at: kept.at,
        shas: kept.shas,
        conflicts: kept.conflicts,
        files: kept.files,
        commits: kept.commits,
        draft: kept.draft,
    }
}

/// A revision row read back as its revision, checked against the node it was read for
///
/// # Arguments
///
/// * `node` - The node whose partition was read
/// * `row` - The row
fn revision(node: &NodeName, row: RevisionRow) -> anyhow::Result<Revision> {
    // a row of another node whose key hashed the same is a collision, not this node's
    let key = node_key(node);
    anyhow::ensure!(
        row.project == node.plan.project && row.node == key,
        "revision row `{}` of `{}/{}` was stored where `{node}`'s revisions are: their partition \
         keys' hashes collide",
        row.revision,
        row.project,
        row.node
    );
    // its number is its padded sort key
    let number: u32 = row.revision.parse().map_err(|_| {
        anyhow::anyhow!(
            "revision row `{}` of `{node}` isn't named by its number",
            row.revision
        )
    })?;
    let kept: Kept = serde_json::from_str(&row.record).map_err(|error| {
        anyhow::anyhow!("revision {number} of `{node}` doesn't read as one: {error}")
    })?;
    Ok(revised(node, number, kept))
}

/// Refuses a file row stored under another revision's partition, as a hash collision would
///
/// # Arguments
///
/// * `node` - The node whose revision was read
/// * `number` - The revision's number
/// * `row` - The row
fn check_file_key(node: &NodeName, number: u32, row: &RevisionFileRow) -> anyhow::Result<()> {
    let key = node_key(node);
    anyhow::ensure!(
        row.project == node.plan.project && row.node == key && row.revision == u64::from(number),
        "file row `{}` of revision {} of `{}/{}` was stored where revision {number} of `{node}` \
         keeps its files: their partition keys' hashes collide",
        row.path,
        row.revision,
        row.project,
        row.node
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FILE_ROW_MAX, Store, file_row, sort_key};
    use crate::schema::{RevisionFileRow, RevisionRow};
    use reef_api::{FileDiff, FileStatus, Hunk, Line, LineKind};
    use reef_model::NodeName;

    /// A file added with one hunk of a number of lines, each so many bytes long
    ///
    /// # Arguments
    ///
    /// * `lines` - How many lines it adds
    /// * `width` - How long each line is
    fn added(lines: u32, width: usize) -> FileDiff {
        let text = "x".repeat(width);
        FileDiff {
            status: FileStatus::Added,
            old_path: None,
            new_path: Some("big.txt".to_string()),
            old_mode: None,
            new_mode: Some("100644".to_string()),
            old_id: None,
            new_id: Some("a".repeat(40)),
            binary: false,
            too_large: false,
            hunks: vec![Hunk {
                old_start: 0,
                old_lines: 0,
                new_start: 1,
                new_lines: lines,
                lines: (1..=lines)
                    .map(|new| Line {
                        kind: LineKind::Added,
                        old: None,
                        new: Some(new),
                        text: text.clone(),
                        no_newline: false,
                    })
                    .collect(),
            }],
        }
    }

    /// Revisions sort as numbers do, and a file too large to keep is kept without its hunks
    #[test]
    fn rows_sort_by_number_and_hold_under_the_frame() {
        // padded numbers sort in their order
        let mut keys = [sort_key(10), sort_key(9), sort_key(100)];
        keys.sort();
        assert_eq!(keys, [sort_key(9), sort_key(10), sort_key(100)]);
        // a file under the limit is kept whole
        let node: NodeName = "reef/central/C32".parse().unwrap();
        let (row, summary) = file_row(&node, "central/C32", 1, added(3, 10)).unwrap();
        assert!(!summary.too_large);
        assert_eq!(
            serde_json::from_str::<FileDiff>(&row.file)
                .unwrap()
                .hunks
                .len(),
            1
        );
        // one over it is kept without its hunks, its counts still the hunks'
        let lines = (FILE_ROW_MAX / 1000 + 1) as u32;
        let (row, summary) = file_row(&node, "central/C32", 1, added(lines, 1000)).unwrap();
        assert!(row.file.len() < 1024, "{} bytes kept", row.file.len());
        let kept: FileDiff = serde_json::from_str(&row.file).unwrap();
        assert!(kept.too_large && kept.hunks.is_empty());
        assert!(summary.too_large);
        assert_eq!(summary.added, lines);
    }

    /// Rows stored in a node's partition under another node's key are an error, never served
    #[tokio::test]
    async fn collided_revision_rows_are_errors() -> anyhow::Result<()> {
        // a revision of another project's node, landing in reef's C32's partition
        let store = Store::memory();
        let ours: NodeName = "reef/central/C32".parse()?;
        let stray = RevisionRow {
            project: "other".to_string(),
            node: "central/C32".to_string(),
            revision: sort_key(1),
            version: 1,
            record: "{}".to_string(),
        };
        store.backend().plant_revision("reef", "central/C32", stray);
        let error = store
            .revisions(&ours)
            .await
            .expect_err("a collided row was served");
        assert!(format!("{error:#}").contains("collide"), "{error:#}");
        // and a file row of another revision, in revision 1's partition
        let store = Store::memory();
        let node = "central/C32".to_string();
        let listed = RevisionRow {
            project: "reef".to_string(),
            node: node.clone(),
            revision: sort_key(1),
            version: 1,
            record: serde_json::json!({
                "at": 1,
                "head": "a".repeat(40),
                "tree": "b".repeat(40),
                "merge_base": null,
                "base": "c".repeat(40),
                "files": [added(1, 1).summary()],
            })
            .to_string(),
        };
        store.backend().plant_revision("reef", &node, listed);
        let (mut file, _) = file_row(&ours, &node, 2, added(1, 1))?;
        file.revision = 2;
        let file: RevisionFileRow = file;
        store.backend().plant_revision_file("reef", &node, 1, file);
        let error = store
            .revision_file(&ours, 1, "big.txt")
            .await
            .expect_err("a collided row was served");
        assert!(format!("{error:#}").contains("collide"), "{error:#}");
        Ok(())
    }
}
