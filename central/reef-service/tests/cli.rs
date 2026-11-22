//! The CLI against a served service: each command run in-process, as the terminal runs it
//!
//! Each case serves the in-memory twin on a loopback port of its own, names that port in a
//! config of the case's own, and runs `reef`'s commands from the working directory it picks. The
//! JSON API's suite already runs every route against both backends, so these run on the twin.

// with `embedded`, the shared fixtures' pool open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef_model::spec::parse_node_file;
use reef_model::{Payload, PlanName};
use reef_render::ROOT;
use reef_render::book::{Plan, read_nodes};
use reef_render::paths;
use reef_service::MemoryBackend;
use serde_json::Value;
use std::collections::HashSet;
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

    /// Runs one `reef` command that is to fail, returning what it printed before it did and its
    /// error
    ///
    /// # Arguments
    ///
    /// * `cwd` - The working directory to run it from
    /// * `args` - The command line after `reef --conf <config>`
    async fn failing(&self, cwd: &Path, args: &[&str]) -> anyhow::Result<(String, String)> {
        // the command line as typed, with the case's config named
        let conf = self.conf.to_str().expect("the config is a utf-8 path");
        let line = ["reef", "--conf", conf]
            .into_iter()
            .chain(args.iter().copied());
        let cli = reef::Cli::from_args(line)?;
        // run it, keeping what it printed and how it failed
        let mut out = Vec::new();
        let error = reef::run(cli, cwd, &mut out)
            .await
            .expect_err(&format!("`reef {}` succeeded", args.join(" ")));
        Ok((String::from_utf8(out)?, error.to_string()))
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

/// Marking a group's last member done makes the group done, and what needs it ready
#[tokio::test]
async fn group_done_when_its_last_member_is() -> anyhow::Result<()> {
    let cx = Cx::new("group-done").await?;
    let repo = cx.ax.repo.clone();
    // a group of two members, and a node that needs the group
    cx.put("g.yml", GROUP).await?;
    cx.put("d.yml", &work("D", &["G"])).await?;
    // the members are ready, and the node waits on the group
    let next = cx.reef(&repo, &["next"]).await?;
    assert_eq!(
        next,
        "Ready to start (2), in plan order:\n  m1  m1 Some work\n  m2  m2 Some work\n\n\
         Waiting (1):\n  D   D Some work\n        needs G (todo)\n"
    );
    // one member done leaves the group in progress, and the node waiting on it
    let done = cx.reef(&repo, &["status", "m1", "done"]).await?;
    assert_eq!(done, "`m1` is done, at version 2\n");
    let next = cx.reef(&repo, &["next"]).await?;
    assert!(
        next.starts_with("Ready to start (1), in plan order:\n  m2"),
        "{next}"
    );
    assert!(next.contains("needs G (in-progress)"), "{next}");
    // the last one done makes the group done, and the node ready
    cx.reef(&repo, &["status", "m2", "done"]).await?;
    let next = cx.reef(&repo, &["next"]).await?;
    assert_eq!(
        next,
        "Ready to start (1), in plan order:\n  D  D Some work\n\nWaiting (0):\n  nothing\n"
    );
    Ok(())
}

/// A group's status is never set, a status already set writes nothing, and a node not in the
/// plan is named
#[tokio::test]
async fn status_is_set_on_work_alone() -> anyhow::Result<()> {
    let cx = Cx::new("status").await?;
    let repo = cx.ax.repo.clone();
    cx.put("g.yml", GROUP).await?;
    // a group is done when its members are, so its own is refused
    let error = cx
        .reef(&repo, &["status", "G", "done"])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("`G` is a group"), "{error}");
    // a member's is set once, and setting it again says so and writes nothing
    let started = cx.reef(&repo, &["status", "m1", "in-progress"]).await?;
    assert_eq!(started, "`m1` is in-progress, at version 2\n");
    let again = cx.reef(&repo, &["status", "m1", "wip"]).await?;
    assert_eq!(again, "`m1` is already in-progress\n");
    let got = cx.reef(&repo, &["node", "get", "m1"]).await?;
    assert!(got.starts_with("# version: 2\n"), "{got}");
    assert!(got.contains("status: in-progress\n"), "{got}");
    // back to to do, which the canonical YAML leaves out
    cx.reef(&repo, &["status", "m1", "todo"]).await?;
    let got = cx.reef(&repo, &["node", "get", "m1"]).await?;
    assert!(!got.contains("status:"), "{got}");
    // and a node the plan doesn't have is named in it
    let error = cx
        .reef(&repo, &["status", "nope", "done"])
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("`reef/central/nope` is not in its plan"),
        "{error}"
    );
    Ok(())
}

/// A node's criteria are listed by number, one is set met with its test, and one met by its
/// metric, or one it doesn't have, is refused
#[tokio::test]
async fn accept_lists_and_sets_criteria() -> anyhow::Result<()> {
    let cx = Cx::new("accept").await?;
    let repo = cx.ax.repo.clone();
    // a node with a unit criterion and a performance one met by its metric's figures
    let yaml = format!(
        "{}acceptance:\n  - unit: Rows round-trip\n  - kind: performance\n    text: Scans are \
         fast\n    metric: scan-ms\n    at-most: 10\nperformance:\n  metrics:\n    - name: \
         scan-ms\n      before: 20\n      after: 5\n",
        work("C1", &[])
    );
    cx.put("c1.yml", &yaml).await?;
    // listed by number, with whether each is met
    let listed = cx.reef(&repo, &["accept", "C1"]).await?;
    assert_eq!(
        listed,
        "1. todo  unit: Rows round-trip\n2. by its metric  perf: Scans are fast\n"
    );
    // the first set met, with the test that checks it
    let set = [
        "accept",
        "C1",
        "1",
        "met",
        "--test",
        "tests/rows.rs::round_trip",
    ];
    let met = cx.reef(&repo, &set).await?;
    assert_eq!(met, "criterion 1 of `C1` is met, at version 2\n");
    let got = cx.reef(&repo, &["node", "get", "C1"]).await?;
    assert!(
        got.contains("- unit: Rows round-trip\n  status: met\n  test: tests/rows.rs::round_trip\n"),
        "{got}"
    );
    let listed = cx.reef(&repo, &["accept", "C1"]).await?;
    assert!(
        listed.starts_with("1. met  unit: Rows round-trip\n   test: tests/rows.rs::round_trip\n"),
        "{listed}"
    );
    // the second is met or not by its metric, so it isn't set by hand
    let error = cx
        .reef(&repo, &["accept", "C1", "2", "met"])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("by its metric"), "{error}");
    // and a third isn't there
    let error = cx
        .reef(&repo, &["accept", "C1", "3", "met"])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("`C1` has 2 acceptance criteria"), "{error}");
    Ok(())
}

/// A plan with no problems says so, and one with a problem prints it and fails, and `next`
/// warns of it
#[tokio::test]
async fn check_fails_on_a_plan_with_a_problem() -> anyhow::Result<()> {
    let cx = Cx::new("check").await?;
    let repo = cx.ax.repo.clone();
    // a node whose doc is in the working tree, which leaves the plan whole
    std::fs::create_dir_all(repo.join("docs"))?;
    std::fs::write(repo.join("docs/a.md"), "# A\n\nThe design.\n")?;
    let linked = format!("{}docs: [docs/a.md]\n", work("C1", &[]));
    cx.put("c1.yml", &linked).await?;
    let checked = cx.reef(&repo, &["check"]).await?;
    assert_eq!(checked, "`reef/central` has no problems\n");
    // the doc's removal breaks it: check prints the problem, then fails naming the count
    std::fs::remove_file(repo.join("docs/a.md"))?;
    let (printed, error) = cx.failing(&repo, &["check"]).await?;
    assert_eq!(
        printed,
        "error: reef/central/C1: doc `docs/a.md` not found (looked for docs/a.md)\n"
    );
    assert_eq!(error, "`reef/central` has 1 problem");
    // and next still says what is ready, after a warning that the plan has a problem
    let next = cx.reef(&repo, &["next"]).await?;
    assert!(
        next.starts_with(
            "warning: `reef/central` has 1 problem; `reef check` lists them\nReady to start (1)"
        ),
        "{next}"
    );
    Ok(())
}

