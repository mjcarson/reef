//! reef's commands: projects, plans and nodes, each a few calls to the JSON API
//!
//! ```text
//! reef [--conf PATH] project add <NAME> [--repo DIR]
//! reef [--conf PATH] project ls
//! reef [--conf PATH] project show [--project NAME]
//! reef [--conf PATH] plan new <NAME> [--project NAME]
//! reef [--conf PATH] plan ls [--project NAME]
//! reef [--conf PATH] node put [FILE] [--project NAME] [--plan NAME]
//! reef [--conf PATH] node get <ID> [--project NAME] [--plan NAME]
//! reef [--conf PATH] node rm <ID>... [--project NAME] [--plan NAME]
//! reef [--conf PATH] status <ID> <STATUS> [--project NAME] [--plan NAME]
//! reef [--conf PATH] accept <ID> [<N> <STATUS> [--test TEST]] [--project NAME] [--plan NAME]
//! reef [--conf PATH] check [--project NAME] [--plan NAME]
//! reef [--conf PATH] next [--project NAME] [--plan NAME]
//! reef [--conf PATH] mcp [--project NAME] [--plan NAME]
//! ```
//!
//! The working directory picks the project: the registered project whose repo holds it, its main
//! repo's for a linked worktree. `--project` names one instead. A node's plan is the project's
//! only plan, unless `--plan` names one.

use clap::{Args, Parser, Subcommand};
use reef_api::{AddedProject, Project};
use reef_model::{Met, PlanName, Status};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::client::Client;
use crate::conf::Conf;
use crate::mcp::{self, Tools};
use crate::node;
use crate::work;

/// reef's terminal client: projects, plans and nodes, through the service's JSON API
#[derive(Debug, Parser)]
#[command(name = "reef", version, about)]
pub struct Cli {
    /// reef's config, else `$REEF_CONFIG`, else `$XDG_CONFIG_HOME/reef/config.toml`
    #[arg(long, global = true, value_name = "PATH")]
    pub conf: Option<PathBuf>,
    /// What to do
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Parses a command line, the program's name first, without clap in the caller's tree
    ///
    /// # Arguments
    ///
    /// * `args` - The command line, such as `["reef", "plan", "ls"]`
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::Cli;
    ///
    /// let cli = Cli::from_args(["reef", "plan", "new", "central"]).unwrap();
    /// assert_eq!(cli.conf, None);
    /// ```
    pub fn from_args<I, T>(args: I) -> Result<Cli, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        <Cli as Parser>::try_parse_from(args)
    }
}

