//! X3's spike: picking out one node's changes from its local checkout, and capturing them
//!
//! A node's changes are extracted from the checkout its work happens in, by a CLI an agent's
//! hook or a git hook runs. Nothing is pushed or fetched: the capture reads local commits, the
//! index and the worktree, and stores what it read in a state dir that stands in for shoal.
//!
//! - `git` runs git's CLI in one checkout. `gix` runs every step but the snapshot through gix,
//!   which the hybrid backend uses and the CLI is the test oracle for.
//! - `rule` holds the candidate rules for a diff's base, and `scenario` the scratch repos that
//!   try them; `matrix` prints how each rule did on each case.
//! - `capture` starts a node and captures it; `hook` is every trigger that captures; `mcp` is
//!   a stub reef MCP server whose `status` tool marks a node done.
//! - `bench` times the CLI's diff against gix's on real commits.

pub mod bench;
pub mod capture;
pub mod diff;
pub mod git;
pub mod gix;
pub mod hook;
pub mod matrix;
pub mod mcp;
pub mod rule;
pub mod scenario;
pub mod store;
