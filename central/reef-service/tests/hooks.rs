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

    /// Runs `reef loop start` as Claude Code runs it in a session's own pane, `! reef loop
    /// start`, returning what it printed and each injector it would have started, which is
    /// recorded rather than started
    ///
    /// # Arguments
    ///
    /// * `cwd` - The session's working directory
    /// * `pane` - The pane's key
    /// * `session` - The session running there, `CLAUDE_CODE_SESSION_ID`
    /// * `pid` - Its Claude Code process, `CLAUDE_PID`
    async fn started_by(
        &self,
        cwd: &Path,
        pane: &str,
        session: &str,
        pid: u32,
    ) -> anyhow::Result<(String, Vec<reef::clear::Clear>)> {
        // the pane, session and process Claude Code names to its `!` commands
        let here = reef::loops::Here {
            pane: pane.parse()?,
            session: Some(session.to_string()),
            pid: Some(pid),
        };
        // each injector it starts recorded, never run, so no pane is typed into
        let mut spawned = Vec::new();
        let spawn = |order: &reef::clear::Clear| {
            spawned.push(order.clone());
            Ok(())
        };
        let mut out = Vec::new();
        reef::loops::start(
            &self.client()?,
            cwd,
            &here,
            &reef::loops::Starts::default(),
            &reef::cli::PlanPick::default(),
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
            None,
            None,
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

    /// Starts a loop as `! reef loop start --width N` starts it in a session's own pane,
    /// returning what it printed
    ///
    /// # Arguments
    ///
    /// * `cwd` - The session's working directory
    /// * `pane` - The pane's key
    /// * `width` - How many nodes it works at once
    /// * `opens` - Whether reef opens its members' panes
    async fn started_wide(
        &self,
        cwd: &Path,
        pane: &str,
        width: u8,
        opens: bool,
    ) -> anyhow::Result<String> {
        // the pane, session and process Claude Code names to its `!` commands
        let here = reef::loops::Here {
            pane: pane.parse()?,
            session: Some("s1".to_string()),
            pid: Some(4242),
        };
        let starts = reef::loops::Starts {
            session: None,
            width: Some(width),
            opens: (!opens).then_some(false),
        };
        let mut out = Vec::new();
        let spawn = |_: &reef::clear::Clear| anyhow::bail!("a fresh start clears nothing");
        reef::loops::start(
            &self.client()?,
            cwd,
            &here,
            &starts,
            &reef::cli::PlanPick::default(),
            spawn,
            &mut out,
        )
        .await?;
        Ok(String::from_utf8(out)?)
    }

    /// Calls next_task as the MCP tool calls it in a pane, returning what it printed and each
    /// member's pane it would have opened, which is recorded rather than opened
    ///
    /// # Arguments
    ///
    /// * `cwd` - The session's working directory
    /// * `pane` - The pane's key
    async fn taken_in(
        &self,
        cwd: &Path,
        pane: &str,
    ) -> anyhow::Result<(String, Vec<reef::open::Open>)> {
        let pane: reef_api::Pane = pane.parse()?;
        // each pane it opens recorded, never opened
        let mut opened = Vec::new();
        let open = |order: &reef::open::Open| {
            opened.push(order.clone());
            Ok(())
        };
        let mut out = Vec::new();
        reef::task::next_task(
            &self.client()?,
            cwd,
            Some(&pane),
            &reef::cli::Pick::default(),
            open,
            &mut out,
        )
        .await?;
        Ok((String::from_utf8(out)?, opened))
    }

    /// Calls Claude Code's WorktreeCreate hook as `claude -w <name>` calls it from a directory
    /// in a pane, returning the path it printed and what else it said
    ///
    /// # Arguments
    ///
    /// * `cwd` - The session's working directory
    /// * `name` - The worktree's name
    /// * `pane` - The new session's pane, if it has one
    async fn worktree_made(
        &self,
        cwd: &Path,
        name: &str,
        pane: Option<&str>,
    ) -> anyhow::Result<(String, String)> {
        // the input Claude Code hands it
        let input = serde_json::json!({
            "session_id": "w1",
            "transcript_path": "/dev/null",
            "cwd": cwd,
            "hook_event_name": "WorktreeCreate",
            "name": name,
        });
        let pane: Option<reef_api::Pane> = pane.map(str::parse).transpose()?;
        let (mut said, mut out) = (Vec::new(), Vec::new());
        reef::hook::worktree_create(
            &self.client()?,
            &self.repo,
            &input.to_string(),
            pane.as_ref(),
            &mut said,
            &mut out,
        )
        .await?;
        Ok((String::from_utf8(out)?, String::from_utf8(said)?))
    }

    /// Calls Claude Code's WorktreeRemove hook as it is called for a worktree it is done with,
    /// returning what it printed
    ///
    /// # Arguments
    ///
    /// * `path` - The worktree's root
    async fn worktree_removed(&self, path: &Path) -> anyhow::Result<String> {
        let input = serde_json::json!({
            "session_id": "w1",
            "transcript_path": "/dev/null",
            "cwd": self.repo,
            "hook_event_name": "WorktreeRemove",
            "worktree_path": path,
        });
        let mut out = Vec::new();
        reef::hook::worktree_remove(&self.client()?, &input.to_string(), &mut out).await?;
        Ok(String::from_utf8(out)?)
    }

    /// Calls Claude Code's Stop hook as it is called when a turn ends in a session in a pane,
    /// naming the session, returning what it printed and each injector it would have started
    ///
    /// # Arguments
    ///
    /// * `cwd` - The session's working directory
    /// * `pane` - The pane's key
    /// * `session` - The session that stopped
    /// * `pid` - Its Claude Code process, `CLAUDE_PID`
    async fn stopped_as(
        &self,
        cwd: &Path,
        pane: &str,
        session: &str,
        pid: u32,
    ) -> anyhow::Result<(String, Vec<reef::clear::Clear>)> {
        let input = serde_json::json!({
            "session_id": session,
            "transcript_path": "/dev/null",
            "cwd": cwd,
            "hook_event_name": "Stop",
            "stop_hook_active": false,
        });
        let pane: reef_api::Pane = pane.parse()?;
        let mut spawned = Vec::new();
        let spawn = |order: &reef::clear::Clear| {
            spawned.push(order.clone());
            Ok(())
        };
        let mut out = Vec::new();
        reef::hook::stop(
            &self.client()?,
            &self.repo,
            &input.to_string(),
            Some(&pane),
            Some(pid),
            spawn,
            &mut out,
        )
        .await?;
        Ok((String::from_utf8(out)?, spawned))
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
        said.ends_with("reef: `cli/docs/B` is done, and the loop is paused: `! reef loop start` moves on to the next node"),
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

/// SessionStart tells the follower of the session's Claude Code process where a clear moved it,
/// whether or not a loop runs in its pane, and writes nothing where no follower's inbox is
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_clear_tells_the_sessions_follower_where_it_moved() -> anyhow::Result<()> {
    let hx = Hx::new("follow-moves", &["A"]).await?;
    // the inbox of a follower of process 4242, under the case's own state directory
    let state = hx.ax.dir.join("state").display().to_string();
    let env = |name: &str| (name == "XDG_STATE_HOME").then(|| state.clone());
    let inbox = reef::follow::inbox_of(env, 4242)?;
    assert!(inbox.starts_with(hx.ax.dir.join("state").join("reef").join("follow")));
    // what Claude Code hands SessionStart after a clear
    let transcript = hx.ax.dir.join("claude").join("s2.jsonl");
    let input = serde_json::json!({
        "session_id": "s2",
        "transcript_path": transcript,
        "cwd": hx.repo,
        "hook_event_name": "SessionStart",
        "source": "clear",
    })
    .to_string();
    let pane: reef_api::Pane = "claude:4242".parse()?;
    let start = |inbox: Option<std::path::PathBuf>| {
        let (hx, input, pane) = (&hx, input.clone(), pane.clone());
        async move {
            let mut out = Vec::new();
            reef::loops::session_start(
                &hx.client()?,
                &input,
                &pane,
                Some(4242),
                inbox.as_deref(),
                &mut out,
            )
            .await?;
            anyhow::Ok(String::from_utf8(out)?)
        }
    };
    // with no follower, no inbox is made
    assert_eq!(start(Some(inbox.clone())).await?, "");
    assert!(!inbox.exists());
    // with one, it is told the new session and its file, though no loop runs in the pane
    std::fs::create_dir_all(inbox.parent().expect("the inbox is in a directory"))?;
    std::fs::write(&inbox, "")?;
    assert_eq!(start(Some(inbox.clone())).await?, "");
    assert_eq!(
        reef::follow::events(&inbox),
        [reef::follow::Event::Moved {
            claude: "s2".to_string(),
            transcript,
        }]
    );
    Ok(())
}

/// A start in a session's own zellij pane moves a loop between nodes on as Stop does: once a
/// paused loop's node is done, it starts the injector once, handed the session, its Claude Code
/// process and the node, and says reef clears. A fresh start, one mid-node, one naming the pane,
/// and one with nothing else ready start none, the named one saying `/clear`, then `next`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_between_nodes_moves_on() -> anyhow::Result<()> {
    let hx = Hx::new("loop-starts", &["A", "B", "C"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    let pane = "zellij:work:3";
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    // a fresh start holds nothing, so it starts nothing
    let (said, spawned) = hx.started_by(&a, pane, "s1", 4242).await?;
    assert!(said.starts_with("started a loop in zellij"), "{said}");
    assert_eq!(spawned.len(), 0);
    // nor does a start mid-node, which leaves the session on its node
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/A`"), "{took}");
    let (said, spawned) = hx.started_by(&a, pane, "s1", 4242).await?;
    assert!(said.ends_with("holding `cli/docs/A`\n"), "{said}");
    assert_eq!(spawned.len(), 0);
    // paused, A's end says the loop stops there, and nothing is cleared
    hx.reef(&a, &["loop", "pause", "--pane", pane]).await?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    let said = told(&hx.stopped_in(&a, Some(pane), false).await?);
    assert!(
        said.ends_with("`! reef loop start` moves on to the next node"),
        "{said}"
    );
    // a start then moves on: the injector started once, and reef says it clears
    let (said, spawned) = hx.started_by(&a, pane, "s2", 4242).await?;
    assert!(
        said.ends_with(
            "\n`cli/docs/A` is done. `cli/docs/B` is next: reef clears the context and asks for it \
             once Claude is idle and the input box is empty\n"
        ),
        "{said}"
    );
    assert_eq!(
        spawned,
        vec![reef::clear::Clear {
            pane: pane.parse()?,
            session: "s2".to_string(),
            pid: 4242,
            after: "cli/docs/A".parse()?,
        }]
    );
    // a start naming the pane, from wherever, types nothing and leaves the clear to the user
    let said = hx.reef(&a, &["loop", "start", "--pane", pane]).await?;
    assert!(
        said.ends_with("`cli/docs/A` is done. `cli/docs/B` is next: `/clear`, then say `next`\n"),
        "{said}"
    );
    // and once the last ready node is done, a start finds nothing next to clear for
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/B`"), "{took}");
    hx.reef(&a, &["status", "C", "done"]).await?;
    hx.reef(&a, &["status", "B", "done"]).await?;
    let said = told(&hx.stopped_in(&a, Some(pane), false).await?);
    assert!(said.contains("nothing else is ready"), "{said}");
    let (said, spawned) = hx.started_by(&a, pane, "s3", 4242).await?;
    assert!(said.contains("nothing else is ready to take"), "{said}");
    assert_eq!(spawned.len(), 0);
    Ok(())
}

/// Two ready nodes are worked at once, each in its own worktree, and the next group starts only
/// once both are done: a loop two wide in zellij takes A with B beside it and opens B's pane;
/// B's session gets its worktree from WorktreeCreate and takes B; A's end waits on B and clears
/// nothing; B's end, the last, seals only B's changes and clears the loop's own pane; and
/// next_task takes no new group until `reef/B` is merged, a merge that drafts nothing into A
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_group_is_worked_at_once_and_ends_once() -> anyhow::Result<()> {
    let hx = Hx::new("group", &["A", "B", "C"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    // a loop two wide in a zellij pane, its session s1 in Claude Code process 4242
    let lead = "zellij:work:3";
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    let said = hx.started_wide(&a, lead, 2, true).await?;
    assert!(said.ends_with(", 2 nodes at once\n"), "{said}");
    // it takes A for itself and B beside it, opening B's pane from its own checkout
    let (took, opened) = hx.taken_in(&a, lead).await?;
    assert!(
        took.starts_with(
            "Took `cli/docs/A` for the loop in zellij session `work`, pane 3, with 1 more of its \
             group beside it for sessions of their own. Work only `cli/docs/A` here."
        ),
        "{took}"
    );
    assert!(
        took.contains("cli/docs/B  a pane of its own was opened, running `claude -w B`"),
        "{took}"
    );
    assert_eq!(
        opened,
        vec![reef::open::Open {
            beside: lead.parse()?,
            root: a.clone(),
            id: "B".to_string(),
        }]
    );
    // B's session, `claude -w B` in zellij pane 7, gets its worktree from reef
    let member = "zellij:work:7";
    let (path, said) = hx.worktree_made(&a, "B", Some(member)).await?;
    let b = hx.repo.join(".claude/worktrees/B").canonicalize()?;
    assert_eq!(path, format!("{}\n", b.display()));
    assert!(said.contains("reef: started `B` in"), "{said}");
    assert!(
        said.ends_with(
            "reef: this session works `cli/docs/B` for the loop in zellij session `work`, pane 3\n"
        ),
        "{said}"
    );
    assert_eq!(git::run(&b, &["branch", "--show-current"])?, "reef/B");
    let listed = hx.reef(&a, &["loop", "ls"]).await?;
    assert!(
        listed.contains("2 at once, with `cli/docs/B` in zellij:work:7"),
        "{listed}"
    );
    // and next_task there hands it B, started in its worktree, which a second call does again
    for _ in 0..2 {
        let (took, opened) = hx.taken_in(&b, member).await?;
        assert!(
            took.starts_with("`cli/docs/B` is this session's, a member of the loop in zellij session `work`, pane 3's group"),
            "{took}"
        );
        assert!(
            took.contains(&format!("started in {} on `reef/B`", b.display())),
            "{took}"
        );
        assert!(opened.is_empty());
    }
    // both worked at once, each committing in its own worktree
    committed(&a, "a.txt", "A's\n", "A").await?;
    committed(&b, "b.txt", "B's\n", "B").await?;
    // A done first: its end waits on B, and nothing is cleared
    hx.reef(&a, &["status", "A", "done"]).await?;
    let (said, spawned) = hx.stopped_as(&a, lead, "s1", 4242).await?;
    let said = told(&said);
    assert!(
        said.ends_with(
            "reef: `cli/docs/A` is done. Its group waits on `cli/docs/B`: this pane moves on \
             once they are all done"
        ),
        "{said}"
    );
    assert!(spawned.is_empty(), "{spawned:?}");
    let (took, _) = hx.taken_in(&a, lead).await?;
    assert!(
        took.starts_with("`cli/docs/A` is done, and its group waits on `cli/docs/B`"),
        "{took}"
    );
    // B done last: its end seals B and clears the loop's own pane, with the loop's session
    hx.reef(&b, &["status", "B", "done"]).await?;
    let (said, spawned) = hx.stopped_as(&b, member, "w1", 7777).await?;
    let said = told(&said);
    assert!(said.starts_with("reef: sealed `B` as revision 1"), "{said}");
    assert!(
        said.ends_with(
            "reef: `cli/docs/B` was the last of its group to be done, so the loop's pane moves \
             on. `cli/docs/A` is done. `cli/docs/C` is next: reef clears the context and asks \
             for it once Claude is idle and the input box is empty"
        ),
        "{said}"
    );
    assert_eq!(
        spawned,
        vec![reef::clear::Clear {
            pane: lead.parse()?,
            session: "s1".to_string(),
            pid: 4242,
            after: "cli/docs/A".parse()?,
        }]
    );
    // each captured only its own changes
    for (id, file) in [("A", "a.txt"), ("B", "b.txt")] {
        let changes = hx.changes(id).await?;
        let paths: Vec<String> = changes
            .last()
            .map(|(_, files)| files.iter().map(|f| f.path().to_string()).collect())
            .unwrap_or_default();
        assert_eq!(paths, [file], "{id}");
    }
    // the group's end is acted on once, from either pane
    for (cwd, pane) in [(&a, lead), (&b, member)] {
        let (said, spawned) = hx.stopped_as(cwd, pane, "s1", 4242).await?;
        assert_eq!((said.as_str(), spawned.len()), ("", 0), "{pane}");
    }
    // after the clear, no new group is taken until B's branch is merged
    let (took, opened) = hx.taken_in(&a, lead).await?;
    assert!(took.contains("so no new group is taken"), "{took}");
    assert!(
        took.contains("  git merge reef/B    # cli/docs/B"),
        "{took}"
    );
    assert!(opened.is_empty());
    let listed = hx.reef(&a, &["loop", "ls"]).await?;
    assert!(listed.contains("on `cli/docs/A`"), "{listed}");
    // merged, concluded by `git commit` as a conflict's resolution is, which runs post-commit
    // on the merge and drafts nothing of B's into A, the next group is C alone
    hooked(&a, &["merge", "-q", "--no-ff", "--no-commit", "reef/B"]).await?;
    hooked(&a, &["commit", "-q", "--no-edit"]).await?;
    let revisions: Vec<bool> = hx
        .changes("A")
        .await?
        .iter()
        .map(|(revision, _)| revision.draft)
        .collect();
    assert_eq!(revisions, [false], "A's changes after the merge");
    let (took, opened) = hx.taken_in(&a, lead).await?;
    assert!(
        took.starts_with(
            "Took `cli/docs/C` for the loop in zellij session `work`, pane 3. Plan it now"
        ),
        "{took}"
    );
    assert!(opened.is_empty());
    Ok(())
}

/// WorktreeCreate makes `.claude/worktrees/<id>` on `reef/<id>` with the node started there,
/// kept out of the main worktree's snapshots, and made again is the same; a member another
/// session works is refused, and a name that is no node gets a plain worktree. `reef loop
/// attach` gives an open session a member and says to enter its worktree, a session in a
/// member's worktree that no pane joined is joined by its next_task, a loop that opens no panes
/// names the ways a member gets a session, and WorktreeRemove drafts the node, then removes its
/// worktree but never one with changes, keeping the branch
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worktrees_are_made_for_nodes_and_attached() -> anyhow::Result<()> {
    let hx = Hx::new("worktrees", &["A", "B", "C", "D"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    let main = git::run(&hx.repo, &["branch", "--show-current"])?;
    // a loop four wide in tmux, opening no panes, in a checkout of its own
    let lead = "tmux:/tmp/t:%1";
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    hx.started_wide(&a, lead, 4, false).await?;
    let (took, opened) = hx.taken_in(&a, lead).await?;
    assert!(opened.is_empty());
    for id in ["B", "C"] {
        assert!(
            took.contains(&format!(
                "cli/docs/{id}  waits for a session of its own: `claude -w {id}` run from {}, or \
                 `! reef loop attach {id}` in a Claude session already open",
                a.display()
            )),
            "{took}"
        );
    }
    // B's worktree made from the main checkout, on its own branch, based on main's
    let (path, said) = hx
        .worktree_made(&hx.repo, "B", Some("tmux:/tmp/t:%2"))
        .await?;
    let b = hx.repo.join(".claude/worktrees/B").canonicalize()?;
    assert_eq!(path.trim_end(), b.display().to_string());
    assert!(said.contains(&format!("reef: base      {main}")), "{said}");
    let record = hx.reef(&b, &["work"]).await?;
    assert!(record.contains("branch    reef/B"), "{record}");
    // out of the main worktree's snapshots, excluded once however often it is made
    let excluded = |repo: &Path| -> anyhow::Result<usize> {
        let common = git::run(
            repo,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        let text = std::fs::read_to_string(Path::new(&common).join("info/exclude"))?;
        Ok(text
            .lines()
            .filter(|line| *line == "/.claude/worktrees/")
            .count())
    };
    assert_eq!(git::run(&hx.repo, &["status", "--porcelain"])?, "");
    // made again from the same pane, it is the same worktree
    let (again, said) = hx
        .worktree_made(&hx.repo, "B", Some("tmux:/tmp/t:%2"))
        .await?;
    assert_eq!(again, path);
    assert!(said.contains("is in"), "{said}");
    assert_eq!(excluded(&hx.repo)?, 1);
    // and asked for from another pane, it is refused, since B has a session
    let error = hx
        .worktree_made(&hx.repo, "B", Some("tmux:/tmp/t:%9"))
        .await
        .expect_err("a second session was given B");
    assert!(
        format!("{error:#}").contains("has a session already, in tmux pane %2"),
        "{error:#}"
    );
    // a name that is no node gets a plain worktree, starting nothing
    let (plain, _) = hx.worktree_made(&hx.repo, "scratch", None).await?;
    let scratch = hx.repo.join(".claude/worktrees/scratch").canonicalize()?;
    assert_eq!(plain.trim_end(), scratch.display().to_string());
    assert_eq!(
        git::run(&scratch, &["branch", "--show-current"])?,
        "worktree-scratch"
    );
    // an open session in tmux pane %3 attaches to the member still waiting, C
    let attached = hx
        .reef(&hx.repo, &["loop", "attach", "--pane", "tmux:/tmp/t:%3"])
        .await?;
    let c = hx.repo.join(".claude/worktrees/C").canonicalize()?;
    assert!(
        attached.contains(&format!(
            "`cli/docs/C` is this session's, a member of the group of the loop in tmux pane %1 \
             on /tmp/t, in {} on `reef/C`. Enter that worktree with EnterWorktree (path: {}), \
             then take the node with next_task",
            c.display(),
            c.display()
        )),
        "{attached}"
    );
    // and its next_task, served from the main checkout, hands it C
    let (took, _) = hx.taken_in(&hx.repo, "tmux:/tmp/t:%3").await?;
    assert!(took.starts_with("`cli/docs/C` is this session's"), "{took}");
    // D's worktree made where WorktreeCreate finds no pane, its session's next_task joins it there
    let (path, said) = hx.worktree_made(&hx.repo, "D", None).await?;
    let d = hx.repo.join(".claude/worktrees/D").canonicalize()?;
    assert_eq!(path.trim_end(), d.display().to_string());
    assert!(!said.contains("this session works"), "{said}");
    let (took, _) = hx.taken_in(&d, "claude:5555").await?;
    assert!(took.starts_with("`cli/docs/D` is this session's"), "{took}");
    let listed = hx.reef(&a, &["loop", "ls"]).await?;
    assert!(
        listed.contains("with `cli/docs/D` in claude:5555"),
        "{listed}"
    );
    // with every member joined, an attach finds none waiting, and the loop's pane joins none
    let error = hx
        .reef(&hx.repo, &["loop", "attach", "--pane", "tmux:/tmp/t:%4"])
        .await
        .expect_err("an attach found a member no session had");
    assert!(
        format!("{error:#}").contains("waits for a session"),
        "{error:#}"
    );
    let error = hx
        .reef(&hx.repo, &["loop", "attach", "B", "--pane", lead])
        .await
        .expect_err("the loop's own pane attached to a member");
    assert!(
        format!("{error:#}").contains("runs a loop of its own"),
        "{error:#}"
    );
    // B's worktree, its work committed, is drafted and removed, its branch kept
    committed(&b, "b.txt", "B's\n", "B").await?;
    let said = hx.worktree_removed(&b).await?;
    assert!(
        said.ends_with(&format!("reef: removed {}\n", b.display())),
        "{said}"
    );
    assert!(!b.exists());
    assert!(git::succeeds(
        &hx.repo,
        &["rev-parse", "--verify", "-q", "refs/heads/reef/B"]
    )?);
    // and C's, holding a change nothing committed, is left as it is
    std::fs::write(c.join("base.txt"), "changed\n")?;
    let error = hx
        .worktree_removed(&c)
        .await
        .expect_err("a worktree with changes was removed");
    assert!(
        format!("{error:#}").contains("was left as it is"),
        "{error:#}"
    );
    assert!(c.exists());
    Ok(())
}

/// A member's worktree has the repo's local settings: WorktreeCreate copies the asking
/// checkout's `.claude/settings.local.json` into each worktree it makes, a plain one's included,
/// once git ignores it, and never while git would take it in as the node's change
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_members_worktree_gets_the_local_settings() -> anyhow::Result<()> {
    let hx = Hx::new("local", &["B", "C"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    // local settings that git doesn't ignore yet, whatever the machine's own ignore file says,
    // where Claude Code puts `**/.claude/settings.local.json`
    let local =
        r#"{ "enabledMcpjsonServers": ["reef"], "permissions": { "allow": ["mcp__reef"] } }"#;
    std::fs::create_dir_all(hx.repo.join(".claude"))?;
    std::fs::write(hx.repo.join(reef::claude::LOCAL), local)?;
    let checkout = reef::git::Checkout::open(&hx.repo)?;
    checkout.exclude(&format!("!{}", reef::claude::LOCAL_EXCLUDED))?;
    // B's worktree is made without them, since they would be its change
    let (_, said) = hx.worktree_made(&hx.repo, "B", None).await?;
    let b = hx.repo.join(".claude/worktrees/B");
    assert!(!b.join(reef::claude::LOCAL).exists());
    assert!(!said.contains("copied"), "{said}");
    // ignored as `reef claude install` leaves them, its line after the one above, C's worktree
    // gets the same bytes
    checkout.exclude(reef::claude::LOCAL_EXCLUDED)?;
    let (_, said) = hx.worktree_made(&hx.repo, "C", None).await?;
    let c = hx.repo.join(".claude/worktrees/C").canonicalize()?;
    assert_eq!(std::fs::read_to_string(c.join(reef::claude::LOCAL))?, local);
    assert!(
        said.contains(&format!(
            "reef: copied .claude/settings.local.json into {}",
            c.display()
        )),
        "{said}"
    );
    // which leaves C's worktree clean, so no capture of C holds them
    assert_eq!(git::run(&c, &["status", "--porcelain"])?, "");
    // a plain worktree gets them too
    let (plain, _) = hx.worktree_made(&hx.repo, "scratch", None).await?;
    let scratch = Path::new(plain.trim_end());
    assert_eq!(
        std::fs::read_to_string(scratch.join(reef::claude::LOCAL))?,
        local
    );
    // and made again, a worktree keeps its own
    std::fs::write(c.join(reef::claude::LOCAL), "{}")?;
    let (_, said) = hx.worktree_made(&hx.repo, "C", None).await?;
    assert!(said.contains("is in"), "{said}");
    assert_eq!(std::fs::read_to_string(c.join(reef::claude::LOCAL))?, "{}");
    Ok(())
}

/// Comments on a node of `cli/docs`, as the app does, returning the thread's number
///
/// # Arguments
///
/// * `hx` - The case
/// * `id` - The node's id
/// * `target` - What the thread is on
/// * `body` - Its first comment
async fn commented(hx: &Hx, id: &str, target: reef_api::Target, body: &str) -> anyhow::Result<u32> {
    // a thread written by the user, through the JSON API
    let node = PlanName::new("cli", "docs")?.node(id)?;
    let new = reef_api::NewThread {
        target,
        body: body.to_string(),
        by: reef_api::Writer::User,
        review: false,
    };
    let path = reef_api::paths::node_comments(&node);
    let (_, commented): (_, reef_api::Commented) = hx.ax.client.post(&path, &new).await?;
    match commented {
        reef_api::Commented::Added { thread, .. } => Ok(thread.number),
        other => panic!("a thread on `{node}` wasn't kept: {other:?}"),
    }
}

/// A thread of `cli/docs`, which must be there
///
/// # Arguments
///
/// * `hx` - The case
/// * `number` - The thread's number
async fn thread_of(hx: &Hx, number: u32) -> anyhow::Result<reef_api::Thread> {
    let plan = PlanName::new("cli", "docs")?;
    Ok(hx
        .client()?
        .thread(&plan, number)
        .await?
        .expect("the thread is there"))
}

/// A line of `a.txt` in one of a node's revisions, on the side its worktree has
///
/// # Arguments
///
/// * `revision` - The revision's number
/// * `line` - The line's number
fn line_of(revision: u32, line: u32) -> reef_api::Target {
    reef_api::Target::Line(reef_api::Anchor {
        revision,
        path: "a.txt".to_string(),
        side: reef_api::Side::New,
        line,
    })
}

/// Squashes the fixups on a worktree's branch into their commits, off the runtime's threads,
/// as a round's steps say
///
/// # Arguments
///
/// * `dir` - The worktree
/// * `onto` - The commit the rebase starts after
async fn squashed(dir: &Path, onto: &str) -> anyhow::Result<String> {
    hooked(
        dir,
        &[
            "-c",
            "sequence.editor=true",
            "rebase",
            "-q",
            "-i",
            "--autosquash",
            "--autostash",
            "--rebase-merges",
            onto,
        ],
    )
    .await
}

/// A comment left on a done node comes back before any new node, as a round of it: the node is
/// started again here and handed back with its threads and how its fix goes into its commit. The
/// round's fixup resolves the thread it names, its squash leaves the node one commit holding the
/// fix, and the round's revision holds only the fix. Its end is acted on as a node's, and a
/// thread reopened since comes back for a second round, which resolves it again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_comment_on_a_done_node_comes_back_as_a_round_squashed_into_its_commit()
-> anyhow::Result<()> {
    let hx = Hx::new("rounds", &["A", "B"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    let base = git::run(&hx.repo, &["rev-parse", "HEAD"])?;
    // a loop takes A into a worktree of its own, and A's work is committed, done and sealed
    let pane = "claude:4242";
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    hx.reef(&a, &["loop", "start", "--pane", pane]).await?;
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/A`"), "{took}");
    let first = committed(&a, "a.txt", "one\ntwo\n", "Add a").await?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    let said = told(&hx.stopped_in(&a, Some(pane), false).await?);
    assert!(
        said.ends_with("`cli/docs/B` is next: `/clear`, then say `next`"),
        "{said}"
    );
    // the user comments on A's second line, and on A as a whole, once its end was acted on
    let on_line = commented(&hx, "A", line_of(1, 2), "Two should be a number").await?;
    let general = commented(&hx, "A", reef_api::Target::Node, "Why a.txt?").await?;
    // so A comes back before B, started again here and in progress, its threads and its
    // commit's fixup named
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(
        took.starts_with("`cli/docs/A` is done, but 2 of its comment threads want an answer"),
        "{took}"
    );
    assert!(!took.contains("cli/docs/B"), "{took}");
    assert!(
        took.contains(&format!(
            "#{on_line} on `cli/docs/A`, a.txt:2 in revision 1: open"
        )),
        "{took}"
    );
    assert!(took.contains("        Two should be a number"), "{took}");
    let fixup =
        format!("git commit --fixup={first} --trailer 'Reef-Resolves: {on_line}, {general}'");
    assert!(took.contains(&fixup), "{took}");
    assert!(
        took.contains(&format!("--rebase-merges {first}~1")),
        "{took}"
    );
    assert!(took.contains("status: in-progress"), "{took}");
    let record = hx
        .client()?
        .work_of(&PlanName::new("cli", "docs")?.node("A")?)
        .await?;
    assert_eq!(record.map(|record| record.start.start), Some(first.clone()));
    // a second call mid-round hands out the round again
    let again = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(
        again.contains("for a round of its open comments"),
        "{again}"
    );
    assert!(again.contains(&fixup), "{again}");
    // the round: the general thread answered with no change, as Claude, then A done
    let client = hx.client()?;
    let mut out = Vec::new();
    reef::comment::resolve(
        &client,
        &a,
        general,
        reef::comment::Answering::Own,
        None,
        Some("It holds the numbers."),
        reef_api::Writer::Claude,
        &reef::cli::PlanPick::default(),
        &mut out,
    )
    .await?;
    let answered = thread_of(&hx, general).await?;
    assert!(!answered.is_open());
    assert_eq!(
        answered.comments.last().map(|comment| comment.by),
        Some(reef_api::Writer::Claude)
    );
    std::fs::write(a.join("a.txt"), "one\n2\n")?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    hx.status_used(&a, "A", "done").await?;
    // the fixup naming the line's thread resolves it when its post-commit captures it
    hooked(
        &a,
        &[
            "commit",
            "-qa",
            &format!("--fixup={first}"),
            "--trailer",
            &format!("Reef-Resolves: {on_line}"),
        ],
    )
    .await?;
    let resolved = thread_of(&hx, on_line).await?;
    assert!(!resolved.is_open(), "{resolved:?}");
    let by = resolved.resolved_by().expect("a change resolved it");
    assert_eq!(by.revision, 2);
    assert_eq!(
        by.commit.as_ref().map(|commit| commit.summary.as_str()),
        Some("fixup! Add a")
    );
    // squashed in, A is one commit again, holding the fix
    squashed(&a, &format!("{first}~1")).await?;
    let commits = git::run(&a, &["log", "--format=%s", &format!("{base}..HEAD")])?;
    assert_eq!(commits, "Add a");
    let rewritten = git::run(&a, &["rev-parse", "HEAD"])?;
    assert_ne!(rewritten, first);
    assert_eq!(git::run(&a, &["show", "HEAD:a.txt"])?, "one\n2");
    // the round's end seals its revision, which holds only the fix, and names B next
    let said = told(&hx.stopped_in(&a, Some(pane), false).await?);
    assert!(said.starts_with("reef: sealed `A` as revision 2"), "{said}");
    assert!(
        said.ends_with("`cli/docs/B` is next: `/clear`, then say `next`"),
        "{said}"
    );
    let node = PlanName::new("cli", "docs")?.node("A")?;
    let revisions = client.revisions(&node).await?;
    assert_eq!(revisions.len(), 2);
    assert!(!revisions[1].draft);
    assert_eq!(revisions[1].counts(), (1, 1));
    assert!(
        revisions[1].commits.is_empty(),
        "{:?}",
        revisions[1].commits
    );
    assert_eq!(
        revisions[0]
            .commits
            .iter()
            .map(|commit| commit.commit.as_str())
            .collect::<Vec<_>>(),
        [first.as_str()]
    );
    // the user reopens the line's thread, so A comes back for a second round, its commit found
    // again though the squash rewrote it
    let sealed = thread_of(&hx, on_line).await?;
    assert_eq!(sealed.outdated, Some(2), "its line was edited");
    let reopen = reef_api::MarkThread {
        state: reef_api::ThreadState::Open,
        read: sealed.version,
    };
    let path = reef_api::paths::thread_state(&PlanName::new("cli", "docs")?, on_line);
    let (_, marked): (_, reef_api::Marked) = hx.ax.client.put(&path, &reopen).await?;
    assert!(
        matches!(marked, reef_api::Marked::Marked { .. }),
        "{marked:?}"
    );
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(
        took.starts_with("`cli/docs/A` is done, but 1 of its comment threads wants an answer"),
        "{took}"
    );
    assert!(
        took.contains(&format!(
            "git commit --fixup={rewritten} --trailer 'Reef-Resolves: {on_line}'"
        )),
        "{took}"
    );
    // a second fixup, a change of its own, resolves it again
    std::fs::write(a.join("a.txt"), "1\n2\n")?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    hooked(
        &a,
        &[
            "commit",
            "-qa",
            &format!("--fixup={rewritten}"),
            "--date=2030-01-01T00:00:00",
            "--trailer",
            &format!("Reef-Resolves: {on_line}"),
        ],
    )
    .await?;
    let again = thread_of(&hx, on_line).await?;
    assert!(!again.is_open(), "{again:?}");
    assert_eq!(again.resolutions.len(), 2);
    squashed(&a, &format!("{rewritten}~1")).await?;
    assert_eq!(
        git::run(&a, &["log", "--format=%s", &format!("{base}..HEAD")])?,
        "Add a"
    );
    let said = told(&hx.stopped_in(&a, Some(pane), false).await?);
    assert!(said.starts_with("reef: sealed `A` as revision 3"), "{said}");
    // and with nothing left to answer, B is taken
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/B`"), "{took}");
    Ok(())
}

/// A thread Claude answered last, with a question back, waits on the user rather than calling
/// for another round, and a round answered without a change keeps no empty revision; the user's
/// answer brings the node back for a round again
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_thread_claude_answered_last_waits_on_the_user() -> anyhow::Result<()> {
    let hx = Hx::new("waits", &["A", "B"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    // A worked, done and sealed in a loop's worktree
    let pane = "claude:4242";
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    hx.reef(&a, &["loop", "start", "--pane", pane]).await?;
    hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    committed(&a, "a.txt", "one\ntwo\n", "Add a").await?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    hx.stopped_in(&a, Some(pane), false).await?;
    // a question on A's line takes A again for a round
    let asked = commented(&hx, "A", line_of(1, 1), "Is one right?").await?;
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(
        took.starts_with("`cli/docs/A` is done, but 1 of its"),
        "{took}"
    );
    // Claude asks back, as reef's reply tool does, and marks A done with no change
    let client = hx.client()?;
    let mut out = Vec::new();
    reef::comment::reply(
        &client,
        &a,
        asked,
        "Which one?",
        reef_api::Writer::Claude,
        &reef::cli::PlanPick::default(),
        &mut out,
    )
    .await?;
    let waiting = thread_of(&hx, asked).await?;
    assert!(waiting.is_open() && !waiting.wants_answer());
    hx.reef(&a, &["status", "A", "done"]).await?;
    hx.status_used(&a, "A", "done").await?;
    // its end keeps no empty revision, and B is next, not another round
    let said = told(&hx.stopped_in(&a, Some(pane), false).await?);
    assert!(!said.contains("sealed"), "{said}");
    assert!(
        said.ends_with("`cli/docs/B` is next: `/clear`, then say `next`"),
        "{said}"
    );
    let node = PlanName::new("cli", "docs")?.node("A")?;
    assert_eq!(client.revisions(&node).await?.len(), 1);
    let listed = hx.reef(&a, &["comment", "ls"]).await?;
    assert!(
        listed.contains(&format!("#{asked} on `cli/docs/A`, a.txt:1 in revision 1: open, waiting on the user's answer to Claude")),
        "{listed}"
    );
    assert!(
        listed.contains("    #2 by Claude:\n        Which one?"),
        "{listed}"
    );
    // the user's answer brings A back before B
    hx.reef(&a, &["comment", "reply", &asked.to_string(), "The first"])
        .await?;
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(
        took.starts_with("`cli/docs/A` is done, but 1 of its"),
        "{took}"
    );
    Ok(())
}

/// A round on a node whose commit a remote branch holds already never rewrites it: its fix is
/// a fixup of its own, left for a later squash
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pushed_nodes_round_leaves_its_fixup_unsquashed() -> anyhow::Result<()> {
    let hx = Hx::new("pushed", &["A", "B"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    // A worked, done and sealed, then pushed, as a fetch would record it
    let pane = "claude:4242";
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    hx.reef(&a, &["loop", "start", "--pane", pane]).await?;
    hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    let first = committed(&a, "a.txt", "one\ntwo\n", "Add a").await?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    hx.stopped_in(&a, Some(pane), false).await?;
    git::run(&a, &["update-ref", "refs/remotes/origin/cli-a", &first])?;
    // a comment brings A back, its fix a fixup of the pushed commit with no rebase
    let asked = commented(&hx, "A", line_of(1, 2), "Two?").await?;
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(
        took.contains(
            "is on `origin/cli-a` already, so it isn't rewritten: commit everything the round \
             changed as a fixup of it"
        ),
        "{took}"
    );
    assert!(
        took.contains(&format!(
            "git commit --fixup={first} --trailer 'Reef-Resolves: {asked}'"
        )),
        "{took}"
    );
    assert!(!took.contains("GIT_SEQUENCE_EDITOR"), "{took}");
    Ok(())
}

/// A comment written in a review is hidden from Claude until the review is submitted: no round
/// of it, nothing in `reef comment ls`, no resolve by hand; once submitted, it comes back as a
/// round before anything else
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_review_is_hidden_from_the_loop_until_submitted() -> anyhow::Result<()> {
    let hx = Hx::new("reviews", &["A", "B", "C"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    // A worked, done and sealed in a loop's worktree
    let pane = "claude:4242";
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    hx.reef(&a, &["loop", "start", "--pane", pane]).await?;
    hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    committed(&a, "a.txt", "one\ntwo\n", "Add a").await?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    hx.stopped_in(&a, Some(pane), false).await?;
    // the user writes a comment on A in a review, which stays pending
    let node = PlanName::new("cli", "docs")?.node("A")?;
    let new = reef_api::NewThread {
        target: line_of(1, 2),
        body: "Two should be a number".to_string(),
        by: reef_api::Writer::User,
        review: true,
    };
    let (_, commented): (_, reef_api::Commented) = hx
        .ax
        .client
        .post(&reef_api::paths::node_comments(&node), &new)
        .await?;
    let reef_api::Commented::Added { thread, .. } = commented else {
        anyhow::bail!("the review's comment wasn't kept: {commented:?}");
    };
    // so the loop takes B, Claude's listing shows nothing, and it can't be resolved by hand
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(took.starts_with("Took `cli/docs/B`"), "{took}");
    assert!(!took.contains("comment thread"), "{took}");
    let listed = hx.reef(&a, &["comment", "ls"]).await?;
    assert_eq!(listed, "`cli/docs` has no open comment threads\n");
    let number = thread.number.to_string();
    let refused = hx
        .reef(&a, &["comment", "resolve", &number, "--node", "A"])
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("has no comment thread"),
        "{refused:#}"
    );
    // B done, the review submitted: B's end names A's comments next, and A is taken again
    hx.reef(&a, &["status", "B", "done"]).await?;
    let review: Option<reef_api::Review> = hx
        .ax
        .client
        .get(&reef_api::paths::node_review(&node))
        .await?
        .1;
    let review = review.expect("A's review is open");
    let submit = reef_api::MarkReview {
        state: reef_api::ReviewState::Submitted,
        read: review.version,
    };
    let (_, reviewed): (_, reef_api::Reviewed) = hx
        .ax
        .client
        .put(&reef_api::paths::node_review(&node), &submit)
        .await?;
    assert!(
        matches!(reviewed, reef_api::Reviewed::Marked { .. }),
        "{reviewed:?}"
    );
    let said = told(&hx.stopped_in(&a, Some(pane), false).await?);
    assert!(
        said.ends_with("`cli/docs/B` is done. `cli/docs/A`'s open comments come next: `/clear`, then say `next`"),
        "{said}"
    );
    let took = hx.reef(&a, &["loop", "take", "--pane", pane]).await?;
    assert!(
        took.starts_with("`cli/docs/A` is done, but 1 of its comment threads wants an answer"),
        "{took}"
    );
    assert!(took.contains("Two should be a number"), "{took}");
    Ok(())
}

/// Comments on a node of `cli/docs` in its review, as the app does, returning the thread's
/// number: it stays pending until the review is submitted
///
/// # Arguments
///
/// * `hx` - The case
/// * `id` - The node's id
/// * `body` - Its first comment, on the node as a whole
async fn reviewed(hx: &Hx, id: &str, body: &str) -> anyhow::Result<u32> {
    // a thread written by the user in the node's review, through the JSON API
    let node = PlanName::new("cli", "docs")?.node(id)?;
    let new = reef_api::NewThread {
        target: reef_api::Target::Node,
        body: body.to_string(),
        by: reef_api::Writer::User,
        review: true,
    };
    let path = reef_api::paths::node_comments(&node);
    let (_, commented): (_, reef_api::Commented) = hx.ax.client.post(&path, &new).await?;
    match commented {
        reef_api::Commented::Added { thread, .. } => Ok(thread.number),
        other => panic!("a review's thread on `{node}` wasn't kept: {other:?}"),
    }
}

/// Submits a node's open review in `cli/docs`, as the app's *Submit review* does, publishing
/// its comments at once
///
/// # Arguments
///
/// * `hx` - The case
/// * `id` - The node's id
async fn submitted(hx: &Hx, id: &str) -> anyhow::Result<()> {
    // the review as it stands, then submitted over the version read
    let node = PlanName::new("cli", "docs")?.node(id)?;
    let path = reef_api::paths::node_review(&node);
    let (_, review): (_, Option<reef_api::Review>) = hx.ax.client.get(&path).await?;
    let review = review.expect("the node's review is open");
    let submit = reef_api::MarkReview {
        state: reef_api::ReviewState::Submitted,
        read: review.version,
    };
    let (_, reviewed): (_, reef_api::Reviewed) = hx.ax.client.put(&path, &submit).await?;
    assert!(
        matches!(reviewed, reef_api::Reviewed::Marked { .. }),
        "{reviewed:?}"
    );
    Ok(())
}

/// The paths a node's revisions hold, each revision's in order, with whether it is sealed and
/// its added and removed lines
///
/// # Arguments
///
/// * `hx` - The case
/// * `id` - The node's id in `cli/docs`
async fn held(hx: &Hx, id: &str) -> anyhow::Result<Vec<(bool, Vec<String>, (u32, u32))>> {
    let node = PlanName::new("cli", "docs")?.node(id)?;
    let revisions = hx.client()?.revisions(&node).await?;
    Ok(revisions
        .iter()
        .map(|revision| {
            let paths = revision
                .files
                .iter()
                .map(|file| file.path().to_string())
                .collect();
            (!revision.draft, paths, revision.counts())
        })
        .collect())
}

/// The loop end to end, as C44 ran it live in zellij: three queued nodes taken in the queue's
/// order in one pane, each handed out with the steps that finish it. A comment left on the
/// first while it is planned comes back as a round of it before the second, and that node's
/// file, which its first pass left untracked, comes into the round's revision rather than its
/// start. A review of the second is hidden until it is submitted mid-node, and a pause made
/// mid-node stops the loop once the node is done, clearing nothing; a start then moves it on to
/// that node's round, squashed into its commit, then the third. Each node's end is acted on once,
/// starting one injector while another node is next, and each node's revisions hold only its own
/// file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn three_queued_nodes_run_end_to_end() -> anyhow::Result<()> {
    let hx = Hx::new("end-to-end", &["A", "B", "C"]).await?;
    committed(&hx.repo, "base.txt", "base\n", "base").await?;
    let base = git::run(&hx.repo, &["rev-parse", "HEAD"])?;
    // the user's queue, out of plan order
    for id in ["C", "A", "B"] {
        hx.reef(&hx.repo, &["queue", "add", id]).await?;
    }
    // a loop started in its session's own zellij pane, in a worktree of its own
    let pane = "zellij:work:3";
    let a = git::worktree(&hx.repo, &hx.ax.dir.join("cli-a"))?;
    let (said, spawned) = hx.started_by(&a, pane, "s1", 4242).await?;
    assert!(said.starts_with("started a loop in zellij"), "{said}");
    assert!(spawned.is_empty(), "{spawned:?}");
    // C, the queue's first, is taken first, with the steps that finish it for its plan
    let (took, _) = hx.taken_in(&a, pane).await?;
    assert!(took.starts_with("Took `cli/docs/C`"), "{took}");
    assert!(
        took.contains("End your plan with the steps that finish it"),
        "{took}"
    );
    assert!(
        took.contains(&reef::task::finish_steps(&"cli/docs/C".parse()?)),
        "{took}"
    );
    // the user comments on C while it is planned; its first pass leaves c.txt untracked
    let general = commented(&hx, "C", reef_api::Target::Node, "Give c.txt a second line").await?;
    std::fs::write(a.join("c.txt"), "c\n")?;
    hx.reef(&a, &["status", "C", "done"]).await?;
    hx.status_used(&a, "C", "done").await?;
    // so C's end seals nothing, and names its comments next, starting one injector
    let (said, spawned) = hx
        .stopped_by(&a, Some(pane), false, Some(4242), false)
        .await?;
    let said = told(&said);
    assert!(
        said.starts_with("reef: sealed `C` as revision 1, 0 files"),
        "{said}"
    );
    assert!(
        said.ends_with(
            "\nreef: `cli/docs/C` is done, and its open comments come next: reef clears the \
             context and asks for them once Claude is idle and the input box is empty"
        ),
        "{said}"
    );
    let clear = |session: &str, after: &str| -> anyhow::Result<reef::clear::Clear> {
        Ok(reef::clear::Clear {
            pane: pane.parse()?,
            session: session.to_string(),
            pid: 4242,
            after: after.parse()?,
        })
    };
    assert_eq!(spawned, vec![clear("s1", "cli/docs/C")?]);
    // the clear's session is told the round comes next, and takes C again before A
    let context = hx.cleared_in(pane, "s2").await?;
    assert!(
        context.contains("`cli/docs/C` is done, and its open comments come next"),
        "{context}"
    );
    let (took, _) = hx.taken_in(&a, pane).await?;
    assert!(
        took.starts_with("`cli/docs/C` is done, but 1 of its comment threads wants an answer"),
        "{took}"
    );
    assert!(!took.contains("cli/docs/A"), "{took}");
    assert!(took.contains("as a commit of its own"), "{took}");
    // the round commits c.txt, naming the thread, which its post-commit resolves
    std::fs::write(a.join("c.txt"), "c\nC\n")?;
    hx.reef(&a, &["status", "C", "done"]).await?;
    hx.status_used(&a, "C", "done").await?;
    hooked(&a, &["add", "c.txt"]).await?;
    let trailer = format!("Reef-Resolves: {general}");
    hooked(&a, &["commit", "-q", "-m", "Add c", "--trailer", &trailer]).await?;
    assert!(!thread_of(&hx, general).await?.is_open());
    // the round's end seals c.txt whole, left out of the round's start, and names A next
    let (said, spawned) = hx.stopped_as(&a, pane, "s2", 4242).await?;
    let said = told(&said);
    assert!(
        said.starts_with("reef: sealed `C` as revision 2, 1 file +2 -0"),
        "{said}"
    );
    assert!(
        said.ends_with(
            "\nreef: `cli/docs/C` is done. `cli/docs/A` is next: reef clears the context and \
             asks for it once Claude is idle and the input box is empty"
        ),
        "{said}"
    );
    assert_eq!(spawned, vec![clear("s2", "cli/docs/C")?]);
    let context = hx.cleared_in(pane, "s3").await?;
    assert!(context.contains("`cli/docs/A` is next"), "{context}");
    let (took, _) = hx.taken_in(&a, pane).await?;
    assert!(took.starts_with("Took `cli/docs/A`"), "{took}");
    // mid-node, the user writes a review of A, which Claude can't see, pauses the loop, and
    // submits the review
    let review = reviewed(&hx, "A", "End the greeting with `!`").await?;
    let listed = hx.reef(&a, &["comment", "ls"]).await?;
    assert_eq!(listed, "`cli/docs` has no open comment threads\n");
    let paused = hx.reef(&a, &["loop", "pause", "--pane", pane]).await?;
    assert!(
        paused.ends_with("it stops once `cli/docs/A` is done\n"),
        "{paused}"
    );
    submitted(&hx, "A").await?;
    let listed = hx.reef(&a, &["comment", "ls"]).await?;
    assert!(
        listed.contains(&format!(
            "#{review} on `cli/docs/A`, the node as a whole: open"
        )),
        "{listed}"
    );
    // A committed and done: the loop stops there, clearing nothing, and says so once
    let first_a = committed(&a, "a.txt", "hello\n", "Add a").await?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    hx.status_used(&a, "A", "done").await?;
    let (said, spawned) = hx.stopped_as(&a, pane, "s3", 4242).await?;
    let said = told(&said);
    assert!(
        said.starts_with("reef: sealed `A` as revision 1, 1 file +1 -0"),
        "{said}"
    );
    assert!(
        said.ends_with(
            "\nreef: `cli/docs/A` is done, and the loop is paused: `! reef loop start` moves on \
             to the next node"
        ),
        "{said}"
    );
    assert!(spawned.is_empty(), "{spawned:?}");
    let (said, spawned) = hx
        .stopped_by(&a, Some(pane), true, Some(4242), false)
        .await?;
    assert_eq!((said.as_str(), spawned.len()), ("", 0));
    // a start in the pane moves it on to A's round, starting one injector
    let (said, spawned) = hx.started_by(&a, pane, "s3", 4242).await?;
    assert!(
        said.ends_with(
            "\n`cli/docs/A` is done, and its open comments come next: reef clears the context \
             and asks for them once Claude is idle and the input box is empty\n"
        ),
        "{said}"
    );
    assert_eq!(spawned, vec![clear("s3", "cli/docs/A")?]);
    hx.cleared_in(pane, "s4").await?;
    let (took, _) = hx.taken_in(&a, pane).await?;
    assert!(
        took.starts_with("`cli/docs/A` is done, but 1 of its comment threads wants an answer"),
        "{took}"
    );
    assert!(took.contains("End the greeting with `!`"), "{took}");
    let fixup = format!("git commit --fixup={first_a} --trailer 'Reef-Resolves: {review}'");
    assert!(took.contains(&fixup), "{took}");
    // A's round: a fixup that resolves the review's thread, squashed into A's commit
    std::fs::write(a.join("a.txt"), "hello!\n")?;
    hx.reef(&a, &["status", "A", "done"]).await?;
    hx.status_used(&a, "A", "done").await?;
    let trailer = format!("Reef-Resolves: {review}");
    hooked(
        &a,
        &[
            "commit",
            "-qa",
            &format!("--fixup={first_a}"),
            "--trailer",
            &trailer,
        ],
    )
    .await?;
    assert!(!thread_of(&hx, review).await?.is_open());
    squashed(&a, &format!("{first_a}~1")).await?;
    let (said, spawned) = hx.stopped_as(&a, pane, "s4", 4242).await?;
    let said = told(&said);
    assert!(
        said.starts_with("reef: sealed `A` as revision 2, 1 file +1 -1"),
        "{said}"
    );
    assert!(
        said.ends_with(
            "`cli/docs/A` is done. `cli/docs/B` is next: reef clears the context and \
             asks for it once Claude is idle and the input box is empty"
        ),
        "{said}"
    );
    assert_eq!(spawned, vec![clear("s4", "cli/docs/A")?]);
    // B, the queue's last, then nothing else is ready, and nothing is cleared
    hx.cleared_in(pane, "s5").await?;
    let (took, _) = hx.taken_in(&a, pane).await?;
    assert!(took.starts_with("Took `cli/docs/B`"), "{took}");
    committed(&a, "b.txt", "red\n", "Add b").await?;
    hx.reef(&a, &["status", "B", "done"]).await?;
    hx.status_used(&a, "B", "done").await?;
    let (said, spawned) = hx.stopped_as(&a, pane, "s5", 4242).await?;
    let said = told(&said);
    assert!(
        said.ends_with(
            "reef: `cli/docs/B` is done, and nothing else is ready to take: `reef next` says \
             what everything waits on"
        ),
        "{said}"
    );
    assert!(spawned.is_empty(), "{spawned:?}");
    // each node is one commit, its round's fix squashed in where it had one
    let commits = git::run(&a, &["log", "--format=%s", &format!("{base}..HEAD")])?;
    assert_eq!(commits, "Add b\nAdd a\nAdd c");
    assert_eq!(git::run(&a, &["show", "HEAD~1:a.txt"])?, "hello!");
    // and each node's sealed revisions hold only its own file
    let file = |path: &str| vec![path.to_string()];
    assert_eq!(
        held(&hx, "C").await?,
        vec![(true, Vec::new(), (0, 0)), (true, file("c.txt"), (2, 0))]
    );
    assert_eq!(
        held(&hx, "A").await?,
        vec![(true, file("a.txt"), (1, 0)), (true, file("a.txt"), (1, 1))]
    );
    assert_eq!(held(&hx, "B").await?, vec![(true, file("b.txt"), (1, 0))]);
    Ok(())
}

/// One line of a session's transcript, as Claude Code writes it: a call or a result, at a time
///
/// # Arguments
///
/// * `second` - The second of 2026-11-23T17:00 it was written at
/// * `block` - The call or result
/// * `record` - Claude Code's own record of a result, if it kept one
fn said(second: u32, block: serde_json::Value, record: Option<serde_json::Value>) -> String {
    // a call is the assistant's, a result the user's
    let kind = match block["type"].as_str() {
        Some("tool_use") => "assistant",
        _ => "user",
    };
    let mut line = serde_json::json!({
        "type": kind,
        "sessionId": "s1",
        "timestamp": format!("2026-11-23T17:00:{second:02}.000Z"),
        "message": { "role": kind, "content": [block] },
    });
    if let Some(record) = record {
        line["toolUseResult"] = record;
    }
    line.to_string()
}

/// A session's plans and questions are kept with the nodes its next_task took, read from its
/// transcript at each turn's end and at the session's, each once, an answer read later filling
/// its question in; a plan approved with a clear is read as approved from the session the clear
/// starts, and stays so; one asked before any next_task goes, at a turn's end, to the node its
/// checkout was started on
#[tokio::test]
async fn a_sessions_decisions_are_kept_with_its_nodes() -> anyhow::Result<()> {
    use reef_api::{DecisionKind, Outcome, Via};
    use serde_json::json;
    let hx = Hx::new("decisions", &["A", "B"]).await?;
    let client = hx.client()?;
    let (a, b): (reef_model::NodeName, reef_model::NodeName) =
        ("cli/docs/A".parse()?, "cli/docs/B".parse()?);
    let question = json!({"questions": [{
        "question": "Which greeting?", "header": "Greeting", "multiSelect": false,
        "options": [{"label": "Hello", "description": ""}, {"label": "Hi", "description": ""}],
    }]});
    let terminal_no = "The user doesn't want to proceed with this tool use. The tool use was \
                       rejected. To tell you how to proceed, the user said:\nName the test.";
    // a turn: A taken, its question answered, a plan rejected in the terminal, the next approved,
    // then B taken and its question asked, each line written twice as Claude Code does
    let mut lines = vec![
        said(
            1,
            json!({"type": "tool_use", "id": "n1", "name": "mcp__reef__next_task", "input": {}}),
            None,
        ),
        said(
            2,
            json!({"type": "tool_result", "tool_use_id": "n1", "content": [{"type": "text", "text": "Took `cli/docs/A` for the loop in tmux pane %3."}]}),
            None,
        ),
        said(
            3,
            json!({"type": "tool_use", "id": "q1", "name": "AskUserQuestion", "input": question}),
            None,
        ),
        said(
            5,
            json!({"type": "tool_result", "tool_use_id": "q1", "content": "Your questions have been answered: \"Which greeting?\"=\"Hi\"."}),
            Some(json!({"answers": {"Which greeting?": "Hi"}})),
        ),
        said(
            6,
            json!({"type": "tool_use", "id": "p1", "name": "ExitPlanMode", "input": {"plan": "# Plan\n\nWrite it."}}),
            None,
        ),
        said(
            9,
            json!({"type": "tool_result", "tool_use_id": "p1", "content": terminal_no, "is_error": true}),
            None,
        ),
        said(
            10,
            json!({"type": "tool_use", "id": "p2", "name": "ExitPlanMode", "input": {"plan": "# Plan\n\nWrite it, tested."}}),
            None,
        ),
        said(
            12,
            json!({"type": "tool_result", "tool_use_id": "p2", "content": "User has approved your plan."}),
            None,
        ),
        said(
            20,
            json!({"type": "tool_use", "id": "n2", "name": "mcp__reef__next_task", "input": {}}),
            None,
        ),
        said(
            21,
            json!({"type": "tool_result", "tool_use_id": "n2", "content": [{"type": "text", "text": "Took `cli/docs/B` for the loop in tmux pane %3."}]}),
            None,
        ),
        said(
            22,
            json!({"type": "tool_use", "id": "q2", "name": "AskUserQuestion", "input": question}),
            None,
        ),
    ];
    lines.extend(lines.clone());
    let transcript = hx.ax.dir.join("s1.jsonl");
    std::fs::write(&transcript, lines.join("\n") + "\n")?;
    let input =
        json!({ "session_id": "s1", "cwd": hx.repo, "transcript_path": transcript }).to_string();
    // the turn's end keeps them, in a terminal
    let mut out = Vec::new();
    reef::hook::stop(&client, &hx.repo, &input, None, None, |_| Ok(()), &mut out).await?;
    let kept = client.decisions(&a).await?;
    assert_eq!(kept.len(), 3, "{kept:#?}");
    assert!(
        kept.iter()
            .all(|decision| decision.decided.via == Via::Terminal)
    );
    assert_eq!(
        kept.iter()
            .map(|decision| decision.number)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let answers =
        std::collections::BTreeMap::from([("Which greeting?".to_string(), "Hi".to_string())]);
    assert_eq!(
        kept[0].decided.outcome,
        Some(Outcome::Answered {
            answers: answers.clone()
        })
    );
    assert_eq!(
        kept[1].decided.outcome,
        Some(Outcome::Rejected {
            feedback: "Name the test.".to_string()
        })
    );
    assert!(
        matches!(&kept[2].decided.kind, DecisionKind::Plan { html, .. } if html.contains("<h1"))
    );
    assert_eq!(kept[2].decided.outcome, Some(Outcome::Approved));
    let pending = client.decisions(&b).await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].decided.outcome, None);
    // the session's end, B's question answered since: nothing twice, and B's filled in
    let answer = said(
        30,
        json!({"type": "tool_result", "tool_use_id": "q2", "content": "Your questions have been answered: \"Which greeting?\"=\"Hello\"."}),
        None,
    );
    std::fs::write(&transcript, lines.join("\n") + "\n" + &answer + "\n")?;
    reef::hook::session_end(&client, &hx.repo, &input, None, &mut out).await?;
    assert_eq!(client.decisions(&a).await?, kept, "A's are as they were");
    let answered = client.decisions(&b).await?;
    assert_eq!((answered.len(), answered[0].version), (1, 2));
    let hello =
        std::collections::BTreeMap::from([("Which greeting?".to_string(), "Hello".to_string())]);
    assert_eq!(
        answered[0].decided.outcome,
        Some(Outcome::Answered { answers: hello })
    );
    assert_eq!(
        answered[0].decided.answered_at,
        Some(answered[0].decided.asked_at + 8000)
    );
    // a plan approved with a clear: refused with no words in the planning session, whose end
    // keeps that, then approved as that call's in the session the clear starts, on A, which the
    // checkout was started on
    hx.reef(&hx.repo, &["start", "A"]).await?;
    let planning = [
        said(
            50,
            json!({"type": "tool_use", "id": "n3", "name": "mcp__reef__next_task", "input": {}}),
            None,
        ),
        said(
            51,
            json!({"type": "tool_result", "tool_use_id": "n3", "content": "`cli/docs/A` is still this loop's, in progress: carry on with it."}),
            None,
        ),
        said(
            52,
            json!({"type": "tool_use", "id": "p4", "name": "ExitPlanMode", "input": {"plan": "# Plan\n\nIn capitals."}}),
            None,
        ),
        said(
            55,
            json!({"type": "tool_result", "tool_use_id": "p4", "is_error": true, "content": "The user doesn't want to proceed with this tool use. STOP what you are doing."}),
            Some(json!("User rejected tool use")),
        ),
    ];
    let planned = hx.ax.dir.join("s3.jsonl");
    std::fs::write(&planned, planning.join("\n") + "\n")?;
    let ended = json!({ "cwd": hx.repo, "transcript_path": planned }).to_string();
    reef::hook::session_end(&client, &hx.repo, &ended, None, &mut out).await?;
    let refused = client.decisions(&a).await?;
    assert_eq!(refused.len(), 4);
    assert_eq!(
        refused[3].decided.outcome,
        Some(Outcome::Rejected {
            feedback: String::new()
        })
    );
    let carried = [
        json!({"type": "user", "timestamp": "2026-11-23T17:00:56.000Z", "message": {"role": "user", "content": "Implement the following plan:\n\n# Plan"}, "planContent": "# Plan\n\nIn capitals."}).to_string(),
        json!({"type": "data-flow-records", "sessionId": "s4", "records": [{"tool_name": "ExitPlanMode", "tool_use_id": "p4"}]}).to_string(),
    ];
    let cleared = hx.ax.dir.join("s4.jsonl");
    std::fs::write(&cleared, carried.join("\n") + "\n")?;
    let stopped =
        json!({ "session_id": "s4", "cwd": hx.repo, "transcript_path": cleared }).to_string();
    reef::hook::stop(
        &client,
        &hx.repo,
        &stopped,
        None,
        None,
        |_| Ok(()),
        &mut out,
    )
    .await?;
    // the approval stands, however often the planning session is read again
    reef::hook::session_end(&client, &hx.repo, &ended, None, &mut out).await?;
    let approved = client.decisions(&a).await?;
    assert_eq!(approved.len(), 4, "{approved:#?}");
    assert_eq!(approved[3].decided.outcome, Some(Outcome::Approved));
    assert_eq!(approved[3].decided.session, "s1");
    // a session that asks before any next_task: its end places nothing, since its pane's loop may
    // have moved on, and its Stop places it on the node its checkout is on, B, via a runner's app
    hx.reef(&hx.repo, &["start", "B"]).await?;
    let early = [
        said(
            40,
            json!({"type": "tool_use", "id": "p3", "name": "ExitPlanMode", "input": {"plan": "# B's plan"}}),
            None,
        ),
        said(
            41,
            json!({"type": "tool_result", "tool_use_id": "p3", "content": "User has approved your plan."}),
            None,
        ),
    ];
    let second = hx.ax.dir.join("s2.jsonl");
    std::fs::write(&second, early.join("\n") + "\n")?;
    let input = json!({ "cwd": hx.repo, "transcript_path": second }).to_string();
    let runner: reef_api::Pane = "reef:k3x9-4242".parse()?;
    reef::hook::session_end(&client, &hx.repo, &input, Some(&runner), &mut out).await?;
    assert_eq!(client.decisions(&b).await?.len(), 1);
    reef::hook::stop(
        &client,
        &hx.repo,
        &input,
        Some(&runner),
        None,
        |_| Ok(()),
        &mut out,
    )
    .await?;
    let on_b = client.decisions(&b).await?;
    assert_eq!(on_b.len(), 2, "{on_b:#?}");
    assert_eq!((on_b[1].number, on_b[1].decided.via), (2, Via::App));
    Ok(())
}
