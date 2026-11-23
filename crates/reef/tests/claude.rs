//! A repo set up for the loop by `reef claude install`: what it writes, that a second run
//! changes nothing, that what the repo had stays, and that the git hooks it writes run reef
//!
//! Each case makes a scratch repo under cargo's target dir and works it with real git, as the
//! skill's suite does. install runs through `reef::run`, as the command line runs it, with no
//! config and no service, which it needs neither of.

use reef::Cli;
use reef::claude::{GIT_HOOKS, HOOKS, LOCAL, MCP, SETTINGS};
use reef::skill::{FILES, PLACE};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The line a git hook's block of reef's starts with
const BEGIN: &str = "# >>> reef";

/// A scratch directory of the case's own under cargo's target dir, empty
///
/// # Arguments
///
/// * `name` - The case's name, which the directory is named after
fn scratch(name: &str) -> PathBuf {
    // a fresh directory each run, so an earlier run's files can't pass this one
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("claude-{name}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// Runs git in a directory as the tests' own author, with no one's config, returning its output
///
/// # Arguments
///
/// * `dir` - The directory to run in
/// * `args` - Git's arguments
/// * `path` - The `PATH` git and its hooks run with, else this process's
fn git_with(dir: &Path, args: &[&str], path: Option<&str>) -> String {
    // from the directory alone, as this test's author
    let mut git = Command::new("git");
    git.arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(["-c", "init.defaultBranch=main"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    if let Some(path) = path {
        git.env("PATH", path);
    }
    let output = git.output().expect("git runs");
    assert!(
        output.status.success(),
        "git {} in {}: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// Runs git in a directory as the tests' own author, with no one's config
///
/// # Arguments
///
/// * `dir` - The directory to run in
/// * `args` - Git's arguments
fn git(dir: &Path, args: &[&str]) -> String {
    git_with(dir, args, None)
}

/// A fresh repo, with one commit
///
/// # Arguments
///
/// * `name` - The case's name, which names its directory
fn repo(name: &str) -> PathBuf {
    // a repo of its own, nearer than the one cargo's target dir is in
    let root = scratch(name);
    git(&root, &["init", "-q"]);
    std::fs::write(root.join("README.md"), "# scratch\n").unwrap();
    git(&root, &["add", "README.md"]);
    git(&root, &["commit", "-q", "-m", "first"]);
    root
}

/// Runs `reef claude install` from a directory, returning what it printed
///
/// # Arguments
///
/// * `cwd` - The directory it runs in
async fn install(cwd: &Path) -> anyhow::Result<String> {
    // the command line as typed, with no config named
    let cli = Cli::from_args(["reef", "claude", "install"])?;
    let mut out = Vec::new();
    reef::run(cli, cwd, &mut out).await?;
    Ok(String::from_utf8(out)?)
}

/// Every file install writes in a repo, by its path from the root, and what it holds
///
/// # Arguments
///
/// * `root` - The repo's root
/// * `hooks` - The directory git runs its hooks from
fn written(root: &Path, hooks: &Path) -> BTreeMap<PathBuf, String> {
    // the JSON, the skill, the local settings' exclude and each git hook
    let mut paths: Vec<PathBuf> = [MCP, SETTINGS, LOCAL, ".git/info/exclude"]
        .iter()
        .map(|path| root.join(path))
        .collect();
    paths.extend(FILES.iter().map(|(name, _)| root.join(PLACE).join(name)));
    paths.extend(GIT_HOOKS.iter().map(|hook| hooks.join(hook.name)));
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
            (path, text)
        })
        .collect()
}

/// A JSON file of the repo, read
///
/// # Arguments
///
/// * `root` - The repo's root
/// * `path` - The file's path from it
fn read_json(root: &Path, path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(root.join(path)).unwrap()).unwrap()
}

/// How many times reef's command for a hook runs on its event
///
/// # Arguments
///
/// * `settings` - The `.claude/settings.json` object
/// * `event` - The event
/// * `command` - reef's command for it
fn runs(settings: &Value, event: &str, command: &str) -> usize {
    // every hook of every group on the event
    settings["hooks"][event]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["hooks"].as_array().unwrap().iter())
        .filter(|hook| hook["command"] == command)
        .count()
}

/// Two runs leave one MCP entry and one of each hook: the first writes everything, the second
/// says it is set up already and leaves every byte as it was
#[tokio::test]
async fn two_runs_leave_one_entry_and_one_of_each_hook() -> anyhow::Result<()> {
    // a repo, set up from a directory deep in it
    let root = repo("twice");
    let deeper = root.join("src/deeper");
    std::fs::create_dir_all(&deeper)?;
    let first = install(&deeper).await?;
    let hooks = root.join(".git/hooks");
    // the first run writes each file at the root, and says to commit the shared ones
    for line in [
        "wrote .mcp.json: the reef MCP server, `reef mcp`",
        "wrote .claude/settings.json: reef's hooks on SessionStart, Stop, PostToolUse, \
         WorktreeCreate and WorktreeRemove",
        "wrote .claude/settings.local.json: reef's server enabled, its tools allowed",
        "wrote .git/hooks/post-commit: git's post-commit running `reef hook post-commit`",
        "wrote .git/hooks/post-rewrite: git's post-rewrite running `reef hook post-rewrite`",
        "wrote .claude/skills/reef: the reef skill",
        "commit .mcp.json, .claude/settings.json and .claude/skills/reef, so every worktree \
         checks them out",
    ] {
        assert!(first.contains(line), "{line}\n{first}");
    }
    let once = written(&root, &hooks);
    // the second says each is set up already, and writes nothing
    let second = install(&root).await?;
    assert_eq!(
        second,
        ".mcp.json is set up already\n\
         .claude/settings.json is set up already\n\
         .claude/settings.local.json is set up already\n\
         .git/hooks/post-commit is set up already\n\
         .git/hooks/post-rewrite is set up already\n\
         .claude/skills/reef is set up already\n\
         nothing to change\n"
    );
    assert_eq!(written(&root, &hooks), once);
    // one MCP server, reef's
    let mcp = read_json(&root, MCP);
    assert_eq!(
        mcp,
        json!({ "mcpServers": { "reef": { "type": "stdio", "command": "reef", "args": ["mcp"] } } })
    );
    // one of each hook, PostToolUse on reef's status tool alone
    let settings = read_json(&root, SETTINGS);
    for hook in HOOKS {
        assert_eq!(runs(&settings, hook.event, &hook.command()), 1, "{hook:?}");
        assert_eq!(settings["hooks"][hook.event].as_array().unwrap().len(), 1);
    }
    assert_eq!(
        settings["hooks"]["PostToolUse"][0]["matcher"],
        "mcp__reef__status"
    );
    // the server enabled, its tools allowed and approval's clear offered, each once
    let local = read_json(&root, LOCAL);
    assert_eq!(
        local,
        json!({
            "enabledMcpjsonServers": ["reef"],
            "permissions": { "allow": ["mcp__reef"] },
            "showClearContextOnPlanAccept": true,
        })
    );
    // each git hook a shell script running reef's block once, executable
    for hook in GIT_HOOKS {
        let path = hooks.join(hook.name);
        let text = &once[&path];
        assert!(text.starts_with("#!/bin/sh\n"), "{text}");
        assert_eq!(text.matches(BEGIN).count(), 1, "{text}");
        assert_eq!(text.matches(hook.line).count(), 1, "{text}");
        let mode = std::fs::metadata(&path)?.permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "{} is {mode:o}", hook.name);
    }
    // the local settings excluded once, so git offers only the shared files to commit
    let exclude = &once[&root.join(".git/info/exclude")];
    assert_eq!(
        exclude
            .lines()
            .filter(|line| *line == "/.claude/settings.local.json")
            .count(),
        1
    );
    let status = git(&root, &["status", "--porcelain", "--untracked-files=all"]);
    assert!(!status.contains("settings.local.json"), "{status}");
    assert!(status.contains("?? .mcp.json"), "{status}");
    assert!(status.contains("?? .claude/settings.json"), "{status}");
    assert!(
        status.contains("?? .claude/skills/reef/SKILL.md"),
        "{status}"
    );
    // and nothing went where it ran
    assert!(!deeper.join(".claude").exists());
    Ok(())
}

/// What the repo had stays: another MCP server, the user's own hooks and settings, a shell hook
/// of its own, whose lines run after reef's block, and a reef hook pointed elsewhere by hand
#[tokio::test]
async fn install_keeps_what_was_there() -> anyhow::Result<()> {
    // a repo with its own Claude Code setup and a shell post-commit
    let root = repo("kept");
    std::fs::create_dir_all(root.join(".claude"))?;
    std::fs::write(
        root.join(MCP),
        r#"{ "mcpServers": { "other": { "command": "other-mcp" } } }"#,
    )?;
    let settings = json!({
        "model": "sonnet",
        "hooks": {
            "Stop": [{ "hooks": [{ "type": "command", "command": "notify-send done" }] }],
            "SessionStart": [{ "hooks": [
                { "type": "command", "command": "/opt/reef --conf /x hook session-start" }
            ] }],
        },
    });
    std::fs::write(root.join(SETTINGS), settings.to_string())?;
    std::fs::write(
        root.join(LOCAL),
        r#"{ "permissions": { "allow": ["Bash(cargo test:*)"] }, "showClearContextOnPlanAccept": false }"#,
    )?;
    let own = "#!/usr/bin/env bash\nset -e\nexec lefthook run post-commit \"$@\"\n";
    let hook = root.join(".git/hooks/post-commit");
    std::fs::write(&hook, own)?;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700))?;
    install(&root).await?;
    // the other server beside reef's
    let mcp = read_json(&root, MCP);
    assert_eq!(mcp["mcpServers"]["other"]["command"], "other-mcp");
    assert_eq!(mcp["mcpServers"]["reef"]["command"], "reef");
    // the user's Stop hook first and reef's after, and the hand-written SessionStart alone
    let settings = read_json(&root, SETTINGS);
    assert_eq!(settings["model"], "sonnet");
    let stop = settings["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop[0]["hooks"][0]["command"], "notify-send done");
    assert_eq!(stop[1]["hooks"][0]["command"], "reef hook stop");
    assert_eq!(
        settings["hooks"]["SessionStart"],
        json!([{ "hooks": [
            { "type": "command", "command": "/opt/reef --conf /x hook session-start" }
        ] }])
    );
    // the user's rules kept, reef's added, and the clear they turned off left off
    let local = read_json(&root, LOCAL);
    assert_eq!(
        local["permissions"]["allow"],
        json!(["Bash(cargo test:*)", "mcp__reef"])
    );
    assert_eq!(local["showClearContextOnPlanAccept"], json!(false));
    // the shell hook's own lines after reef's block, which its exec can't skip, its mode kept
    let text = std::fs::read_to_string(&hook)?;
    let (head, tail) = text.split_once("# <<< reef\n").expect("reef's block ends");
    assert!(
        head.starts_with(&format!("#!/usr/bin/env bash\n{BEGIN}")),
        "{text}"
    );
    assert!(head.contains(GIT_HOOKS[0].line), "{text}");
    assert_eq!(tail, "set -e\nexec lefthook run post-commit \"$@\"\n");
    let mode = std::fs::metadata(&hook)?.permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    // and run again, nothing changes
    let again = written(&root, &root.join(".git/hooks"));
    assert!(install(&root).await?.ends_with("nothing to change\n"));
    assert_eq!(written(&root, &root.join(".git/hooks")), again);
    Ok(())
}

