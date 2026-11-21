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

    /// Writes YAML to a file in the case's directory, returning its path for a command line
    ///
    /// # Arguments
    ///
    /// * `name` - The file's name
    /// * `text` - The YAML
    fn yaml(&self, name: &str, text: &str) -> anyhow::Result<String> {
        let path = self.ax.dir.join(name);
        std::fs::write(&path, text)?;
        Ok(path.to_str().expect("the case's dir is utf-8").to_string())
    }

    /// Puts YAML from a file, from the `reef` project's repo, returning what it printed
    ///
    /// # Arguments
    ///
    /// * `name` - The file to write the YAML to
    /// * `text` - The YAML
    async fn put(&self, name: &str, text: &str) -> anyhow::Result<String> {
        let file = self.yaml(name, text)?;
        self.reef(&self.ax.repo, &["node", "put", &file]).await
    }
}

/// A piece of work's YAML that says why and what, with the needs given
///
/// # Arguments
///
/// * `id` - The node's id
/// * `needs` - The ids it needs
fn work(id: &str, needs: &[&str]) -> String {
    format!(
        "id: {id}\ntitle: {id} Some work\nneeds: [{}]\ndescription:\n  why: Without {id}, \
         something is missing.\n  what: With {id}, it is there.\n",
        needs.join(", ")
    )
}

/// A group's YAML, with two members inline that say why and what
const GROUP: &str = "\
id: G
title: G A group
nodes:
  - id: m1
    title: m1 Some work
    description:
      why: Without m1, something is missing.
      what: With m1, it is there.
  - id: m2
    title: m2 Some work
    description:
      why: Without m2, something is missing.
      what: With m2, it is there.
";

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

/// A put naming a need that isn't a node gets check's message, and writes nothing
#[tokio::test]
async fn put_naming_an_unknown_need_gets_checks_message() -> anyhow::Result<()> {
    let cx = Cx::new("unknown-need").await?;
    // a node that needs one the plan doesn't have
    let error = cx
        .put("c99.yml", &work("C99", &["nope"]))
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.starts_with("the put was refused"), "{message}");
    assert!(
        message.contains("reef/central/C99: needs `nope`, which is not a node"),
        "{message}"
    );
    // and nothing was written
    let error = cx
        .reef(&cx.ax.repo, &["node", "get", "C99"])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not in its plan"), "{error}");
    Ok(())
}

/// A node got, edited and put back is written, and a put over a version since moved on, or a
/// new node over one that exists, is stale
#[tokio::test]
async fn got_node_put_back_and_stale_put_refused() -> anyhow::Result<()> {
    let cx = Cx::new("round-trip").await?;
    let repo = cx.ax.repo.clone();
    // a new node, written by hand with no version line
    let new = work("C1", &[]);
    assert_eq!(cx.put("c1.yml", &new).await?, "put `C1` at version 1\n");
    // got after the version it was read at, and the YAML it was put as
    let got = cx.reef(&repo, &["node", "get", "C1"]).await?;
    assert!(got.starts_with("# version: 1\nid: C1\n"), "{got}");
    assert!(got.contains("title: C1 Some work\n"), "{got}");
    // edited and put back, it moves on a version
    let edited = got.replace("title: C1 Some work", "title: C1 Edited work");
    assert_eq!(cx.put("c1.yml", &edited).await?, "put `C1` at version 2\n");
    let again = cx.reef(&repo, &["node", "get", "C1"]).await?;
    assert!(again.starts_with("# version: 2\n"), "{again}");
    assert!(again.contains("title: C1 Edited work\n"), "{again}");
    // what was got before that is stale now, and says what reads it again
    let error = cx.put("c1.yml", &got).await.unwrap_err().to_string();
    assert!(
        error.contains("`C1`: it changed since it was read"),
        "{error}"
    );
    assert!(error.contains("`reef node get C1`"), "{error}");
    // and a new node over one that exists says to keep the version line
    let error = cx.put("c1.yml", &new).await.unwrap_err().to_string();
    assert!(error.contains("`C1`: it already exists"), "{error}");
    assert!(error.contains("`# version:`"), "{error}");
    // a group goes in with its members inline, each a node of its own in it
    let put = cx.put("g.yml", GROUP).await?;
    assert_eq!(
        put,
        "put `G` at version 1\nput `m1` at version 1\nput `m2` at version 1\n"
    );
    let member = cx.reef(&repo, &["node", "get", "m1"]).await?;
    assert!(member.contains("group: G\n"), "{member}");
    let group = cx.reef(&repo, &["node", "get", "G"]).await?;
    assert!(!group.contains("nodes:"), "{group}");
    Ok(())
}

/// A node another needs can't be removed, nor a group without its members, and the nodes named
/// together go
#[tokio::test]
async fn rm_names_every_node_it_removes() -> anyhow::Result<()> {
    let cx = Cx::new("rm").await?;
    let repo = cx.ax.repo.clone();
    // a group of two, and work that needs one of them
    cx.put("g.yml", GROUP).await?;
    cx.put("w.yml", &work("W", &["m1"])).await?;
    // a node another needs is kept, with check's message
    let error = cx.reef(&repo, &["node", "rm", "m1"]).await.unwrap_err();
    let message = error.to_string();
    assert!(message.starts_with("the removal was refused"), "{message}");
    assert!(
        message.contains("reef/central/W: needs `m1`, which is not a node"),
        "{message}"
    );
    // a group alone is kept, since its members would name a group that isn't a node
    let error = cx.reef(&repo, &["node", "rm", "G"]).await.unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("reef/central/m1: group `G` is not a node"),
        "{message}"
    );
    // a node the plan doesn't have is named, and nothing is sent
    let error = cx
        .reef(&repo, &["node", "rm", "W", "nope"])
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "node `reef/central/nope` is not in its plan"
    );
    // what needs the member goes first, then the group with its members named
    assert_eq!(cx.reef(&repo, &["node", "rm", "W"]).await?, "removed `W`\n");
    let removed = cx.reef(&repo, &["node", "rm", "G", "m1", "m2"]).await?;
    assert_eq!(removed, "removed `G`\nremoved `m1`\nremoved `m2`\n");
    let error = cx.reef(&repo, &["node", "get", "G"]).await.unwrap_err();
    assert!(error.to_string().contains("not in its plan"), "{error}");
    Ok(())
}

/// A node's plan is the project's only plan, and with several, the one `--plan` names
#[tokio::test]
async fn node_commands_take_the_only_plan_or_the_one_named() -> anyhow::Result<()> {
    let cx = Cx::new("pick-plan").await?;
    let repo = cx.ax.repo.clone();
    // with one plan, it is the one put to
    cx.put("a.yml", &work("A", &[])).await?;
    // with two, a command says which there are and how to name one
    cx.reef(&repo, &["plan", "new", "other"]).await?;
    let error = cx
        .reef(&repo, &["node", "get", "A"])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("`reef/central`, `reef/other`"), "{error}");
    assert!(error.contains("`--plan`"), "{error}");
    // and each plan named is its own
    let got = cx
        .reef(&repo, &["node", "get", "A", "--plan", "central"])
        .await?;
    assert!(got.starts_with("# version: 1\nid: A\n"), "{got}");
    let error = cx
        .reef(&repo, &["node", "get", "A", "--plan", "other"])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not in its plan"), "{error}");
    Ok(())
}