/// reef's commands
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Register, list and show projects
    #[command(subcommand)]
    Project(ProjectCommand),
    /// List a plan in a project, and list a project's plans
    #[command(subcommand)]
    Plan(PlanCommand),
    /// Put, get and remove a plan's nodes as YAML
    #[command(subcommand)]
    Node(NodeCommand),
    /// Set where a node's work stands: todo, in-progress, done or blocked
    ///
    /// A group's status is derived from its members, so only a member's is set.
    Status {
        /// The node's id
        id: String,
        /// Where its work now stands
        #[arg(value_name = "STATUS", value_parser = work::parse_status)]
        status: Status,
        /// The plan it is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// List a node's acceptance criteria, or set whether one is met: met, unmet or todo
    ///
    /// Criteria are numbered from 1, in the order written. A performance criterion with a
    /// target is met or not by its metric's figures, so its status isn't set here.
    Accept {
        /// The node's id
        id: String,
        /// The criterion's number, from the list this prints without one
        #[arg(value_name = "N", requires = "met")]
        criterion: Option<usize>,
        /// Whether it is met
        #[arg(value_name = "STATUS", value_parser = work::parse_met)]
        met: Option<Met>,
        /// The test that checks it, such as `tests/scan.rs::dropped_client`
        #[arg(long, value_name = "TEST", requires = "criterion")]
        test: Option<String>,
        /// The plan it is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Print the plan's problems, as `mdbook-reef check` prints a book's, failing if it has any
    Check {
        /// The plan to check, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Print the work ready to start, in plan order, and what the rest waits on
    Next {
        /// The plan to look in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Serve get, check, next, put, status and accept as MCP tools over standard input and
    /// output, for Claude Code
    ///
    /// get, check and next only read, and say so, so plan mode runs them without a prompt.
    Mcp {
        /// The project and plan the tools work in, if not the working directory's and its
        /// only plan
        #[command(flatten)]
        pick: PlanPick,
    },
}

/// What can be done with projects
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Register a project for the repo that holds a directory
    Add {
        /// The project's name
        name: String,
        /// A directory in its repo, else the working directory
        #[arg(long, value_name = "DIR")]
        repo: Option<PathBuf>,
    },
    /// List every registered project and its repo
    Ls,
    /// Show the project the working directory belongs to
    Show {
        /// The project to show instead
        #[command(flatten)]
        pick: Pick,
    },
}

/// What can be done with a project's plans
#[derive(Debug, Subcommand)]
pub enum PlanCommand {
    /// List a new, empty plan in the project
    New {
        /// The plan's name within its project
        name: String,
        /// The project to list it in, if not the working directory's
        #[command(flatten)]
        pick: Pick,
    },
    /// List the project's plans and the revision each is at
    Ls {
        /// The project whose plans to list, if not the working directory's
        #[command(flatten)]
        pick: Pick,
    },
}

/// What can be done with a plan's nodes
#[derive(Debug, Subcommand)]
pub enum NodeCommand {
    /// Put a node's YAML and its inline members, unless it is stale or breaks the plan
    ///
    /// YAML that starts with the `# version:` line `get` printed changes the node got; YAML
    /// without one is a new node.
    Put {
        /// The YAML to put, else standard input, which `-` names too
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
        /// The plan to put it in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Print a node's YAML, after the `# version:` line a put of it keeps
    Get {
        /// The node's id
        id: String,
        /// The plan it is in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
    /// Remove the nodes named as one change, unless it breaks the plan
    ///
    /// A group goes only with its members named too, and a node another needs only once
    /// nothing needs it.
    Rm {
        /// The ids of the nodes to remove
        #[arg(required = true, value_name = "ID")]
        ids: Vec<String>,
        /// The plan they are in, if not the project's only plan
        #[command(flatten)]
        pick: PlanPick,
    },
}

/// A project named on the command line, else the working directory's
#[derive(Debug, Clone, Default, Args)]
pub struct Pick {
    /// The project to use, instead of the one the working directory belongs to
    #[arg(long, value_name = "NAME")]
    pub project: Option<String>,
}

/// A plan named on the command line, else the picked project's only plan
#[derive(Debug, Clone, Default, Args)]
pub struct PlanPick {
    /// The project the plan is in
    #[command(flatten)]
    pub project: Pick,
    /// The plan to use within the project, instead of its only plan
    #[arg(long, value_name = "NAME")]
    pub plan: Option<String>,
}

/// Runs a command against the service reef's config names
///
/// # Arguments
///
/// * `cli` - The command line, parsed
/// * `cwd` - The working directory, absolute, which picks the project and anchors relative paths
/// * `out` - Where to print what the command found or did
///
/// # Examples
///
/// ```no_run
/// use clap::Parser;
/// use reef::Cli;
///
/// # async fn example() -> anyhow::Result<()> {
/// let cli = Cli::parse_from(["reef", "plan", "ls"]);
/// reef::run(cli, &std::env::current_dir()?, &mut std::io::stdout()).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run<W: Write>(cli: Cli, cwd: &Path, out: &mut W) -> anyhow::Result<()> {
    // the config named, read from the working directory, else the one the service reads
    let conf = Conf::load(cli.conf.map(|path| cwd.join(path)).as_deref())?;
    let client = Client::new(&conf)?;
    match cli.command {
        Command::Project(ProjectCommand::Add { name, repo }) => {
            project_add(&client, cwd, &name, repo.as_deref(), out).await
        }
        Command::Project(ProjectCommand::Ls) => project_ls(&client, out).await,
        Command::Project(ProjectCommand::Show { pick }) => {
            project_show(&client, cwd, &pick, out).await
        }
        Command::Plan(PlanCommand::New { name, pick }) => {
            plan_new(&client, cwd, &name, &pick, out).await
        }
        Command::Plan(PlanCommand::Ls { pick }) => plan_ls(&client, cwd, &pick, out).await,
        Command::Node(NodeCommand::Put { file, pick }) => {
            // the YAML first, so a file that can't be read asks nothing of the service
            let text = node::read_input(cwd, file.as_deref())?;
            let plan = picked_plan(&client, cwd, &pick).await?;
            node::put(&client, &plan, &text, out).await
        }
        Command::Node(NodeCommand::Get { id, pick }) => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            node::get(&client, &plan, &id, out).await
        }
        Command::Node(NodeCommand::Rm { ids, pick }) => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            node::rm(&client, &plan, &ids, out).await
        }
        Command::Status { id, status, pick } => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            work::status(&client, &plan, &id, status, out).await
        }
        Command::Accept {
            id,
            criterion,
            met,
            test,
            pick,
        } => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            // a criterion named comes with whether it is met, which clap requires
            let set = criterion.zip(met);
            work::accept(&client, &plan, &id, set, test.as_deref(), out).await
        }
        Command::Check { pick } => {
            // each problem, then a plan with any fails, as `mdbook-reef check` does
            let plan = picked_plan(&client, cwd, &pick).await?;
            let count = work::check(&client, &plan, out).await?;
            if count > 0 {
                anyhow::bail!("{}", work::problems(&plan, count));
            }
            writeln!(out, "{}", work::problems(&plan, 0))?;
            Ok(())
        }
        Command::Next { pick } => {
            let plan = picked_plan(&client, cwd, &pick).await?;
            work::next(&client, &plan, out).await
        }
        Command::Mcp { pick } => {
            // the protocol owns standard output, so nothing is printed to `out`
            let tools = Tools::new(client, cwd.to_path_buf(), pick);
            mcp::serve(tools, tokio::io::stdin(), tokio::io::stdout()).await
        }
    }
}