/// A queued entry that waits shows the needs it waits on, and next skips it for the queue's
/// ready entries until they are done
#[tokio::test]
async fn a_waiting_entry_shows_the_needs_it_waits_on() -> anyhow::Result<()> {
    let cx = Cx::new("queue-waits").await?;
    let repo = cx.ax.repo.clone();
    // A, B needing A, and C
    cx.put("a.yml", &work("A", &[])).await?;
    cx.put("b.yml", &work("B", &["A"])).await?;
    cx.put("c.yml", &work("C", &[])).await?;
    // B queued, then C at the top
    let added = cx.reef(&repo, &["queue", "add", "B"]).await?;
    assert_eq!(added, "`B` is queued 1 of 1\n");
    let added = cx.reef(&repo, &["queue", "add", "C", "--top"]).await?;
    assert_eq!(added, "`C` is queued 1 of 2\n");
    // the queue says C is ready, and B waits on A
    let ls = cx.reef(&repo, &["queue", "ls"]).await?;
    assert_eq!(
        ls,
        "1. C  ready    C Some work\n2. B  waiting  B Some work\n      needs A (todo)\n"
    );
    // next takes C first, then A in plan order, and says it skipped B, and why
    let next = cx.reef(&repo, &["next"]).await?;
    assert_eq!(
        next,
        "Ready to start (2), queued first, then in plan order:\n  C  C Some work\n  A  A Some \
         work\n\nSkipped in the queue (1):\n  B  B Some work\n       needs A (todo)\n\n\
         Waiting (1):\n  B  B Some work\n       needs A (todo)\n"
    );
    // once A is done, B is ready, and next takes the queue in its order
    cx.reef(&repo, &["status", "A", "done"]).await?;
    let ls = cx.reef(&repo, &["queue", "ls"]).await?;
    assert_eq!(ls, "1. C  ready  C Some work\n2. B  ready  B Some work\n");
    let next = cx.reef(&repo, &["next"]).await?;
    assert_eq!(
        next,
        "Ready to start (2), queued first, then in plan order:\n  C  C Some work\n  B  B Some \
         work\n\nWaiting (0):\n  nothing\n"
    );
    // and an entry taken off is gone
    let off = cx.reef(&repo, &["queue", "rm", "C", "B"]).await?;
    assert_eq!(off, "took `C` off the queue\ntook `B` off the queue\n");
    let ls = cx.reef(&repo, &["queue", "ls"]).await?;
    assert_eq!(
        ls,
        "the queue of `reef` is empty: `reef queue add <ID>` queues a node\n"
    );
    Ok(())
}

/// next spans the project's plans, naming nodes in full, and `--plan` narrows it to one, leaving
/// the other plans' entries out
#[tokio::test]
async fn next_spans_the_projects_plans_and_plan_narrows_it() -> anyhow::Result<()> {
    let cx = Cx::new("queue-spans").await?;
    let repo = cx.ax.repo.clone();
    // a second plan, and a piece of work in each
    cx.reef(&repo, &["plan", "new", "other"]).await?;
    let a = cx.yaml("a.yml", &work("A", &[]))?;
    cx.reef(&repo, &["node", "put", &a, "--plan", "central"])
        .await?;
    let x = cx.yaml("x.yml", &work("X", &[]))?;
    cx.reef(&repo, &["node", "put", &x, "--plan", "other"])
        .await?;
    // X queued by its full name, which needs no plan picked
    let added = cx.reef(&repo, &["queue", "add", "reef/other/X"]).await?;
    assert_eq!(added, "`reef/other/X` is queued 1 of 1\n");
    // across both plans, X first, every node named in full
    let next = cx.reef(&repo, &["next"]).await?;
    assert_eq!(
        next,
        "Ready to start (2), queued first, then in plan order:\n  reef/other/X    X Some work\n  \
         reef/central/A  A Some work\n\nWaiting (0):\n  nothing\n"
    );
    let ls = cx.reef(&repo, &["queue", "ls"]).await?;
    assert_eq!(ls, "1. reef/other/X  ready  X Some work\n");
    // narrowed to central, as next of that plan alone was before the queue
    let next = cx.reef(&repo, &["next", "--plan", "central"]).await?;
    assert_eq!(
        next,
        "Ready to start (1), in plan order:\n  A  A Some work\n\nWaiting (0):\n  nothing\n"
    );
    // and a bare id beside it is in its own plan
    let added = cx
        .reef(
            &repo,
            &[
                "queue",
                "add",
                "A",
                "--plan",
                "central",
                "--before",
                "reef/other/X",
            ],
        )
        .await?;
    assert_eq!(added, "`A` is queued 1 of 2\n");
    Ok(())
}

/// Each queue command says what it refuses and how to see what is queued
#[tokio::test]
async fn queue_commands_say_what_they_refuse() -> anyhow::Result<()> {
    let cx = Cx::new("queue-refuses").await?;
    let repo = cx.ax.repo.clone();
    cx.put("a.yml", &work("A", &[])).await?;
    // a move of a node not queued, or one beside a node not queued
    let (_, error) = cx.failing(&repo, &["queue", "move", "A", "--top"]).await?;
    assert_eq!(error, "`A` isn't queued: `reef queue ls` lists what is");
    let (_, error) = cx
        .failing(&repo, &["queue", "add", "A", "--before", "B"])
        .await?;
    assert_eq!(error, "`B` isn't queued: `reef queue ls` lists what is");
    // a move that says nowhere, and a removal of a node not queued
    let (_, error) = cx.failing(&repo, &["queue", "move", "A"]).await?;
    assert!(error.contains("needs where to"), "{error}");
    let (_, error) = cx.failing(&repo, &["queue", "rm", "A"]).await?;
    assert_eq!(error, "`A` isn't queued: `reef queue ls` lists what is");
    // a node not in its plan, and one of another project
    let (_, error) = cx.failing(&repo, &["queue", "add", "nope"]).await?;
    assert_eq!(error, "node `reef/central/nope` is not in its plan");
    let (_, error) = cx.failing(&repo, &["queue", "add", "app/main/A"]).await?;
    assert!(error.contains("not of project `reef`"), "{error}");
    // and nothing was queued
    let ls = cx.reef(&repo, &["queue", "ls"]).await?;
    assert!(ls.starts_with("the queue of `reef` is empty"), "{ls}");
    Ok(())
}

/// The keys of a node in the viewer's payload that say where it was written, which a book's
/// nodes and a stored plan's give differently
const PROVENANCE: [&str; 5] = ["source", "line", "name", "inline", "page"];

/// Copies a directory's files and directories into another, making it
///
/// # Arguments
///
/// * `from` - The directory to copy
/// * `to` - Where to copy it to
fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        // each directory copied in turn, each file as it is
        if path.is_dir() {
            copy_dir(&path, &to.join(entry.file_name()))?;
        } else {
            std::fs::copy(&path, to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Writes files under a directory, making the directories each is in
///
/// # Arguments
///
/// * `dir` - The directory to write them under
/// * `files` - Each file's path under it, and its text
fn write_files(dir: &Path, files: &[(&str, &str)]) -> anyhow::Result<()> {
    for (path, text) in files {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)?;
    }
    Ok(())
}

/// Where a book's links are served once it is imported
struct Served<'a> {
    /// Where the service serves its plan's docs
    docs: &'a str,
    /// The book's `src`, from the repo's root
    prefix: &'a str,
    /// The book's pages, by their paths from its `src`, which the plan keeps at the same paths
    pages: &'a HashSet<String>,
}

