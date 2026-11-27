//! reef's MCP server: the CLI's node and work commands as tools, over stdio
//!
//! ```text
//! reef mcp [--project NAME] [--plan NAME]
//! ```
//!
//! Claude Code starts it in a repo, so the working directory picks the project as it does for
//! the CLI, and `--project` and `--plan` name one instead. Each tool takes a `plan` of its own
//! too.
//!
//! - **Read-only tools are marked `readOnlyHint`:** `get`, `history`, `check`, `next` and
//!   `comments`. Plan mode runs them without a prompt, and asks for every other tool, so nothing
//!   Claude needs while it plans writes (X4).
//! - **Writes are the CLI's:** `put` takes a node's YAML, as `reef node put` does, and
//!   `status` and `accept` set a node's status and a criterion's.
//! - **Comments are answered as Claude's.** `comments` lists open threads as `reef comment ls`
//!   does. `reply` keeps a reply in one, which leaves it waiting on the user, and `resolve`
//!   resolves one answered without a change, linking its own node's latest revision. Both write
//!   as Claude, so a thread whose last word is Claude's never calls for another round.
//! - **`next_task` takes a node for the session's loop,** as `reef loop take` does, and starts
//!   it, so Claude calls it before plan mode. The loop is the pane's the server runs in: its
//!   multiplexer's, else the Claude Code process that started it
//!   ([`server_pane`](crate::loops::server_pane)).
//! - **Each tool says what the CLI prints.** A tool runs the same function its command does,
//!   so its text is the command's. A write that is stale or breaks the plan, or a service that
//!   isn't running, comes back as a tool error carrying the message, which Claude reads, rather
//!   than a protocol error, which it can't.
//! - **Nothing else writes to standard output** while it serves, since the protocol runs there.

use reef_api::loops::member_of;
use reef_api::{Loop, Pane};
use reef_model::{Met, PlanName, Status};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::cli::{Pick, PlanPick, next_plan, picked, picked_plan};
use crate::client::Client;
use crate::comment::{self, Answering};
use crate::{history, node, open, task, work};
use reef_api::Writer;

/// The tools one session's server offers, over the service reef's config names
#[derive(Clone)]
pub struct Tools {
    /// The service's client
    client: Arc<Client>,
    /// The directory the server was started in, which picks the project
    cwd: PathBuf,
    /// The project and plan named when it was started, if any
    pick: PlanPick,
    /// The pane the session that started it runs in, whose loop `next_task` takes nodes for
    pane: Option<Pane>,
}

/// The plan a pane's loop is held to, the pane's own loop's or the loop whose member it works,
/// if that loop feeds the project and is held to one plan of it
///
/// # Arguments
///
/// * `loops` - Every loop
/// * `pane` - The pane the session runs in
/// * `project` - The project the session works in
pub fn loop_plan(loops: &[Loop], pane: &Pane, project: &str) -> Option<String> {
    // the pane's own loop, else the loop whose member the pane works
    let feeding = loops
        .iter()
        .find(|feeding| feeding.pane == *pane)
        .or_else(|| member_of(loops, pane).map(|(feeding, _)| feeding))?;
    // only one of this project's, held to one plan
    (feeding.project == project)
        .then(|| feeding.plan.clone())
        .flatten()
}

/// Names a plan, if not the one the server was started for
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PlanArgs {
    /// The plan's name within the project, such as `central`, if the project has several and
    /// neither the server nor this session's loop names one
    pub plan: Option<String>,
}

/// Names one node
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NodeArgs {
    /// The node's id, such as C18
    pub id: String,
    /// The plan's name within the project, such as `central`, if the project has several and
    /// neither the server nor this session's loop names one
    pub plan: Option<String>,
}

/// Names one node, and which page of its history to read
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryArgs {
    /// The node's id, such as C18
    pub id: String,
    /// The plan's name within the project, such as `central`, if the project has several and
    /// neither the server nor this session's loop names one
    pub plan: Option<String>,
    /// The cursor a page ended by naming, after `--older`, to read the 200 edits older than it;
    /// none for the newest 200
    pub older: Option<String>,
}