/// Registers a project for the repo that holds a directory
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `name` - The project's name
/// * `repo` - A directory in its repo, else the working directory
/// * `out` - Where to print the project registered
async fn project_add<W: Write>(
    client: &Client,
    cwd: &Path,
    name: &str,
    repo: Option<&Path>,
    out: &mut W,
) -> anyhow::Result<()> {
    // the service reads the repo as this host sees it, so it is sent absolute
    let dir = std::path::absolute(cwd.join(repo.unwrap_or(Path::new("."))))?;
    match client.add_project(name, &dir).await? {
        // the service located the repo's root, which is what was registered
        AddedProject::Added(project) => {
            writeln!(
                out,
                "registered `{}` for {}",
                project.name,
                project.repo.display()
            )?;
            Ok(())
        }
        AddedProject::NameTaken => {
            anyhow::bail!("a project named `{name}` is already registered")
        }
        AddedProject::RepoTaken { project } => anyhow::bail!(
            "the repo that holds {} is already registered as `{project}`",
            dir.display()
        ),
    }
}

/// Lists every registered project and its repo
///
/// # Arguments
///
/// * `client` - The service's client
/// * `out` - Where to print them
async fn project_ls<W: Write>(client: &Client, out: &mut W) -> anyhow::Result<()> {
    // every project, by name, its repo beside it
    let projects = client.projects().await?;
    let rows = projects
        .iter()
        .map(|project| (project.name.clone(), project.repo.display().to_string()));
    columns(out, rows)
}

/// Shows the project the working directory belongs to, or the one named
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `pick` - The project named, if one was
/// * `out` - Where to print it
async fn project_show<W: Write>(
    client: &Client,
    cwd: &Path,
    pick: &Pick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the project and its repo
    let project = picked(client, cwd, pick).await?;
    columns(
        out,
        [(project.name, project.repo.display().to_string())].into_iter(),
    )
}

/// Lists a new, empty plan in the picked project
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `name` - The plan's name within its project
/// * `pick` - The project named, if one was
/// * `out` - Where to print what came of it
async fn plan_new<W: Write>(
    client: &Client,
    cwd: &Path,
    name: &str,
    pick: &Pick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the plan's full name keeps the rule names share
    let project = picked(client, cwd, pick).await?;
    let plan = PlanName::new(project.name, name)?;
    // a plan already listed is left as it is, which is no failure
    match client.add_plan(&plan).await?.added {
        true => writeln!(out, "listed `{plan}`")?,
        false => writeln!(out, "`{plan}` is already listed")?,
    }
    Ok(())
}

/// Lists the picked project's plans and the revision each is at
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `pick` - The project named, if one was
/// * `out` - Where to print them
async fn plan_ls<W: Write>(
    client: &Client,
    cwd: &Path,
    pick: &Pick,
    out: &mut W,
) -> anyhow::Result<()> {
    // every plan of the project, by name, its revision beside it
    let project = picked(client, cwd, pick).await?;
    let plans = client.plans(&project.name).await?;
    let rows = plans
        .iter()
        .map(|info| (info.name.to_string(), format!("revision {}", info.revision)));
    columns(out, rows)
}

