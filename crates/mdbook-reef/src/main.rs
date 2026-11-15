//! mdbook-reef: planning graphs for mdbook.

mod check;
mod extract;
mod import;
mod install;
mod paths;
mod plan;
mod preprocess;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Planning graphs for mdbook: nodes written as markdown, drawn as a zoomable, collapsible DAG")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Tell mdbook whether a renderer is supported (called by mdbook).
    Supports { renderer: String },
    /// Copy reef.js and reef.css beside book.toml and add them and the preprocessor to it.
    Install {
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Check a book's nodes and reef fences without building it. Exits non-zero on problems.
    Check {
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(short, long)]
        quiet: bool,
    },
    /// Turn a mermaid flowchart into reef nodes.
    Import(import::Args),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => preprocess::run_stdio(),
        Some(Command::Supports { renderer }) => std::process::exit(if renderer == "html" { 0 } else { 1 }),
        Some(Command::Install { dir }) => install::install(&dir),
        Some(Command::Check { dir, quiet }) => {
            let n = check::check(&dir, quiet)?;
            std::process::exit(if n == 0 { 0 } else { 1 })
        }
        Some(Command::Import(args)) => import::run(args),
    }
}
