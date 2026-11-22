//! reef's JSON API: the service's data types, and the requests, answers and changes its routes
//! carry
//!
//! `reef-service` keeps plans in shoal and checks every write against the whole plan. Its JSON
//! API is a plain axum router the CLI, the MCP server and the hooks call, so none of them depends
//! on the app's server functions or on shoal. These are the types both sides speak, and they are
//! the service's own: a [`NodeWrite`] given to the service is the one a client sends.
//!
//! - **Nodes** ([`node`]): a node as read, as written, as put and as removed, what came of a
//!   batch or a whole-plan write, a plan's listing, and a spec's inline members lifted out.
//! - **Projects and plans** ([`project`]): a project and the repo its plans describe, a plan and
//!   the revision readers load, and the requests that add them.
//! - **The queue** ([`queue`]): a project's order of work, where an edit puts an entry, and
//!   what came of one.
//! - **Changes** ([`change`]): what the change stream tells of each write.
//! - **Errors** ([`error`]): what a route answers when it can't do what was asked.
//! - **Paths** ([`paths`]): every route's path, for the router and for clients to agree on.
//! - **Config** ([`config`]): where reef's config file is, which the service and its clients read.
//!
//! The service serves localhost only until users sign in (M10), at [`DEFAULT_ADDRESS`] unless
//! reef's config names another.

pub mod change;
pub mod config;
pub mod error;
pub mod node;
pub mod paths;
pub mod project;
pub mod queue;

pub use change::{Change, PutNode};
pub use error::{ApiError, ErrorCode};
pub use node::{
    Checked, Listing, NodePut, NodeRemove, NodeWrite, PlanWrite, Puts, Refusal, Rewrite, Rewritten,
    StalePut, StoredNode, lift, puts_for, records,
};
pub use project::{AddedPlan, AddedProject, NewPlan, NewProject, PlanInfo, Project};
pub use queue::{Listed, Place, QueueEdit, QueueListing, Queued, Ranked};

/// Where the service listens when reef's config names no address: localhost only
pub const DEFAULT_ADDRESS: &str = "127.0.0.1:12180";