/// Text with each link a book's payload writes from its root made to lead where the service's
/// payload links lead: under its plan's docs, to the plan's page if the link names one of the
/// book's pages, and from the repo's root otherwise
///
/// A link is a field that starts with the root, or an `href` or `src` that does. The root named
/// anywhere else is prose, such as a node's text quoting it, which the service swaps for its
/// docs' path as it is.
///
/// # Arguments
///
/// * `text` - The text the links are in
/// * `book` - Where the book's links are served
fn served(text: &str, book: &Served) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(ROOT) {
        // the text before the link, then the link's path from where the service serves it
        out.push_str(&rest[..at]);
        let link = out.is_empty() || out.ends_with("href=\"") || out.ends_with("src=\"");
        let after = &rest[at + ROOT.len()..];
        let end = after
            .find(['"', '\'', '#', '<', '>', ' ', '\n', ')'])
            .unwrap_or(after.len());
        let path = &after[..end];
        out.push_str(book.docs);
        // a page at the same path, else the repo's file
        let page = paths::md_of(path).iter().any(|md| book.pages.contains(md));
        match link && !page {
            true => out.push_str(&paths::join(book.prefix, path)),
            false => out.push_str(path),
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// A payload as JSON, without where each node was written, and with a book's links made to
/// lead where the service's do
///
/// # Arguments
///
/// * `payload` - The viewer's payload
/// * `book` - Where a book's links are served, for a book's payload
fn comparable(payload: Payload, book: Option<&Served>) -> anyhow::Result<Value> {
    /// Each string in a value with its book links served, as `served` makes them
    ///
    /// # Arguments
    ///
    /// * `value` - The value to change
    /// * `book` - Where the book's links are served
    fn serve_all(value: &mut Value, book: &Served) {
        match value {
            Value::String(text) => *text = served(text, book),
            Value::Array(items) => items.iter_mut().for_each(|item| serve_all(item, book)),
            Value::Object(map) => map.values_mut().for_each(|item| serve_all(item, book)),
            _ => {}
        }
    }
    let mut json = serde_json::to_value(payload)?;
    // where each node was written is left out
    for node in json["nodes"].as_array_mut().into_iter().flatten() {
        let node = node.as_object_mut().expect("a node is an object");
        PROVENANCE.iter().for_each(|key| {
            node.remove(*key);
        });
    }
    // and a book's links are served as the service's are
    if let Some(book) = book {
        serve_all(&mut json, book);
    }
    Ok(json)
}

/// Checks that the plan a book was imported into draws as mdbook-reef draws the book, apart
/// from where each node was written, naming the first node that doesn't
///
/// # Arguments
///
/// * `cx` - The case, whose plan the book was imported into
/// * `book` - The book's directory
/// * `prefix` - The book's `src`, from the repo's root
async fn draws_as_the_book(cx: &Cx, book: &Path, prefix: &str) -> anyhow::Result<()> {
    // the book's payload, as mdbook-reef builds it from its files
    let opened = reef::import::open_book(book)?;
    let plan = Plan::load(&opened.src, &opened.config);
    anyhow::ensure!(
        plan.diags.is_empty(),
        "the book has problems:\n{}",
        plan.errors()
    );
    let docs = reef_api::paths::docs(&cx.ax.plan);
    let (files, _) = read_nodes(&opened.src, &opened.config.roots);
    let pages: HashSet<String> = reef::import::pages_of(&opened.src, &files)?
        .into_iter()
        .collect();
    let book = Served {
        docs: &docs,
        prefix,
        pages: &pages,
    };
    let from_book = comparable(plan.payload(&HashSet::new()), Some(&book))?;
    // and the plan's, as the service serves it
    let stored = comparable(cx.ax.service.payload(&cx.ax.plan).await?, None)?;
    // the same nodes in the same order, each the same, or the first that differs named
    let ids = |json: &Value| -> Vec<String> {
        json["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|node| node["id"].as_str().unwrap_or_default().to_string())
            .collect()
    };
    assert_eq!(ids(&from_book), ids(&stored));
    for (book_node, stored_node) in from_book["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .zip(stored["nodes"].as_array().into_iter().flatten())
    {
        assert_eq!(book_node, stored_node, "node `{}` differs", book_node["id"]);
    }
    Ok(())
}

/// The example book imported into a plan draws as mdbook-reef draws the book, apart from where
/// each node was written
#[tokio::test]
async fn imported_book_draws_as_mdbook_does() -> anyhow::Result<()> {
    let cx = Cx::new("import-example").await?;
    // the example book, copied into the `reef` project's repo
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../example-book");
    let book = cx.ax.repo.join("example-book");
    copy_dir(&example.join("src"), &book.join("src"))?;
    std::fs::copy(example.join("book.toml"), book.join("book.toml"))?;
    // imported into the project's only plan, from the repo's root
    let out = cx.reef(&cx.ax.repo, &["import", "example-book"]).await?;
    assert!(
        out.starts_with(
            "imported 202 nodes and 100 pages from example-book into `reef/central` at revision "
        ),
        "{out}"
    );
    // its nodes check as the book's do, and draw as the book's do
    let checked = cx.reef(&cx.ax.repo, &["check"]).await?;
    assert_eq!(checked, "`reef/central` has no problems\n");
    draws_as_the_book(&cx, &book, "example-book/src").await
}

/// A synthetic book whose nodes link every way a node can, from its body, its description, its
/// design choices, its measurements' note and its criteria, inline, by reference, to a heading
/// of its own file and out of the book into the repo, and whose page links to a page, to a
/// node's file, out of the book, to a heading of its own, and names a page in a fence
const LINKED_BOOK: [(&str, &str); 6] = [
    ("README.md", "# The repo\n\n## Setup\n\nSet it up.\n"),
    (
        "book/book.toml",
        "[book]\nsrc = \"src\"\n\n[preprocessor.reef]\n",
    ),
    (
        "book/src/design.md",
        "# Design\n\n## Rows\n\nEach node a row: [notes](plan/notes.md), [g](plan/g.md#x), \
         [setup](../../README.md#setup), [rows](#rows).\n\n```reef\npage = \"plan/notes.md\"\n```\n",
    ),
    (
        "book/src/plan/a.md",
        "---\nid: a\ntitle: a Some work\ngroup: g\ndocs: [../design.md]\n\
         description: { why: Without a., what: With a. }\n---\nSee [g](g.md) and [rows](../design.md#rows).\n",
    ),
    (
        "book/src/plan/g.md",
        "---\nid: g\ntitle: g The group\ndocs: [../design.md#rows]\nnodes:\n  - id: m\n    \
         body: \"# M heading\\n\\nSee [the repo](../../../README.md#setup) and [rows][r].\\n\\n[r]: ../design.md#rows\\n\"\n    \
         description:\n      why: \"Without [rows](../design.md#rows), slow.\"\n      what: \"Fast, see [x](#x).\"\n      \
         details: \"![a picture](../picture.png)\"\n    decisions: [\"[the design](../design.md)\"]\n    \
         performance:\n      note: \"Measured as [rows](../design.md#rows) says.\"\n      metrics: [{ name: n, before: 2, after: 1 }]\n    \
         acceptance:\n      - unit: \"Works as [the design](../design.md) says\"\n---\n\
         # G\n\n## X\n\nThe group: [rows](../design.md#rows), [its own](#x), [the repo](../../../README.md), <a@b.org>.\n",
    ),
    ("book/src/plan/notes.md", "# Notes\n\nA doc, not a node.\n"),
];

/// Every link a book's node writes leads where it did once the book is imported, and a member's
/// body stays in its spec
#[tokio::test]
async fn import_rebases_every_link() -> anyhow::Result<()> {
    let cx = Cx::new("import-links").await?;
    write_files(&cx.ax.repo, &LINKED_BOOK)?;
    // imported from the book's own directory, which picks its project
    let out = cx.reef(&cx.ax.repo.join("book"), &["import"]).await?;
    assert!(
        out.starts_with("imported 3 nodes and 2 pages from . into `reef/central`"),
        "{out}"
    );
    draws_as_the_book(&cx, &cx.ax.repo.join("book"), "book/src").await?;
    // the links lead from the repo's root, out of the book where it was left, a README to
    // the index mdbook writes for it
    let payload = cx.ax.service.payload(&cx.ax.plan).await?;
    let docs = reef_api::paths::docs(&cx.ax.plan);
    let g = payload
        .nodes
        .iter()
        .find(|node| node.id == "g")
        .expect("g is drawn");
    assert!(
        g.html.contains(&format!("href=\"{docs}index.html\"")),
        "{}",
        g.html
    );
    assert!(
        g.html
            .contains(&format!("href=\"{docs}book/src/plan/g.html#x\"")),
        "{}",
        g.html
    );
    assert!(g.html.contains("href=\"mailto:a@b.org\""), "{}", g.html);
    // the member keeps its body in its spec, its row's is empty, and it is titled by its id
    let m = cx
        .ax
        .service
        .get(&cx.ax.plan, "m")
        .await?
        .expect("m is stored");
    assert!(
        m.spec
            .body
            .as_deref()
            .is_some_and(|body| body.starts_with("# M heading")),
        "{m:?}"
    );
    assert_eq!(m.body, "");
    let drawn = payload
        .nodes
        .iter()
        .find(|node| node.id == "m")
        .expect("m is drawn");
    assert_eq!(drawn.title, "m");
    // its stored links are written from the repo's root, over which the book's pages lie
    assert_eq!(
        m.spec.description.as_ref().and_then(|d| d.why.as_deref()),
        Some("Without [rows](design.md#rows), slow.")
    );
    assert_eq!(m.spec.docs, Vec::<String>::new());
    let g_stored = cx
        .ax
        .service
        .get(&cx.ax.plan, "g")
        .await?
        .expect("g is stored");
    assert_eq!(g_stored.spec.docs, ["design.md#rows"]);
    // the book's pages are the plan's, at their paths from its `src`
    let pages = cx.ax.service.pages(&cx.ax.plan).await?.pages;
    let paths: Vec<&str> = pages.iter().map(|page| page.path.as_str()).collect();
    assert_eq!(paths, ["design.md", "plan/notes.md"]);
    // a page's links to pages and its own headings are as written, its fence too, and its
    // others lead to the repo's files from where it is
    assert_eq!(
        pages[0].text,
        "# Design\n\n## Rows\n\nEach node a row: [notes](plan/notes.md), [g](book/src/plan/g.md#x), \
         [setup](README.md#setup), [rows](#rows).\n\n```reef\npage = \"plan/notes.md\"\n```\n"
    );
    // and served, they lead where they did: the page, the node's file, and the repo's README
    let page = cx
        .ax
        .service
        .doc_page(&cx.ax.plan, "design.html")
        .await?
        .expect("the page is served");
    for href in [
        format!("href=\"{docs}plan/notes.html\""),
        format!("href=\"{docs}book/src/plan/g.html#x\""),
        format!("href=\"{docs}index.html#setup\""),
        format!("href=\"{docs}design.html#rows\""),
    ] {
        assert!(page.contains(&href), "{href} isn't in {page}");
    }
    Ok(())
}

/// An import refuses a plan that has nodes unless told to replace them, a book that names its
/// own sections, a book with a problem and an id a plan can't name, writing nothing
#[tokio::test]
async fn import_refuses() -> anyhow::Result<()> {
    let cx = Cx::new("import-refuses").await?;
    let repo = cx.ax.repo.clone();
    let node = |id: &str, extra: &str| {
        format!(
            "---\nid: \"{id}\"\ntitle: {id} Some work\n{extra}description: {{ why: W, what: X }}\n---\n"
        )
    };
    write_files(
        &repo,
        &[
            ("book/book.toml", "[preprocessor.reef]\n"),
            ("book/src/plan/a.md", &node("a", "")),
        ],
    )?;
    // a first import into the empty plan, then a second only with `--replace`
    cx.reef(&repo, &["import", "book"]).await?;
    let (_, error) = cx.failing(&repo, &["import", "book"]).await?;
    assert_eq!(
        error,
        "`reef/central` has 1 node, which an import replaces: pass `--replace` to replace them"
    );
    let out = cx.reef(&repo, &["import", "book", "--replace"]).await?;
    assert!(out.starts_with("imported 1 node from book"), "{out}");
    let revision = cx.ax.service.list(&cx.ax.plan).await?.revision;
    // a book briefed from sections of its own
    write_files(
        &repo,
        &[
            (
                "own/book.toml",
                "[preprocessor.reef]\nwhy-sections = [\"Why\"]\n",
            ),
            ("own/src/a.md", &node("a", "")),
        ],
    )?;
    let (_, error) = cx.failing(&repo, &["import", "own", "--replace"]).await?;
    assert!(error.contains("names its own doc sections"), "{error}");
    // a book with a problem says it at its file and line
    write_files(
        &repo,
        &[
            ("broken/book.toml", "[preprocessor.reef]\n"),
            ("broken/src/a.md", &node("a", "docs: [missing.md]\n")),
        ],
    )?;
    let (_, error) = cx
        .failing(&repo, &["import", "broken", "--replace"])
        .await?;
    assert!(
        error.starts_with("broken has 1 problem, so nothing was imported:"),
        "{error}"
    );
    assert!(
        error.contains("a.md:") && error.contains("missing.md"),
        "{error}"
    );
    // an id a plan can't name
    write_files(
        &repo,
        &[
            ("slash/book.toml", "[preprocessor.reef]\n"),
            ("slash/src/a.md", &node("a/b", "")),
        ],
    )?;
    let (_, error) = cx.failing(&repo, &["import", "slash", "--replace"]).await?;
    assert!(
        error.starts_with("a.md: `a/b` can't be a node of `reef/central`"),
        "{error}"
    );
    // a link to a repo file a page of the book would hide
    write_files(
        &repo,
        &[
            ("README.md", "# The repo\n"),
            ("shadow/book.toml", "[preprocessor.reef]\n"),
            ("shadow/src/a.md", &node("a", "docs: [README.md]\n")),
            ("shadow/src/README.md", "# The book's own\n"),
            (
                "shadow/src/b.md",
                &format!("{}[the repo](../../README.md)\n", node("b", "")),
            ),
        ],
    )?;
    let (_, error) = cx
        .failing(&repo, &["import", "shadow", "--replace"])
        .await?;
    assert_eq!(
        error,
        "b.md: the link `../../README.md` leads to the repo's `README.md`, which the book's page \
         at that path would hide once imported"
    );
    // a node file outside the book's roots, which would be kept as a page a book reads as a node
    write_files(
        &repo,
        &[
            (
                "rooted/book.toml",
                "[preprocessor.reef]\nroots = [\"plan\"]\n",
            ),
            ("rooted/src/plan/a.md", &node("a", "")),
            ("rooted/src/notes.md", &node("n", "")),
        ],
    )?;
    let (_, error) = cx
        .failing(&repo, &["import", "rooted", "--replace"])
        .await?;
    assert!(
        error.starts_with("notes.md has a node's frontmatter, an `id`, but is outside"),
        "{error}"
    );
    // and none of them wrote anything
    assert_eq!(cx.ax.service.list(&cx.ax.plan).await?.revision, revision);
    // a plan with pages and no nodes is replaced only when asked too
    cx.reef(&repo, &["plan", "new", "paged"]).await?;
    write_files(&repo, &[("plan.md", "# Plan\n")])?;
    cx.reef(
        &repo,
        &["page", "put", "plan.md", "plan.md", "--plan", "paged"],
    )
    .await?;
    let (_, error) = cx
        .failing(&repo, &["import", "book", "--plan", "paged"])
        .await?;
    assert_eq!(
        error,
        "`reef/paged` has 0 nodes and 1 page, which an import replaces: pass `--replace` to \
         replace them"
    );
    Ok(())
}

/// Copies the example book's `book.toml` and `src` into a directory
///
/// # Arguments
///
/// * `to` - The directory to copy it to
fn example_book(to: &Path) -> anyhow::Result<()> {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../example-book");
    copy_dir(&example.join("src"), &to.join("src"))?;
    std::fs::copy(example.join("book.toml"), to.join("book.toml"))?;
    Ok(())
}

/// Every file under a directory, by its path from it, with its text
///
/// # Arguments
///
/// * `dir` - The directory to read
fn read_tree(dir: &Path) -> anyhow::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(dir)?.to_string_lossy().to_string();
                out.push((rel, std::fs::read_to_string(&path)?));
            }
        }
    }
    // in an order that doesn't hang on the order they were read in
    out.sort();
    Ok(out)
}

