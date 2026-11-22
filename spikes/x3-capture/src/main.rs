//! X3's CLI: start and capture nodes from a local checkout, run the triggers, and the spike's
//! matrix and bench
//!
//! Every command that keeps state takes the state dir, by `--state` or `X3_STATE`, shared by
//! the CLI, the hooks Claude Code and git run, and the stub MCP server.

use clap::{Parser, Subcommand};
use std::io::Read;
use std::path::PathBuf;

use x3_capture::capture::{self, Options, Reader};
use x3_capture::rule::Rule;
use x3_capture::store::{Status, Store};
use x3_capture::{bench, hook, matrix, mcp};

/// X3's spike: capturing a node's changes from its local checkout
#[derive(Parser)]
struct Args {
    /// The state dir, outside every checkout
    #[arg(long, env = "X3_STATE", global = true)]
    state: Option<PathBuf>,
    /// What to do
    #[command(subcommand)]
    cmd: Cmd,
}

/// The commands the spike offers
#[derive(Subcommand)]
enum Cmd {
    /// Start a node in this checkout, recording and pinning what it starts from
    Start {
        /// The node
        node: String,
        /// The branch other people's work comes in on
        #[arg(long, default_value = "main")]
        base: String,
        /// The nodes it needs
        #[arg(long, value_delimiter = ',')]
        needs: Vec<String>,
        /// The checkout, if not this directory
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Capture a node's changes, or the one this checkout is working on
    Capture {
        /// The node, if not this checkout's current one
        node: Option<String>,
        /// The rule that finds the base
        #[arg(long, value_enum, default_value_t = Rule::CHOSEN)]
        rule: Rule,
        /// The reader that extracts the diff
        #[arg(long, value_enum, default_value_t = Reader::Cli)]
        reader: Reader,
        /// The checkout, if not this directory
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Set where a node's work stands
    Status {
        /// The node
        node: String,
        /// Its new status
        #[arg(value_enum)]
        status: Status,
    },
    /// Print a node's captures, or one of them as JSON
    Show {
        /// The node
        node: String,
        /// One revision, printed whole
        #[arg(long)]
        rev: Option<u32>,
    },
    /// Run a hook: PostToolUse, Stop, WorktreeCreate or WorktreeRemove from Claude Code, or
    /// post-commit or post-rewrite from git
    Hook {
        /// The event
        event: String,
        /// The hook's own arguments, as git passes post-rewrite its kind
        args: Vec<String>,
    },
    /// Serve the stub reef MCP server over stdio
    Mcp,
    /// Print the event log
    Events,
    /// Try every rule on every scratch-repo case and print the matrix
    Matrix {
        /// Only these cases
        cases: Vec<String>,
        /// Also print every rule's note at every check
        #[arg(long)]
        notes: bool,
    },
    /// Time the CLI against gix on real commits, and the snapshot
    Bench {
        /// The checkout to time in, a clone rather than a live repo
        #[arg(long)]
        repo: PathBuf,
        /// The commits whose own changes are read
        #[arg(long = "commit", required = true)]
        commits: Vec<String>,
        /// Timed rounds
        #[arg(long, default_value_t = 10)]
        rounds: usize,
        /// Untimed rounds first
        #[arg(long, default_value_t = 3)]
        warmup: usize,
    },
}

/// Opens the state dir a command needs
///
/// # Arguments
///
/// * `state` - The `--state` given, if any
fn store(state: &Option<PathBuf>) -> anyhow::Result<Store> {
    match state {
        Some(dir) => Store::new(dir),
        None => anyhow::bail!("this needs a state dir: --state or X3_STATE"),
    }
}

/// Runs one command
///
/// # Arguments
///
/// * `args` - The parsed command line
fn run(args: Args) -> anyhow::Result<()> {
    match args.cmd {
        Cmd::Start {
            node,
            base,
            needs,
            dir,
        } => {
            let store = store(&args.state)?;
            let work = capture::start(&store, &store.git(&dir), &node, Some(&base), &needs)?;
            println!(
                "{node} started in {} at {}, base {}",
                work.worktree.display(),
                &work.start[..7],
                work.base_branch.as_deref().unwrap_or("none")
            );
        }
        Cmd::Capture {
            node,
            rule,
            reader,
            dir,
        } => {
            let store = store(&args.state)?;
            // the node named, or the one this worktree started last
            let node = match node {
                Some(node) => node,
                None => {
                    let root = capture::worktree_of(&store, &dir)?;
                    store
                        .load()?
                        .current(&root)
                        .map(|work| work.node.clone())
                        .ok_or_else(|| anyhow::anyhow!("no node started in {}", root.display()))?
                }
            };
            // a capture asked for by hand seals the node's revision
            let options = Options {
                rule,
                reader,
                trigger: "cli".to_owned(),
                seal: true,
            };
            match capture::capture(&store, &node, &options)? {
                Some(capture) => {
                    let (added, removed) = capture.diff.counts();
                    println!(
                        "{node} revision {}: {} files, +{added} -{removed} ({})",
                        capture.revision,
                        capture.diff.files.len(),
                        capture.how
                    );
                }
                None => println!("{node}: nothing changed since its last revision"),
            }
        }
        Cmd::Status { node, status } => {
            store(&args.state)?.set_status(&node, status)?;
            println!("{node} is {status:?}");
        }
        Cmd::Show { node, rev } => {
            let state = store(&args.state)?.load()?;
            let captures = state.captures.get(&node).cloned().unwrap_or_default();
            match rev {
                // one revision, whole
                Some(rev) => {
                    let capture = captures
                        .iter()
                        .find(|capture| capture.revision == rev)
                        .ok_or_else(|| anyhow::anyhow!("{node} has no revision {rev}"))?;
                    println!("{}", serde_json::to_string_pretty(capture)?);
                }
                // a line a revision, and its files
                None => {
                    for capture in &captures {
                        let (added, removed) = capture.diff.counts();
                        println!(
                            "{node} r{} by {} ({}): +{added} -{removed}",
                            capture.revision, capture.trigger, capture.how
                        );
                        for file in &capture.diff.files {
                            println!("  {:?} {}", file.status, file.path());
                        }
                    }
                }
            }
        }
        Cmd::Hook {
            event,
            args: hook_args,
        } => {
            let store = store(&args.state)?;
            // Claude Code's hooks take JSON on stdin, and post-rewrite its rewritten commits
            let mut input = Vec::new();
            if event != "post-commit" {
                std::io::stdin().read_to_end(&mut input)?;
            }
            // WorktreeCreate answers with the worktree's path, its stdout's last line
            if let Some(out) = hook::run(
                &store,
                &event,
                &hook_args,
                &input,
                &std::env::current_dir()?,
            )? {
                println!("{out}");
            }
        }
        Cmd::Mcp => {
            let store = store(&args.state)?;
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(mcp::serve(store))?;
        }
        Cmd::Events => {
            for event in store(&args.state)?.events()? {
                println!("{event}");
            }
        }
        Cmd::Matrix { cases, notes } => {
            let outcomes = matrix::run(&cases)?;
            print!("{}", matrix::table(&outcomes));
            if notes {
                println!();
                print!("{}", matrix::notes(&outcomes));
            }
        }
        Cmd::Bench {
            repo,
            commits,
            rounds,
            warmup,
        } => {
            let mut timed = Vec::new();
            for commit in &commits {
                timed.extend(bench::diff(&repo, commit, warmup, rounds)?);
            }
            timed.push(bench::snapshot(&repo, warmup, rounds)?);
            print!("{}", bench::table(&timed));
        }
    }
    Ok(())
}

/// Runs the CLI, logging to stderr so stdout stays free for the MCP server
fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_env("X3_LOG"))
        .with_writer(std::io::stderr)
        .init();
    run(Args::parse())
}
