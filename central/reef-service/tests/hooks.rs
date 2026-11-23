//! The capture hooks against a served service, git's run by real git and Claude Code's handed
//! the input it hands them
//!
//! Each case serves the in-memory twin on a loopback port of its own, as the CLI's suite does,
//! and installs git's post-commit and post-rewrite hooks in its repo's common hooks dir, which
//! every worktree of the repo runs. A hook execs this test binary, naming [`reef_shim`] alone,
//! which runs `reef hook …` in-process with the case's config and logs what it printed, so the
//! hooks git runs are reef's own code. Claude Code's PostToolUse and Stop are called in-process
//! with the JSON Claude Code hands them. git runs off the runtime's threads, since a hook it
//! runs calls back into the service this process serves.

// with `embedded`, the shared fixtures' pool open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef_api::{FileDiff, Revision};
use reef_model::PlanName;
use reef_service::MemoryBackend;
use std::path::{Path, PathBuf};

use common::git;
use common::http::Ax;

#[path = "../../../crates/reef/tests/support/oracle.rs"]
mod oracle;

/// The variable naming the command line a hook runs this binary as `reef` with
const SHIM_ARGS: &str = "REEF_SHIM_ARGS";

/// The variable naming the file a hook logs each run to
const SHIM_LOG: &str = "REEF_SHIM_LOG";

/// Not a case: what the git hooks these cases install run, `reef` in this binary's place
///
/// A hook execs this binary naming this test alone, with the command line to run `reef` with in
/// [`SHIM_ARGS`], the case's config in `REEF_CONFIG` and its log in [`SHIM_LOG`]. It runs the
/// command from the hook's working directory, logs the command and what it printed, and exits
/// as `reef` would. Run by the suite, with no command line, it does nothing.
#[test]
fn reef_shim() {
    let Ok(args) = std::env::var(SHIM_ARGS) else {
        return;
    };
    // the command as `reef` runs it, on a runtime of its own
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime builds");
    let line = std::iter::once("reef").chain(args.split_whitespace());
    let mut out = Vec::new();
    let ran = runtime.block_on(async {
        let cli = reef::Cli::from_args(line)?;
        let cwd = std::env::current_dir()?;
        reef::run(cli, &cwd, &mut out).await
    });
    let printed = String::from_utf8_lossy(&out).into_owned();
    print!("{printed}");
    // each run logged as its command, then what it printed or how it failed
    let said = match &ran {
        Ok(()) => printed.replace('\n', "\\n"),
        Err(error) => format!("failed: {error:#}"),
    };
    if let Ok(log) = std::env::var(SHIM_LOG) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .expect("the shim's log opens");
        writeln!(file, "{args}\t{said}").expect("the shim's log is written");
    }
    std::process::exit(match ran {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("reef: {error:#}");
            1
        }
    });
}

/// A served twin, a config naming its address, and a repo with reef's git hooks installed
struct Hx {
    /// The served service, and the case's own directory
    ax: Ax<MemoryBackend>,
    /// The config file naming the server's address
    conf: PathBuf,
    /// The repo of project `cli`, whose plan `docs` holds the case's nodes
    repo: PathBuf,
    /// The file the hooks log each run to
    log: PathBuf,
}

impl Hx {
    /// Serves a fresh twin, registers project `cli` for a fresh repo with plan `docs` holding
    /// the nodes named, and installs reef's git hooks in the repo
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its directory
    /// * `ids` - The ids of the plan's nodes, each a piece of work
    async fn new(case: &str, ids: &[&str]) -> anyhow::Result<Hx> {
        // a twin served on a port of its own, and the config the CLI reads its address from
        let ax = common::Fx::memory(&format!("hooks-{case}"))
            .await?
            .api()
            .await?;
        let conf = ax.dir.join("reef.toml");
        std::fs::write(&conf, format!("[api]\naddress = \"{}\"\n", ax.address))?;
        let repo = git::repo(&ax.dir.join("cli"))?;
        let log = ax.dir.join("hooks.log");
        let hx = Hx {
            ax,
            conf,
            repo,
            log,
        };
        // the project, its plan and its nodes
        hx.reef(&hx.repo, &["project", "add", "cli"]).await?;
        hx.reef(&hx.repo, &["plan", "new", "docs"]).await?;
        for id in ids {
            let file = hx.ax.dir.join(format!("{id}.yml"));
            std::fs::write(&file, work(id))?;
            let file = file.to_str().expect("the case's dir is utf-8");
            hx.reef(&hx.repo, &["node", "put", file]).await?;
        }
        // and reef's git hooks where every worktree of the repo runs them
        hx.install()?;
        Ok(hx)
    }