/// Checks that two plans hold the same nodes, each with its spec and body, and in the same
/// places if asked, and the same pages
///
/// # Arguments
///
/// * `cx` - The case, whose service holds both plans
/// * `a` - One plan
/// * `b` - The other
/// * `placed` - Whether each node must have the same place in the written order too
async fn same_plans(cx: &Cx, a: &PlanName, b: &PlanName, placed: bool) -> anyhow::Result<()> {
    let mut first = cx.ax.service.list(a).await?.nodes;
    let mut second = cx.ax.service.list(b).await?.nodes;
    // by id, if their places may differ
    if !placed {
        first.sort_by(|x, y| x.spec.id.cmp(&y.spec.id));
        second.sort_by(|x, y| x.spec.id.cmp(&y.spec.id));
    }
    // each node its spec, its body and its place, if asked
    assert_eq!(first.len(), second.len());
    for (x, y) in first.iter().zip(&second) {
        assert_eq!(
            (&x.spec, &x.body),
            (&y.spec, &y.body),
            "`{}` differs",
            x.spec.id
        );
        if placed {
            assert_eq!(x.order, y.order, "`{}` moved", x.spec.id);
        }
    }
    // and each page
    let pages = |pages: Vec<reef_api::StoredPage>| -> Vec<(String, String)> {
        pages
            .into_iter()
            .map(|page| (page.path, page.text))
            .collect()
    };
    let first = cx.ax.service.pages(a).await?.pages;
    let second = cx.ax.service.pages(b).await?.pages;
    assert_eq!(pages(first), pages(second));
    Ok(())
}