/// git's hooks go where `core.hooksPath` says git runs them, not in the repo's own hooks dir
#[tokio::test]
async fn hooks_go_where_core_hooks_path_says() -> anyhow::Result<()> {
    // a repo whose hooks are its tracked `.githooks`
    let root = repo("hooks-path");
    git(&root, &["config", "core.hooksPath", ".githooks"]);
    let said = install(&root).await?;
    // written there, and not in `.git/hooks`
    for hook in GIT_HOOKS {
        assert!(
            said.contains(&format!("wrote .githooks/{}:", hook.name)),
            "{said}"
        );
        assert!(root.join(".githooks").join(hook.name).is_file());
        assert!(!root.join(".git/hooks").join(hook.name).exists());
    }
    Ok(())
}

/// A hook that isn't a shell script is left byte for byte, with the line to add it by hand,
/// and the rest is set up anyway
#[tokio::test]
async fn a_hook_that_isnt_shell_is_left_alone() -> anyhow::Result<()> {
    // a repo with a Python post-commit
    let root = repo("python");
    let own = "#!/usr/bin/env python3\nprint('committed')\n";
    let hook = root.join(".git/hooks/post-commit");
    std::fs::write(&hook, own)?;
    let said = install(&root).await?;
    // it is left, and the output names the line
    assert_eq!(std::fs::read_to_string(&hook)?, own);
    assert!(
        said.contains(
            "left .git/hooks/post-commit as it is, since it isn't a shell script: add `reef hook \
             post-commit </dev/null || true` to it"
        ),
        "{said}"
    );
    // while the rest was written
    assert!(said.contains("wrote .git/hooks/post-rewrite:"), "{said}");
    assert!(root.join(MCP).is_file());
    // and a second run says the same and writes nothing
    let again = install(&root).await?;
    assert!(
        again.contains("left .git/hooks/post-commit as it is"),
        "{again}"
    );
    assert!(again.ends_with("nothing to change\n"), "{again}");
    Ok(())
}