/// A node's YAML to put
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PutArgs {
    /// The node's YAML, with the keys a book's frontmatter takes. YAML that `get` returned
    /// keeps its `# version:` line, so a put over another writer's change is refused; YAML
    /// without one is a new node. Inline `nodes:` go in as members of their own, with it.
    pub yaml: String,
    /// The plan's name within the project, such as `central`, if the project has several and
    /// neither the server nor this session's loop names one
    pub plan: Option<String>,
}

/// Where a node's work stands
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WorkStatus {
    /// Not started
    Todo,
    /// Started
    InProgress,
    /// Finished, its criteria set
    Done,
    /// Stuck, its body saying on what
    Blocked,
}

impl From<WorkStatus> for Status {
    /// The model's status of the same name
    ///
    /// # Arguments
    ///
    /// * `status` - The status a tool was given
    fn from(status: WorkStatus) -> Status {
        match status {
            WorkStatus::Todo => Status::Todo,
            WorkStatus::InProgress => Status::InProgress,
            WorkStatus::Done => Status::Done,
            WorkStatus::Blocked => Status::Blocked,
        }
    }
}

/// Sets where a node's work stands
#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatusArgs {
    /// The node's id, such as C18. A group's status is derived from its members, so it is
    /// refused.
    pub id: String,
    /// Where its work now stands
    pub status: WorkStatus,
    /// The plan's name within the project, such as `central`, if the project has several and
    /// neither the server nor this session's loop names one
    pub plan: Option<String>,
}

/// Lists comment threads: a node's, else the plan's
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CommentsArgs {
    /// The node whose threads to list, such as C18, else every thread of the plan
    pub id: Option<String>,
    /// Whether to list resolved threads too, rather than only the open ones
    #[serde(default)]
    pub all: bool,
    /// The plan's name within the project, such as `central`, if the project has several and
    /// neither the server nor this session's loop names one
    pub plan: Option<String>,
}

/// Replies in a comment thread
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReplyArgs {
    /// The thread's number, such as 12
    pub thread: u32,
    /// The reply, as markdown
    pub body: String,
    /// The plan's name within the project, such as `central`, if the project has several and
    /// neither the server nor this session's loop names one
    pub plan: Option<String>,
}

/// Resolves a comment thread answered without a change
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveArgs {
    /// The thread's number, such as 12
    pub thread: u32,
    /// A reply to keep in it first, as markdown, saying how it was answered
    pub reply: Option<String>,
    /// The plan's name within the project, such as `central`, if the project has several and
    /// neither the server nor this session's loop names one
    pub plan: Option<String>,
}

/// Whether a criterion is met
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CriterionStatus {
    /// Not yet known
    Todo,
    /// It is met
    Met,
    /// It was tried, and isn't met
    Unmet,
}

impl From<CriterionStatus> for Met {
    /// The model's word for it
    ///
    /// # Arguments
    ///
    /// * `status` - The status a tool was given
    fn from(status: CriterionStatus) -> Met {
        match status {
            CriterionStatus::Todo => Met::Todo,
            CriterionStatus::Met => Met::Met,
            CriterionStatus::Unmet => Met::Unmet,
        }
    }
}

/// Sets whether one of a node's acceptance criteria is met
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AcceptArgs {
    /// The node's id, such as C18
    pub id: String,
    /// The criterion's number, from 1, in the order its `acceptance` lists them
    pub criterion: usize,
    /// Whether it is met. A performance criterion with a target is met by its metric's
    /// figures, so it is refused.
    pub status: CriterionStatus,
    /// The test that checks it, such as `tests/scan.rs::dropped_client`
    pub test: Option<String>,
    /// The plan's name within the project, such as `central`, if the project has several and
    /// neither the server nor this session's loop names one
    pub plan: Option<String>,
}