/// A plan exported to a book and imported again gives the same specs, bodies and order, and
/// the book exports again to the same files
#[tokio::test]
async fn export_then_import_gives_equal_specs() -> anyhow::Result<()> {
    let cx = Cx::new("export-round-trip").await?;
    let repo = cx.ax.repo.clone();
    // the example book, imported
    example_book(&repo.join("example-book"))?;
    cx.reef(&repo, &["import", "example-book"]).await?;
    // exported to a new book in the repo, a file for each of the book's node files
    let out = cx
        .reef(&repo, &["export", "out", "--plan", "central"])
        .await?;
    assert!(
        out.starts_with("exported 202 nodes and 100 pages of `reef/central` to out in 112 files:"),
        "{out}"
    );
    // which mdbook-reef finds no problems in
    let opened = reef::import::open_book(&repo.join("out"))?;
    let plan = Plan::load(&opened.src, &opened.config);
    assert!(plan.diags.is_empty(), "{}", plan.errors());
    // imported again, into a plan of its own, each node and page as the first import wrote it
    cx.reef(&repo, &["import", "out", "--plan", "again"])
        .await?;
    let again = PlanName::new("reef", "again")?;
    same_plans(&cx, &cx.ax.plan, &again, true).await?;
    assert_eq!(cx.ax.service.pages(&again).await?.pages.len(), 100);
    // the summary lists each page after the node files
    let summary = std::fs::read_to_string(repo.join("out/src/SUMMARY.md"))?;
    assert!(
        summary.contains("\n# Pages\n\n- [") && summary.contains("](plan.md)\n"),
        "{summary}"
    );
    // and exported again, it writes the same files, titled by its own plan
    cx.reef(&repo, &["export", "out2", "--plan", "again"])
        .await?;
    let retitled: Vec<(String, String)> = read_tree(&repo.join("out"))?
        .into_iter()
        .map(|(path, text)| (path, text.replace("reef/central", "reef/again")))
        .collect();
    assert_eq!(retitled, read_tree(&repo.join("out2"))?);
    Ok(())
}

/// An export refuses a directory that holds anything, and one outside the project's repo,
/// leaving no directory it made behind
#[tokio::test]
async fn export_refuses() -> anyhow::Result<()> {
    let cx = Cx::new("export-refuses").await?;
    let repo = cx.ax.repo.clone();
    cx.put("a.yml", &work("A", &[])).await?;
    // a directory with anything in it
    write_files(&repo, &[("full/notes.md", "# Notes\n")])?;
    let (_, error) = cx.failing(&repo, &["export", "full"]).await?;
    assert_eq!(
        error,
        "full isn't empty: export writes a new book, or over a reef book with `--replace`"
    );
    // a directory outside the project's repo, which the export takes away again
    let (_, error) = cx
        .failing(&repo, &["export", "../outside", "--project", "reef"])
        .await?;
    assert!(error.contains("is not in project `reef`'s repo"), "{error}");
    assert!(!cx.ax.dir.join("outside").exists());
    // a page at the path a node's file is written to
    write_files(&repo, &[("clash.md", "# Clash\n")])?;
    cx.reef(&repo, &["page", "put", "001-a.md", "clash.md"])
        .await?;
    let (_, error) = cx.failing(&repo, &["export", "clash"]).await?;
    assert_eq!(
        error,
        "page `001-a.md` has the name the file of node `A` is written to: move the page"
    );
    assert!(!repo.join("clash").exists());
    cx.reef(&repo, &["page", "rm", "001-a.md"]).await?;
    // and the plan exports where it may
    let out = cx.reef(&repo, &["export", "book"]).await?;
    assert!(
        out.starts_with("exported 1 node of `reef/central` to book in 1 file:"),
        "{out}"
    );
    Ok(())
}

