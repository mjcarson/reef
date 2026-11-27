//! reef's store and service: the only code that touches shoal
//!
//! A `Service` is what the front doors call: the JSON API (`api`), and the app's server
//! functions. It checks every write against the whole plan with `mdbook-reef check`'s rules,
//! reading docs from the plan's own pages over the project's working tree, which no link may
//! leave, builds a plan into
//! the viewer's payload, renders each of its docs as the page the payload links to, and tells
//! the change stream of each write it applies. Its data types are `reef-api`'s, so the
//! JSON API's clients share them. It reads and writes through a `Store`, which keeps plans,
//! their nodes and their pages, each node's work record and captured revisions, and each plan's comments, by version over a `Backend`: a `ShoalBackend`, which runs a pool in this process
//! with `embedded` or reaches an external node such as `reef-shoal`, or a `MemoryBackend`, its
//! in-memory twin. The rows reef keeps in shoal are its `schema`, the user's queue sorts by
//! `rank` strings, and reef's `conf` says how to
//! reach shoal and where to serve: HTTPS on loopback, and beside it for other hosts (`tls`),
//! with a certificate of reef's own (`certs`) unless one is named, every write a browser sends
//! judged by its `origin`. Projects name the repos their plans describe, and `repo`
//! locates the one a working directory, or a linked worktree, is in. A project registered by its
//! git URL is a remote one: `remote` checks the URL and keeps a clone of it, and `tree` reads its
//! docs at the commit last fetched. Before a pool starts,
//! `checks` refuses a config shoal would fail on obscurely, naming the fix. Without `embedded`
//! this is shoal's client half alone and builds on stable.

pub mod api;
pub mod auth;
pub mod backend;
pub mod certs;
#[cfg(feature = "embedded")]
pub mod checks;
pub mod client;
pub mod conf;
pub mod memory;
pub mod origin;
#[cfg(feature = "embedded")]
pub mod pool;
pub mod rank;
pub mod remote;
pub mod repo;
pub mod schema;
pub mod service;
pub mod store;
pub mod tls;
pub mod tree;

pub use backend::{Backend, Expect, Pointers, Refusal, RevisionKey, RowKey, Written};
pub use client::{Mode, ShoalBackend};
pub use conf::{ApiSettings, AuthSettings, Conf, RemoteSettings, ShoalSettings, TlsSettings};
pub use memory::MemoryBackend;
pub use service::{Asker, NoUser, NotAllowed, NotSignedIn, Recovered, Service, TooManyTries};
pub use store::{
    AddCapture, AddedProject, BadCursor, CaptureRead, CaptureShas, Captured, Checked, FILE_ROW_MAX,
    FileDiff, FileSummary, Followed, KeptRemote, Listing, NodePut, NodeRemove, NodeWrite,
    NotListed, NotRegistered, PageEdit, PageEdited, PageListing, PageWrite, Pending, Place,
    PlanInfo, PlanWrite, Project, Proposal, Put, Puts, QueueEdit, Queued, Ranked, Revision,
    Rewrite, Rewritten, StalePut, StartWork, Started, Store, StoredNode, StoredPage, StoredRemote,
    StoredToken, StoredUser, Unkeepable, UserDropped, UserKept, WorkRecord, WorkStart,
};
