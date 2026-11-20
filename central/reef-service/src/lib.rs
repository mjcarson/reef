//! reef's store and service: the only code that touches shoal
//!
//! The rows reef keeps in shoal are its `schema`, and reef's `conf` says how to reach shoal. A
//! `Store` reads and writes plans and their nodes by version, over a `Backend`: a `ShoalBackend`,
//! which runs a pool in this process with `embedded` or reaches an external node such as
//! `reef-shoal`, or a `MemoryBackend`, its in-memory twin. Projects name the repos their plans
//! describe, and `repo` locates the one a working directory, or a linked worktree, is in. Before a pool starts, `checks` refuses
//! a config shoal would fail on obscurely, naming the fix. Without `embedded` this is shoal's
//! client half alone and builds on stable.

pub mod backend;
#[cfg(feature = "embedded")]
pub mod checks;
pub mod client;
pub mod conf;
pub mod memory;
#[cfg(feature = "embedded")]
pub mod pool;
pub mod repo;
pub mod schema;
pub mod store;

pub use backend::{Backend, Expect, Pointers, Refusal, RevisionKey, Written};
pub use client::{Mode, ShoalBackend};
pub use conf::{Conf, ShoalSettings};
pub use memory::MemoryBackend;
pub use store::{
    AddedProject, Listing, NodeWrite, NotListed, NotRegistered, PlanInfo, Project, Put, Rewritten,
    Store, StoredNode,
};
