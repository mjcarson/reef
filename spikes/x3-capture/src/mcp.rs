//! The stub reef MCP server: `get` reads a node, `status` sets where its work stands
//!
//! It is named `reef`, and `status` takes the same arguments as reef's own, so the PostToolUse
//! hook sees `mcp__reef__status` with the same input it would in a real session.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tracing::instrument;

use crate::store::{Status, Store};

/// The stub server, one per Claude Code session
#[derive(Clone)]
pub struct Reef {
    /// The state dir
    store: Store,
}

/// Names one node
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NodeArgs {
    /// The node's id, such as N1
    pub id: String,
    /// The plan within the project, if the project has several and the server names none
    pub plan: Option<String>,
}

/// Sets where a node's work stands
#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatusArgs {
    /// The node's id, such as N1
    pub id: String,
    /// Where its work now stands
    pub status: Status,
    /// The plan within the project, if the project has several and the server names none
    pub plan: Option<String>,
}

#[tool_router]
impl Reef {
    /// Creates the server over a state dir
    ///
    /// # Arguments
    ///
    /// * `store` - The state dir
    pub fn new(store: Store) -> Self {
        Reef { store }
    }

    /// Reads one node's status and work record
    #[tool(
        description = "Read one reef node by id: its status and where its work happens.",
        annotations(read_only_hint = true)
    )]
    async fn get(&self, Parameters(args): Parameters<NodeArgs>) -> Result<String, String> {
        // the node and its work record, as JSON
        let state = self.store.load().map_err(|error| format!("{error:#}"))?;
        let node = state.nodes.get(&args.id);
        let work = state.work.get(&args.id);
        if node.is_none() && work.is_none() {
            return Err(format!("No node {}.", args.id));
        }
        Ok(json!({ "id": args.id, "node": node, "work": work }).to_string())
    }

    /// Sets where a node's work stands
    #[tool(
        description = "Set where a reef node's work stands: todo, in-progress, done or blocked."
    )]
    async fn status(&self, Parameters(args): Parameters<StatusArgs>) -> Result<String, String> {
        // set it, then log the call so a run shows when it came
        self.store
            .set_status(&args.id, args.status)
            .map_err(|error| format!("{error:#}"))?;
        let _ = self.store.log(
            "tool",
            json!({ "tool": "status", "id": args.id, "status": args.status }),
        );
        Ok(format!("`{}` is {:?}", args.id, args.status))
    }
}

#[tool_handler(
    name = "reef",
    instructions = "A stub of reef's MCP server for X3's spike."
)]
impl ServerHandler for Reef {}

/// Serves the stub server over stdio until Claude Code closes it
///
/// # Arguments
///
/// * `store` - The state dir
#[instrument(name = "mcp::serve", skip_all, err(Debug))]
pub async fn serve(store: Store) -> anyhow::Result<()> {
    // serve over stdin and stdout and wait for the client to go away
    let service = Reef::new(store).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
