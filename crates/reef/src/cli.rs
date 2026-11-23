//! reef's commands: projects, plans and nodes, each a few calls to the JSON API
//!
//! ```text
//! reef [--conf PATH] project add <NAME> [--repo DIR]
//! reef [--conf PATH] project ls
//! reef [--conf PATH] project show [--project NAME]
//! reef [--conf PATH] plan new <NAME> [--project NAME]
//! reef [--conf PATH] plan ls [--project NAME]
//! reef [--conf PATH] node put [FILE] [--project NAME] [--plan NAME]
//! reef [--conf PATH] node get <ID> [--md] [--project NAME] [--plan NAME]
//! reef [--conf PATH] node rm <ID>... [--project NAME] [--plan NAME]
//! reef [--conf PATH] node capture [ID] [--project NAME] [--plan NAME]
//! reef [--conf PATH] page ls [--project NAME] [--plan NAME]
//! reef [--conf PATH] page get <PATH> [--project NAME] [--plan NAME]
//! reef [--conf PATH] page put <PATH> [FILE] [--project NAME] [--plan NAME]
//! reef [--conf PATH] page rm <PATH> [--project NAME] [--plan NAME]
//! reef [--conf PATH] start <ID> [--base BRANCH] [--restart] [--project NAME] [--plan NAME]
//! reef [--conf PATH] work [ID] [--project NAME] [--plan NAME]
//! reef [--conf PATH] comment resolve <N> [--node ID] [--revision N] [--project NAME] [--plan NAME]
//! reef [--conf PATH] status <ID> <STATUS> [--project NAME] [--plan NAME]
//! reef [--conf PATH] accept <ID> [<N> <STATUS> [--test TEST]] [--project NAME] [--plan NAME]
//! reef [--conf PATH] check [--project NAME] [--plan NAME]
//! reef [--conf PATH] next [--project NAME] [--plan NAME]
//! reef [--conf PATH] queue add <ID> [--top | --bottom | --before ID | --after ID] [--project NAME] [--plan NAME]
//! reef [--conf PATH] queue move <ID> (--top | --bottom | --before ID | --after ID) [--project NAME] [--plan NAME]
//! reef [--conf PATH] queue rm <ID>... [--project NAME] [--plan NAME]
//! reef [--conf PATH] queue ls [--project NAME]
//! reef [--conf PATH] loop start [--plan NAME] [--pane KEY] [--session ID] [--project NAME]
//! reef [--conf PATH] loop pause [--pane KEY]
//! reef [--conf PATH] loop stop [--pane KEY]
//! reef [--conf PATH] loop ls [--project NAME]
//! reef [--conf PATH] loop take [--pane KEY] [--project NAME]
//! reef [--conf PATH] hook session-start [--pane KEY]
//! reef [--conf PATH] hook post-commit
//! reef [--conf PATH] hook post-rewrite <KIND>
//! reef [--conf PATH] hook post-tool-use
//! reef [--conf PATH] hook stop [--pane KEY]
//! reef [--conf PATH] hook clear --pane KEY --session ID --pid PID --after NODE
//! reef [--conf PATH] mcp [--project NAME] [--plan NAME]
//! reef [--conf PATH] import [BOOK] [--replace] [--project NAME] [--plan NAME]
//! reef [--conf PATH] export <DIR> [--replace] [--project NAME] [--plan NAME]
//! reef skill [DIR] [--print]
//! ```
//!
//! The working directory picks the project: the registered project whose repo holds it, its main
//! repo's for a linked worktree. `--project` names one instead. A node's plan is the project's
//! only plan, unless `--plan` names one, and so is a page's. `start` and `node capture` read the
//! checkout they run in, which must be the project's repo or a linked worktree of it, and a
//! capture's node must be worked there. `comment resolve` links the latest revision of the node
//! the working directory's worktree is on, unless `--node` names one. `next` and the queue span
//! the project's plans, and `--plan` narrows `next` to one. `loop` works on the loop of the pane
//! it runs in, zellij's or tmux's or else the Claude Code process that ran it, unless `--pane`
//! names one, and a loop spans the project's plans unless `--plan` names one. `loop take`, as
//! the MCP tool next_task, takes the loop's next node and starts it in the working directory's
//! checkout. The capture hooks work on the node the working directory's worktree is on, and do
//! nothing where there is none. `import` picks the project the book is in, and lists the plan
//! `--plan` names if it isn't listed. `export` picks the project the new book's directory is in.
//! `skill` writes into the repo that holds its directory, and needs neither reef's config nor
//! its service.

use clap::{Args, Parser, Subcommand};
use reef_api::{AddedProject, Project};
use reef_model::{Met, PlanName, Status};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::capture;
use crate::clear;
use crate::client::Client;
use crate::comment;
use crate::conf::Conf;
use crate::export;
use crate::hook;
use crate::import;
use crate::loops;
use crate::mcp::{self, Tools};
use crate::node;
use crate::page;
use crate::queue;
use crate::skill;
use crate::start;
use crate::task;
use crate::work;

/// reef's terminal client: projects, plans and nodes, through the service's JSON API
#[derive(Debug, Parser)]
#[command(name = "reef", version, about)]
pub struct Cli {
    /// reef's config, else `$REEF_CONFIG`, else `$XDG_CONFIG_HOME/reef/config.toml`
    #[arg(long, global = true, value_name = "PATH")]
    pub conf: Option<PathBuf>,
    /// What to do
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Parses a command line, the program's name first, without clap in the caller's tree
    ///
    /// # Arguments
    ///
    /// * `args` - The command line, such as `["reef", "plan", "ls"]`
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::Cli;
    ///
    /// let cli = Cli::from_args(["reef", "plan", "new", "central"]).unwrap();
    /// assert_eq!(cli.conf, None);
    /// ```
    pub fn from_args<I, T>(args: I) -> Result<Cli, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        <Cli as Parser>::try_parse_from(args)
    }
}

