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
//! - **Starting work** ([`start`]): a node started in the working directory's checkout, its
//!   worktree, branch, base branch and start recorded and the start pinned, and the record of
//!   a node, or of the one a worktree is on, shown.
//! - **Capturing changes** ([`capture`]): a node's diff read in its own worktree, from its start
//!   carried forward to what is tracked there, and sent to be kept as a revision, with the
//!   threads its commits' `Reef-Resolves` trailers name.
//! - **Hooks** ([`hook`]): git's post-commit and post-rewrite, and Claude Code's PostToolUse on
//!   reef's status tool, keep the draft of the node a worktree is on current, and Stop seals it
//!   once the node is done.
//! - **Comments** ([`comment`]): a thread resolved by hand, the revision that answered it
//!   linked to it.
//! - **The queue** ([`queue`]): the user's order of work across a project's plans, added to,
//!   moved, taken off and listed, each entry with what next makes of it.
//! - **The loop** ([`loops`]): the terminal loop of the pane a session runs in, started,
//!   paused, stopped and listed, and handed to each session that starts in its pane by the
//!   SessionStart hook.
//! - **Import** ([`import`]): a book's plan read from its files, checked, and written into a
//!   plan as one revision, each inline member a node of its own, each page the plan's own, and
//!   each link leading where it did.
//! - **Export** ([`export`]): a plan written out as a new book, a file for each group with its
//!   members inline and a file for each page, which imports back as the same plan.
//! - **The MCP server** ([`mcp`]): the node and work commands as tools over stdio, for Claude
//!   Code, the ones that only read marked so, which plan mode runs without a prompt.
//! - **The skill** ([`skill`]): the Claude skill for central reef, written to the
//!   `.claude/skills/reef` of a repo, which keeps the plan in step with the work through the
//!   MCP tools and the CLI.
//! - **git** ([`git`]): the checkout a node's work happens in, read through gix, its worktree
//!   written as a tree through git's CLI, its start pinned under `refs/reef/start/`, and its
//!   diff, the carry of its start and its commits' trailers taken through gix.
//! - **The client** ([`client`]): one typed method a route. When nothing answers, it says where
//!   it looked and how to start the service, rather than starting one itself.
//! - **Config** ([`conf`]): `[api] address` alone, from the file the service reads.
//!
//! The binary is a thin `main` over [`run`], so the service's tests drive the same commands
//! against a service they serve.

pub mod capture;
pub mod cli;
pub mod client;
pub mod comment;
pub mod conf;
pub mod export;
pub mod git;
pub mod hook;
pub mod import;
pub mod loops;
pub mod mcp;
pub mod node;
pub mod page;
pub mod queue;
pub mod skill;
pub mod start;
pub mod work;

pub use cli::{Cli, run};
pub use client::Client;
pub use conf::Conf;
