//! reef: projects, plans and nodes from the terminal, through the service's JSON API
//!
//! ```sh
//! reef project add reef        # register this repo as `reef`
//! reef plan new central        # list a plan in it
//! reef plan ls                 # and list its plans
//! reef node get C17 > c17.yml  # print a node's YAML, after the version it was read at
//! reef node put c17.yml        # and put it back, refused if it is stale or breaks the plan
//! reef node rm C62             # remove a node, refused if another needs it
//! ```

use clap::Parser;
use reef::Cli;
use std::process::ExitCode;

/// Runs one command, printing its error and exiting 1 if it fails
fn main() -> ExitCode {
    // clap prints usage and exits 2 on a command line it can't read
    let cli = Cli::parse();
    match execute(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("reef: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Runs one command on a runtime of its own, from the working directory
///
/// # Arguments
///
/// * `cli` - The command line, parsed
fn execute(cli: Cli) -> anyhow::Result<()> {
    // one command at a time needs one thread
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let cwd = std::env::current_dir()?;
    runtime.block_on(reef::run(cli, &cwd, &mut std::io::stdout().lock()))
}
