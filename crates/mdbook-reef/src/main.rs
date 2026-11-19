//! mdbook-reef: planning graphs for mdbook.

mod check;
mod import;
mod install;
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
    /// Copy reef.js and reef.css beside book.toml and add them and the preprocessor to it,
    /// and write the reef Claude skill to the repository's .claude/skills/reef.
    Install {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Don't write the Claude skill.
        #[arg(long)]
        no_skill: bool,
    },
    /// Write (or refresh) the reef Claude skill in the repository that holds DIR.
    Skill {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Print SKILL.md instead of writing it.
        #[arg(long)]
        print: bool,
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
        Some(Command::Install { dir, no_skill }) => install::install(&dir, !no_skill),
        Some(Command::Skill { dir, print }) => {
            if print {
                use std::io::Write;
                std::io::stdout().write_all(install::SKILL[0].1)?;
                Ok(())
            } else {
                install::write_skill(&dir)
            }
        }
        Some(Command::Check { dir, quiet }) => {
            let n = check::check(&dir, quiet)?;
            std::process::exit(if n == 0 { 0 } else { 1 })
        }
        Some(Command::Import(args)) => import::run(args),
    }
}
