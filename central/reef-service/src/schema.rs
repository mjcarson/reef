//! reef's shoal schema: projects, plans, each revision's nodes and pages, each project's queue,
//! its nodes' work records, each node's captured revisions and their files, each plan's
//! comments, and the terminal loops
//!
//! Shoal on its `ScanSupport` branch has no client table scans, so everything reef lists sits in
//! a partition whose key it already knows. All projects share one constant partition, a project's
//! plans sit in that project's, a plan's nodes and its pages each sit in a partition of their
//! revision, and a project's queue and its work records each sit in a partition of their own. A
//! node's revisions sit in the node's partition, and each revision's files in the revision's.
//! Every loop shares one constant partition, as projects do, sorted by the pane it feeds.
//! Sort keys are a single `String` there, so each table sorts by a name, an id, a path, a rank, a
//! node's name within its project, or a revision's number padded to sort as a number does.
//!
//! This one module is both halves of the schema. Built with `embedded` it is the whole schema,
//! which `reef-shoal` serves and an in-process pool runs. Built without it, only the client half
//! is generated. Both come from the same rows, so they agree on the fingerprint a client checks
//! when it connects.

use deepsize2::DeepSizeOf;
use rkyv::{Archive, Deserialize, Serialize};
use shoal::{ShoalProjection, ShoalSortedTable};

// a client schema reads its tables' types for their names alone, so only the engine imports them
#[cfg(feature = "embedded")]
use shoal::{FileSystem, PersistentSortedTable};

/// The one partition every project is listed in
pub const PROJECTS: &str = "projects";

/// The one partition every loop is kept in
pub const LOOPS: &str = "loops";

/// A project: a name and the repo its plans describe
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct ProjectRow {
    /// The listing partition, always [`PROJECTS`]
    #[shoal(partition)]
    pub list: String,
    /// The project's name, which sorts the listing
    #[shoal(sort)]
    pub name: String,
    /// The absolute path of the repo this project's plans describe
    pub repo: String,
}

/// A plan, listed in its project's partition, and the pointers to its current revisions
///
/// A bulk write names its new revision in `pending` before writing any of its rows, and names
/// the revision it replaced in `retired` when it makes its own current. One that carries pages
/// writes them under the same revision, and names the pages' revision it replaced in
/// `pages_retired`. Whatever a write killed midway leaves behind is named here, so the next write
/// can prune it without a scan.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct PlanRow {
    /// The project this plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The plan's name within its project, which sorts the listing
    #[shoal(sort)]
    pub name: String,
    /// The revision readers load
    #[shoal(update)]
    pub current: u64,
    /// A revision being written and not yet current, or 0
    #[shoal(update)]
    pub pending: u64,
    /// A revision replaced and not yet pruned, or 0
    #[shoal(update)]
    pub retired: u64,
    /// The revision of the plan's pages readers load
    ///
    /// Only a whole-plan write that carries pages moves it, so a batch of node writes leaves
    /// the pages where they are. It can name the same revision as `current`, or an older one.
    #[shoal(update)]
    pub pages_current: u64,
    /// A revision of pages replaced and not yet pruned, or 0
    #[shoal(update)]
    pub pages_retired: u64,
    /// Bumped by every change to this row, and named by the next one
    #[shoal(filter, update)]
    pub version: u64,
}

/// One node of one revision of a plan
///
/// Its spec is one JSON field, so reef's keys can change without changing this table. Beside it
/// sit the few fields reef filters or sorts on.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct NodeRow {
    /// The project the node's plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The node's plan
    #[shoal(partition)]
    pub plan: String,
    /// The revision of the plan this row is part of
    #[shoal(partition)]
    pub revision: u64,
    /// The node's id, which sorts the partition
    #[shoal(sort)]
    pub id: String,
    /// Bumped by every write of this node, and named by the next one
    #[shoal(filter, update)]
    pub version: u64,
    /// The status its author wrote, or empty
    #[shoal(filter)]
    pub status: String,
    /// The group it belongs to, or empty
    #[shoal(filter)]
    pub group: String,
    /// Where it falls in the order its plan was written in
    pub order: u64,
    /// Its `NodeSpec` as JSON, with no inline members
    pub spec: String,
    /// Its body as markdown: a file node's page, empty for one that was listed inline
    pub body: String,
}

/// A node row's keys and version alone, for pruning a revision and carrying versions forward
///
/// Shoal's docs say a projection may leave its partition key out since F27, but the derive on
/// `ScanSupport` still refuses one that does, so it carries all three.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalProjection)]
#[rkyv(derive(Debug))]
#[shoal_projection(table = "NodeRow")]
pub struct NodeKey {
    /// The project the node's plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The node's plan
    #[shoal(partition)]
    pub plan: String,
    /// The revision of the plan this row is part of
    #[shoal(partition)]
    pub revision: u64,
    /// The node's id
    pub id: String,
    /// The node's version
    pub version: u64,
}