    /// Installs post-commit and post-rewrite in the repo's common hooks dir, each exec'ing this
    /// binary as `reef hook …`
    fn install(&self) -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        // the common dir, which every linked worktree runs its hooks from
        let common = git::run(
            &self.repo,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        let hooks = Path::new(&common).join("hooks");
        std::fs::create_dir_all(&hooks)?;
        let exe = std::env::current_exe()?;
        for (hook, args) in [
            ("post-commit", "hook post-commit"),
            ("post-rewrite", "hook post-rewrite $1"),
        ] {
            let script = format!(
                "#!/bin/sh\nREEF_CONFIG='{}' {SHIM_LOG}='{}' {SHIM_ARGS}=\"{args}\" exec '{}' \
                 reef_shim --exact --nocapture --quiet\n",
                self.conf.display(),
                self.log.display(),
                exe.display()
            );
            let path = hooks.join(hook);
            std::fs::write(&path, script)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
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
        let mut out = Vec::new();
        reef::run(cli, cwd, &mut out).await?;
        Ok(String::from_utf8(out)?)
    }

    /// A client of the case's service, as the CLI makes one
    fn client(&self) -> anyhow::Result<reef::Client> {
        let conf = reef::Conf::load(Some(&self.conf))?;
        reef::Client::new(&conf)
    }

    /// Calls Claude Code's PostToolUse hook as it is called after reef's status tool sets a
    /// node's status, returning what it printed
    ///
    /// # Arguments
    ///
    /// * `cwd` - The session's working directory
    /// * `id` - The node set
    /// * `status` - The status it was set to
    async fn status_used(&self, cwd: &Path, id: &str, status: &str) -> anyhow::Result<String> {
        // the input Claude Code hands it, as X3's runs saw it
        let input = serde_json::json!({
            "session_id": "s1",
            "transcript_path": "/dev/null",
            "cwd": cwd,
            "hook_event_name": "PostToolUse",
            "tool_name": "mcp__reef__status",
            "tool_input": { "id": id, "status": status },
            "tool_response": [{ "type": "text", "text": format!("`{id}` is {status}\n") }],
        });
        let mut out = Vec::new();
        reef::hook::post_tool_use(&self.client()?, &self.repo, &input.to_string(), &mut out)
            .await?;
        Ok(String::from_utf8(out)?)
    }

    /// Calls Claude Code's Stop hook as it is called when a turn ends in a session with no pane,
    /// returning what it printed
    ///
    /// # Arguments
    ///
    /// * `cwd` - The session's working directory
    async fn stopped(&self, cwd: &Path) -> anyhow::Result<String> {
        self.stopped_in(cwd, None, false).await
    }

    /// Calls Claude Code's Stop hook as it is called when a turn ends in a pane, returning what
    /// it printed
    ///
    /// # Arguments
    ///
    /// * `cwd` - The session's working directory
    /// * `pane` - The pane's key, if the session runs in one
    /// * `active` - Whether Claude Code says a Stop hook kept this turn going
    async fn stopped_in(
        &self,
        cwd: &Path,
        pane: Option<&str>,
        active: bool,
    ) -> anyhow::Result<String> {
        // no Claude Code process named, so no injector is ever started
        let (said, spawned) = self.stopped_by(cwd, pane, active, None, false).await?;
        assert!(spawned.is_empty(), "{spawned:?}");
        Ok(said)
    }

    /// Calls Claude Code's Stop hook as it is called when a turn ends in a pane, the Claude
    /// Code process named or not, returning what it printed and each injector it would have
    /// started, which is recorded rather than started
    ///
    /// # Arguments
    ///
    /// * `cwd` - The session's working directory
    /// * `pane` - The pane's key, if the session runs in one
    /// * `active` - Whether Claude Code says a Stop hook kept this turn going
    /// * `pid` - The Claude Code process, `CLAUDE_PID`, if Claude Code named it
    /// * `fails` - Whether starting the injector fails
    async fn stopped_by(
        &self,
        cwd: &Path,
        pane: Option<&str>,
        active: bool,
        pid: Option<u32>,
        fails: bool,
    ) -> anyhow::Result<(String, Vec<reef::clear::Clear>)> {
        // the input Claude Code hands it
        let input = serde_json::json!({
            "session_id": "s1",
            "transcript_path": "/dev/null",
            "cwd": cwd,
            "hook_event_name": "Stop",
            "stop_hook_active": active,
        });
        let pane: Option<reef_api::Pane> = pane.map(str::parse).transpose()?;
        // each injector it starts recorded, never run, so no pane is typed into
        let mut spawned = Vec::new();
        let spawn = |order: &reef::clear::Clear| {
            spawned.push(order.clone());
            match fails {
                true => anyhow::bail!("no such program"),
                false => Ok(()),
            }
        };
        let mut out = Vec::new();
        reef::hook::stop(
            &self.client()?,
            &self.repo,
            &input.to_string(),
            pane.as_ref(),
            pid,
            spawn,
            &mut out,
        )
        .await?;
        Ok((String::from_utf8(out)?, spawned))
    }

    /// Calls Claude Code's SessionStart hook as it is called when a clear starts a new session
    /// in a pane, returning what it printed
    ///
    /// # Arguments
    ///
    /// * `pane` - The pane's key
    /// * `session` - The new session's id
    async fn cleared_in(&self, pane: &str, session: &str) -> anyhow::Result<String> {
        // the input Claude Code hands it after a clear
        let input = serde_json::json!({
            "session_id": session,
            "transcript_path": "/dev/null",
            "cwd": self.repo,
            "hook_event_name": "SessionStart",
            "source": "clear",
        });
        let mut out = Vec::new();
        reef::loops::session_start(
            &self.client()?,
            &input.to_string(),
            &pane.parse()?,
            &mut out,
        )
        .await?;
        Ok(String::from_utf8(out)?)
    }

    /// A node's revisions, each with its files fetched
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id in `cli/docs`
    async fn changes(&self, id: &str) -> anyhow::Result<Vec<(Revision, Vec<FileDiff>)>> {
        let client = self.client()?;
        let node = PlanName::new("cli", "docs")?.node(id)?;
        let mut changes = Vec::new();
        for revision in client.revisions(&node).await? {
            let mut files = Vec::with_capacity(revision.files.len());
            for file in &revision.files {
                let read = client
                    .revision_file(&node, revision.revision, file.path())
                    .await?;
                files.push(read.expect("a file its revision lists reads"));
            }
            changes.push((revision, files));
        }
        Ok(changes)
    }

    /// How many captures a node's record has added since its start
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id in `cli/docs`
    async fn captures(&self, id: &str) -> anyhow::Result<Vec<reef_api::CaptureRead>> {
        let node = PlanName::new("cli", "docs")?.node(id)?;
        let record = self.client()?.work_of(&node).await?;
        Ok(record.expect("the node was started").captures)
    }

    /// Each run of a git hook logged so far: its command line and what it printed
    fn logged(&self) -> anyhow::Result<Vec<(String, String)>> {
        // a log no hook wrote is empty
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        Ok(text
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(args, said)| (args.to_string(), said.replace("\\n", "\n")))
            .collect())
    }
}

