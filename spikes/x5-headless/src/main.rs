//! X5's spike: a stub host driving a headless Claude Code session over stream-json
//!
//! `claude -p` with `--input-format stream-json --output-format stream-json` and
//! `--permission-prompt-tool stdio` speaks the Agent SDK's host protocol: every prompt the session
//! would show a user arrives on its stdout as a `control_request`, and waits for the host's
//! `control_response` on its stdin. The stub host answers them as a user in the app would.

mod host;
mod report;

use anyhow::Error;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// X5's stub host and its report
#[derive(Parser)]
struct Args {
    /// What to do
    #[command(subcommand)]
    cmd: Cmd,
}

/// The commands the spike offers
#[derive(Subcommand)]
enum Cmd {
    /// Run one headless session in a repo, answering its prompts as the stub user
    Drive(host::Drive),
    /// Print a run's timeline as markdown
    Report {
        /// The run's log, written by `drive`
        log: PathBuf,
        /// X4's event log, whose hook events are merged in by time
        #[arg(long)]
        events: Option<PathBuf>,
    },
}

/// Runs the spike's command
#[tokio::main]
async fn main() -> Result<(), Error> {
    // the command line
    let args = Args::parse();
    match args.cmd {
        // one session, start to end
        Cmd::Drive(drive) => host::drive(drive).await,
        // a run's timeline
        Cmd::Report { log, events } => {
            let text = report::timeline(&log, events.as_deref())?;
            print!("{text}");
            Ok(())
        }
    }
}
