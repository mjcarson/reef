//! What a store needs of the rows under it: reads by key, and writes that hold a condition
//!
//! A [`Store`](crate::Store) keeps the logic: the locks, versions, revisions and full-key checks.
//! A backend only reads and writes rows the way shoal does, so the store's logic runs the same
//! over shoal and over [`MemoryBackend`](crate::MemoryBackend), its in-memory twin, which builds
//! on stable. One test suite runs against both, so shoal is the twin's oracle.

use reef_model::PlanName;
use std::future::Future;

use crate::schema::{
    CommentRow, NodeKey, NodeRow, PageKey, PageRow, PlanRow, ProjectRow, QueueRow, RevisionFileRow,
    RevisionRow, WorkRow,
};

/// Why a conditional write was refused, kept in `reef-api` since a stale put's answer names it
pub use reef_api::Refusal;

/// What came of a conditional write
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// It was written
    Applied,
    /// Its condition did not hold, so nothing was written
    Refused(Refusal),
}

/// The condition a write holds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// No row is stored under its key
    Absent,
    /// A row is stored under its key, at this version
    Version(u64),
}

/// The revisions a plan's row names that change in one write; `None` leaves one as it is
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pointers {
    /// The revision readers load
    pub current: Option<u64>,
    /// A revision being written and not yet current, or 0
    pub pending: Option<u64>,
    /// A revision replaced and not yet pruned, or 0
    pub retired: Option<u64>,
    /// The revision of the plan's pages readers load
    pub pages_current: Option<u64>,
    /// A revision of pages replaced and not yet pruned, or 0
    pub pages_retired: Option<u64>,
}

/// The key of one revision of a plan: the partition its nodes sit in, and the one its pages do
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RevisionKey {
    /// The project the plan belongs to
    pub project: String,
    /// The plan's name within its project
    pub plan: String,
    /// The revision
    pub revision: u64,
}

impl RevisionKey {
    /// The key of one revision of a plan
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision
    pub fn new(plan: &PlanName, revision: u64) -> RevisionKey {
        RevisionKey {
            project: plan.project.clone(),
            plan: plan.plan.clone(),
            revision,
        }
    }

    /// Whether a node row is part of this revision
    ///
    /// # Arguments
    ///
    /// * `row` - The row to check
    pub fn holds(&self, row: &NodeRow) -> bool {
        row.project == self.project && row.plan == self.plan && row.revision == self.revision
    }

    /// Whether a node's key is part of this revision
    ///
    /// # Arguments
    ///
    /// * `key` - The key to check
    pub fn holds_key(&self, key: &NodeKey) -> bool {
        key.project == self.project && key.plan == self.plan && key.revision == self.revision
    }

    /// Whether a page row is part of this revision
    ///
    /// # Arguments
    ///
    /// * `row` - The row to check
    pub fn holds_page(&self, row: &PageRow) -> bool {
        row.project == self.project && row.plan == self.plan && row.revision == self.revision
    }

    /// Whether a page's key is part of this revision
    ///
    /// # Arguments
    ///
    /// * `key` - The key to check
    pub fn holds_page_key(&self, key: &PageKey) -> bool {
        key.project == self.project && key.plan == self.plan && key.revision == self.revision
    }
}

/// Reads and writes reef's rows the way shoal does
///
/// Reads return what is stored under a key's partition, unchecked: a partition key whose hash
/// collides with another's can hold the other's rows, so the store checks every row against
/// the full key it asked for. A read of a partition that holds nothing is empty, never an error.
pub trait Backend: Send + Sync {
    /// Lists the project rows stored in the one partition every project is listed in
    fn projects(&self) -> impl Future<Output = anyhow::Result<Vec<ProjectRow>>> + Send;

    /// Writes a project's row only if none is stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The project's row
    fn insert_project(
        &self,
        row: ProjectRow,
    ) -> impl Future<Output = anyhow::Result<Written>> + Send;

    /// Lists the plan rows stored in a project's partition, in name order
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    fn plans(&self, project: &str) -> impl Future<Output = anyhow::Result<Vec<PlanRow>>> + Send;

    /// Gets the row stored under a plan's key, if one is
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    fn plan(&self, plan: &PlanName)
    -> impl Future<Output = anyhow::Result<Option<PlanRow>>> + Send;

    /// Writes a plan's row only if none is stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The plan's row
    fn insert_plan(&self, row: PlanRow) -> impl Future<Output = anyhow::Result<Written>> + Send;

    /// Reads the node rows stored in a revision's partition, in id order, or the one under an id
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `id` - The one node to read, or `None` for all of them
    fn nodes(
        &self,
        key: &RevisionKey,
        id: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<Vec<NodeRow>>> + Send;

    /// Writes one node row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    fn write_node(
        &self,
        row: NodeRow,
        expect: Expect,
    ) -> impl Future<Output = anyhow::Result<Written>> + Send;