impl Tools {
    /// The tools of a server started in a directory
    ///
    /// Every write the tools make says it is Claude's, so the service names Claude, for the user
    /// the client's token signs in, as its author.
    ///
    /// # Arguments
    ///
    /// * `client` - The service's client
    /// * `cwd` - The directory it was started in, absolute, which picks the project
    /// * `pick` - The project and plan named when it was started, if any
    pub fn new(client: Client, cwd: PathBuf, pick: PlanPick) -> Tools {
        Tools {
            client: Arc::new(client.writing_as(Writer::Claude)),
            cwd,
            pick,
            pane: None,
        }
    }

    /// The same tools, taking nodes for the loop of a pane
    ///
    /// # Arguments
    ///
    /// * `pane` - The pane the session runs in, if it could be found
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::cli::PlanPick;
    /// use reef::mcp::Tools;
    /// use reef_api::Pane;
    ///
    /// let client = reef::Client::new(&reef::Conf::default()).unwrap();
    /// let tools = Tools::new(client, "/repo".into(), PlanPick::default())
    ///     .in_pane(Some(Pane::Claude { pid: 4242 }));
    /// ```
    pub fn in_pane(mut self, pane: Option<Pane>) -> Tools {
        self.pane = pane;
        self
    }

    /// The plan a tool works in: the one it names, else the server's, else the one its pane's
    /// loop is held to, else the project's only
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan the tool was given, if any
    async fn plan(&self, plan: Option<String>) -> anyhow::Result<PlanName> {
        // a plan the call or the server names wins
        let mut pick = self.pick_of(plan);
        if pick.plan.is_none() {
            // else the one the pane's loop takes nodes from, if it is held to one
            pick.plan = self.loop_plan(&pick.project).await?;
        }
        picked_plan(&self.client, &self.cwd, &pick).await
    }

    /// The plan the loop of this session's pane is held to, if the pane has a loop of the
    /// picked project held to one, or works a member of one
    ///
    /// # Arguments
    ///
    /// * `project` - The project named, if one was
    async fn loop_plan(&self, project: &Pick) -> anyhow::Result<Option<String>> {
        // a session in no pane has no loop
        let Some(pane) = &self.pane else {
            return Ok(None);
        };
        // the loops, and the project this session works in
        let loops = self.client.loops().await?;
        let project = picked(&self.client, &self.cwd, project).await?;
        Ok(loop_plan(&loops, pane, &project.name))
    }

    /// The project and plan a tool works in, as a command would be given them
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan the tool was given, if any
    fn pick_of(&self, plan: Option<String>) -> PlanPick {
        // a plan named by the call wins over the one the server was started for
        PlanPick {
            project: Pick {
                project: self.pick.project.project.clone(),
            },
            plan: plan.or_else(|| self.pick.plan.clone()),
        }
    }
}

/// What a tool says: what its command printed, or what it printed and why it failed
///
/// # Arguments
///
/// * `out` - What the command printed
/// * `done` - Whether it succeeded, or why not
fn said(out: Vec<u8>, done: anyhow::Result<()>) -> Result<String, String> {
    let text = String::from_utf8_lossy(&out).into_owned();
    match done {
        Ok(()) => Ok(text),
        // the error after whatever was printed before it
        Err(error) => Err(format!("{text}{error:#}")),
    }
}

