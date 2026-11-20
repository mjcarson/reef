//! reef's store and service: the only code that touches shoal
//!
//! The rows reef keeps in shoal are its `schema`, and reef's `conf` says how to reach shoal. A
//! `Store` reaches it through a `ShoalBackend`: a pool started in this process with `embedded`,
//! else an external node, such as `reef-shoal`. Before a pool starts, `checks` refuses a config
//! shoal would fail on obscurely, naming the fix. Without `embedded` this is shoal's client half
//! alone and builds on stable.

#[cfg(feature = "embedded")]
pub mod checks;
pub mod client;
pub mod conf;
#[cfg(feature = "embedded")]
pub mod pool;
pub mod schema;
pub mod store;

pub use client::{Mode, ShoalBackend};
pub use conf::{Conf, ShoalSettings};
pub use store::Store;