/// A plan's pages are listed, got, put and removed by version, each edit checked against the
/// plan, and a page put over a file of the repo says so
#[tokio::test]
async fn page_commands_edit_by_version() -> anyhow::Result<()> {
    let cx = Cx::new("page-commands").await?;
    let repo = cx.ax.repo.clone();
    // a plan keeps no pages at first
    let ls = cx.reef(&repo, &["page", "ls"]).await?;
    assert_eq!(ls, "`reef/central` keeps no pages\n");
    // a new page is put from a file, and put again as new is refused as taken
    write_files(
        &repo,
        &[(
            "scratch/design.md",
            "# Design\n\n## Rows\n\nEach node a row.\n",
        )],
    )?;
    let put = cx
        .reef(&repo, &["page", "put", "design.md", "scratch/design.md"])
        .await?;
    assert_eq!(put, "put page `design.md` at version 1\n");
    let (_, error) = cx
        .failing(&repo, &["page", "put", "design.md", "scratch/design.md"])
        .await?;
    assert_eq!(
        error,
        "the put was refused, since page `design.md` of `reef/central` is not as it was read: it \
         already exists: to change it, put what `reef page get design.md` prints, keeping its \
         `<!-- version: … -->` line"
    );
    // what get prints is put back as a change to the version it read
    let got = cx.reef(&repo, &["page", "get", "design.md"]).await?;
    assert_eq!(
        got,
        "<!-- version: 1 -->\n# Design\n\n## Rows\n\nEach node a row.\n"
    );
    write_files(&repo, &[("scratch/v1.md", &format!("{got}\nMore.\n"))])?;
    let put = cx
        .reef(&repo, &["page", "put", "design.md", "scratch/v1.md"])
        .await?;
    assert_eq!(put, "put page `design.md` at version 2\n");
    // and put at that version again it is stale
    let (_, error) = cx
        .failing(&repo, &["page", "put", "design.md", "scratch/v1.md"])
        .await?;
    assert!(error.contains("it changed since it was read"), "{error}");
    // a node links its heading, so it can't be taken out, nor lose that heading
    let linked =
        "id: A\ntitle: A Some work\ndocs: [design.md#rows]\ndescription: { why: W, what: X }\n";
    cx.put("a.yml", linked).await?;
    let (_, error) = cx.failing(&repo, &["page", "rm", "design.md"]).await?;
    assert!(
        error.starts_with(
            "the removal was refused, since it would add these problems to `reef/central`:"
        ),
        "{error}"
    );
    assert!(
        error.contains("doc `design.md#rows` not found (looked for design.md)"),
        "{error}"
    );
    write_files(
        &repo,
        &[(
            "scratch/v2.md",
            "<!-- version: 2 -->\n# Design\n\nNo rows.\n",
        )],
    )?;
    let (_, error) = cx
        .failing(&repo, &["page", "put", "design.md", "scratch/v2.md"])
        .await?;
    assert!(
        error.contains("design.md has no heading with the id `rows`"),
        "{error}"
    );
    // a page over a file of the repo says it hides it
    write_files(&repo, &[("notes.md", "# The repo's notes\n")])?;
    let put = cx
        .reef(&repo, &["page", "put", "notes.md", "scratch/design.md"])
        .await?;
    assert_eq!(
        put,
        "put page `notes.md` at version 1\nnote: the page hides the file `notes.md` in `reef`'s \
         repo, which its links led to\n"
    );
    let ls = cx.reef(&repo, &["page", "ls"]).await?;
    assert_eq!(ls, "design.md (version 2)\nnotes.md (version 1)\n");
    // once nothing links it, it comes out, and a path no page can have is refused
    cx.reef(&repo, &["node", "rm", "A"]).await?;
    let rm = cx.reef(&repo, &["page", "rm", "design.md"]).await?;
    assert_eq!(rm, "removed page `design.md`\n");
    let (_, error) = cx
        .failing(&repo, &["page", "put", "../out.md", "scratch/design.md"])
        .await?;
    assert_eq!(
        error,
        "`../out.md` has a part starting with `.`, which a book leaves out"
    );
    let (_, error) = cx.failing(&repo, &["page", "get", "design.md"]).await?;
    assert!(
        error.contains("no page `design.md` in `reef/central`"),
        "{error}"
    );
    Ok(())
}

/// This plan's own book, imported, checks clean, draws as the book does and serves every
/// design page once the book is taken away, and its export has the page its overview's fence
/// names
#[tokio::test]
async fn imported_pages_outlive_the_book() -> anyhow::Result<()> {
    let cx = Cx::new("plan-outlives").await?;
    let repo = cx.ax.repo.clone();
    // this repo's plan, copied into the project's repo
    let book = this_plan(&repo)?;
    let pages = reef::import::pages_of(&book.join("src"), &read_nodes(&book.join("src"), &[]).0)?;
    // imported, every design page with it
    let out = cx.reef(&repo, &["import", "plan"]).await?;
    assert!(
        out.contains(&format!(
            " and {} pages from plan into `reef/central`",
            pages.len()
        )),
        "{out}"
    );
    draws_as_the_book(&cx, &book, "plan/src").await?;
    // the book taken away, the plan still checks clean
    std::fs::remove_dir_all(&book)?;
    let checked = cx.reef(&repo, &["check"]).await?;
    assert_eq!(checked, "`reef/central` has no problems\n");
    // and serves each of its pages
    for page in &pages {
        let html = paths::html_of(page);
        let served = cx.ax.service.doc_page(&cx.ax.plan, &html).await?;
        assert!(
            served.is_some_and(|html| html.contains("<main>")),
            "{page} isn't served"
        );
    }
    // exported, the overview's fence names a page the export has
    cx.reef(&repo, &["export", "out"]).await?;
    let overview = read_tree(&repo.join("out/src"))?
        .into_iter()
        .find(|(_, text)| text.starts_with("---\nid: central\n"))
        .expect("the overview is exported");
    assert!(overview.1.contains("page = \"plan.md\""), "{}", overview.0);
    assert!(repo.join("out/src/plan.md").is_file());
    Ok(())
}

/// Copies this repo's plan, its `book.toml` and its `src`, into a repo, returning where it is
///
/// # Arguments
///
/// * `repo` - The repo to copy it into
fn this_plan(repo: &Path) -> anyhow::Result<PathBuf> {
    let plan_book = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan");
    let book = repo.join("plan");
    copy_dir(&plan_book.join("src"), &book.join("src"))?;
    std::fs::copy(plan_book.join("book.toml"), book.join("book.toml"))?;
    Ok(book)
}

/// Checks that a book written over holds what it did: the same files, each node file's nodes
/// the same and its body byte for byte, and every other file byte for byte
///
/// # Arguments
///
/// * `before` - The book's files before, as `read_tree` reads them
/// * `after` - The book's files after
fn kept_as_written(before: &[(String, String)], after: &[(String, String)]) -> anyhow::Result<()> {
    // the same files
    let paths = |tree: &[(String, String)]| -> Vec<String> {
        tree.iter().map(|(path, _)| path.clone()).collect()
    };
    assert_eq!(paths(before), paths(after));
    for ((path, was), (_, now)) in before.iter().zip(after) {
        // each read as a node file, if it is one
        let was_node =
            parse_node_file(was).map_err(|error| anyhow::anyhow!("{path}: {error:?}"))?;
        let now_node =
            parse_node_file(now).map_err(|error| anyhow::anyhow!("{path}: {error:?}"))?;
        match (was_node, now_node) {
            // a node file's nodes the same, however its frontmatter is styled, and its body
            (Some((was_spec, was_fm)), Some((now_spec, now_fm))) => {
                assert_eq!(
                    reef_api::lift(was_spec),
                    reef_api::lift(now_spec),
                    "{path}'s nodes differ"
                );
                assert_eq!(was_fm.body, now_fm.body, "{path}'s body differs");
            }
            // and any other file as it was
            _ => assert_eq!(was, now, "{path} differs"),
        }
    }
    Ok(())
}