#[tool_router]
impl Tools {
    /// Reads one node as YAML
    #[tool(
        description = "Read one reef node as YAML: the keys a book's frontmatter takes, after a \
                       `# version: N` line. Put it back with `put`, keeping that line, to change it.",
        annotations(read_only_hint = true)
    )]
    async fn get(&self, Parameters(args): Parameters<NodeArgs>) -> Result<String, String> {
        // the node as `reef node get` prints it
        let mut out = Vec::new();
        let done = async {
            let plan = self.plan(args.plan).await?;
            node::get(&self.client, &plan, &args.id, false, &mut out).await
        }
        .await;
        said(out, done)
    }

    /// Reads a node's history: who made each edit, what it changed and when
    #[tool(
        description = "Read a reef node's history, newest first, as `reef node log` prints it: \
                       a line an edit, with the version it left, when, who made it (\"Claude, \
                       for ada\" where Claude did, through these tools) and what it changed. A \
                       page holds 200 edits; one with older edits kept ends naming a cursor \
                       after `--older`, which `older` takes to read the next 200.",
        annotations(read_only_hint = true)
    )]
    async fn history(&self, Parameters(args): Parameters<HistoryArgs>) -> Result<String, String> {
        // the history as `reef node log` prints it, a page at a time
        let mut out = Vec::new();
        let done = async {
            let plan = self.plan(args.plan).await?;
            let node = plan.node(args.id)?;
            history::node(&self.client, &node, args.older.as_deref(), false, &mut out).await
        }
        .await;
        said(out, done)
    }

    /// Lists the plan's problems
    #[tool(
        description = "Check the reef plan as `mdbook-reef check` checks a book, listing each \
                       problem: unknown ids, cycles, doc links to missing files or headings, \
                       work that says no why or what.",
        annotations(read_only_hint = true)
    )]
    async fn check(&self, Parameters(args): Parameters<PlanArgs>) -> Result<String, String> {
        // each problem, then how many there are; a plan with problems is an answer, not a failure
        let mut out = Vec::new();
        let done = async {
            let plan = self.plan(args.plan).await?;
            let count = work::check(&self.client, &plan, &mut out).await?;
            writeln!(out, "{}", work::problems(&plan, count))?;
            Ok(())
        }
        .await;
        said(out, done)
    }

    /// Lists the work ready to start, the user's queue first, and what the rest waits on
    #[tool(
        description = "List the reef work ready to start, the user's queue first, then in plan \
                       order, then each piece of work that waits and the needs it waits on. It \
                       looks in the plan named, else the one this session's loop is held to, \
                       else the one the checkout's branch follows, and spans the project's \
                       plans where none of those picks one.",
        annotations(read_only_hint = true)
    )]
    async fn next(&self, Parameters(args): Parameters<PlanArgs>) -> Result<String, String> {
        // the report `reef next` prints, narrowed to a plan the call, the server, the pane's loop
        // or the checkout picks
        let mut out = Vec::new();
        let done = async {
            let mut pick = self.pick_of(args.plan);
            if pick.plan.is_none() {
                pick.plan = self.loop_plan(&pick.project).await?;
            }
            match next_plan(&self.client, &self.cwd, &pick, false).await? {
                (_, Some(plan)) => work::next(&self.client, &plan, &mut out).await,
                (project, None) => work::project_next(&self.client, &project.name, &mut out).await,
            }
        }
        .await;
        said(out, done)
    }

    /// Takes the next node for this session's loop, starts it here, and hands it back whole
    #[tool(
        description = "Take the next reef node for the loop of the pane this session runs in, \
                       and call it before plan mode, since it writes: the user's queue first, \
                       then ready work in plan order, passing over any node another loop holds. \
                       It records the node's start in this checkout, marks it in progress, and \
                       returns it whole (why, what, acceptance, docs, needs and body), with the \
                       queue's entries it passed over and why. Called again before the node is \
                       done, it returns the same node. With nothing to take, it says what \
                       everything waits on. A pane with no loop is refused: the user starts one \
                       with `! reef loop start`. A loop started wider than one node takes a \
                       group: this session works the first node only, and each other one gets a \
                       session of its own in a worktree, which reef opens in zellij or tmux. In \
                       such a member's session it returns that member's node. Before the next \
                       group it asks for each member's branch to be merged, naming the `git \
                       merge` to run."
    )]
    async fn next_task(&self) -> Result<String, String> {
        // the node `reef loop take` takes, for the pane the server was started in
        let client = self.client.clone();
        let cwd = self.cwd.clone();
        let pane = self.pane.clone();
        let pick = self.pick.project.clone();
        // a start reads its checkout through gix, whose handles aren't `Sync`, so it runs on a
        // blocking thread of its own, driven by the server's runtime
        let runtime = tokio::runtime::Handle::current();
        let taken = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            let done = runtime.block_on(task::next_task(
                &client,
                &cwd,
                pane.as_ref(),
                &pick,
                open::open,
                &mut out,
            ));
            (out, done)
        })
        .await;
        match taken {
            Ok((out, done)) => said(out, done),
            // a panic or a cancel, said rather than lost
            Err(error) => Err(format!("next_task stopped before it answered: {error}")),
        }
    }

    /// Puts a node's YAML
    #[tool(
        description = "Put a reef node's YAML, with its inline members, checked against the whole \
                       plan. A put over a version since changed is refused as stale, and one that \
                       adds a problem is refused with check's messages; either writes nothing."
    )]
    async fn put(&self, Parameters(args): Parameters<PutArgs>) -> Result<String, String> {
        // the YAML as `reef node put` takes it
        let mut out = Vec::new();
        let done = async {
            let plan = self.plan(args.plan).await?;
            node::put(&self.client, &plan, &args.yaml, &mut out).await
        }
        .await;
        said(out, done)
    }

    /// Sets where a node's work stands
    #[tool(
        description = "Set where a reef node's work stands: todo, in-progress, done or blocked. \
                       A group's status is derived from its members, so set theirs."
    )]
    async fn status(&self, Parameters(args): Parameters<StatusArgs>) -> Result<String, String> {
        // the node put back at the version read, as `reef status` does
        let mut out = Vec::new();
        let done = async {
            let plan = self.plan(args.plan).await?;
            work::status(&self.client, &plan, &args.id, args.status.into(), &mut out).await
        }
        .await;
        said(out, done)
    }

    /// Lists comment threads, each whole
    #[tool(
        description = "List the open comment threads on a reef node's work, or the plan's: each \
                       thread's number, its node, where it is (a file's line in a revision, or \
                       the node as a whole) and every comment, with who wrote it.",
        annotations(read_only_hint = true)
    )]
    async fn comments(&self, Parameters(args): Parameters<CommentsArgs>) -> Result<String, String> {
        // as `reef comment ls` lists them
        let mut out = Vec::new();
        let pick = self.pick_of(args.plan);
        let done = comment::ls(
            &self.client,
            &self.cwd,
            args.id.as_deref(),
            args.all,
            &pick,
            &mut out,
        )
        .await;
        said(out, done)
    }

    /// Replies in a comment thread, as Claude
    #[tool(
        description = "Reply in a reef comment thread, such as with a question back. The thread \
                       stays open, waiting on the user's answer rather than on yours."
    )]
    async fn reply(&self, Parameters(args): Parameters<ReplyArgs>) -> Result<String, String> {
        // as `reef comment reply` keeps one, written as Claude's
        let mut out = Vec::new();
        let pick = self.pick_of(args.plan);
        let done = comment::reply(
            &self.client,
            &self.cwd,
            args.thread,
            &args.body,
            Writer::Claude,
            &pick,
            &mut out,
        )
        .await;
        said(out, done)
    }

    /// Resolves a comment thread answered without a change, as Claude
    #[tool(
        description = "Resolve a reef comment thread answered without a change, linking its \
                       node's latest revision, after a reply saying how if one is given. A \
                       thread a commit answers is resolved by the commit's `Reef-Resolves: <n>` \
                       trailer instead."
    )]
    async fn resolve(&self, Parameters(args): Parameters<ResolveArgs>) -> Result<String, String> {
        // as `reef comment resolve` does, over the thread's own node
        let mut out = Vec::new();
        let pick = self.pick_of(args.plan);
        let done = comment::resolve(
            &self.client,
            &self.cwd,
            args.thread,
            Answering::Own,
            None,
            args.reply.as_deref(),
            Writer::Claude,
            &pick,
            &mut out,
        )
        .await;
        said(out, done)
    }

    /// Sets whether a node's criterion is met
    #[tool(
        description = "Set whether one of a reef node's acceptance criteria is met, numbered from \
                       1 in the order its `acceptance` lists them, and the test that checks it."
    )]
    async fn accept(&self, Parameters(args): Parameters<AcceptArgs>) -> Result<String, String> {
        // the node put back at the version read, as `reef accept` does
        let mut out = Vec::new();
        let done = async {
            let plan = self.plan(args.plan).await?;
            let set = Some((args.criterion, args.status.into()));
            work::accept(
                &self.client,
                &plan,
                &args.id,
                set,
                args.test.as_deref(),
                &mut out,
            )
            .await
        }
        .await;
        said(out, done)
    }
}

