//! reef's terminal client: an HTTP client of the service's JSON API
//!
//! The service in `central/reef-service` is the only writer of reef's plans. `reef` reaches it
//! over its JSON API, speaking `reef-api`'s types, so it needs neither shoal nor Dioxus and builds
//! on stable.
//!
//! - **Commands** ([`cli`]): register and list projects, list a plan in a project and list a
//!   project's plans. The working directory picks the project, its main repo's in a linked
//!   worktree, and `--project` names one instead.
//! - **The client** ([`client`]): one typed method a route. When nothing answers, it says where
//!   it looked and how to start the service, rather than starting one itself.
//! - **Config** ([`conf`]): `[api] address` alone, from the file the service reads.
//!
//! The binary is a thin `main` over [`run`], so the service's tests drive the same commands
//! against a service they serve.

pub mod cli;
pub mod client;
pub mod conf;

pub use cli::{Cli, run};
pub use client::Client;
pub use conf::Conf;