/// Checks that a book imported, then exported over itself, holds what it did, draws as it
/// did, is written as it is by a second export, and imports again as the same plan, returning
/// what the first export printed
///
/// # Arguments
///
/// * `cx` - The case, whose project's repo holds the book
/// * `book` - The book's directory
/// * `name` - The book's directory, as the command line names it from the repo's root
async fn written_back(cx: &Cx, book: &Path, name: &str) -> anyhow::Result<String> {
    let repo = &cx.ax.repo;
    let before = read_tree(book)?;
    // imported, then exported over the book it came from
    cx.reef(repo, &["import", name]).await?;
    let out = cx.reef(repo, &["export", name, "--replace"]).await?;
    let after = read_tree(book)?;
    kept_as_written(&before, &after)?;
    draws_as_the_book(cx, book, &format!("{name}/src")).await?;
    // a second export finds every file as it would write it
    let again = cx.reef(repo, &["export", name, "--replace"]).await?;
    assert!(again.contains(": 0 files written, "), "{again}");
    assert_eq!(read_tree(book)?, after);
    // and imported again, it is the same plan, each node in its place
    cx.reef(repo, &["import", name, "--plan", "again"]).await?;
    same_plans(cx, &cx.ax.plan, &PlanName::new("reef", "again")?, true).await?;
    Ok(out)
}

/// This plan, imported and exported over the book it came from, keeps every file at its path,
/// each node file's body, its pages, its summary, its `book.toml` and its other files as they
/// were, and a second export writes nothing
#[tokio::test]
async fn export_replace_writes_this_plan_back_as_it_was() -> anyhow::Result<()> {
    let cx = Cx::new("replace-this-plan").await?;
    let book = this_plan(&cx.ax.repo)?;
    // the viewer and a picture, which no export writes
    write_files(
        &book,
        &[
            ("reef.css", "/* the viewer */\n"),
            ("src/x.svg", "<svg/>\n"),
        ],
    )?;
    let out = written_back(&cx, &book, "plan").await?;
    assert!(
        out.contains(" pages of `reef/central` over the book in plan: ")
            && out.ends_with(" unchanged, 0 removed\n"),
        "{out}"
    );
    Ok(())
}

/// The example book, its node files deep under its `src` and unlisted in its summary, is
/// written back over itself as it was
#[tokio::test]
async fn export_replace_keeps_the_example_books_layout() -> anyhow::Result<()> {
    let cx = Cx::new("replace-example").await?;
    let book = cx.ax.repo.join("example-book");
    example_book(&book)?;
    let out = written_back(&cx, &book, "example-book").await?;
    assert!(
        out.starts_with(
            "exported 202 nodes and 100 pages of `reef/central` over the book in example-book: "
        ),
        "{out}"
    );
    Ok(())
}

/// Every link a node or a page writes leads to the same file and heading once the plan is
/// written back over its book: a heading of its own file, another node's file, a page, a
/// reference, a picture and the repo outside the book, and a book with no summary gets one
#[tokio::test]
async fn export_replace_rebases_links_from_each_file() -> anyhow::Result<()> {
    let cx = Cx::new("replace-links").await?;
    let repo = cx.ax.repo.clone();
    write_files(&repo, &LINKED_BOOK)?;
    let book = repo.join("book");
    let before = read_tree(&book)?;
    cx.reef(&repo, &["import", "book"]).await?;
    let out = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(
        out.ends_with(", 0 removed\n  added src/SUMMARY.md\n"),
        "{out}"
    );
    // a summary written as a new book's is, listing the node files, then the pages
    let mut after = read_tree(&book)?;
    let at = after
        .iter()
        .position(|(path, _)| path == "src/SUMMARY.md")
        .expect("a summary is written");
    let (_, summary) = after.remove(at);
    assert_eq!(
        summary,
        "# Summary\n\n- [a Some work](plan/a.md)\n- [g The group](plan/g.md)\n\n# Pages\n\n\
         - [Design](design.md)\n- [Notes](plan/notes.md)\n"
    );
    // and every other file as it was, each link as written
    kept_as_written(&before, &after)?;
    draws_as_the_book(&cx, &book, "book/src").await?;
    let again = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(
        again.contains(": 0 files written, 5 unchanged, 0 removed\n"),
        "{again}"
    );
    Ok(())
}

/// A book a plan is written back over: a root, two groups in files of their own, each with a
/// member, a page, a page no node links, a picture, and a summary in parts
const KEPT_BOOK: [(&str, &str); 8] = [
    (
        "book/book.toml",
        "[book]\nsrc = \"src\"\n\n[preprocessor.reef]\n",
    ),
    (
        "book/src/SUMMARY.md",
        "# Summary\n\n[Root](root.md)\n\n# Groups\n\n- [G The group](groups/g.md)\n\
         - [H Another](groups/h.md)\n\n# Design\n\n- [Design](design.md)\n",
    ),
    (
        "book/src/root.md",
        "---\nid: root\ntitle: Root\ndescription: { why: W, what: X }\n---\n# Root\n",
    ),
    (
        "book/src/groups/g.md",
        "---\nid: G\ntitle: G The group\ngroup: root\nnodes:\n  - id: g1\n    title: g1 First\n    \
         description: { why: W, what: X }\n---\n# G\n\nSee [the design](../design.md#rows).\n",
    ),
    (
        "book/src/groups/h.md",
        "---\nid: H\ntitle: H Another\ngroup: root\nnodes:\n  - id: h1\n    title: h1 Only\n    \
         description: { why: W, what: X }\n---\n# H\n",
    ),
    (
        "book/src/design.md",
        "# Design\n\n## Rows\n\nRows, as [G](groups/g.md) says.\n",
    ),
    ("book/src/old.md", "# Old notes\n"),
    ("book/src/picture.svg", "<svg/>\n"),
];

