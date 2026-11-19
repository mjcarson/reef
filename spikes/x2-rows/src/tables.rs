//! X2's rows: projects, plans and nodes, laid out for a shoal with no client scans
//!
//! Everything reef lists sits in a partition whose key it already knows. All projects share one
//! constant partition, a project's plans sit in that project's, and a plan's nodes sit in a
//! partition of their revision. Sort keys are a single `String` on shoal's `ScanSupport` branch,
//! so each table sorts by a name or an id.

use deepsize2::DeepSizeOf;
use rkyv::{Archive, Deserialize, Serialize};
use shoal::{FileSystem, PersistentSortedTable, ShoalProjection, ShoalSortedTable};

/// The one partition every project is listed in
pub const PROJECTS: &str = "projects";

/// A project: a name and the repo its plans describe
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "X2")]
pub struct ProjectRow {
    /// The listing partition, always [`PROJECTS`]
    #[shoal(partition)]
    pub list: String,
    /// The project's name, which sorts the listing
    #[shoal(sort)]
    pub name: String,
    /// The path of the repo this project's plans describe
    pub repo: String,
}

/// A plan, listed in its project's partition, and the pointer to its current revision
///
/// A bulk write names its new revision in `pending` before writing any of its rows, and names
/// the revision it replaced in `retired` when it makes its own current. Whatever a write killed
/// midway leaves behind is named here, so the next write can prune it without a scan.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "X2")]
pub struct PlanRow {
    /// The project this plan belongs to
    #[shoal(partition)]
    pub project: String,
    /// The plan's name within its project, which sorts the listing
    #[shoal(sort)]
    pub name: String,
    /// The revision readers load, or 0 before the first is written
    #[shoal(update)]
    pub current: u64,
    /// A revision being written and not yet current, or 0
    #[shoal(update)]
    pub pending: u64,
    /// A revision replaced and not yet pruned, or 0
    #[shoal(update)]
    pub retired: u64,
    /// Bumped by every change to this row, and named by the next one
    #[shoal(filter, update)]
    pub version: u64,
}

/// One node of one revision of a plan
///
/// Its spec is one serialized field, so reef's keys can change without changing this table.
/// Beside it sit the few fields reef filters or sorts on.
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalSortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "X2")]
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
    /// Its spec, serialized as JSON or YAML
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

/// X2's database
#[shoal::db]
pub struct X2 {
    /// Every project, in one partition
    pub projects: PersistentSortedTable<ProjectRow, FileSystem>,
    /// Each project's plans and their pointers to their current revisions
    pub plans: PersistentSortedTable<PlanRow, FileSystem>,
    /// Each revision's nodes
    #[shoal(projections(NodeKey))]
    pub nodes: PersistentSortedTable<NodeRow, FileSystem>,
}