#[tool_handler(
    name = "reef",
    instructions = "reef keeps this repo's plan: a graph of nodes, each a piece of work with a \
                    why, a what, needs and acceptance criteria. `next` says what is ready to \
                    start, the user's queue first, `get` reads a node as YAML, `history` who \
                    changed it and how, and `check` lists the plan's problems; these only read, \
                    as does `comments`, which lists open comment threads. `next_task` takes the next node for this session's loop \
                    and starts it, before plan mode, or a done node again for a round of its \
                    open comments. `put` writes a node's YAML, `status` sets where its work \
                    stands and `accept` whether a criterion is met. `reply` answers a comment \
                    and `resolve` resolves one answered without a change. Every write is checked \
                    against the whole plan, and one that is stale or adds a problem writes \
                    nothing and says why."
)]
impl ServerHandler for Tools {}

/// The name of every tool the server offers, as its listing names them
///
/// # Examples
///
/// ```
/// let names = reef::mcp::tool_names();
/// assert!(names.iter().any(|name| name == "get"));
/// ```
pub fn tool_names() -> Vec<String> {
    // the router `#[tool_router]` builds, which the server lists its tools from
    Tools::tool_router()
        .list_all()
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect()
}

/// Serves the tools over a reader and a writer until the client goes away
///
/// # Arguments
///
/// * `tools` - The tools to serve
/// * `reader` - Where the client's messages come from: standard input, for Claude Code
/// * `writer` - Where the answers go: standard output, for Claude Code
pub async fn serve<R, W>(tools: Tools, reader: R, writer: W) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    // answer until the client closes its end
    let service = tools.serve((reader, writer)).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::loop_plan;
    use reef_api::{Author, Loop, LoopState, Member, Pane};

    /// A running loop in a pane, feeding a project, held to a plan or none
    ///
    /// # Arguments
    ///
    /// * `pane` - The pane's key
    /// * `project` - The project it feeds
    /// * `plan` - The one plan it is held to, or `None` for every plan
    fn feeding(pane: &str, project: &str, plan: Option<&str>) -> Loop {
        Loop {
            changed_by: Author::default(),
            started_by: Author::default(),
            pane: pane.parse::<Pane>().unwrap(),
            project: project.to_string(),
            plan: plan.map(str::to_string),
            session: None,
            node: None,
            acted: None,
            state: LoopState::Running,
            version: 1,
            started_at: 1,
            width: 1,
            opens: true,
            pid: None,
            members: Vec::new(),
        }
    }

    /// A bare id resolves in the plan the pane's loop narrows to, its own loop's or the loop
    /// whose member it works, and in none where the loop feeds every plan or another project
    #[test]
    fn a_bare_id_resolves_in_the_plan_the_panes_loop_narrows_to() {
        // a loop held to `allow`, one over every plan, and one of another project
        let mut held = feeding("zellij:work:1", "live", Some("allow"));
        let member: Pane = "zellij:work:4".parse().unwrap();
        held.members.push(Member {
            node: "live/allow/B".parse().unwrap(),
            pane: Some(member.clone()),
        });
        let loops = vec![
            held,
            feeding("zellij:work:2", "live", None),
            feeding("zellij:work:3", "shoal", Some("engine")),
        ];
        // the pane's own loop narrows to its plan, and so does the loop whose member it works
        let own: Pane = "zellij:work:1".parse().unwrap();
        assert_eq!(loop_plan(&loops, &own, "live").as_deref(), Some("allow"));
        assert_eq!(loop_plan(&loops, &member, "live").as_deref(), Some("allow"));
        // a loop over every plan narrows to none, and nor does one of another project
        let every: Pane = "zellij:work:2".parse().unwrap();
        assert_eq!(loop_plan(&loops, &every, "live"), None);
        let other: Pane = "zellij:work:3".parse().unwrap();
        assert_eq!(loop_plan(&loops, &other, "live"), None);
        // and a pane with no loop has none
        let none: Pane = "zellij:work:9".parse().unwrap();
        assert_eq!(loop_plan(&loops, &none, "live"), None);
    }
}
