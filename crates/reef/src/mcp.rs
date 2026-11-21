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
//! - **Read-only tools are marked `readOnlyHint`:** `get`, `check` and `next`. Plan mode runs
//!   them without a prompt, and asks for every other tool, so nothing Claude needs while it
//!   plans writes (X4).
//! - **Writes are the CLI's:** `put` takes a node's YAML, as `reef node put` does, and
//!   `status` and `accept` set a node's status and a criterion's.
//! - **Each tool says what the CLI prints.** A tool runs the same function its command does,
//!   so its text is the command's. A write that is stale or breaks the plan, or a service that
//!   isn't running, comes back as a tool error carrying the message, which Claude reads, rather
//!   than a protocol error, which it can't.
//! - **Nothing else writes to standard output** while it serves, since the protocol runs there.

use reef_model::{Met, PlanName, Status};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::cli::{Pick, PlanPick, picked_plan};
use crate::client::Client;
use crate::{node, work};

/// The tools one session's server offers, over the service reef's config names
#[derive(Clone)]
pub struct Tools {
    /// The service's client
    client: Arc<Client>,
    /// The directory the server was started in, which picks the project
    cwd: PathBuf,
    /// The project and plan named when it was started, if any
    pick: PlanPick,
}

/// Names a plan, if not the one the server was started for
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PlanArgs {
    /// The plan within the project, if the project has several and the server names none
    pub plan: Option<String>,
}

/// Names one node
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NodeArgs {
    /// The node's id, such as C18
    pub id: String,
    /// The plan within the project, if the project has several and the server names none
    pub plan: Option<String>,
}

/// A node's YAML to put
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PutArgs {
    /// The node's YAML, with the keys a book's frontmatter takes. YAML that `get` returned
    /// keeps its `# version:` line, so a put over another writer's change is refused; YAML
    /// without one is a new node. Inline `nodes:` go in as members of their own, with it.
    pub yaml: String,
    /// The plan within the project, if the project has several and the server names none
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
    /// The plan within the project, if the project has several and the server names none
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
    /// The plan within the project, if the project has several and the server names none
    pub plan: Option<String>,
}

impl Tools {
    /// The tools of a server started in a directory
    ///
    /// # Arguments
    ///
    /// * `client` - The service's client
    /// * `cwd` - The directory it was started in, absolute, which picks the project
    /// * `pick` - The project and plan named when it was started, if any
    pub fn new(client: Client, cwd: PathBuf, pick: PlanPick) -> Tools {
        Tools {
            client: Arc::new(client),
            cwd,
            pick,
        }
    }

    /// The plan a tool works in: the one it names, else the server's, else the project's only
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan the tool was given, if any
    async fn plan(&self, plan: Option<String>) -> anyhow::Result<PlanName> {
        // a plan named by the call wins over the one the server was started for
        let pick = PlanPick {
            project: Pick {
                project: self.pick.project.project.clone(),
            },
            plan: plan.or_else(|| self.pick.plan.clone()),
        };
        picked_plan(&self.client, &self.cwd, &pick).await
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
            node::get(&self.client, &plan, &args.id, &mut out).await
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

    /// Lists the work ready to start, and what the rest waits on
    #[tool(
        description = "List the reef work ready to start, in plan order, then each piece of work \
                       that waits and the needs it waits on.",
        annotations(read_only_hint = true)
    )]
    async fn next(&self, Parameters(args): Parameters<PlanArgs>) -> Result<String, String> {
        // the report `reef next` prints
        let mut out = Vec::new();
        let done = async {
            let plan = self.plan(args.plan).await?;
            work::next(&self.client, &plan, &mut out).await
        }
        .await;
        said(out, done)
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
                    start, `get` reads a node as YAML and `check` lists the plan's problems; \
                    these only read. `put` writes a node's YAML, `status` sets where its work \
                    stands and `accept` whether a criterion is met. Every write is checked \
                    against the whole plan, and one that is stale or adds a problem writes \
                    nothing and says why."
)]
impl ServerHandler for Tools {}

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