/// A piece of work's YAML that says why and what
///
/// # Arguments
///
/// * `id` - The node's id
fn work(id: &str) -> String {
    format!(
        "id: {id}\ntitle: {id} Some work\ndescription:\n  why: Without {id}, something is \
         missing.\n  what: With {id}, it is there.\n"
    )
}

/// Runs git off the runtime's threads, since a hook it runs calls back into the service this
/// process serves, returning its output trimmed
///
/// # Arguments
///
/// * `dir` - The directory to run in
/// * `args` - Git's arguments
async fn hooked(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let dir = dir.to_path_buf();
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        git::run(&dir, &args)
    })
    .await?
}

/// Writes a file and commits it off the runtime's threads, returning the commit
///
/// # Arguments
///
/// * `dir` - The worktree
/// * `path` - The file's path in it
/// * `text` - What it holds
/// * `message` - The commit's message
async fn committed(dir: &Path, path: &str, text: &str, message: &str) -> anyhow::Result<String> {
    // write it, then stage and commit it, git's hooks and all
    std::fs::write(dir.join(path), text)?;
    hooked(dir, &["add", path]).await?;
    hooked(dir, &["commit", "-q", "-m", message]).await?;
    hooked(dir, &["rev-parse", "HEAD"]).await
}

/// A node Claude marks done has a captured revision before the next node starts: each commit
/// keeps its draft current, the mark drafts what is uncommitted, Stop seals it once and only
/// once it is done, and the next node starts after it
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_node_marked_done_is_captured_before_the_next_starts() -> anyhow::Result<()> {
    let hx = Hx::new("done", &["A", "B"]).await?;
    // a commit on main, before any node starts, captures nothing
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    assert_eq!(
        hx.logged()?,
        [("hook post-commit".to_string(), String::new())]
    );
    // A worked in a linked worktree, which runs the repo's common hooks
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    hx.reef(&a, &["start", "A"]).await?;
    committed(&a, "a.txt", "A's first\n", "A one").await?;
    let changes = hx.changes("A").await?;
    let [(r1, _)] = &changes[..] else {
        panic!("A's commit kept {} revisions", changes.len());
    };
    assert_eq!((r1.revision, r1.draft), (1, true));
    let (_, said) = hx.logged()?.pop().expect("A's commit ran post-commit");
    assert!(
        said.starts_with("reef: drafted `A` as revision 1, 1 file +1 -0\n"),
        "{said}"
    );
    // a turn that ends while A is in progress seals nothing
    assert_eq!(hx.stopped(&a).await?, "");
    assert!(hx.changes("A").await?[0].0.draft);
    // more of A's work, left uncommitted, then A marked done through reef's status tool
    std::fs::write(a.join("a.txt"), "A's first\nA's second\n")?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    let said = hx.status_used(&a, "A", "done").await?;
    assert_eq!(
        said,
        "reef: updated `A`'s draft, revision 1, 1 file +2 -0\n"
    );
    // the plan's export committed after the mark, which only Stop comes after
    std::fs::write(a.join("plan.md"), "A is done\n")?;
    hooked(&a, &["add", "-A"]).await?;
    hooked(&a, &["commit", "-q", "-m", "A done"]).await?;
    let (_, said) = hx.logged()?.pop().expect("the export ran post-commit");
    assert!(
        said.starts_with("reef: updated `A`'s draft, revision 1, 2 files"),
        "{said}"
    );
    // the turn's end seals it as it now is, git's diff of its base and tree
    let said = hx.stopped(&a).await?;
    assert_eq!(said, "reef: sealed `A` as revision 1, 2 files +3 -0\n");
    let sealed = hx.changes("A").await?;
    let [(revision, files)] = &sealed[..] else {
        panic!("A's seal left {} revisions", sealed.len());
    };
    assert!(!revision.draft);
    assert_eq!(
        revision.shas.head,
        hooked(&a, &["rev-parse", "HEAD"]).await?
    );
    let expected = oracle::git_diff(&a, &revision.shas.base, &revision.shas.tree);
    assert_eq!(files, &expected);
    // the next turn's end seals nothing again
    assert_eq!(hx.stopped(&a).await?, "");
    assert_eq!(hx.changes("A").await?, sealed);
    // and B starts there after it, with A sealed and B's commits B's own
    let out = hx.reef(&a, &["start", "B"]).await?;
    assert!(!out.contains("sealed"), "{out}");
    committed(&a, "b.txt", "B's\n", "B one").await?;
    assert_eq!(hx.changes("A").await?, sealed);
    assert_eq!(hx.changes("B").await?.len(), 1);
    // a status tool that marked another node, or set no node done, drafts nothing
    assert_eq!(hx.status_used(&a, "A", "done").await?, "");
    assert_eq!(hx.status_used(&a, "B", "blocked").await?, "");
    Ok(())
}