/// One of a plan's own pages, in one revision of the plan's pages
///
/// A page is markdown kept with its plan, which nodes link to as they do to the repo's files.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct PageRow {
    /// The project the page's plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The page's plan
    #[shoal(partition)]
    pub plan: String,
    /// The revision of the plan's pages this row is part of
    #[shoal(partition)]
    pub revision: u64,
    /// The page's path, which sorts the partition
    #[shoal(sort)]
    pub path: String,
    /// Bumped by every write of this page, and named by the next one
    #[shoal(filter, update)]
    pub version: u64,
    /// Its markdown as written
    pub text: String,
}

/// A page row's keys and version alone, for pruning a revision and carrying versions forward
///
/// It carries the partition key, as [`NodeKey`] does, for the derive on `ScanSupport`.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalProjection)]
#[rkyv(derive(Debug))]
#[shoal_projection(table = "PageRow")]
pub struct PageKey {
    /// The project the page's plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The page's plan
    #[shoal(partition)]
    pub plan: String,
    /// The revision of the plan's pages this row is part of
    #[shoal(partition)]
    pub revision: u64,
    /// The page's path
    pub path: String,
    /// The page's version
    pub version: u64,
}

/// One entry of a project's queue, sorted by its rank
///
/// A row is written once and deleted, never updated. A move writes the entry's new row before
/// it deletes the old one, so a move killed between the two leaves the node in two rows: the
/// one with the higher `seq` is the move, and wins. Ranks repeat across projects, so a project
/// whose partition key's hash collides with another's could overwrite its rows; the store reads
/// each back against its full key, as it does every row.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct QueueRow {
    /// The project whose queue this is
    #[shoal(partition)]
    pub project: String,
    /// The entry's rank, which sorts the queue
    #[shoal(sort)]
    pub rank: String,
    /// The plan of the node queued
    #[shoal(filter)]
    pub plan: String,
    /// The id of the node queued, within its plan
    #[shoal(filter)]
    pub id: String,
    /// The queue's write count when this row was written, higher for each later write
    #[shoal(filter)]
    pub seq: u64,
}

/// One node's work record: where its work happens, what it started from, and what each capture
/// since read
///
/// A project's records share its partition, so the node a worktree is on is found by reading one
/// partition, across the project's plans. The record is one JSON field, as a node's spec is, so
/// what `reef start` and a capture read can change without changing this table. A start writes
/// the row whole, `if_absent` for a node not started and over the version read to start it again,
/// and so does each capture it adds.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct WorkRow {
    /// The project the node's plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The node's name within its project, `plan/id`, which sorts the partition
    #[shoal(sort)]
    pub node: String,
    /// Bumped by every write of this record, and named by the next one
    #[shoal(filter)]
    pub version: u64,
    /// The record as JSON, its name and version left to the fields beside it
    pub record: String,
}

/// One revision of a node's captured changes: what the capture read, and its files without their
/// hunks
///
/// A node's revisions share its partition, so they are listed by reading one partition, oldest
/// first. A revision is written once, `if_absent`, after its files, so a revision that reads
/// has every file it lists. Its record is one JSON field, as a work record is, so what a revision
/// keeps can change without changing this table.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct RevisionRow {
    /// The project the node's plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The node's name within its project, `plan/id`
    #[shoal(partition)]
    pub node: String,
    /// The revision's number, padded with zeros to ten digits so the partition sorts by it
    #[shoal(sort)]
    pub revision: String,
    /// Bumped by every write of this row, so a later write can name the one it read
    #[shoal(filter)]
    pub version: u64,
    /// The revision as JSON, its node and number left to the fields beside it
    pub record: String,
}

/// One file of a node's captured revision, its hunks and lines
///
/// A revision's files share its partition, sorted by the path each is shown under, so a file is
/// read alone when it is opened. A file's row is bounded by shoal's frame, so one whose hunks
/// would pass the store's limit is kept without them.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct RevisionFileRow {
    /// The project the node's plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The node's name within its project, `plan/id`
    #[shoal(partition)]
    pub node: String,
    /// The revision's number
    #[shoal(partition)]
    pub revision: u64,
    /// The path the file is shown under, which sorts the partition
    #[shoal(sort)]
    pub path: String,
    /// The file's diff as JSON, its hunks and all
    pub file: String,
}

/// One comment on a node's work: a thread's first, which holds the thread's target and state,
/// or a reply in a thread
///
/// A plan's comments share its partition, sorted by their numbers, so a node's threads and the
/// plan's are each read from one partition, and the next number is the last one's after it.
/// Each comment is written once, `if_absent`, and only a thread's first is written again, by
/// version, when the thread is marked. Its record is one JSON field, as a work record is, so
/// what a comment keeps can change without changing this table.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct CommentRow {
    /// The project the plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The plan's name within its project
    #[shoal(partition)]
    pub plan: String,
    /// The comment's number in its plan, padded with zeros to ten digits so the partition sorts
    /// by it
    #[shoal(sort)]
    pub number: String,
    /// Bumped by every write of this row, so a later write can name the one it read
    #[shoal(filter)]
    pub version: u64,
    /// The comment as JSON, its plan and number left to the fields beside it
    pub record: String,
}

