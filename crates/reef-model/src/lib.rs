//! The model behind reef planning graphs: what an author writes (`spec`), the plan as one
//! checked graph (`graph`) built from what a plan keeps (`record`), what a view of it shows
//! (`view`), and the names that hold across projects and plans (`name`).

pub mod accept;
pub mod graph;
pub mod name;
pub mod perf;
pub mod record;
pub mod spec;
pub mod view;

pub use accept::{AccChart, Criterion, Kind, Met, Resolved};
pub use graph::{Diagnostic, EdgeKind, Graph, Link, Node, PerfDoc};
pub use name::{NameError, NodeName, PlanName};
pub use perf::{Better, Metric, Performance, Value};
pub use record::{Origin, Record, nodes_from_records};
pub use spec::{Description, NodeSpec, Status};
pub use view::{Direction, Expand, Layout, ViewSpec, VisEdge, VisItem, Visible};

/// What the preprocessor embeds in a page and the viewer reads.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Payload {
    pub nodes: Vec<Node>,
}