/// A file that isn't a JSON object stops install before it writes anything
#[tokio::test]
async fn a_broken_file_writes_nothing() -> anyhow::Result<()> {
    // a repo whose local settings don't parse
    let root = repo("broken");
    std::fs::create_dir_all(root.join(".claude"))?;
    std::fs::write(root.join(LOCAL), "{ not json")?;
    let error = install(&root).await.expect_err("a broken file was merged");
    assert!(
        format!("{error:#}").contains("settings.local.json as JSON"),
        "{error:#}"
    );
    // and nothing else was written
    assert!(!root.join(MCP).exists());
    assert!(!root.join(SETTINGS).exists());
    assert!(!root.join(".git/hooks/post-commit").exists());
    assert!(!root.join(PLACE).exists());
    Ok(())
}

/// The git hooks install writes run reef, in a linked worktree too: a commit runs
/// `reef hook post-commit`, and an amend that and `reef hook post-rewrite amend`
#[tokio::test]
async fn the_git_hooks_run_reef() -> anyhow::Result<()> {
    // a repo set up, and a linked worktree of it
    let root = repo("run");
    install(&root).await?;
    let linked = root.parent().unwrap().join("claude-run-linked");
    if linked.exists() {
        std::fs::remove_dir_all(&linked)?;
    }
    git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );
    // a `reef` first on the PATH that logs what it was run with and reads what it is handed
    let bin = scratch("run-bin");
    let log = bin.join("reef.log");
    let fake = bin.join("reef");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nprintf '%s|%s\\n' \"$*\" \"$(cat)\" >> '{}'\n",
            log.display()
        ),
    )?;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))?;
    let path = format!("{}:{}", bin.display(), std::env::var("PATH")?);
    // a commit, then an amend, in the linked worktree
    std::fs::write(linked.join("a.txt"), "a\n")?;
    git_with(&linked, &["add", "a.txt"], Some(&path));
    git_with(&linked, &["commit", "-q", "-m", "a"], Some(&path));
    git_with(
        &linked,
        &["commit", "-q", "--amend", "-m", "a, amended"],
        Some(&path),
    );
    // each ran reef's hook command, its standard input closed
    let ran = std::fs::read_to_string(&log)?;
    assert_eq!(
        ran,
        "hook post-commit|\nhook post-commit|\nhook post-rewrite amend|\n"
    );
    Ok(())
}
