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
//! - **Clearing between nodes** ([`clear`]): once a loop's node is done, Stop's injector types
//!   `/clear` and the next node's kickoff into the session's zellij or tmux pane, once Claude is
//!   idle and its input box empty.
//! - **The runner** ([`run`]): `reef loop run` runs a loop's sessions headless here, a fresh one
//!   a node, and relays each one's transcript and prompts through central reef to the app.
//! - **Decisions** ([`decisions`]): each plan a session asked approval for and its verdict, and
//!   each question and the answer picked, read from its transcript, which Stop and SessionEnd
//!   keep with the nodes they were made on.
//! - **Comments** ([`comment`]): a plan's threads listed whole, replied in, and resolved by
//!   hand, the revision that answered one linked to it.
//! - **The queue** ([`queue`]): the user's order of work across a project's plans, added to,
//!   moved, taken off and listed, each entry with what next makes of it.
//! - **The loop** ([`loops`]): the terminal loop of the pane a session runs in, started,
//!   paused, stopped and listed, and handed to each session that starts in its pane by the
//!   SessionStart hook.
//! - **next_task** ([`task`]): the next node taken for a pane's loop, the queue's first, started
//!   in the session's checkout and handed back whole, or what everything waits on. A loop wider
//!   than one node takes a group, each member worked by a session of its own. A done node whose
//!   comments want an answer is taken again first, for a round of them.
//! - **A node's worktree** ([`worktree`]): `.claude/worktrees/<id>` on `reef/<id>`, made for
//!   `claude -w <id>` or `reef loop attach`, with the node started in it.
//! - **A member's pane** ([`open`]): `claude -w <id>` opened beside the loop's, in its zellij
//!   session or on its tmux server.
//! - **Import** ([`import`]): a book's plan read from its files, checked, and written into a
//!   plan as one revision, each inline member a node of its own, each page the plan's own, and
//!   each link leading where it did.
//! - **Export** ([`export`]): a plan written out as a new book, a file for each group with its
//!   members inline and a file for each page, which imports back as the same plan.
//! - **The MCP server** ([`mcp`]): the node, work and comment commands as tools over stdio, for
//!   Claude Code, the ones that only read marked so, which plan mode runs without a prompt.
//! - **The skill** ([`skill`]): the Claude skill for central reef, written to the
//!   `.claude/skills/reef` of a repo, which keeps the plan in step with the work through the
//!   MCP tools and the CLI.
//! - **Setting a repo up** ([`claude`]): reef's MCP entry, Claude Code's and git's hooks, the
//!   local settings that enable the server and allow its tools, and the skill, written by
//!   `reef claude install`, which changes nothing run again.
//! - **git** ([`git`]): the checkout a node's work happens in, read through gix, its worktree
//!   written as a tree through git's CLI, its start pinned under `refs/reef/start/`, and its
//!   diff, the carry of its start and its commits' trailers taken through gix.
//! - **The client** ([`client`]): one typed method a route. When nothing answers, it says where
//!   it looked and how to start the service, rather than starting one itself.
//! - **Config** ([`conf`]): `[api] address` alone, from the file the service reads.
//! - **Credentials** ([`credentials`]): the token each service issued at `reef login`, kept a
//!   server each in a file private to the user, and sent with every request.
//! - **Signing in** ([`login`]): `reef login`, `logout` and `whoami`, and adding and listing the
//!   service's users, a password asked for on the terminal with echo off.
//!
//! The binary is a thin `main` over [`run`], so the service's tests drive the same commands
//! against a service they serve.

pub mod capture;
pub mod claude;
pub mod clear;
pub mod cli;
pub mod client;
pub mod comment;
pub mod conf;
pub mod credentials;
pub mod decisions;
pub mod export;
pub mod git;
pub mod hook;
pub mod import;
pub mod login;
pub mod loops;
pub mod mcp;
pub mod node;
pub mod open;
pub mod page;
pub mod queue;
pub mod run;
pub mod skill;
pub mod start;
pub mod task;
pub mod work;
pub mod worktree;

pub use cli::{Cli, run};
pub use client::Client;
pub use conf::Conf;
