//! The CLI against a served service: each command run in-process, as the terminal runs it
//!
//! Each case serves the in-memory twin on a loopback port of its own, names that port in a
//! config of the case's own, and runs `reef`'s commands from the working directory it picks. The
//! JSON API's suite already runs every route against both backends, so these run on the twin.

// with `embedded`, the shared fixtures' pool open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef_service::MemoryBackend;
use std::path::{Path, PathBuf};

use common::git;
use common::http::Ax;

/// A served twin, and a config naming its address
struct Cx {
    /// The served service, and the case's own directory
    ax: Ax<MemoryBackend>,
    /// The config file naming the server's address
    conf: PathBuf,
}

impl Cx {
    /// Serves a fresh twin and writes a config naming its address
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its directory
    async fn new(case: &str) -> anyhow::Result<Cx> {
        // a twin with `reef` registered and `reef/central` listed, served on a port of its own
        let ax = common::Fx::memory(&format!("cli-{case}"))
            .await?
            .api()
            .await?;
        // the config the CLI reads its address from
        let conf = ax.dir.join("reef.toml");
        std::fs::write(&conf, format!("[api]\naddress = \"{}\"\n", ax.address))?;
        Ok(Cx { ax, conf })
    }

    /// Runs one `reef` command from a directory, returning what it printed
    ///
    /// # Arguments
    ///
    /// * `cwd` - The working directory to run it from
    /// * `args` - The command line after `reef --conf <config>`
    async fn reef(&self, cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
        // the command line as typed, with the case's config named
        let conf = self.conf.to_str().expect("the config is a utf-8 path");
        let line = ["reef", "--conf", conf]
            .into_iter()
            .chain(args.iter().copied());
        let cli = reef::Cli::from_args(line)?;
        // run it, keeping what it printed
        let mut out = Vec::new();
        reef::run(cli, cwd, &mut out).await?;
        Ok(String::from_utf8(out)?)
    }

    /// Makes a fresh repo in the case's directory
    ///
    /// # Arguments
    ///
    /// * `name` - The repo's directory name
    fn repo(&self, name: &str) -> anyhow::Result<PathBuf> {
        git::repo(&self.ax.dir.join(name))
    }
}

/// The first word of each line a command printed: the names it listed
///
/// # Arguments
///
/// * `out` - What the command printed
fn names(out: &str) -> Vec<&str> {
    out.lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect()
}

/// A project registered from its repo and a plan listed in it show in `reef plan ls`
#[tokio::test]
async fn project_add_and_plan_new_show_in_plan_ls() -> anyhow::Result<()> {
    let cx = Cx::new("plan-ls").await?;
    // a project registered from a directory deep in its repo is registered for the repo's root
    let repo = cx.repo("cli")?;
    let deep = repo.join("src");
    std::fs::create_dir_all(&deep)?;
    let added = cx.reef(&deep, &["project", "add", "cli"]).await?;
    assert_eq!(added, format!("registered `cli` for {}\n", repo.display()));
    // a plan listed in the working directory's project, once
    let listed = cx.reef(&deep, &["plan", "new", "docs"]).await?;
    assert_eq!(listed, "listed `cli/docs`\n");
    let again = cx.reef(&repo, &["plan", "new", "docs"]).await?;
    assert_eq!(again, "`cli/docs` is already listed\n");
    // shows in `plan ls`, with the revision readers load
    let plans = cx.reef(&repo, &["plan", "ls"]).await?;
    assert_eq!(names(&plans), ["cli/docs"]);
    assert!(plans.contains("revision "), "{plans}");
    // and `project ls` lists it beside the project the fixture registered, by name
    let projects = cx.reef(&repo, &["project", "ls"]).await?;
    assert_eq!(names(&projects), ["cli", "reef"]);
    // `--project` reaches another project's plans from here
    let other = cx.reef(&repo, &["plan", "ls", "--project", "reef"]).await?;
    assert_eq!(names(&other), ["reef/central"]);
    Ok(())
}

/// A linked worktree picks its main repo's project
#[tokio::test]
async fn linked_worktree_picks_its_main_repos_project() -> anyhow::Result<()> {
    let cx = Cx::new("worktree").await?;
    // a project for a repo, and a worktree of it beside it
    let repo = cx.repo("cli")?;
    cx.reef(&repo, &["project", "add", "cli"]).await?;
    let linked = git::worktree(&repo, &cx.ax.dir.join("cli-linked"))?;
    // from the worktree, the main repo's project is the one shown
    let shown = cx.reef(&linked, &["project", "show"]).await?;
    assert_eq!(names(&shown), ["cli"]);
    assert!(shown.contains(&repo.display().to_string()), "{shown}");
    // and a plan listed from it lands in that project
    let listed = cx.reef(&linked, &["plan", "new", "docs"]).await?;
    assert_eq!(listed, "listed `cli/docs`\n");
    Ok(())
}

/// A directory no project holds, and a project that isn't registered, say how to fix it
#[tokio::test]
async fn unregistered_dir_says_how_to_register() -> anyhow::Result<()> {
    let cx = Cx::new("unregistered").await?;
    // a repo no project is registered for
    let lonely = cx.repo("lonely")?;
    let error = cx.reef(&lonely, &["plan", "ls"]).await.unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("no project is registered for"),
        "{message}"
    );
    assert!(message.contains("reef project add"), "{message}");
    assert!(message.contains("--project"), "{message}");
    // a project named that isn't registered
    let error = cx
        .reef(&lonely, &["project", "show", "--project", "nope"])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("`nope`"), "{error}");
    Ok(())
}

/// A taken name, and a repo another project holds, are refused, saying which
#[tokio::test]
async fn taken_name_and_repo_are_refused() -> anyhow::Result<()> {
    let cx = Cx::new("taken").await?;
    // `cli` registered for one repo
    let repo = cx.repo("cli")?;
    cx.reef(&repo, &["project", "add", "cli"]).await?;
    // the name again, for another repo
    let other = cx.repo("other")?;
    let error = cx
        .reef(&other, &["project", "add", "cli"])
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("a project named `cli` is already registered"),
        "{error}"
    );
    // the repo again, under another name, named by `--repo` from elsewhere
    let repo_arg = repo.to_str().expect("the repo is a utf-8 path");
    let error = cx
        .reef(&other, &["project", "add", "again", "--repo", repo_arg])
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("already registered as `cli`"),
        "{error}"
    );
    Ok(())
}
