//! The stub reef MCP server, served over stdio with rmcp
//!
//! `next_task` claims work and so writes. `get_node`, `next` and `check` only read and are
//! marked `readOnlyHint`. `set_status` and `note` write and are not, so a run shows what plan
//! mode does with each kind.

use anyhow::Error;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tracing::instrument;

use crate::state::{Claim, Node, Status, Store};

/// The stub reef server, one per Claude Code session
#[derive(Clone)]
pub struct Loop {
    /// The loop's state dir
    store: Store,
}

/// Names one node
#[derive(Deserialize, JsonSchema)]
pub struct NodeId {
    /// The node's id, such as N1
    pub id: String,
}

/// Sets a node's status
#[derive(Deserialize, JsonSchema)]
pub struct SetStatus {
    /// The node's id, such as N1
    pub id: String,
    /// The node's new status
    pub status: Status,
}

/// A note on a node
#[derive(Deserialize, JsonSchema)]
pub struct Note {
    /// The node's id, such as N1
    pub id: String,
    /// The note's text
    pub text: String,
}

/// Writes a node out as next_task hands it to Claude, with the loop's steps
///
/// # Arguments
///
/// * `node` - The node to hand out
/// * `held` - Whether the loop already held it before this call
fn brief(node: &Node, held: bool) -> String {
    // say whether this is new work or the node already under way
    let lead = if held {
        "You are already on this node"
    } else {
        "Your next node"
    };
    let acceptance: Vec<String> = node
        .acceptance
        .iter()
        .map(|line| format!("- {line}"))
        .collect();
    let mut out = format!(
        "{lead}: {} ({})\n\nWhy: {}\nWhat: {}\nAcceptance:\n{}\n\nSteps:\n\
         1. Call EnterPlanMode now and plan this node in the textkit crate.\n\
         2. Ask the user at least one question with AskUserQuestion about the choice the node leaves open.\n",
        node.id,
        node.title,
        node.why,
        node.what,
        acceptance.join("\n"),
    );
    // the probe's extra step goes in while planning
    let mut step = 3;
    if let Some(probe) = &node.probe {
        out.push_str(&format!("{step}. {probe}\n"));
        step += 1;
    }
    out.push_str(&format!(
        "{step}. Call ExitPlanMode for the user's approval. Put the node's id in the plan's title.\n\
         {}. Once approved, implement it, run `cargo test`, then call the x4 set_status tool with id {} and status done.\n\
         {}. Then end your turn. reef starts the next node.\n",
        step + 1,
        node.id,
        step + 2,
    ));
    out
}

#[tool_router]
impl Loop {
    /// Creates the server over a state dir
    ///
    /// # Arguments
    ///
    /// * `store` - The loop's state dir
    pub fn new(store: Store) -> Self {
        Loop { store }
    }

    /// Logs a tool call and its reply, then hands the reply back
    ///
    /// # Arguments
    ///
    /// * `tool` - The tool's name
    /// * `args` - What it was called with
    /// * `reply` - What it answered
    fn logged(&self, tool: &str, args: serde_json::Value, reply: String) -> String {
        // a failed log must not fail the tool, so it is only traced
        if let Err(error) = self.store.log(
            "tool",
            json!({ "tool": tool, "args": args, "reply": reply }),
        ) {
            tracing::warn!(?error, "logging a tool call");
        }
        reply
    }

