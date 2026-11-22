//! The CLI against a served service: each command run in-process, as the terminal runs it
//!
//! Each case serves the in-memory twin on a loopback port of its own, names that port in a
//! config of the case's own, and runs `reef`'s commands from the working directory it picks. The
//! JSON API's suite already runs every route against both backends, so these run on the twin.

// with `embedded`, the shared fixtures' pool open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef_model::Payload;
use reef_render::ROOT;
use reef_render::book::Plan;
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
            .expect_err("the command succeeded");
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

/// Text with each link a book's payload writes from its root made to lead where the service's
/// payload links lead: under its plan's docs, from the repo's root
///
/// # Arguments
///
/// * `text` - The text the links are in
/// * `docs` - Where the service serves its plan's docs
/// * `prefix` - The book's `src`, from the repo's root
fn served(text: &str, docs: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(ROOT) {
        // the text before the link, then the link's path from the repo's root
        out.push_str(&rest[..at]);
        let after = &rest[at + ROOT.len()..];
        let end = after
            .find(['"', '\'', '#', '<', '>', ' ', '\n', ')'])
            .unwrap_or(after.len());
        out.push_str(docs);
        out.push_str(&paths::join(prefix, &after[..end]));
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
/// * `book` - Where the service serves its plan's docs, and the book's `src` from the repo's
///   root, for a book's payload
fn comparable(payload: Payload, book: Option<(&str, &str)>) -> anyhow::Result<Value> {
    /// Each string in a value with its book links served, as `served` makes them
    ///
    /// # Arguments
    ///
    /// * `value` - The value to change
    /// * `docs` - Where the service serves its plan's docs
    /// * `prefix` - The book's `src`, from the repo's root
    fn serve_all(value: &mut Value, docs: &str, prefix: &str) {
        match value {
            Value::String(text) => *text = served(text, docs, prefix),
            Value::Array(items) => items
                .iter_mut()
                .for_each(|item| serve_all(item, docs, prefix)),
            Value::Object(map) => map
                .values_mut()
                .for_each(|item| serve_all(item, docs, prefix)),
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
    if let Some((docs, prefix)) = book {
        serve_all(&mut json, docs, prefix);
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
    let from_book = comparable(plan.payload(&HashSet::new()), Some((&docs, prefix)))?;
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
        out.starts_with("imported 202 nodes from example-book into `reef/central` at revision "),
        "{out}"
    );
    // its nodes check as the book's do, and draw as the book's do
    let checked = cx.reef(&cx.ax.repo, &["check"]).await?;
    assert_eq!(checked, "`reef/central` has no problems\n");
    draws_as_the_book(&cx, &book, "example-book/src").await
}

/// A synthetic book whose nodes link every way a node can, from its body, its description, its
/// design choices, its measurements' note and its criteria, inline, by reference, to a heading
/// of its own file and out of the book into the repo
const LINKED_BOOK: [(&str, &str); 6] = [
    ("README.md", "# The repo\n\n## Setup\n\nSet it up.\n"),
    (
        "book/book.toml",
        "[book]\nsrc = \"src\"\n\n[preprocessor.reef]\n",
    ),
    (
        "book/src/design.md",
        "# Design\n\n## Rows\n\nEach node a row.\n",
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
        out.starts_with("imported 3 nodes from . into `reef/central`"),
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
    // its stored links are written from the repo's root
    assert_eq!(
        m.spec.description.as_ref().and_then(|d| d.why.as_deref()),
        Some("Without [rows](book/src/design.md#rows), slow.")
    );
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
    // and none of them wrote anything
    assert_eq!(cx.ax.service.list(&cx.ax.plan).await?.revision, revision);
    Ok(())
}