/// The project a command works in: the one named, else the working directory's
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `pick` - The project named, if one was
async fn picked(client: &Client, cwd: &Path, pick: &Pick) -> anyhow::Result<Project> {
    // a project named must be registered
    if let Some(name) = &pick.project {
        let projects = client.projects().await?;
        return projects
            .into_iter()
            .find(|project| &project.name == name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no project named `{name}` is registered: `reef project ls` lists them"
                )
            });
    }
    // else the one whose repo holds the working directory, its main repo's in a worktree
    client.find_project(cwd).await?.ok_or_else(|| {
        anyhow::anyhow!(
            "no project is registered for {}: register one with `reef project add <name>`, or \
             name one with `--project`",
            cwd.display()
        )
    })
}

/// The plan a node command works in: the one named in the picked project, else its only plan
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `pick` - The project and plan named, if they were
pub async fn picked_plan(client: &Client, cwd: &Path, pick: &PlanPick) -> anyhow::Result<PlanName> {
    // the plan is in the project named, else the working directory's
    let project = picked(client, cwd, &pick.project).await?;
    // a plan named keeps the rule names share, and the service says if it isn't listed
    if let Some(plan) = &pick.plan {
        return Ok(PlanName::new(project.name, plan)?);
    }
    // else the project's only plan
    let plans = client.plans(&project.name).await?;
    match plans.as_slice() {
        [only] => Ok(only.name.clone()),
        [] => anyhow::bail!(
            "project `{}` lists no plans: list one with `reef plan new <name>`",
            project.name
        ),
        many => {
            let names: Vec<String> = many.iter().map(|info| format!("`{}`", info.name)).collect();
            anyhow::bail!(
                "project `{}` lists {} plans, {}: name one with `--plan`",
                project.name,
                many.len(),
                names.join(", ")
            )
        }
    }
}