/// One terminal loop: the pane its session runs in, the project it feeds it from, the node it
/// holds and whether it is paused
///
/// Every loop shares one partition, sorted by its pane's key, so one pane keeps one loop, and a
/// node another loop holds is found by reading one partition. Its record is one JSON field, as a
/// work record is, so what a loop keeps can change without changing this table. A start writes
/// the row `if_absent`, every other change over the version read, and a stop deletes it by
/// version.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct LoopRow {
    /// The listing partition, always [`LOOPS`]
    #[shoal(partition)]
    pub list: String,
    /// The pane's key, such as `zellij:reef:0`, which sorts the listing
    #[shoal(sort)]
    pub pane: String,
    /// Bumped by every write of this row, so a later write can name the one it read
    #[shoal(filter)]
    pub version: u64,
    /// The loop as JSON, its pane and version left to the fields beside it
    pub record: String,
}

/// One decision made on a node's work: a plan a session asked approval for, or questions it
/// asked, and what came of it
///
/// Each node's decisions share a partition, sorted by number, as its revisions do. Its record is
/// one JSON field, so what a decision keeps can change without changing this table. A new one is
/// written `if_absent`, and an answer to one kept already over the version read.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "Reef")]
pub struct DecisionRow {
    /// The project the node's plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The node's name within its project, `plan/id`
    #[shoal(partition)]
    pub node: String,
    /// The decision's number on its node, padded with zeros to ten digits so the partition sorts
    /// by it
    #[shoal(sort)]
    pub number: String,
    /// Bumped by every write of this row, so a later write can name the one it read
    #[shoal(filter)]
    pub version: u64,
    /// The decision as JSON, its node and number left to the fields beside it
    pub record: String,
}

/// reef's database
///
/// With `embedded` this is the full schema, which also generates the client. Without it only the
/// client half is generated, and the field types are read for their names alone.
#[cfg_attr(feature = "embedded", shoal::db)]
#[cfg_attr(not(feature = "embedded"), shoal::db(client))]
pub struct Reef {
    /// Every project, in one partition
    pub projects: PersistentSortedTable<ProjectRow, FileSystem>,
    /// Each project's plans and their pointers to their current revisions
    pub plans: PersistentSortedTable<PlanRow, FileSystem>,
    /// Each revision's nodes
    #[shoal(projections(NodeKey))]
    pub nodes: PersistentSortedTable<NodeRow, FileSystem>,
    /// Each project's queue, by rank
    pub queue: PersistentSortedTable<QueueRow, FileSystem>,
    /// Each revision's pages
    #[shoal(projections(PageKey))]
    pub pages: PersistentSortedTable<PageRow, FileSystem>,
    /// Each project's work records, by node
    pub work: PersistentSortedTable<WorkRow, FileSystem>,
    /// Each node's captured revisions, by number
    pub revisions: PersistentSortedTable<RevisionRow, FileSystem>,
    /// Each revision's files, by path
    pub revision_files: PersistentSortedTable<RevisionFileRow, FileSystem>,
    /// Each plan's comments, by number
    pub comments: PersistentSortedTable<CommentRow, FileSystem>,
    /// Every terminal loop, by pane
    pub loops: PersistentSortedTable<LoopRow, FileSystem>,
    /// Each node's decisions, by number
    pub decisions: PersistentSortedTable<DecisionRow, FileSystem>,
}

#[cfg(test)]
mod tests {
    use super::ReefClient;
    use shoal::shared::traits::QuerySupport;

    /// The schema's structural id, which moves with any change to a row, a table or a projection
    const SCHEMA_ID: u64 = 3071008669871081648;

    /// Each table's fingerprint, in the order the schema serves them
    const TABLES: &[(&str, u64)] = &[
        ("ProjectRow", 12832186597557680855),
        ("PlanRow", 15730717529105804003),
        ("NodeRow", 1919551679666722412),
        ("QueueRow", 7438970626929457281),
        ("PageRow", 13886911418170667521),
        ("WorkRow", 606948397267046790),
        ("RevisionRow", 17497476575540100941),
        ("RevisionFileRow", 2270476186678581084),
        ("CommentRow", 11872189191251839750),
        ("LoopRow", 8656995277351789761),
        ("DecisionRow", 18226743367225770895),
    ];

    /// The schema is pinned, so changing it is a deliberate act that needs a fresh data directory
    ///
    /// Shoal can't migrate a table yet, so a row that changes shape can't be read from a data
    /// directory written before it. The pin leaves out shoal's protocol version, which
    /// `SCHEMA_FINGERPRINT` folds in, so a shoal bump that keeps every row moves nothing here. It
    /// runs in both builds, so the client half and the whole schema are shown to agree.
    #[test]
    fn fingerprint_is_pinned() {
        // the structure of the whole schema
        assert_eq!(
            <ReefClient as QuerySupport>::SCHEMA_ID,
            SCHEMA_ID,
            "reef's schema moved: a changed row needs a fresh data directory, then a new pin"
        );
        // and each table, so a failure says which one moved
        assert_eq!(
            <ReefClient as QuerySupport>::table_fingerprints(),
            TABLES,
            "a table of reef's schema moved: it needs a fresh data directory, then a new pin"
        );
    }
}
