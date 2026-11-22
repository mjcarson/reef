//! reef's terminal client and MCP server: an HTTP client of the service's JSON API
//!
//! The service in `central/reef-service` is the only writer of reef's plans. `reef` reaches it
//! over its JSON API, speaking `reef-api`'s types, so it needs neither shoal nor Dioxus and builds
//! on stable.
//!
//! - **Commands** ([`cli`]): register and list projects, list a plan in a project and list a
//!   project's plans. The working directory picks the project, its main repo's in a linked
//!   worktree, and `--project` names one instead.
//! - **Nodes** ([`node`]): put, get and remove a plan's nodes as the YAML a book's frontmatter
//!   holds. `get` prints the version a node was read at as a comment, which `put` reads back, so
//!   a put made after another writer's is refused as stale.
//! - **Pages** ([`page`]): list, put, get and remove a plan's own markdown pages, which lie
//!   over its repo's files, by version as nodes are.
//! - **Work** ([`work`]): set a node's status and whether each criterion is met, check the plan,
//!   and say what is ready to start, the user's queue first, and what the rest waits on.
//! - **The queue** ([`queue`]): the user's order of work across a project's plans, added to,
//!   moved, taken off and listed, each entry with what next makes of it.
//! - **Import** ([`import`]): a book's plan read from its files, checked, and written into a
//!   plan as one revision, each inline member a node of its own, each page the plan's own, and
//!   each link leading where it did.
//! - **Export** ([`export`]): a plan written out as a new book, a file for each group with its
//!   members inline and a file for each page, which imports back as the same plan.
//! - **The MCP server** ([`mcp`]): the node and work commands as tools over stdio, for Claude
//!   Code, the ones that only read marked so, which plan mode runs without a prompt.
//! - **git** ([`git`]): the checkout a node's work happens in, read through gix, its worktree
//!   written as a tree through git's CLI, and its start pinned under `refs/reef/start/`.
//! - **The client** ([`client`]): one typed method a route. When nothing answers, it says where
//!   it looked and how to start the service, rather than starting one itself.
//! - **Config** ([`conf`]): `[api] address` alone, from the file the service reads.
//!
//! The binary is a thin `main` over [`run`], so the service's tests drive the same commands
//! against a service they serve.

pub mod cli;
pub mod client;
pub mod conf;
pub mod export;
pub mod git;
pub mod import;
pub mod mcp;
pub mod node;
pub mod page;
pub mod queue;
pub mod work;

pub use cli::{Cli, run};
pub use client::Client;
pub use conf::Conf;