/// Nodes and pages put and removed through the terminal land in the book written back over:
/// a member in its group's file, a top-level node in a file of its own, a group's file and a
/// page removed, a new page listed after the last, and a group retitled in the summary. A file
/// moved by hand stays where it was put, and the book imports as the plan, each node in the
/// book's order
#[tokio::test]
async fn export_replace_places_new_and_removed_nodes() -> anyhow::Result<()> {
    let cx = Cx::new("replace-places").await?;
    let repo = cx.ax.repo.clone();
    write_files(&repo, &KEPT_BOOK)?;
    let book = repo.join("book");
    cx.reef(&repo, &["import", "book"]).await?;
    // a member of G and a node of its own put, H and its member removed, a page swapped
    cx.put(
        "g2.yml",
        "id: g2\ntitle: g2 Second\ngroup: G\ndocs: [design.md#rows]\ndescription: { why: W, what: X }\n",
    )
    .await?;
    cx.put(
        "t.yml",
        "id: T\ntitle: T Top\ndocs: [design.md]\ndescription: { why: W, what: X }\n",
    )
    .await?;
    cx.reef(&repo, &["node", "rm", "H", "h1"]).await?;
    cx.reef(&repo, &["page", "rm", "old.md"]).await?;
    write_files(&repo, &[("scratch/new.md", "# New notes\n")])?;
    cx.reef(&repo, &["page", "put", "notes/new.md", "scratch/new.md"])
        .await?;
    // and G retitled
    let got = cx.reef(&repo, &["node", "get", "G"]).await?;
    cx.put(
        "g.yml",
        &got.replace("title: G The group", "title: G Renamed"),
    )
    .await?;
    let before = read_tree(&book)?;
    // written back over the book, each change where it belongs
    let out = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(
        out.starts_with("exported 5 nodes and 2 pages of `reef/central` over the book in book: ")
            && out.ends_with(
                ", 2 removed\n  added src/notes/new.md\n  added src/t.md\n  removed src/groups/h.md\n  \
                 removed src/old.md\n"
            ),
        "{out}"
    );
    let summary = std::fs::read_to_string(book.join("src/SUMMARY.md"))?;
    assert_eq!(
        summary,
        "# Summary\n\n[Root](root.md)\n\n# Groups\n\n- [G Renamed](groups/g.md)\n- [T Top](t.md)\n\n\
         # Design\n\n- [Design](design.md)\n- [New notes](notes/new.md)\n"
    );
    // the new member listed in its group's file, after the one before it, its link from there
    let g = std::fs::read_to_string(book.join("src/groups/g.md"))?;
    let (spec, fm) = parse_node_file(&g)
        .map_err(|error| anyhow::anyhow!("{error:?}"))?
        .expect("G's file holds G");
    assert_eq!(spec.title.as_deref(), Some("G Renamed"));
    let members: Vec<&str> = spec.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(members, ["g1", "g2"]);
    assert_eq!(spec.nodes[1].docs, ["../design.md#rows"]);
    assert_eq!(fm.body, "# G\n\nSee [the design](../design.md#rows).\n");
    // the files gone, and the book's others left alone
    assert!(!book.join("src/groups/h.md").exists() && !book.join("src/old.md").exists());
    for (path, text) in &before {
        if ["book.toml", "src/picture.svg", "src/design.md"].contains(&path.as_str()) {
            assert_eq!(
                &std::fs::read_to_string(book.join(path))?,
                text,
                "{path} changed"
            );
        }
    }
    // a file moved by hand stays where it was put, its links leading from there
    std::fs::create_dir_all(book.join("src/groups"))?;
    std::fs::rename(book.join("src/t.md"), book.join("src/groups/t.md"))?;
    std::fs::write(
        book.join("src/SUMMARY.md"),
        summary.replace("(t.md)", "(groups/t.md)"),
    )?;
    let out = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(
        out.contains(": 1 file written, 5 unchanged, 0 removed\n"),
        "{out}"
    );
    let t = std::fs::read_to_string(book.join("src/groups/t.md"))?;
    assert!(t.contains("docs:\n- ../design.md\n"), "{t}");
    let again = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(again.contains(": 0 files written, "), "{again}");
    // imported again, the plan comes back, each node in the order the book's files sort in
    cx.reef(&repo, &["import", "book", "--plan", "again"])
        .await?;
    let again = PlanName::new("reef", "again")?;
    same_plans(&cx, &cx.ax.plan, &again, false).await?;
    let ids: Vec<String> = cx
        .ax
        .service
        .list(&again)
        .await?
        .nodes
        .into_iter()
        .map(|node| node.spec.id)
        .collect();
    assert_eq!(ids, ["G", "g1", "g2", "T", "root"]);
    // which the plan takes on when the book is imported over it
    cx.reef(&repo, &["import", "book", "--plan", "central", "--replace"])
        .await?;
    draws_as_the_book(&cx, &book, "book/src").await
}

/// Runs an export over a directory that is to be refused, checking that it leaves the
/// directory as it was, returning the error
///
/// # Arguments
///
/// * `cx` - The case, whose project's repo the directory is named from
/// * `dir` - The directory, as the command line names it from the repo's root
async fn refused_over(cx: &Cx, dir: &str) -> anyhow::Result<String> {
    let path = cx.ax.repo.join(dir);
    let before = read_tree(&path)?;
    let (_, error) = cx
        .failing(
            &cx.ax.repo,
            &["export", dir, "--replace", "--project", "reef"],
        )
        .await?;
    assert_eq!(read_tree(&path)?, before, "{dir} changed");
    Ok(error)
}

/// An export over a directory refuses one that holds no reef book, one briefed from sections
/// of its own, one outside the project's repo, one whose files aren't all nodes or pages, a
/// page at a node file's path, and a file or directory in the way, leaving each as it was
#[tokio::test]
async fn export_replace_refuses() -> anyhow::Result<()> {
    let cx = Cx::new("replace-refuses").await?;
    let repo = cx.ax.repo.clone();
    cx.put("a.yml", &work("A", &[])).await?;
    write_files(
        &repo,
        &[
            // no book, and a book with no reef plan
            ("notes/a.md", "# A\n"),
            ("plain/book.toml", "[book]\ntitle = \"x\"\n"),
            ("plain/src/a.md", "# A\n"),
            // a book briefed from sections of its own
            (
                "own/book.toml",
                "[preprocessor.reef]\nwhy-sections = [\"Why\"]\n",
            ),
            ("own/src/a.md", "# A\n"),
            // a file that isn't a node, and a node heading two files
            ("bad/book.toml", "[preprocessor.reef]\n"),
            ("bad/src/a.md", "---\nid: a\nstatus: sideways\n---\n"),
            ("twice/book.toml", "[preprocessor.reef]\n"),
            ("twice/src/a.md", "---\nid: A\n---\n"),
            ("twice/src/b.md", "---\nid: A\n---\n"),
        ],
    )?;
    write_files(
        &cx.ax.dir,
        &[
            ("outside/book.toml", "[preprocessor.reef]\n"),
            ("outside/src/a.md", "# A\n"),
        ],
    )?;
    assert_eq!(
        refused_over(&cx, "notes").await?,
        "notes holds no `book.toml`: `--replace` writes over a reef book, and an export without \
         it writes a new one"
    );
    let error = refused_over(&cx, "plain").await?;
    assert!(error.contains("has no `[preprocessor.reef]`"), "{error}");
    let error = refused_over(&cx, "own").await?;
    assert!(error.contains("names its own doc sections"), "{error}");
    let error = refused_over(&cx, "../outside").await?;
    assert!(error.contains("is not in project `reef`'s repo"), "{error}");
    let error = refused_over(&cx, "bad").await?;
    assert!(
        error.starts_with("the book's files can't all be read as nodes"),
        "{error}"
    );
    assert_eq!(
        refused_over(&cx, "twice").await?,
        "`A` heads both a.md and b.md in the book, so which file is its isn't known"
    );
    // a page at the path of a node's file in the book
    cx.reef(&repo, &["export", "kept"]).await?;
    write_files(&repo, &[("scratch/page.md", "# A page\n")])?;
    cx.reef(&repo, &["page", "put", "001-a.md", "scratch/page.md"])
        .await?;
    assert_eq!(
        refused_over(&cx, "kept").await?,
        "page `001-a.md` has the path of node `A`'s file in the book: move the page, or the \
         node's file"
    );
    cx.reef(&repo, &["page", "rm", "001-a.md"]).await?;
    // a directory where a page goes, and a file where its directory goes
    cx.reef(&repo, &["page", "put", "notes/new.md", "scratch/page.md"])
        .await?;
    write_files(&repo, &[("kept/src/notes/new.md/held.txt", "held\n")])?;
    assert_eq!(
        refused_over(&cx, "kept").await?,
        "notes/new.md is a directory in the book's src, where the export would write a file"
    );
    std::fs::remove_dir_all(repo.join("kept/src/notes"))?;
    write_files(&repo, &[("kept/src/notes", "a file\n")])?;
    assert_eq!(
        refused_over(&cx, "kept").await?,
        "notes is a file in the book's src, where the export would make a directory for \
         notes/new.md"
    );
    // and once out of the way, the plan is written over the book
    std::fs::remove_file(repo.join("kept/src/notes"))?;
    let out = cx.reef(&repo, &["export", "kept", "--replace"]).await?;
    assert!(
        out.starts_with("exported 1 node and 1 page of `reef/central` over the book in kept: ")
            && out.ends_with("  added src/notes/new.md\n"),
        "{out}"
    );
    Ok(())
}
