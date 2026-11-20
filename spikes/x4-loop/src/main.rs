//! X4's spike: a stub reef MCP server and Claude Code hooks that feed one session node by
//! node, clearing its context between nodes
//!
//! Every command takes the loop's state dir, shared by the MCP server Claude Code starts, the
//! hooks it runs, the injector the Stop hook starts, and the CLI.

mod hook;
mod inject;
mod mcp;
mod report;
mod state;

use anyhow::Error;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::state::{Mode, State, Store};

/// X4's stub of reef's MCP server, hooks and loop
#[derive(Parser)]
struct Args {
    /// The loop's state dir
    #[arg(long)]
    dir: PathBuf,
    /// What to do
    #[command(subcommand)]
    cmd: Cmd,
}

/// The commands the spike offers
#[derive(Subcommand)]
enum Cmd {
    /// Serve the stub MCP server over stdio
    Mcp,
    /// Run a Claude Code hook, reading its input from stdin
    Hook {
        /// The hook's event name, such as Stop
        event: String,
    },
    /// Type /clear into the session's pane once it is idle, then start the next node
    Inject {
        /// The session whose pane to type into
        #[arg(long)]
        session: String,
        /// The node whose end this acts on
        #[arg(long)]
        after: String,
    },
    /// Run the status line, recording the session's context
    StatusLine,
    /// Set how the loop moves between nodes
    Mode {
        /// The mode to use from now on
        mode: Mode,
    },
    /// Stop the loop after the current node
    Pause,
    /// Let the loop go on after the current node
    Resume,
    /// Start the stub plan again
    Reset {
        /// The mode to start in
        #[arg(long, default_value = "inject")]
        mode: Mode,
    },
    /// Print the loop's state
    Show,
    /// Print the run's timeline and its context per node
    Report,
}

/// Logs to a file in the state dir, since stdout is the MCP transport and the hooks' reply
///
/// # Arguments
///
/// * `store` - The loop's state dir
fn init_tracing(store: &Store) -> Result<(), Error> {
    // append to one log every process shares
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(store.path("x4.log"))?;
    tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    Ok(())
}

/// Runs one command against the loop's state dir
fn main() -> Result<(), Error> {
    // parse the command and open the state dir
    let args = Args::parse();
    let store = Store::new(&args.dir)?;
    init_tracing(&store)?;
    match args.cmd {
        // the MCP server is the only async command
        Cmd::Mcp => tokio::runtime::Runtime::new()?.block_on(mcp::serve(store)),
        Cmd::Hook { event } => hook::run(&store, &event),
        Cmd::Inject { session, after } => inject::run_injector(&store, &session, &after),
        Cmd::StatusLine => report::status_line(&store),
        Cmd::Mode { mode } => {
            store.update(|state| state.mode = mode)?;
            store.log("cli", serde_json::json!({ "mode": format!("{mode:?}") }))
        }
        Cmd::Pause => {
            store.update(|state| state.paused = true)?;
            store.log("cli", serde_json::json!({ "paused": true }))
        }
        Cmd::Resume => {
            store.update(|state| state.paused = false)?;
            store.log("cli", serde_json::json!({ "paused": false }))
        }
        Cmd::Reset { mode } => store.reset(&State::stub(mode)),
        Cmd::Show => {
            println!("{}", serde_json::to_string_pretty(&store.load()?)?);
            Ok(())
        }
        Cmd::Report => report::report(&store),
    }
}
