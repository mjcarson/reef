//! reef's store and service: the only code that touches shoal
//!
//! A `Service` is what the front doors call: the JSON API (`api`), and the app's server
//! functions. It checks every write against the whole plan with `mdbook-reef check`'s rules,
//! reading docs from the plan's own pages over the project's working tree, which no link may
//! leave, builds a plan into
//! the viewer's payload, renders each of its docs as the page the payload links to, and tells
//! the change stream of each write it applies. Its data types are `reef-api`'s, so the
//! JSON API's clients share them. It reads and writes through a `Store`, which keeps plans,
//! their nodes and their pages, and each node's work record, by version over a `Backend`: a `ShoalBackend`, which runs a pool in this process
//! with `embedded` or reaches an external node such as `reef-shoal`, or a `MemoryBackend`, its
//! in-memory twin. The rows reef keeps in shoal are its `schema`, the user's queue sorts by
//! `rank` strings, and reef's `conf` says how to
//! reach shoal and where to serve. Projects name the repos their plans describe, and `repo`
//! locates the one a working directory, or a linked worktree, is in. Before a pool starts,
//! `checks` refuses a config shoal would fail on obscurely, naming the fix. Without `embedded`
//! this is shoal's client half alone and builds on stable.

pub mod api;
pub mod backend;
#[cfg(feature = "embedded")]
pub mod checks;
pub mod client;
pub mod conf;
pub mod memory;
#[cfg(feature = "embedded")]
pub mod pool;
pub mod rank;
pub mod repo;
pub mod schema;
pub mod service;
pub mod store;

pub use backend::{Backend, Expect, Pointers, Refusal, RevisionKey, Written};
pub use client::{Mode, ShoalBackend};
pub use conf::{ApiSettings, Conf, ShoalSettings};
pub use memory::MemoryBackend;
pub use service::Service;
pub use store::{
    AddCapture, AddedProject, CaptureRead, CaptureShas, Captured, Checked, Listing, NodePut,
    NodeRemove, NodeWrite, NotListed, NotRegistered, PageEdit, PageEdited, PageListing, PageWrite,
    Place, PlanInfo, PlanWrite, Project, Proposal, Put, Puts, QueueEdit, Queued, Ranked, Rewrite,
    Rewritten, StalePut, StartWork, Started, Store, StoredNode, StoredPage, Unkeepable, WorkRecord,
    WorkStart,
};