    /// Hands Claude the node to work on now, claiming the next one if the loop holds none
    #[tool(
        description = "Claim the next reef node and get its why, what, acceptance and the loop's steps. \
                       Returns the node the loop already holds if it isn't done yet."
    )]
    async fn next_task(&self) -> String {
        // claim under the lock so two calls never take two nodes
        let reply = match self.store.update(|state| state.claim_next()) {
            Ok(Claim::Claimed(node)) => brief(&node, false),
            Ok(Claim::Holding(node)) => brief(&node, true),
            Ok(Claim::Paused) => "The loop is paused. Stop here.".to_owned(),
            Ok(Claim::Finished) => "Every node is done. Stop here.".to_owned(),
            Err(error) => format!("next_task failed: {error:#}"),
        };
        self.logged("next_task", json!({}), reply)
    }

    /// Reads one node
    #[tool(
        description = "Read one reef node by id.",
        annotations(read_only_hint = true)
    )]
    async fn get_node(&self, Parameters(NodeId { id }): Parameters<NodeId>) -> String {
        // read the state and find the node
        let reply = match self.store.load() {
            Ok(state) => match state.node(&id) {
                Some(node) => serde_json::to_string_pretty(node).unwrap_or_default(),
                None => format!("No node {id}."),
            },
            Err(error) => format!("get_node failed: {error:#}"),
        };
        self.logged("get_node", json!({ "id": id }), reply)
    }

    /// Lists the nodes ready to start, in plan order
    #[tool(
        description = "List the reef nodes ready to start, in plan order.",
        annotations(read_only_hint = true)
    )]
    async fn next(&self) -> String {
        // every todo node is ready, since the stub plan has no needs
        let reply = match self.store.load() {
            Ok(state) => {
                let ready: Vec<String> = state
                    .nodes
                    .iter()
                    .filter(|node| node.status == Status::Todo)
                    .map(|node| format!("{} {}", node.id, node.title))
                    .collect();
                let current = state.current.unwrap_or_else(|| "none".to_owned());
                format!("Held: {current}\nReady: {}", ready.join(", "))
            }
            Err(error) => format!("next failed: {error:#}"),
        };
        self.logged("next", json!({}), reply)
    }

    /// Checks the plan
    #[tool(
        description = "Check the reef plan for problems.",
        annotations(read_only_hint = true)
    )]
    async fn check(&self) -> String {
        // the stub plan can't break, so this only counts its nodes
        let reply = match self.store.load() {
            Ok(state) => format!("{} nodes, no problems.", state.nodes.len()),
            Err(error) => format!("check failed: {error:#}"),
        };
        self.logged("check", json!({}), reply)
    }

    /// Sets a node's status
    #[tool(description = "Set a reef node's status: todo, in-progress or done.")]
    async fn set_status(
        &self,
        Parameters(SetStatus { id, status }): Parameters<SetStatus>,
    ) -> String {
        // change the node under the lock
        let reply = match self.store.update(|state| match state.node_mut(&id) {
            Some(node) => {
                node.status = status;
                format!("{id} is {}.", status.as_str())
            }
            None => format!("No node {id}."),
        }) {
            Ok(reply) => reply,
            Err(error) => format!("set_status failed: {error:#}"),
        };
        self.logged(
            "set_status",
            json!({ "id": id, "status": status.as_str() }),
            reply,
        )
    }

    /// Leaves a note on a node: a write, for the plan-mode probe
    #[tool(description = "Leave a note on a reef node.")]
    async fn note(&self, Parameters(Note { id, text }): Parameters<Note>) -> String {
        // append the note under the lock
        let reply = match self.store.update(|state| match state.node_mut(&id) {
            Some(node) => {
                node.notes.push(text.clone());
                format!("Noted on {id}.")
            }
            None => format!("No node {id}."),
        }) {
            Ok(reply) => reply,
            Err(error) => format!("note failed: {error:#}"),
        };
        self.logged("note", json!({ "id": id, "text": text }), reply)
    }
}

#[tool_handler(
    name = "x4",
    instructions = "A stub of reef's MCP server for X4's spike."
)]
impl ServerHandler for Loop {}

/// Serves the stub server over stdio until Claude Code closes it
///
/// # Arguments
///
/// * `store` - The loop's state dir
#[instrument(name = "mcp::serve", skip_all, err(Debug))]
pub async fn serve(store: Store) -> Result<(), Error> {
    // log the server's start, so a run shows when Claude Code started it
    store.log(
        "mcp",
        json!({ "step": "started", "pid": std::process::id() }),
    )?;
    // serve over stdin and stdout and wait for the client to go away
    let service = Loop::new(store.clone()).serve(stdio()).await?;
    let reason = service.waiting().await?;
    store.log(
        "mcp",
        json!({ "step": "stopped", "reason": format!("{reason:?}") }),
    )?;
    Ok(())
}
