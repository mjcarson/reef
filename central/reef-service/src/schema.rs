//! reef's shoal schema: projects, plans and each revision's nodes
//!
//! Shoal on its `ScanSupport` branch has no client table scans, so everything reef lists sits in
//! a partition whose key it already knows. All projects share one constant partition, a project's
//! plans sit in that project's, and a plan's nodes sit in a partition of their revision. Sort keys
//! are a single `String` there, so each table sorts by a name or an id.
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

/// A plan, listed in its project's partition, and the pointer to its current revision
///
/// A bulk write names its new revision in `pending` before writing any of its rows, and names
/// the revision it replaced in `retired` when it makes its own current. Whatever a write killed
/// midway leaves behind is named here, so the next write can prune it without a scan.
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
}

#[cfg(test)]
mod tests {
    use super::ReefClient;
    use shoal::shared::traits::QuerySupport;

    /// The schema's structural id, which moves with any change to a row, a table or a projection
    const SCHEMA_ID: u64 = 8968132830820625058;

    /// Each table's fingerprint, in the order the schema serves them
    const TABLES: &[(&str, u64)] = &[
        ("ProjectRow", 12832186597557680855),
        ("PlanRow", 2457880101249943207),
        ("NodeRow", 1919551679666722412),
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