/// Prints rows of two columns, the first padded to its widest
///
/// # Arguments
///
/// * `out` - Where to print them
/// * `rows` - Each row's two cells
fn columns<W: Write>(
    out: &mut W,
    rows: impl Iterator<Item = (String, String)>,
) -> anyhow::Result<()> {
    // the first column is as wide as its widest cell
    let rows: Vec<(String, String)> = rows.collect();
    let width = rows
        .iter()
        .map(|(left, _)| left.chars().count())
        .max()
        .unwrap_or(0);
    for (left, right) in rows {
        writeln!(out, "{left:<width$}  {right}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, NodeCommand, PlanCommand, ProjectCommand, columns};
    use clap::{CommandFactory, Parser};
    use reef_model::{Met, Status};
    use std::path::PathBuf;

    /// clap's own checks of the command line's shape
    #[test]
    fn command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    /// `--conf` goes anywhere, and `--project` only where a project is picked
    #[test]
    fn arguments_go_where_they_mean_something() {
        // `--conf` before or after the command
        let before = Cli::try_parse_from(["reef", "--conf", "a.toml", "plan", "ls"]).unwrap();
        let after = Cli::try_parse_from(["reef", "plan", "ls", "--conf", "a.toml"]).unwrap();
        assert_eq!(before.conf, Some(PathBuf::from("a.toml")));
        assert_eq!(after.conf, Some(PathBuf::from("a.toml")));
        // a plan's project is picked on the command line
        let new = Cli::try_parse_from(["reef", "plan", "new", "central", "--project", "reef"]);
        match new.unwrap().command {
            Command::Plan(PlanCommand::New { name, pick }) => {
                assert_eq!(name, "central");
                assert_eq!(pick.project.as_deref(), Some("reef"));
            }
            other => panic!("parsed as {other:?}"),
        }
        // a project added takes its repo, and `--project` means nothing there
        let add = Cli::try_parse_from(["reef", "project", "add", "reef", "--repo", "../reef"]);
        assert!(matches!(
            add.unwrap().command,
            Command::Project(ProjectCommand::Add { repo: Some(_), .. })
        ));
        assert!(Cli::try_parse_from(["reef", "project", "add", "x", "--project", "y"]).is_err());
        assert!(Cli::try_parse_from(["reef", "project", "ls", "--project", "y"]).is_err());
        // a node's project and plan are picked on the command line
        let get = Cli::try_parse_from(["reef", "node", "get", "C17", "--plan", "central"]);
        match get.unwrap().command {
            Command::Node(NodeCommand::Get { id, pick }) => {
                assert_eq!(id, "C17");
                assert_eq!(pick.plan.as_deref(), Some("central"));
                assert_eq!(pick.project.project, None);
            }
            other => panic!("parsed as {other:?}"),
        }
        // a put's file is optional, and `rm` needs at least one id
        let put = Cli::try_parse_from(["reef", "node", "put", "--project", "reef"]);
        assert!(matches!(
            put.unwrap().command,
            Command::Node(NodeCommand::Put { file: None, .. })
        ));
        assert!(Cli::try_parse_from(["reef", "node", "rm"]).is_err());
        let rm = Cli::try_parse_from(["reef", "node", "rm", "M3", "C11"]).unwrap();
        assert!(matches!(
            rm.command,
            Command::Node(NodeCommand::Rm { ids, .. }) if ids == ["M3", "C11"]
        ));
        // and `--plan` means nothing where no plan is picked
        assert!(Cli::try_parse_from(["reef", "plan", "ls", "--plan", "x"]).is_err());
    }

    /// A status and a criterion's status take the frontmatter's names, and a criterion named
    /// needs one
    #[test]
    fn work_commands_take_what_frontmatter_does() {
        // a status by its name or an alias, and a name that isn't one refused
        let status = Cli::try_parse_from(["reef", "status", "C18", "wip", "--plan", "central"]);
        match status.unwrap().command {
            Command::Status { id, status, pick } => {
                assert_eq!((id.as_str(), status), ("C18", Status::InProgress));
                assert_eq!(pick.plan.as_deref(), Some("central"));
            }
            other => panic!("parsed as {other:?}"),
        }
        assert!(Cli::try_parse_from(["reef", "status", "C18", "nearly"]).is_err());
        assert!(Cli::try_parse_from(["reef", "status", "C18"]).is_err());
        // a node's criteria listed, or one set with its test
        let list = Cli::try_parse_from(["reef", "accept", "C18"]).unwrap();
        assert!(matches!(
            list.command,
            Command::Accept {
                criterion: None,
                met: None,
                ..
            }
        ));
        let set = ["reef", "accept", "C18", "2", "met", "--test", "t.rs::a"];
        match Cli::try_parse_from(set).unwrap().command {
            Command::Accept {
                criterion,
                met,
                test,
                ..
            } => {
                assert_eq!((criterion, met), (Some(2), Some(Met::Met)));
                assert_eq!(test.as_deref(), Some("t.rs::a"));
            }
            other => panic!("parsed as {other:?}"),
        }
        // a criterion needs whether it is met, and a test needs its criterion
        assert!(Cli::try_parse_from(["reef", "accept", "C18", "2"]).is_err());
        assert!(Cli::try_parse_from(["reef", "accept", "C18", "--test", "t.rs"]).is_err());
        assert!(Cli::try_parse_from(["reef", "accept", "C18", "x", "met"]).is_err());
        // check and next pick a plan, and take nothing else
        let check = Cli::try_parse_from(["reef", "check", "--project", "reef"]).unwrap();
        assert!(matches!(check.command, Command::Check { .. }));
        assert!(Cli::try_parse_from(["reef", "next", "C18"]).is_err());
        // and the MCP server picks its project and plan the same way
        let mcp = Cli::try_parse_from(["reef", "mcp", "--plan", "central"]).unwrap();
        assert!(matches!(
            mcp.command,
            Command::Mcp { pick } if pick.plan.as_deref() == Some("central")
        ));
    }

    /// The first column is padded to its widest cell
    #[test]
    fn columns_line_up() {
        // two rows of different widths
        let mut out = Vec::new();
        let rows = [
            ("reef".to_string(), "/src/reef".to_string()),
            ("shoal-db".to_string(), "/src/shoal".to_string()),
        ];
        columns(&mut out, rows.into_iter()).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "reef      /src/reef\nshoal-db  /src/shoal\n"
        );
        // and none prints nothing
        let mut out = Vec::new();
        columns(&mut out, std::iter::empty()).unwrap();
        assert!(out.is_empty());
    }
}