/// reef's commands
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Register, list and show projects
    #[command(subcommand)]
    Project(ProjectCommand),
    /// List a plan in a project, and list a project's plans
    #[command(subcommand)]
    Plan(PlanCommand),
    /// Put, get and remove a plan's nodes as YAML
    #[command(subcommand)]
    Node(NodeCommand),
    /// List, put, get and remove a plan's own pages as markdown
    ///
    /// A plan's pages lie over its repo's files, and nodes link to them as they do to the repo's.
    #[command(subcommand)]
    Page(PageCommand),
    /// Start a node here: record this checkout's worktree, branch, base branch and start, pin
    /// the start, and mark a node to do in progress
    ///
    /// The start is HEAD and the worktree written as a tree, anything uncommitted included, so
    /// work an earlier node left uncommitted is the start's. It is pinned under
    /// `refs/reef/start/<plan>/<id>`, so a gc after a rebase can't prune it.
    Start {
        /// The node's id
        id: String,
        /// The branch other work comes in on, else the repo's default: the branch `origin/HEAD`
        /// names, else `main` or `master`
        #[arg(long, value_name = "BRANCH")]
        base: Option<String>,
        /// Start a node started already again, from here, replacing its record
        #[arg(long)]
        restart: bool,
        /// The plan it is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Show a node's work record: where its work happens and what it started from
    ///
    /// Without an id, the node this worktree is on: the one started here last, in any of the
    /// project's plans.
    Work {
        /// The node's id, else the node this worktree is on
        id: Option<String>,
        /// The plan a node named is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Resolve a plan's comment threads by the changes that answered them
    #[command(subcommand)]
    Comment(CommentCommand),
    /// Set where a node's work stands: todo, in-progress, done or blocked
    ///
    /// A group's status is derived from its members, so only a member's is set.
    Status {
        /// The node's id
        id: String,
        /// Where its work now stands
        #[arg(value_name = "STATUS", value_parser = work::parse_status)]
        status: Status,
        /// The plan it is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// List a node's acceptance criteria, or set whether one is met: met, unmet or todo
    ///
    /// Criteria are numbered from 1, in the order written. A performance criterion with a
    /// target is met or not by its metric's figures, so its status isn't set here.
    Accept {
        /// The node's id
        id: String,
        /// The criterion's number, from the list this prints without one
        #[arg(value_name = "N", requires = "met")]
        criterion: Option<usize>,
        /// Whether it is met
        #[arg(value_name = "STATUS", value_parser = work::parse_met)]
        met: Option<Met>,
        /// The test that checks it, such as `tests/scan.rs::dropped_client`
        #[arg(long, value_name = "TEST", requires = "criterion")]
        test: Option<String>,
        /// The plan it is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Print the plan's problems, as `mdbook-reef check` prints a book's, failing if it has any
    Check {
        /// The plan to check, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Print the work ready to start, the queue's first, then in plan order, and what the rest
    /// waits on
    ///
    /// Across the project's plans, unless `--plan` names one, which leaves the queue's entries
    /// of other plans out.
    Next {
        /// The plan to look in, if not all of the project's
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Order the work to take first: the project's queue, which next follows
    #[command(subcommand)]
    Queue(QueueCommand),
    /// Import a book's plan into a plan as one revision: each node of its files a node of the
    /// plan, inline members and all, with its links leading from the repo's root
    ///
    /// The book is checked first, and one with any problem imports nothing. A plan named with
    /// `--plan` is listed if it isn't. A plan that has nodes loses those the book doesn't have,
    /// so it is replaced only with `--replace`.
    Import {
        /// The book's directory, which holds its `book.toml`
        #[arg(value_name = "BOOK", default_value = ".")]
        book: PathBuf,
        /// Replace the plan's nodes, if it has any
        #[arg(long)]
        replace: bool,
        /// The plan to import into, if not the project's only plan, and the project the book
        /// is in, if not the one its directory belongs to
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Export a plan to a new book: a file for each group, its members listed inline, with its
    /// links leading from the book's files
    ///
    /// The directory must be new or empty, and in the project's repo. Importing the book gives
    /// the plan back. With `--replace` it is a reef book instead, which the plan is written back
    /// over, keeping the book's files, its summary and its `book.toml`.
    Export {
        /// The directory to write the book in
        #[arg(value_name = "DIR")]
        dir: PathBuf,
        /// Write over the reef book in the directory, keeping its layout: each node's file, its
        /// summary and its `book.toml`
        #[arg(long)]
        replace: bool,
        /// The plan to export, if not the project's only plan, and the project it is in, if not
        /// the one the directory belongs to
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Start, pause, stop and list the terminal loops that feed sessions their nodes
    ///
    /// A loop is kept by the pane its session runs in, which every `/clear` keeps, so run these
    /// in the session's own pane, as `! reef loop start`, or name one with `--pane`.
    #[command(subcommand)]
    Loop(LoopCommand),
    /// Claude Code's and git's hooks, each of Claude Code's handed its event's input on
    /// standard input
    #[command(subcommand)]
    Hook(HookCommand),
    /// Serve get, check, next, next_task, put, status and accept as MCP tools over standard
    /// input and output, for Claude Code
    ///
    /// get, check and next only read, and say so, so plan mode runs them without a prompt.
    /// next_task takes nodes for the loop of the pane the server runs in: its multiplexer's,
    /// else the Claude Code process that started it.
    Mcp {
        /// The project and plan the tools work in, if not the working directory's and its
        /// only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Write the Claude skill for central reef to `.claude/skills/reef` at the root of the repo
    /// that holds a directory, over the mdbook skill if it is there
    ///
    /// It teaches Claude to keep the plan in step with the work through reef's MCP tools and
    /// this CLI, and names the tools and commands of the reef that wrote it.
    Skill {
        /// A directory in the repo
        #[arg(value_name = "DIR", default_value = ".")]
        dir: PathBuf,
        /// Print its SKILL.md instead of writing it
        #[arg(long)]
        print: bool,
    },
}

/// What can be done with projects
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Register a project for the repo that holds a directory
    Add {
        /// The project's name
        name: String,
        /// A directory in its repo, else the working directory
        #[arg(long, value_name = "DIR")]
        repo: Option<PathBuf>,
    },
    /// List every registered project and its repo
    Ls,
    /// Show the project the working directory belongs to
    Show {
        /// The project to show instead
        #[command(flatten)]
        pick: Pick,
    },
}

/// What can be done with a project's plans
#[derive(Debug, Subcommand)]
pub enum PlanCommand {
    /// List a new, empty plan in the project
    New {
        /// The plan's name within its project
        name: String,
        /// The project to list it in, if not the working directory's
        #[command(flatten)]
        pick: Pick,
    },
    /// List the project's plans and the revision each is at
    Ls {
        /// The project whose plans to list, if not the working directory's
        #[command(flatten)]
        pick: Pick,
    },
}

/// What can be done with a plan's nodes
#[derive(Debug, Subcommand)]
pub enum NodeCommand {
    /// Put a node's YAML or markdown and its inline members, unless it is stale or breaks the
    /// plan
    ///
    /// YAML or frontmatter that starts with the `# version:` line `get` printed changes the
    /// node got; without one it is a new node. YAML keeps the node's body, and markdown, a
    /// node file's frontmatter between `---` lines and then its body, replaces it.
    Put {
        /// The YAML or markdown to put, else standard input, which `-` names too
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
        /// The plan to put it in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Print a node's YAML, after the `# version:` line a put of it keeps
    Get {
        /// The node's id
        id: String,
        /// Print it as a node file: its frontmatter, the `# version:` line inside it, then its
        /// body, whose links lead from the repo's root
        #[arg(long)]
        md: bool,
        /// The plan it is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Remove the nodes named as one change, unless it breaks the plan
    ///
    /// A group goes only with its members named too, and a node another needs only once
    /// nothing needs it.
    Rm {
        /// The ids of the nodes to remove
        #[arg(required = true, value_name = "ID")]
        ids: Vec<String>,
        /// The plan they are in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Capture a node's changes in this worktree, and keep them as a revision of its own
    ///
    /// The diff runs from the node's start, carried over whatever its base branch brought in
    /// since, to this worktree: every tracked file as it is, staged new ones included, and
    /// untracked files left out unless its start had them. Only this checkout is read.
    Capture {
        /// The node's id, else the node this worktree is on: the one started here last
        id: Option<String>,
        /// The plan a node named is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
}

/// What can be done with a pane's loop
#[derive(Debug, Subcommand)]
pub enum LoopCommand {
    /// Start a loop in this pane, feeding its session the project's nodes, or set its paused
    /// loop running again, moving on to the next node when it is between nodes
    Start {
        /// The pane, as `reef loop ls` prints it, else this one
        #[arg(long, value_name = "KEY")]
        pane: Option<String>,
        /// The session feeding it, else the one this runs in, if Claude Code ran it
        #[arg(long, value_name = "ID")]
        session: Option<String>,
        /// The one plan to take nodes from, else every plan of the project
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Pause this pane's loop, so it stops once the node it is on is done
    Pause {
        /// The pane, as `reef loop ls` prints it, else this one
        #[arg(long, value_name = "KEY")]
        pane: Option<String>,
    },
    /// Stop this pane's loop, letting go of the node it holds
    Stop {
        /// The pane, as `reef loop ls` prints it, else this one
        #[arg(long, value_name = "KEY")]
        pane: Option<String>,
    },
    /// List the loops feeding the project: each pane, whether it runs, its plan, its node and
    /// its session
    Ls {
        /// The project whose loops to list, if not the working directory's
        #[command(flatten)]
        pick: Pick,
    },
    /// Take this pane's loop's next node and start it here, as the MCP tool next_task does: the
    /// same node again until it is done, else the queue's first ready one no other loop holds
    Take {
        /// The pane, as `reef loop ls` prints it, else this one
        #[arg(long, value_name = "KEY")]
        pane: Option<String>,
        /// The project whose node to take, if not the working directory's
        #[command(flatten)]
        pick: Pick,
    },
}

/// Claude Code's and git's hooks
#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// SessionStart: hand this pane's loop to the session that started, a clear's included
    SessionStart {
        /// The pane, else the one the session runs in
        #[arg(long, value_name = "KEY")]
        pane: Option<String>,
    },
    /// git's post-commit: draft the node this worktree is on, unless a rebase is under way
    PostCommit,
    /// git's post-rewrite: draft the node this worktree is on once a rebase ends
    PostRewrite {
        /// What rewrote the commits, as git passes it: `rebase`, or `amend`, which post-commit
        /// drafted already
        #[arg(value_name = "KIND")]
        kind: String,
    },
    /// PostToolUse, after reef's status tool: draft the node it marked done, if this worktree
    /// is on it
    PostToolUse,
    /// Stop: seal the node this worktree is on once it is done, its draft or its first revision,
    /// then act once on the end of the node this pane's loop holds, clearing the session in a
    /// zellij or tmux pane when another node is next
    Stop {
        /// The pane, else the one the session runs in
        #[arg(long, value_name = "KEY")]
        pane: Option<String>,
    },
    /// The injector, which Stop, or a start between nodes, starts in the background: once the
    /// session is idle and its input box empty, type `/clear` and the next node's kickoff into
    /// its zellij or tmux pane
    Clear {
        /// The pane to type into, as `reef loop ls` prints it
        #[arg(long, value_name = "KEY")]
        pane: String,
        /// The session running in the pane, which is cleared
        #[arg(long, value_name = "ID")]
        session: String,
        /// The Claude Code process the session runs in, `CLAUDE_PID`
        #[arg(long, value_name = "PID")]
        pid: u32,
        /// The node whose end the loop acted on, which it must still hold
        #[arg(long, value_name = "NODE")]
        after: String,
    },
}

/// What can be done with a plan's comment threads
#[derive(Debug, Subcommand)]
pub enum CommentCommand {
    /// Resolve a thread, linking the revision that answered it, as a commit's `Reef-Resolves`
    /// trailer does when it is captured
    ///
    /// The revision is the latest of the node this worktree is on, the one started here last,
    /// unless `--node` and `--revision` name another.
    Resolve {
        /// The thread's number in its plan
        #[arg(value_name = "N")]
        thread: u32,
        /// The node whose revision answered it: a bare id in the thread's plan, or
        /// `project/plan/id`
        #[arg(long, value_name = "ID")]
        node: Option<String>,
        /// The revision that answered it, else the node's latest
        #[arg(long, value_name = "N")]
        revision: Option<u32>,
        /// The plan the thread is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
}

/// What can be done with a plan's own pages
#[derive(Debug, Subcommand)]
pub enum PageCommand {
    /// List the plan's pages by path, each at its version
    Ls {
        /// The plan whose pages to list, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Print a page's markdown, after the `<!-- version: -->` line a put of it keeps
    Get {
        /// The page's path, as a doc link names it
        path: String,
        /// The plan it is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Put a page's markdown, unless it is stale or breaks the plan
    ///
    /// Markdown whose first line is the `<!-- version: -->` line `get` printed changes the page
    /// got; markdown without one is a new page. A page hides the repo's file at its path.
    Put {
        /// The page's path, as a doc link names it
        path: String,
        /// The markdown to put, else standard input, which `-` names too
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
        /// The plan to put it in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Take a page out of the plan, unless a node's doc link still names it
    Rm {
        /// The page's path
        path: String,
        /// The plan it is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
}

/// What can be done with a project's queue
#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    /// Queue a node, at the bottom unless a place is given, moving it if it is queued already
    Add {
        /// The node: a bare id in the plan, or `project/plan/id`
        id: String,
        /// Where it goes
        #[command(flatten)]
        at: At,
        /// The plan a bare id is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Move a queued node
    Move {
        /// The node: a bare id in the plan, or `project/plan/id`
        id: String,
        /// Where it goes, which a move needs
        #[command(flatten)]
        at: At,
        /// The plan a bare id is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Take nodes off the queue
    Rm {
        /// The nodes: bare ids in the plan, or `project/plan/id`
        #[arg(required = true, value_name = "ID")]
        ids: Vec<String>,
        /// The plan a bare id is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// List the queue, each entry ready, waiting on what, in progress, blocked, done or missing
    Ls {
        /// The project whose queue to list, if not the working directory's
        #[command(flatten)]
        pick: Pick,
    },
}

/// Where in the queue a node goes: at one end, or beside a queued node
#[derive(Debug, Clone, Default, Args)]
#[group(multiple = false)]
pub struct At {
    /// First, ahead of every other entry
    #[arg(long)]
    pub top: bool,
    /// Last, after every other entry
    #[arg(long)]
    pub bottom: bool,
    /// Just before a queued node: a bare id in this node's plan, or `project/plan/id`
    #[arg(long, value_name = "ID")]
    pub before: Option<String>,
    /// Just after a queued node: a bare id in this node's plan, or `project/plan/id`
    #[arg(long, value_name = "ID")]
    pub after: Option<String>,
}

/// A project named on the command line, else the working directory's
#[derive(Debug, Clone, Default, Args)]
pub struct Pick {
    /// The project to use, instead of the one the working directory belongs to
    #[arg(long, value_name = "NAME")]
    pub project: Option<String>,
}

/// A plan named on the command line, else the picked project's only plan
#[derive(Debug, Clone, Default, Args)]
pub struct PlanPick {
    /// The project the plan is in
    #[command(flatten)]
    pub project: Pick,
    /// The plan to use within the project, instead of its only plan
    #[arg(long, value_name = "NAME")]
    pub plan: Option<String>,
}

/// Runs a command against the service reef's config names
///
/// # Arguments
///
/// * `cli` - The command line, parsed
/// * `cwd` - The working directory, absolute, which picks the project and anchors relative paths
/// * `out` - Where to print what the command found or did
///
/// # Examples
///
/// ```no_run
/// use clap::Parser;
/// use reef::Cli;
///
/// # async fn example() -> anyhow::Result<()> {
/// let cli = Cli::parse_from(["reef", "plan", "ls"]);
/// reef::run(cli, &std::env::current_dir()?, &mut std::io::stdout()).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run<W: Write>(cli: Cli, cwd: &Path, out: &mut W) -> anyhow::Result<()> {
    // the skill only writes files, so a broken config or a stopped service can't stop it
    if let Command::Skill { dir, print } = &cli.command {
        return match print {
            true => skill::print(out),
            false => skill::write(&cwd.join(dir), out).map(|_| ()),
        };
    }
    // the config named, read from the working directory, else the one the service reads
    let named = cli.conf.map(|path| cwd.join(path));
    let conf = Conf::load(named.as_deref())?;
    let client = Client::new(&conf)?;
    match cli.command {
        Command::Project(ProjectCommand::Add { name, repo }) => {
            project_add(&client, cwd, &name, repo.as_deref(), out).await
        }
        Command::Project(ProjectCommand::Ls) => project_ls(&client, out).await,
        Command::Project(ProjectCommand::Show { pick }) => {
            project_show(&client, cwd, &pick, out).await
        }
        Command::Plan(PlanCommand::New { name, pick }) => {
            plan_new(&client, cwd, &name, &pick, out).await
        }
        Command::Plan(PlanCommand::Ls { pick }) => plan_ls(&client, cwd, &pick, out).await,
        Command::Node(NodeCommand::Put { file, pick }) => {
            // the YAML first, so a file that can't be read asks nothing of the service
            let text = node::read_input(cwd, file.as_deref())?;
            let plan = picked_plan(&client, cwd, &pick).await?;
            node::put(&client, &plan, &text, out).await
        }
        Command::Node(NodeCommand::Get { id, md, pick }) => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            node::get(&client, &plan, &id, md, out).await
        }
        Command::Node(NodeCommand::Rm { ids, pick }) => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            node::rm(&client, &plan, &ids, out).await
        }
        Command::Node(NodeCommand::Capture { id, pick }) => {
            capture::capture(&client, cwd, id.as_deref(), &pick, out).await
        }
        Command::Page(PageCommand::Ls { pick }) => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            page::ls(&client, &plan, out).await
        }
        Command::Page(PageCommand::Get { path, pick }) => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            page::get(&client, &plan, &path, out).await
        }
        Command::Page(PageCommand::Put { path, file, pick }) => {
            // the markdown first, so a file that can't be read asks nothing of the service
            let text = node::read_input(cwd, file.as_deref())?;
            let plan = picked_plan(&client, cwd, &pick).await?;
            page::put(&client, &plan, &path, &text, out).await
        }
        Command::Page(PageCommand::Rm { path, pick }) => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            page::rm(&client, &plan, &path, out).await
        }
        Command::Start {
            id,
            base,
            restart,
            pick,
        } => start::start(&client, cwd, &id, &pick, base.as_deref(), restart, out).await,
        Command::Work { id, pick } => start::work(&client, cwd, id.as_deref(), &pick, out).await,
        Command::Comment(CommentCommand::Resolve {
            thread,
            node,
            revision,
            pick,
        }) => comment::resolve(&client, cwd, thread, node.as_deref(), revision, &pick, out).await,
        Command::Status { id, status, pick } => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            work::status(&client, &plan, &id, status, out).await
        }
        Command::Accept {
            id,
            criterion,
            met,
            test,
            pick,
        } => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            // a criterion named comes with whether it is met, which clap requires
            let set = criterion.zip(met);
            work::accept(&client, &plan, &id, set, test.as_deref(), out).await
        }
        Command::Check { pick } => {
            // each problem, then a plan with any fails, as `mdbook-reef check` does
            let plan = picked_plan(&client, cwd, &pick).await?;
            let count = work::check(&client, &plan, out).await?;
            if count > 0 {
                anyhow::bail!("{}", work::problems(&plan, count));
            }
            writeln!(out, "{}", work::problems(&plan, 0))?;
            Ok(())
        }
        Command::Next { pick } => {
            // one plan named, else every plan of the project
            match pick.plan {
                Some(_) => {
                    let plan = picked_plan(&client, cwd, &pick).await?;
                    work::next(&client, &plan, out).await
                }
                None => {
                    let project = picked(&client, cwd, &pick.project).await?;
                    work::project_next(&client, &project.name, out).await
                }
            }
        }
        Command::Queue(QueueCommand::Add { id, at, pick }) => {
            queue::add(&client, cwd, &id, &at, &pick, out).await
        }
        Command::Queue(QueueCommand::Move { id, at, pick }) => {
            queue::mv(&client, cwd, &id, &at, &pick, out).await
        }
        Command::Queue(QueueCommand::Rm { ids, pick }) => {
            queue::rm(&client, cwd, &ids, &pick, out).await
        }
        Command::Queue(QueueCommand::Ls { pick }) => {
            let project = picked(&client, cwd, &pick).await?;
            queue::ls(&client, &project.name, out).await
        }
        Command::Import {
            book,
            replace,
            pick,
        } => import::import(&client, cwd, &book, &pick, replace, out).await,
        Command::Export { dir, replace, pick } => {
            export::export(&client, cwd, &dir, &pick, replace, out).await
        }
        Command::Loop(command) => {
            // the pane named, else the one this process runs in, as its environment says
            let var = |name: &str| std::env::var(name).ok();
            match command {
                LoopCommand::Start {
                    pane,
                    session,
                    pick,
                } => {
                    let here = loops::Here::find(pane.as_deref(), var)?;
                    // between nodes, the injector reaches the same service, as Stop's does
                    let spawn = |order: &clear::Clear| clear::spawn(order, named.as_deref());
                    loops::start(&client, cwd, &here, session.as_deref(), &pick, spawn, out).await
                }
                LoopCommand::Pause { pane } => {
                    let here = loops::Here::find(pane.as_deref(), var)?;
                    loops::pause(&client, &here, out).await
                }
                LoopCommand::Stop { pane } => {
                    let here = loops::Here::find(pane.as_deref(), var)?;
                    loops::stop(&client, &here, out).await
                }
                LoopCommand::Ls { pick } => {
                    let here = reef_api::Pane::from_env(var);
                    loops::ls(&client, cwd, here.as_ref(), &pick, out).await
                }
                LoopCommand::Take { pane, pick } => {
                    let here = loops::Here::find(pane.as_deref(), var)?;
                    task::next_task(&client, cwd, Some(&here.pane), &pick, out).await
                }
            }
        }
        Command::Hook(HookCommand::SessionStart { pane }) => {
            // the session's pane, then the event Claude Code handed the hook
            let var = |name: &str| std::env::var(name).ok();
            let here = loops::Here::find(pane.as_deref(), var)?;
            let input = stdin()?;
            loops::session_start(&client, &input, &here.pane, out).await
        }
        Command::Hook(HookCommand::PostCommit) => hook::post_commit(&client, cwd, out).await,
        Command::Hook(HookCommand::PostRewrite { kind }) => {
            // git writes the commits it rewrote to the hook, which are drained unread
            stdin()?;
            hook::post_rewrite(&client, cwd, &kind, out).await
        }
        Command::Hook(HookCommand::PostToolUse) => {
            let input = stdin()?;
            hook::post_tool_use(&client, cwd, &input, out).await
        }
        Command::Hook(HookCommand::Stop { pane }) => {
            // the pane named, else the session's, and none is no loop to end a node of
            let var = |name: &str| std::env::var(name).ok();
            let pane = match pane {
                Some(key) => Some(key.parse::<reef_api::Pane>()?),
                None => reef_api::Pane::from_env(var),
            };
            // the Claude Code process, which the injector asks whether the session is idle
            let pid = var("CLAUDE_PID").and_then(|pid| pid.trim().parse().ok());
            let input = stdin()?;
            // the injector reaches the same service, through the config this reef was named
            let spawn = |order: &clear::Clear| clear::spawn(order, named.as_deref());
            hook::stop(&client, cwd, &input, pane.as_ref(), pid, spawn, out).await
        }
        Command::Hook(HookCommand::Clear {
            pane,
            session,
            pid,
            after,
        }) => {
            // what Stop, or a start, handed it, each part as it was written
            let order = clear::Clear {
                pane: pane.parse()?,
                session,
                pid,
                after: after
                    .parse()
                    .map_err(|error| anyhow::anyhow!("`{after}` names no node: {error}"))?,
            };
            clear::run(&client, &order, &mut std::io::stderr()).await
        }
        Command::Mcp { pick } => {
            // the pane the session runs in, its multiplexer's else the Claude Code that started it
            let var = |name: &str| std::env::var(name).ok();
            let pane = loops::server_pane(var, std::os::unix::process::parent_id());
            // the protocol owns standard output, so nothing is printed to `out`
            let tools = Tools::new(client, cwd.to_path_buf(), pick).in_pane(pane);
            mcp::serve(tools, tokio::io::stdin(), tokio::io::stdout()).await
        }
        // written above, before the config was read
        Command::Skill { .. } => unreachable!("the skill is written before the config is read"),
    }
}

/// Reads all of standard input, as a hook is handed its event
fn stdin() -> anyhow::Result<String> {
    // the whole of it, which the hook's caller closes once written
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
    Ok(input)
}

/// Registers a project for the repo that holds a directory
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `name` - The project's name
/// * `repo` - A directory in its repo, else the working directory
/// * `out` - Where to print the project registered
async fn project_add<W: Write>(
    client: &Client,
    cwd: &Path,
    name: &str,
    repo: Option<&Path>,
    out: &mut W,
) -> anyhow::Result<()> {
    // the service reads the repo as this host sees it, so it is sent absolute
    let dir = std::path::absolute(cwd.join(repo.unwrap_or(Path::new("."))))?;
    match client.add_project(name, &dir).await? {
        // the service located the repo's root, which is what was registered
        AddedProject::Added(project) => {
            writeln!(
                out,
                "registered `{}` for {}",
                project.name,
                project.repo.display()
            )?;
            Ok(())
        }
        AddedProject::NameTaken => {
            anyhow::bail!("a project named `{name}` is already registered")
        }
        AddedProject::RepoTaken { project } => anyhow::bail!(
            "the repo that holds {} is already registered as `{project}`",
            dir.display()
        ),
    }
}

/// Lists every registered project and its repo
///
/// # Arguments
///
/// * `client` - The service's client
/// * `out` - Where to print them
async fn project_ls<W: Write>(client: &Client, out: &mut W) -> anyhow::Result<()> {
    // every project, by name, its repo beside it
    let projects = client.projects().await?;
    let rows = projects
        .iter()
        .map(|project| (project.name.clone(), project.repo.display().to_string()));
    columns(out, rows)
}

/// Shows the project the working directory belongs to, or the one named
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `pick` - The project named, if one was
/// * `out` - Where to print it
async fn project_show<W: Write>(
    client: &Client,
    cwd: &Path,
    pick: &Pick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the project and its repo
    let project = picked(client, cwd, pick).await?;
    columns(
        out,
        [(project.name, project.repo.display().to_string())].into_iter(),
    )
}

/// Lists a new, empty plan in the picked project
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `name` - The plan's name within its project
/// * `pick` - The project named, if one was
/// * `out` - Where to print what came of it
async fn plan_new<W: Write>(
    client: &Client,
    cwd: &Path,
    name: &str,
    pick: &Pick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the plan's full name keeps the rule names share
    let project = picked(client, cwd, pick).await?;
    let plan = PlanName::new(project.name, name)?;
    // a plan already listed is left as it is, which is no failure
    match client.add_plan(&plan).await?.added {
        true => writeln!(out, "listed `{plan}`")?,
        false => writeln!(out, "`{plan}` is already listed")?,
    }
    Ok(())
}

/// Lists the picked project's plans and the revision each is at
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `pick` - The project named, if one was
/// * `out` - Where to print them
async fn plan_ls<W: Write>(
    client: &Client,
    cwd: &Path,
    pick: &Pick,
    out: &mut W,
) -> anyhow::Result<()> {
    // every plan of the project, by name, its revision beside it
    let project = picked(client, cwd, pick).await?;
    let plans = client.plans(&project.name).await?;
    let rows = plans
        .iter()
        .map(|info| (info.name.to_string(), format!("revision {}", info.revision)));
    columns(out, rows)
}

/// The project a command works in: the one named, else the working directory's
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `pick` - The project named, if one was
pub(crate) async fn picked(client: &Client, cwd: &Path, pick: &Pick) -> anyhow::Result<Project> {
    // a project named must be registered
    if let Some(name) = &pick.project {
        let projects = client.projects().await?;
        return projects
            .into_iter()
            .find(|project| &project.name == name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no project named `{name}` is registered: `reef project ls` lists them"
                )
            });
    }
    // else the one whose repo holds the working directory, its main repo's in a worktree
    client.find_project(cwd).await?.ok_or_else(|| {
        anyhow::anyhow!(
            "no project is registered for {}: register one with `reef project add <name>`, or \
             name one with `--project`",
            cwd.display()
        )
    })
}

/// The plan a node command works in: the one named in the picked project, else its only plan
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `pick` - The project and plan named, if they were
pub async fn picked_plan(client: &Client, cwd: &Path, pick: &PlanPick) -> anyhow::Result<PlanName> {
    // the plan is in the project named, else the working directory's
    let project = picked(client, cwd, &pick.project).await?;
    plan_in(client, project, pick.plan.as_deref()).await
}

/// The plan named in a project, else the project's only plan
///
/// # Arguments
///
/// * `client` - The service's client
/// * `project` - The project the plan is in
/// * `plan` - The plan named, if one was
pub(crate) async fn plan_in(
    client: &Client,
    project: Project,
    plan: Option<&str>,
) -> anyhow::Result<PlanName> {
    // a plan named keeps the rule names share, and the service says if it isn't listed
    if let Some(plan) = plan {
        return Ok(PlanName::new(project.name, plan)?);
    }
    // else the project's only plan
    let plans = client.plans(&project.name).await?;
    match plans.as_slice() {
        [only] => Ok(only.name.clone()),
        [] => anyhow::bail!(
            "project `{}` lists no plans: list one with `reef plan new <name>`",
            project.name
        ),
        many => {
            let names: Vec<String> = many.iter().map(|info| format!("`{}`", info.name)).collect();
            anyhow::bail!(
                "project `{}` lists {} plans, {}: name one with `--plan`",
                project.name,
                many.len(),
                names.join(", ")
            )
        }
    }
}

/// Prints rows of two columns, the first padded to its widest
///
/// # Arguments
///
/// * `out` - Where to print them
/// * `rows` - Each row's two cells
pub(crate) fn columns<W: Write>(
    out: &mut W,
    rows: impl Iterator<Item = (String, String)>,
) -> anyhow::Result<()> {
    // the first column is as wide as its widest cell
    let rows: Vec<(String, String)> = rows.collect();
    let width = rows
        .iter()
        .map(|(left, _)| left.chars().count())
        .max()
        .unwrap_or(0);
    for (left, right) in rows {
        writeln!(out, "{left:<width$}  {right}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Cli, Command, CommentCommand, HookCommand, LoopCommand, NodeCommand, PageCommand,
        PlanCommand, ProjectCommand, QueueCommand, columns,
    };
    use clap::{CommandFactory, Parser};
    use reef_model::{Met, Status};
    use std::path::{Path, PathBuf};

    /// clap's own checks of the command line's shape
    #[test]
    fn command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    /// `--conf` goes anywhere, and `--project` only where a project is picked
    #[test]
    fn arguments_go_where_they_mean_something() {
        // `--conf` before or after the command
        let before = Cli::try_parse_from(["reef", "--conf", "a.toml", "plan", "ls"]).unwrap();
        let after = Cli::try_parse_from(["reef", "plan", "ls", "--conf", "a.toml"]).unwrap();
        assert_eq!(before.conf, Some(PathBuf::from("a.toml")));
        assert_eq!(after.conf, Some(PathBuf::from("a.toml")));
        // a plan's project is picked on the command line
        let new = Cli::try_parse_from(["reef", "plan", "new", "central", "--project", "reef"]);
        match new.unwrap().command {
            Command::Plan(PlanCommand::New { name, pick }) => {
                assert_eq!(name, "central");
                assert_eq!(pick.project.as_deref(), Some("reef"));
            }
            other => panic!("parsed as {other:?}"),
        }
        // a project added takes its repo, and `--project` means nothing there
        let add = Cli::try_parse_from(["reef", "project", "add", "reef", "--repo", "../reef"]);
        assert!(matches!(
            add.unwrap().command,
            Command::Project(ProjectCommand::Add { repo: Some(_), .. })
        ));
        assert!(Cli::try_parse_from(["reef", "project", "add", "x", "--project", "y"]).is_err());
        assert!(Cli::try_parse_from(["reef", "project", "ls", "--project", "y"]).is_err());
        // a node's project and plan are picked on the command line
        let get = Cli::try_parse_from(["reef", "node", "get", "C17", "--plan", "central"]);
        match get.unwrap().command {
            Command::Node(NodeCommand::Get { id, md, pick }) => {
                assert_eq!(id, "C17");
                assert!(!md);
                assert_eq!(pick.plan.as_deref(), Some("central"));
                assert_eq!(pick.project.project, None);
            }
            other => panic!("parsed as {other:?}"),
        }
        // and it is printed as a node file when asked
        let get = Cli::try_parse_from(["reef", "node", "get", "C17", "--md"]);
        assert!(matches!(
            get.unwrap().command,
            Command::Node(NodeCommand::Get { md: true, .. })
        ));
        // a put's file is optional, and `rm` needs at least one id
        let put = Cli::try_parse_from(["reef", "node", "put", "--project", "reef"]);
        assert!(matches!(
            put.unwrap().command,
            Command::Node(NodeCommand::Put { file: None, .. })
        ));
        assert!(Cli::try_parse_from(["reef", "node", "rm"]).is_err());
        // a page is named by its path, and put from a file or standard input
        let put = Cli::try_parse_from(["reef", "page", "put", "design/app.md", "app.md"]);
        match put.unwrap().command {
            Command::Page(PageCommand::Put { path, file, .. }) => {
                assert_eq!(path, "design/app.md");
                assert_eq!(file.as_deref(), Some(Path::new("app.md")));
            }
            other => panic!("parsed as {other:?}"),
        }
        assert!(Cli::try_parse_from(["reef", "page", "get"]).is_err());
        assert!(Cli::try_parse_from(["reef", "page", "rm", "a.md", "b.md"]).is_err());
        assert!(Cli::try_parse_from(["reef", "page", "ls", "--plan", "central"]).is_ok());
        let rm = Cli::try_parse_from(["reef", "node", "rm", "M3", "C11"]).unwrap();
        assert!(matches!(
            rm.command,
            Command::Node(NodeCommand::Rm { ids, .. }) if ids == ["M3", "C11"]
        ));
        // and `--plan` means nothing where no plan is picked
        assert!(Cli::try_parse_from(["reef", "plan", "ls", "--plan", "x"]).is_err());
    }

    /// A status and a criterion's status take the frontmatter's names, and a criterion named
    /// needs one
    #[test]
    fn work_commands_take_what_frontmatter_does() {
        // a status by its name or an alias, and a name that isn't one refused
        let status = Cli::try_parse_from(["reef", "status", "C18", "wip", "--plan", "central"]);
        match status.unwrap().command {
            Command::Status { id, status, pick } => {
                assert_eq!((id.as_str(), status), ("C18", Status::InProgress));
                assert_eq!(pick.plan.as_deref(), Some("central"));
            }
            other => panic!("parsed as {other:?}"),
        }
        assert!(Cli::try_parse_from(["reef", "status", "C18", "nearly"]).is_err());
        assert!(Cli::try_parse_from(["reef", "status", "C18"]).is_err());
        // a node's criteria listed, or one set with its test
        let list = Cli::try_parse_from(["reef", "accept", "C18"]).unwrap();
        assert!(matches!(
            list.command,
            Command::Accept {
                criterion: None,
                met: None,
                ..
            }
        ));
        let set = ["reef", "accept", "C18", "2", "met", "--test", "t.rs::a"];
        match Cli::try_parse_from(set).unwrap().command {
            Command::Accept {
                criterion,
                met,
                test,
                ..
            } => {
                assert_eq!((criterion, met), (Some(2), Some(Met::Met)));
                assert_eq!(test.as_deref(), Some("t.rs::a"));
            }
            other => panic!("parsed as {other:?}"),
        }
        // a criterion needs whether it is met, and a test needs its criterion
        assert!(Cli::try_parse_from(["reef", "accept", "C18", "2"]).is_err());
        assert!(Cli::try_parse_from(["reef", "accept", "C18", "--test", "t.rs"]).is_err());
        assert!(Cli::try_parse_from(["reef", "accept", "C18", "x", "met"]).is_err());
        // check and next pick a plan, and take nothing else
        let check = Cli::try_parse_from(["reef", "check", "--project", "reef"]).unwrap();
        assert!(matches!(check.command, Command::Check { .. }));
        assert!(Cli::try_parse_from(["reef", "next", "C18"]).is_err());
        // and the MCP server picks its project and plan the same way
        let mcp = Cli::try_parse_from(["reef", "mcp", "--plan", "central"]).unwrap();
        assert!(matches!(
            mcp.command,
            Command::Mcp { pick } if pick.plan.as_deref() == Some("central")
        ));
    }

    /// A queue command takes one place at most, and its project and plan as a node command does
    #[test]
    fn queue_commands_take_one_place() {
        // a node added with a place beside another, in a plan named
        let add = [
            "reef", "queue", "add", "C24", "--before", "C22", "--plan", "central",
        ];
        match Cli::try_parse_from(add).unwrap().command {
            Command::Queue(QueueCommand::Add { id, at, pick }) => {
                assert_eq!(id, "C24");
                assert_eq!(at.before.as_deref(), Some("C22"));
                assert!(!at.top && !at.bottom && at.after.is_none());
                assert_eq!(pick.plan.as_deref(), Some("central"));
            }
            other => panic!("parsed as {other:?}"),
        }
        // with no place, and moved to the top
        let add = Cli::try_parse_from(["reef", "queue", "add", "C24"]).unwrap();
        assert!(matches!(
            add.command,
            Command::Queue(QueueCommand::Add { at, .. }) if !at.top && at.before.is_none()
        ));
        let mv = Cli::try_parse_from(["reef", "queue", "move", "C24", "--top"]).unwrap();
        assert!(matches!(
            mv.command,
            Command::Queue(QueueCommand::Move { at, .. }) if at.top
        ));
        // two places at once are refused
        assert!(Cli::try_parse_from(["reef", "queue", "add", "C24", "--top", "--bottom"]).is_err());
        let two = [
            "reef", "queue", "move", "C24", "--before", "a", "--after", "b",
        ];
        assert!(Cli::try_parse_from(two).is_err());
        // `rm` needs an id, and `ls` lists a project's queue, so it takes no plan
        assert!(Cli::try_parse_from(["reef", "queue", "rm"]).is_err());
        let rm = Cli::try_parse_from(["reef", "queue", "rm", "C21", "C22"]).unwrap();
        assert!(matches!(
            rm.command,
            Command::Queue(QueueCommand::Rm { ids, .. }) if ids == ["C21", "C22"]
        ));
        assert!(Cli::try_parse_from(["reef", "queue", "ls", "--project", "reef"]).is_ok());
        assert!(Cli::try_parse_from(["reef", "queue", "ls", "--plan", "central"]).is_err());
    }

    /// A start takes its node, a base and whether to start again, and `work` its node or none
    #[test]
    fn start_and_work_take_a_node() {
        // a node started with its base named, starting it again, in a plan named
        let line = [
            "reef",
            "start",
            "C30",
            "--base",
            "main",
            "--restart",
            "--plan",
            "central",
        ];
        match Cli::try_parse_from(line).unwrap().command {
            Command::Start {
                id,
                base,
                restart,
                pick,
            } => {
                assert_eq!(
                    (id.as_str(), base.as_deref(), restart),
                    ("C30", Some("main"), true)
                );
                assert_eq!(pick.plan.as_deref(), Some("central"));
            }
            other => panic!("parsed as {other:?}"),
        }
        // with neither, the repo's default base and a first start
        let plain = Cli::try_parse_from(["reef", "start", "C30"]).unwrap();
        assert!(matches!(
            plain.command,
            Command::Start {
                base: None,
                restart: false,
                ..
            }
        ));
        // a start needs its node, and takes one alone
        assert!(Cli::try_parse_from(["reef", "start"]).is_err());
        assert!(Cli::try_parse_from(["reef", "start", "C30", "C31"]).is_err());
        // `work` names a node, or none for the one this worktree is on
        let named = Cli::try_parse_from(["reef", "work", "C30"]).unwrap();
        assert!(matches!(named.command, Command::Work { id: Some(id), .. } if id == "C30"));
        let here = Cli::try_parse_from(["reef", "work", "--project", "reef"]).unwrap();
        assert!(matches!(here.command, Command::Work { id: None, .. }));
    }

    /// A loop command takes a pane or none, a start its plan and session, and a hook its event
    #[test]
    fn loop_commands_take_a_pane() {
        // a start in a pane named, of one plan, fed by a session named
        let line = [
            "reef",
            "loop",
            "start",
            "--pane",
            "claude:7",
            "--session",
            "s1",
            "--plan",
            "central",
        ];
        match Cli::try_parse_from(line).unwrap().command {
            Command::Loop(LoopCommand::Start {
                pane,
                session,
                pick,
            }) => {
                assert_eq!(
                    (pane.as_deref(), session.as_deref()),
                    (Some("claude:7"), Some("s1"))
                );
                assert_eq!(pick.plan.as_deref(), Some("central"));
            }
            other => panic!("parsed as {other:?}"),
        }
        // pause and stop take a pane or none, and nothing else
        let pause = Cli::try_parse_from(["reef", "loop", "pause"]).unwrap();
        assert!(matches!(
            pause.command,
            Command::Loop(LoopCommand::Pause { pane: None })
        ));
        assert!(Cli::try_parse_from(["reef", "loop", "stop", "--plan", "x"]).is_err());
        // ls lists a project's loops, so it takes no plan
        assert!(Cli::try_parse_from(["reef", "loop", "ls", "--project", "reef"]).is_ok());
        assert!(Cli::try_parse_from(["reef", "loop", "ls", "--plan", "central"]).is_err());
        // take takes a pane or none, and a project, but no plan, which the loop names
        let take = Cli::try_parse_from(["reef", "loop", "take", "--pane", "claude:7"]).unwrap();
        match take.command {
            Command::Loop(LoopCommand::Take { pane, pick }) => {
                assert_eq!((pane.as_deref(), pick.project), (Some("claude:7"), None));
            }
            other => panic!("parsed as {other:?}"),
        }
        assert!(Cli::try_parse_from(["reef", "loop", "take", "--plan", "central"]).is_err());
        // and the SessionStart hook takes a pane or none
        let hook = Cli::try_parse_from(["reef", "hook", "session-start"]).unwrap();
        assert!(matches!(
            hook.command,
            Command::Hook(HookCommand::SessionStart { pane: None })
        ));
    }

    /// git's hooks take what git passes them, and Claude Code's take their input on standard
    /// input alone
    #[test]
    fn capture_hooks_take_what_their_callers_pass() {
        // post-commit takes nothing, and post-rewrite what rewrote the commits
        let commit = Cli::try_parse_from(["reef", "hook", "post-commit"]).unwrap();
        assert!(matches!(
            commit.command,
            Command::Hook(HookCommand::PostCommit)
        ));
        let rewrite = Cli::try_parse_from(["reef", "hook", "post-rewrite", "rebase"]).unwrap();
        assert!(matches!(
            rewrite.command,
            Command::Hook(HookCommand::PostRewrite { kind }) if kind == "rebase"
        ));
        assert!(Cli::try_parse_from(["reef", "hook", "post-rewrite"]).is_err());
        assert!(Cli::try_parse_from(["reef", "hook", "post-commit", "x"]).is_err());
        // PostToolUse and Stop take nothing on the command line
        let used = Cli::try_parse_from(["reef", "hook", "post-tool-use"]).unwrap();
        assert!(matches!(
            used.command,
            Command::Hook(HookCommand::PostToolUse)
        ));
        let stop = Cli::try_parse_from(["reef", "--conf", "a.toml", "hook", "stop"]).unwrap();
        assert!(matches!(
            stop.command,
            Command::Hook(HookCommand::Stop { pane: None })
        ));
        // Stop takes the pane whose loop it ends a node of, as SessionStart does
        let named = Cli::try_parse_from(["reef", "hook", "stop", "--pane", "claude:7"]).unwrap();
        assert!(matches!(
            named.command,
            Command::Hook(HookCommand::Stop { pane: Some(key) }) if key == "claude:7"
        ));
        assert!(Cli::try_parse_from(["reef", "hook", "stop", "x"]).is_err());
    }

    /// A resolve takes its thread's number, and the node and revision that answered it or none
    #[test]
    fn comment_resolve_takes_a_thread() {
        // a thread resolved by a node and revision named, in a plan named
        let line = [
            "reef",
            "comment",
            "resolve",
            "12",
            "--node",
            "C37",
            "--revision",
            "3",
            "--plan",
            "central",
        ];
        match Cli::try_parse_from(line).unwrap().command {
            Command::Comment(CommentCommand::Resolve {
                thread,
                node,
                revision,
                pick,
            }) => {
                assert_eq!(
                    (thread, node.as_deref(), revision),
                    (12, Some("C37"), Some(3))
                );
                assert_eq!(pick.plan.as_deref(), Some("central"));
            }
            other => panic!("parsed as {other:?}"),
        }
        // with neither, the node this worktree is on and its latest revision
        let plain = Cli::try_parse_from(["reef", "comment", "resolve", "12"]).unwrap();
        assert!(matches!(
            plain.command,
            Command::Comment(CommentCommand::Resolve {
                node: None,
                revision: None,
                ..
            })
        ));
        // a thread is a number, and one is needed
        assert!(Cli::try_parse_from(["reef", "comment", "resolve"]).is_err());
        assert!(Cli::try_parse_from(["reef", "comment", "resolve", "#12"]).is_err());
    }

    /// The skill takes a directory or none, and whether to print it, and no project or plan
    #[test]
    fn skill_takes_a_directory() {
        // a directory named, printed instead of written
        let named = Cli::try_parse_from(["reef", "skill", "../other", "--print"]).unwrap();
        match named.command {
            Command::Skill { dir, print } => {
                assert_eq!((dir.as_path(), print), (Path::new("../other"), true));
            }
            other => panic!("parsed as {other:?}"),
        }
        // none is the working directory, written
        let here = Cli::try_parse_from(["reef", "skill"]).unwrap();
        assert!(matches!(
            here.command,
            Command::Skill { dir, print: false } if dir == Path::new(".")
        ));
        // it writes into a repo, not a project's plan
        assert!(Cli::try_parse_from(["reef", "skill", "--project", "reef"]).is_err());
        assert!(Cli::try_parse_from(["reef", "skill", "a", "b"]).is_err());
    }

    /// The first column is padded to its widest cell
    #[test]
    fn columns_line_up() {
        // two rows of different widths
        let mut out = Vec::new();
        let rows = [
            ("reef".to_string(), "/src/reef".to_string()),
            ("shoal-db".to_string(), "/src/shoal".to_string()),
        ];
        columns(&mut out, rows.into_iter()).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "reef      /src/reef\nshoal-db  /src/shoal\n"
        );
        // and none prints nothing
        let mut out = Vec::new();
        columns(&mut out, std::iter::empty()).unwrap();
        assert!(out.is_empty());
    }
}