    /// Changes a plan's pointers, if its row is still at the version read
    ///
    /// The row moves on to version `read + 1`.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `read` - The version the row was read at
    /// * `pointers` - The revisions that change
    fn update_plan(
        &self,
        plan: &PlanName,
        read: u64,
        pointers: Pointers,
    ) -> impl Future<Output = anyhow::Result<Written>> + Send;

    /// Lists the keys and versions stored in a revision's partition, without their specs
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    fn node_keys(
        &self,
        key: &RevisionKey,
    ) -> impl Future<Output = anyhow::Result<Vec<NodeKey>>> + Send;

    /// Writes node rows over whatever is stored under their keys, every answer checked
    ///
    /// The writes are not atomic: one that fails partway leaves the rows before it written.
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    fn write_nodes(&self, rows: Vec<NodeRow>) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Deletes nodes from a revision's partition, every answer checked
    ///
    /// A delete of a row that isn't stored deletes nothing and is no failure, so a prune that is
    /// tried again is harmless.
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `ids` - The ids of the nodes to delete
    fn delete_nodes(
        &self,
        key: &RevisionKey,
        ids: Vec<String>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Reads the page rows stored in a revision's partition, in path order, or the one at a path
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `path` - The one page to read, or `None` for all of them
    fn pages(
        &self,
        key: &RevisionKey,
        path: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<Vec<PageRow>>> + Send;

    /// Writes one page row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    fn write_page(
        &self,
        row: PageRow,
        expect: Expect,
    ) -> impl Future<Output = anyhow::Result<Written>> + Send;

    /// Lists the paths and versions stored in a revision's partition of pages, without their text
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    fn page_keys(
        &self,
        key: &RevisionKey,
    ) -> impl Future<Output = anyhow::Result<Vec<PageKey>>> + Send;

    /// Writes page rows over whatever is stored under their keys, every answer checked
    ///
    /// The writes are not atomic: one that fails partway leaves the rows before it written.
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    fn write_pages(&self, rows: Vec<PageRow>) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Deletes pages from a revision's partition, every answer checked
    ///
    /// A delete of a row that isn't stored deletes nothing and is no failure, so a prune that is
    /// tried again is harmless.
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `paths` - The paths of the pages to delete
    fn delete_pages(
        &self,
        key: &RevisionKey,
        paths: Vec<String>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Lists the queue rows stored in a project's partition, in rank order
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    fn queue(&self, project: &str) -> impl Future<Output = anyhow::Result<Vec<QueueRow>>> + Send;

    /// Writes a queue row only if none is stored under its rank
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    fn insert_queue(&self, row: QueueRow) -> impl Future<Output = anyhow::Result<Written>> + Send;

    /// Deletes a queue row, only if the row stored under its rank is still this one
    ///
    /// A row that isn't stored is refused as missing, and another row under its rank as moved.
    ///
    /// # Arguments
    ///
    /// * `row` - The row to delete, as it was read
    fn delete_queue(&self, row: &QueueRow) -> impl Future<Output = anyhow::Result<Written>> + Send;

    /// Reads the work rows stored in a project's partition, in node order, or the one of a node
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `node` - The one node's name within its project, `plan/id`, or `None` for all of them
    fn work(
        &self,
        project: &str,
        node: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<Vec<WorkRow>>> + Send;

    /// Writes one work row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    fn write_work(
        &self,
        row: WorkRow,
        expect: Expect,
    ) -> impl Future<Output = anyhow::Result<Written>> + Send;

    /// Reads the revision rows stored in a node's partition, oldest first, or the one of a number
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `node` - The node's name within its project, `plan/id`
    /// * `revision` - The one revision's sort key, its number padded, or `None` for all of them
    fn revisions(
        &self,
        project: &str,
        node: &str,
        revision: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<Vec<RevisionRow>>> + Send;

    /// Writes one revision row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    fn write_revision(
        &self,
        row: RevisionRow,
        expect: Expect,
    ) -> impl Future<Output = anyhow::Result<Written>> + Send;

    /// Reads the file rows stored in a revision's partition, in path order, or the one at a path
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `node` - The node's name within its project, `plan/id`
    /// * `revision` - The revision's number
    /// * `path` - The one file's path, or `None` for all of them
    fn revision_files(
        &self,
        project: &str,
        node: &str,
        revision: u64,
        path: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<Vec<RevisionFileRow>>> + Send;

    /// Writes revision file rows over whatever is stored under their keys, every answer checked
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write, one bundle's worth
    fn write_revision_files(
        &self,
        rows: Vec<RevisionFileRow>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Reads the comment rows stored in a plan's partition, in number order, or the one of a
    /// number
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `plan` - The plan's name within its project
    /// * `number` - The one comment's sort key, its number padded, or `None` for all of them
    fn comments(
        &self,
        project: &str,
        plan: &str,
        number: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<Vec<CommentRow>>> + Send;

    /// Writes one comment row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    fn write_comment(
        &self,
        row: CommentRow,
        expect: Expect,
    ) -> impl Future<Output = anyhow::Result<Written>> + Send;
}