/// A node done that no Stop sealed is sealed by the start of the next node in its worktree,
/// as the worktree stands when the next node starts from it
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_seals_the_done_node_it_moves_on_from() -> anyhow::Result<()> {
    let hx = Hx::new("start-seals", &["A", "B", "C"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    // A drafted by its commit, marked done, and its last edit left uncommitted
    hx.reef(&a, &["start", "A"]).await?;
    committed(&a, "a.txt", "A's\n", "A one").await?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    std::fs::write(a.join("a.txt"), "A's\nA's last\n")?;
    // B starts in the same turn, so no Stop came between: the start seals A first
    let out = hx.reef(&a, &["start", "B"]).await?;
    assert!(
        out.starts_with("sealed `A` as revision 1, 1 file +2 -0\nstarted `B` in "),
        "{out}"
    );
    let changes = hx.changes("A").await?;
    let [(revision, _)] = &changes[..] else {
        panic!("A kept {} revisions", changes.len());
    };
    assert!(!revision.draft);
    // A's seal ends where B's start begins
    let b = hx
        .client()?
        .work_of(&PlanName::new("cli", "docs")?.node("B")?)
        .await?;
    assert_eq!(revision.shas.tree, b.expect("B started").start.start_tree);
    // a node not done is left as its draft when the next starts
    committed(&a, "b.txt", "B's\n", "B one").await?;
    let out = hx.reef(&a, &["start", "C"]).await?;
    assert!(!out.contains("sealed"), "{out}");
    assert!(hx.changes("B").await?[0].0.draft);
    Ok(())
}

/// A squash right after a node is done loses none of its changes: a local squash between the
/// mark and the seal leaves the diff Stop seals as it was, and once the node is squash-merged,
/// its branch, worktree and pin deleted and git pruned, every revision reads back as it was
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_squash_right_after_done_loses_nothing() -> anyhow::Result<()> {
    let hx = Hx::new("squash", &["A"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    // A worked in a worktree of its own, two commits of it, then marked done
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    hx.reef(&a, &["start", "A"]).await?;
    let start = hooked(&a, &["rev-parse", "HEAD"]).await?;
    let first = committed(&a, "a.txt", "A's first\n", "A one").await?;
    let second = committed(&a, "a.txt", "A's first\nA's second\n", "A two").await?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    hx.status_used(&a, "A", "done").await?;
    let drafted = hx.changes("A").await?;
    assert!(drafted[0].0.draft);
    // A's commits squashed locally before the turn ends, which leaves its tree as it was
    hooked(&a, &["reset", "-q", "--soft", &start]).await?;
    hooked(&a, &["commit", "-q", "-m", "A, squashed"]).await?;
    let squashed = hooked(&a, &["rev-parse", "HEAD"]).await?;
    assert_eq!(hx.changes("A").await?, drafted);
    // the turn's end seals the same files, naming the squashed head that holds them
    hx.stopped(&a).await?;
    let before = hx.changes("A").await?;
    let [(revision, files)] = &before[..] else {
        panic!("A's seal left {} revisions", before.len());
    };
    assert!(!revision.draft);
    assert_eq!(revision.shas.head, squashed);
    assert_eq!(files, &drafted[0].1);
    // A squash-merged into main, then its worktree, branch and pin deleted
    hooked(&hx.repo, &["merge", "--squash", "-q", "cli-a"]).await?;
    hooked(&hx.repo, &["commit", "-q", "-m", "A, merged"]).await?;
    let path = a.to_str().expect("utf-8");
    hooked(&hx.repo, &["worktree", "remove", "--force", path]).await?;
    hooked(&hx.repo, &["branch", "-q", "-D", "cli-a"]).await?;
    hooked(&hx.repo, &["update-ref", "-d", "refs/reef/start/docs/A"]).await?;
    // every reflog expired and git pruned, so the commits A's captures read are gone
    hooked(&hx.repo, &["reflog", "expire", "--expire=now", "--all"]).await?;
    hooked(&hx.repo, &["gc", "-q", "--prune=now"]).await?;
    for gone in [&first, &second, &squashed] {
        assert!(
            !git::succeeds(&hx.repo, &["cat-file", "-e", gone])?,
            "{gone} survived the prune"
        );
    }
    // and every revision and file reads back byte for byte
    let after = hx.changes("A").await?;
    assert_eq!(
        serde_json::to_string(&after)?,
        serde_json::to_string(&before)?
    );
    Ok(())
}

/// A rebase's picks capture nothing, and its end captures once: post-commit runs for each pick
/// and does nothing, post-rewrite drafts the rebased node once, holding only its own changes,
/// and an amend is drafted once, by post-commit
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rebase_captures_once_at_its_end() -> anyhow::Result<()> {
    let hx = Hx::new("rebase", &["A"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    // A's two commits in a worktree of its own, each drafted
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    hx.reef(&a, &["start", "A"]).await?;
    committed(&a, "a.txt", "A's first\n", "A one").await?;
    committed(&a, "b.txt", "A's second\n", "A two").await?;
    assert_eq!(hx.captures("A").await?.len(), 2);
    // main moves on meanwhile
    committed(&hx.repo, "main.txt", "main's\n", "main moves on").await?;
    let ran = hx.logged()?.len();
    // A rebased onto it, a pick at a time
    hooked(&a, &["rebase", "-q", "main"]).await?;
    let head = hooked(&a, &["rev-parse", "HEAD"]).await?;
    let runs = hx.logged()?.split_off(ran);
    let commits: Vec<&(String, String)> = runs
        .iter()
        .filter(|(args, _)| args == "hook post-commit")
        .collect();
    let rewrites: Vec<&(String, String)> = runs
        .iter()
        .filter(|(args, _)| args.starts_with("hook post-rewrite"))
        .collect();
    // each pick ran post-commit, which drafted nothing, and the end drafted once
    assert_eq!(commits.len(), 2, "{runs:?}");
    assert!(commits.iter().all(|(_, said)| said.is_empty()), "{runs:?}");
    let [(args, said)] = &rewrites[..] else {
        panic!(
            "the rebase ran post-rewrite {} times: {runs:?}",
            rewrites.len()
        );
    };
    assert_eq!(args, "hook post-rewrite rebase");
    assert!(
        said.starts_with("reef: updated `A`'s draft, revision 1, 2 files +2 -0\n"),
        "{said}"
    );
    let captures = hx.captures("A").await?;
    assert_eq!(captures.len(), 3);
    assert_eq!(captures[2].shas.head, head);
    // the draft holds A's own changes, carried over what main brought in
    let changes = hx.changes("A").await?;
    let [(revision, files)] = &changes[..] else {
        panic!("the rebase left {} revisions", changes.len());
    };
    assert!(revision.draft);
    let paths: Vec<&str> = files.iter().map(|file| file.path()).collect();
    assert_eq!(paths, ["a.txt", "b.txt"]);
    assert_eq!(
        revision.shas.merge_base,
        Some(hooked(&a, &["rev-parse", "main"]).await?)
    );
    // an amend runs post-commit and post-rewrite, and is drafted once, by post-commit
    let ran = hx.logged()?.len();
    std::fs::write(a.join("b.txt"), "A's second, amended\n")?;
    hooked(&a, &["commit", "-q", "-a", "--amend", "--no-edit"]).await?;
    let runs = hx.logged()?.split_off(ran);
    let names: Vec<&str> = runs.iter().map(|(args, _)| args.as_str()).collect();
    assert_eq!(names, ["hook post-commit", "hook post-rewrite amend"]);
    assert!(
        runs[0].1.starts_with("reef: updated `A`'s draft"),
        "{runs:?}"
    );
    assert_eq!(runs[1].1, "");
    assert_eq!(hx.captures("A").await?.len(), 4);
    Ok(())
}

/// What a Stop hook told the user, read out of the JSON it printed
///
/// # Arguments
///
/// * `said` - What the hook printed
fn told(said: &str) -> String {
    // one JSON object, whose `systemMessage` Claude Code shows the user
    let value: serde_json::Value = serde_json::from_str(said)
        .unwrap_or_else(|error| panic!("the Stop hook printed no JSON ({error}): {said}"));
    value["systemMessage"]
        .as_str()
        .unwrap_or_else(|| panic!("the Stop hook told the user nothing: {said}"))
        .to_string()
}

/// The Stop hook acts once on the end of each node a pane's loop holds, whatever
/// `stop_hook_active` says: nothing mid-node, then the next node named once it is done, the
/// loop's pause once a paused loop's node is done, and nothing else ready once the last is done,
/// each said once
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_loop_acts_once_on_each_nodes_end() -> anyhow::Result<()> {
    let hx = Hx::new("loop-ends", &["A", "B", "C"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    // a loop in a pane, which takes A into a worktree of its own
    let pane = "claude:4242";
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    hx.reef(&a, &["loop", "start", "--pane", pane]).await?;
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/A`"), "{took}");
    // a turn's end mid-node lets Claude stop and says nothing, chained or not
    committed(&a, "a.txt", "A's\n", "A one").await?;
    for active in [false, true] {
        assert_eq!(hx.stopped_in(&a, Some(pane), active).await?, "");
    }
    // once A is done, the turn's end seals it and names the next node, once
    hx.reef(&a, &["status", "A", "done"]).await?;
    let said = told(&hx.stopped_in(&a, Some(pane), true).await?);
    assert!(said.starts_with("reef: sealed `A` as revision 1"), "{said}");
    assert!(
        said.ends_with(
            "\nreef: `cli/docs/A` is done. `cli/docs/B` is next: `/clear`, then say `next`"
        ),
        "{said}"
    );
    let listed = hx.reef(&a, &["loop", "ls"]).await?;
    assert!(listed.contains("acted on `cli/docs/A`'s end"), "{listed}");
    for active in [true, false] {
        assert_eq!(hx.stopped_in(&a, Some(pane), active).await?, "");
    }
    // B taken, then the loop paused: B's end says the loop stops there, once
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/B`"), "{took}");
    hx.reef(&a, &["loop", "pause", "--pane", pane]).await?;
    assert_eq!(hx.stopped_in(&a, Some(pane), false).await?, "");
    hx.reef(&a, &["status", "B", "done"]).await?;
    let said = told(&hx.stopped_in(&a, Some(pane), true).await?);
    assert!(
        said.ends_with("reef: `cli/docs/B` is done, and the loop is paused: `! reef loop start` sets it running again"),
        "{said}"
    );
    assert!(!said.contains("is next"), "{said}");
    assert_eq!(hx.stopped_in(&a, Some(pane), true).await?, "");
    // running again, it takes C, and C's end finds nothing else ready, once
    hx.reef(&a, &["loop", "start", "--pane", pane]).await?;
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/C`"), "{took}");
    hx.reef(&a, &["status", "C", "done"]).await?;
    let said = told(&hx.stopped_in(&a, Some(pane), false).await?);
    assert!(
        said.ends_with("reef: `cli/docs/C` is done, and nothing else is ready to take: `reef next` says what everything waits on"),
        "{said}"
    );
    assert_eq!(hx.stopped_in(&a, Some(pane), false).await?, "");
    // and a pane with no loop, or no pane, leaves the turn's end to the seal alone
    assert_eq!(hx.stopped_in(&a, Some("claude:5151"), false).await?, "");
    assert_eq!(hx.stopped(&a).await?, "");
    Ok(())
}

/// In a zellij pane, a node's end with another next starts the injector once, handed the pane,
/// the session that stopped, its Claude Code process and the node, and tells the user reef
/// clears; the clear's new session is told which node is next, and a plan approval's mid-node
/// clear is told nothing. An injector that can't start leaves the clear to the user.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_loop_in_zellij_clears_between_nodes() -> anyhow::Result<()> {
    let hx = Hx::new("loop-clears", &["A", "B", "C"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    // a loop in a zellij pane, which takes A into a worktree of its own
    let pane = "zellij:work:3";
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    hx.reef(&a, &["loop", "start", "--pane", pane]).await?;
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/A`"), "{took}");
    // mid-node, nothing is cleared and a plan approval's clear is told nothing
    let (said, spawned) = hx
        .stopped_by(&a, Some(pane), false, Some(4242), false)
        .await?;
    assert_eq!((said.as_str(), spawned.len()), ("", 0));
    assert_eq!(hx.cleared_in(pane, "s2").await?, "");
    // once A is done, the turn's end starts the injector once and says reef clears
    hx.reef(&a, &["status", "A", "done"]).await?;
    let (said, spawned) = hx
        .stopped_by(&a, Some(pane), true, Some(4242), false)
        .await?;
    let said = told(&said);
    assert!(
        said.ends_with(
            "\nreef: `cli/docs/A` is done. `cli/docs/B` is next: reef clears the context and \
             asks for it once Claude is idle and the input box is empty"
        ),
        "{said}"
    );
    assert_eq!(
        spawned,
        vec![reef::clear::Clear {
            pane: pane.parse()?,
            session: "s1".to_string(),
            pid: 4242,
            after: "cli/docs/A".parse()?,
        }]
    );
    let (said, spawned) = hx
        .stopped_by(&a, Some(pane), true, Some(4242), false)
        .await?;
    assert_eq!((said.as_str(), spawned.len()), ("", 0));
    // the clear's new session is told B is next, and how to take it
    let context = hx.cleared_in(pane, "s3").await?;
    let value: serde_json::Value = serde_json::from_str(&context)?;
    assert_eq!(value["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let context = value["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default();
    assert!(
        context.starts_with(
            "reef's loop in this pane is between nodes: `cli/docs/A` is done, and `cli/docs/B` \
             is next."
        ),
        "{context}"
    );
    assert!(context.contains("`mcp__reef__next_task`"), "{context}");
    let listed = hx.reef(&a, &["loop", "ls"]).await?;
    assert!(listed.contains("session s3"), "{listed}");
    // B taken: its plan approval's clear is told nothing
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/B`"), "{took}");
    assert_eq!(hx.cleared_in(pane, "s4").await?, "");
    // an injector that can't start leaves the clear to the user, saying why
    hx.reef(&a, &["status", "B", "done"]).await?;
    let (said, spawned) = hx
        .stopped_by(&a, Some(pane), false, Some(4242), true)
        .await?;
    let said = told(&said);
    assert!(
        said.ends_with(
            "reef: `cli/docs/B` is done. `cli/docs/C` is next: `/clear`, then say `next` (reef \
             couldn't clear it itself: no such program)"
        ),
        "{said}"
    );
    assert_eq!(spawned.len(), 1);
    // and with no Claude Code process named, nothing is started at C's end
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/C`"), "{took}");
    hx.reef(&a, &["status", "C", "done"]).await?;
    let said = told(&hx.stopped_in(&a, Some(pane), false).await?);
    assert!(said.contains("nothing else is ready"), "{said}");
    Ok(())
}
