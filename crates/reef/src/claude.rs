//! A repo set up for the loop: reef's MCP entry, Claude Code's and git's hooks, the local
//! settings and the skill, all written by one command
//!
//! ```text
//! reef claude install [DIR]
//! ```
//!
//! - **Committed, so every worktree has them:** `.mcp.json` runs `reef mcp` as the server
//!   `reef`, and `.claude/settings.json` runs `reef hook …` on SessionStart, Stop, SessionEnd,
//!   PostToolUse after reef's status tool, WorktreeCreate and WorktreeRemove. A group's member is
//!   checked out from HEAD, so these reach its session once they are committed, as the skill
//!   does.
//! - **Local, so trust stays the user's:** `.claude/settings.local.json` enables the server and
//!   allows its tools, which skips both prompts once the folder is trusted, as X4 found, and
//!   offers to clear the planning's context when a plan is approved. It goes in the repo's
//!   `info/exclude`, and reef's WorktreeCreate copies it into each member's worktree.
//! - **git's hooks where git runs them:** post-commit and post-rewrite, in the directory
//!   `git rev-parse --git-path hooks` names, `core.hooksPath`'s if it is set, else the common
//!   dir's, which every worktree runs. reef's lines go in a marked block. A shell script there
//!   already keeps its own lines, with reef's block put just after its shebang so an `exec`
//!   later on can't skip it. Anything else is left as it is, with the line to add said.
//! - **Running it again changes nothing.** What is reef's is found and left, however its command
//!   was written, so a hook pointed at another config by hand stays as it is. A file is written
//!   only when something in it changes, and every file is read before any is written, so one
//!   that can't be read leaves the rest as they were.
//! - **It needs no service,** since it only writes files, as `reef skill` does.

use anyhow::Context;
use serde_json::{Map, Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::git::Checkout;
use crate::skill;

/// The MCP server's name, which names its tools `mcp__reef__*`
pub const SERVER: &str = "reef";

/// The MCP entry, from the checkout's root
pub const MCP: &str = ".mcp.json";

/// The committed settings that hold the hooks, from the checkout's root
pub const SETTINGS: &str = ".claude/settings.json";

/// The local settings that enable the server and allow its tools, from the checkout's root
pub const LOCAL: &str = ".claude/settings.local.json";

/// The pattern that keeps the local settings out of git, in every worktree
pub const LOCAL_EXCLUDED: &str = "/.claude/settings.local.json";

/// The allow rule for every one of the server's tools
pub const ALLOW: &str = "mcp__reef";

/// The setting that offers to clear the planning's context when a plan is approved
pub const CLEAR_ON_ACCEPT: &str = "showClearContextOnPlanAccept";

/// The line a git hook's block of reef's starts with
const BEGIN: &str = "# >>> reef";

/// The line a git hook's block of reef's ends with
const END: &str = "# <<< reef";

/// One of Claude Code's hooks reef runs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaudeHook {
    /// The event it runs on, as Claude Code names it
    pub event: &'static str,
    /// The tools it is matched on, for a tool's event
    pub matcher: Option<&'static str>,
    /// reef's hook command it runs, after `reef hook`
    pub sub: &'static str,
}

impl ClaudeHook {
    /// The command it runs
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::claude::HOOKS;
    ///
    /// assert_eq!(HOOKS[1].command(), "reef hook stop");
    /// ```
    pub fn command(&self) -> String {
        format!("reef hook {}", self.sub)
    }
}

/// Claude Code's hooks reef runs, in the order written
pub const HOOKS: &[ClaudeHook] = &[
    ClaudeHook {
        event: "SessionStart",
        matcher: None,
        sub: "session-start",
    },
    ClaudeHook {
        event: "Stop",
        matcher: None,
        sub: "stop",
    },
    ClaudeHook {
        event: "SessionEnd",
        matcher: None,
        sub: "session-end",
    },
    ClaudeHook {
        event: "PostToolUse",
        matcher: Some("mcp__reef__status"),
        sub: "post-tool-use",
    },
    ClaudeHook {
        event: "WorktreeCreate",
        matcher: None,
        sub: "worktree-create",
    },
    ClaudeHook {
        event: "WorktreeRemove",
        matcher: None,
        sub: "worktree-remove",
    },
];

/// One of git's hooks reef runs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitHook {
    /// The hook's file name, as git runs it
    pub name: &'static str,
    /// The line of reef's block that runs it
    ///
    /// Its standard input is closed, so the hook's own lines read what git writes them, and a
    /// failure never stops them.
    pub line: &'static str,
}

/// git's hooks reef runs
pub const GIT_HOOKS: &[GitHook] = &[
    GitHook {
        name: "post-commit",
        line: "reef hook post-commit </dev/null || true",
    },
    GitHook {
        name: "post-rewrite",
        line: "reef hook post-rewrite \"$1\" </dev/null || true",
    },
];

/// What install did to one file
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Written, new or changed
    Wrote,
    /// Set up already, so left as it was
    Kept,
    /// Not one reef can add to, so left as it was
    Left {
        /// Why it was left
        why: String,
        /// The line to add to it by hand
        line: String,
    },
}

/// One file install set up
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The file, absolute
    pub path: PathBuf,
    /// What it holds of reef's, to say once it is written
    pub holds: String,
    /// What install did to it
    pub outcome: Outcome,
}

/// What install did, file by file
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The root of the checkout set up
    pub root: PathBuf,
    /// Each file, in the order set up
    pub steps: Vec<Step>,
}

impl Installed {
    /// Whether any file was written
    pub fn changed(&self) -> bool {
        // any step that wrote
        self.steps.iter().any(|step| step.outcome == Outcome::Wrote)
    }
}

/// A file's text as install leaves it, before anything is written
struct Planned {
    /// The file, absolute
    path: PathBuf,
    /// What it holds of reef's, to say once it is written
    holds: String,
    /// The text to write, if it changes
    text: Option<String>,
    /// Whether it is a new hook, and so made executable once written
    executable: bool,
    /// Why it is left as it is, with the line to add by hand, if it is
    left: Option<(String, String)>,
}

/// Sets the repo that holds a directory up for the loop, and says what it did
///
/// # Arguments
///
/// * `dir` - A directory in the checkout to set up
/// * `out` - Where to say what it did
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// # fn example() -> anyhow::Result<()> {
/// let installed = reef::claude::install(Path::new("."), &mut std::io::stdout())?;
/// assert!(installed.root.join(".mcp.json").exists());
/// # Ok(())
/// # }
/// ```
pub fn install<W: Write>(dir: &Path, out: &mut W) -> anyhow::Result<Installed> {
    // the checkout, which must be a repo's for git's hooks to go anywhere
    let checkout = Checkout::open(dir)?;
    let root = checkout.root().to_path_buf();
    // every file read and merged before any is written, so a bad one leaves all as they were
    let mut planned = vec![
        planned_json(
            &root.join(MCP),
            "the reef MCP server, `reef mcp`".to_string(),
            with_server,
        )?,
        planned_json(&root.join(SETTINGS), hooks_held(), with_hooks)?,
        planned_json(
            &root.join(LOCAL),
            "reef's server enabled, its tools allowed, and a clear offered at a plan's approval"
                .to_string(),
            with_local,
        )?,
    ];
    let hooks = checkout.hooks_dir()?;
    for hook in GIT_HOOKS {
        planned.push(planned_hook(&hooks.join(hook.name), hook)?);
    }
    // then each that changes is written
    let mut steps = Vec::with_capacity(planned.len() + 1);
    for plan in planned {
        let outcome = write_planned(&plan)?;
        steps.push(Step {
            path: plan.path,
            holds: plan.holds,
            outcome,
        });
    }
    // the local settings stay out of every worktree's snapshots and commits
    checkout.exclude(LOCAL_EXCLUDED)?;
    // and the skill, unless it is the one this reef writes already
    let place = skill::place(&root);
    let outcome = match skill::is_written(&place) {
        true => Outcome::Kept,
        false => {
            skill::write(&root, &mut std::io::sink())?;
            Outcome::Wrote
        }
    };
    steps.push(Step {
        path: place,
        holds: "the reef skill".to_string(),
        outcome,
    });
    let installed = Installed { root, steps };
    say(&installed, out)?;
    Ok(installed)
}

/// What the committed settings hold of reef's, to say once they are written
fn hooks_held() -> String {
    // each event by name, in the order written
    let events: Vec<&str> = HOOKS.iter().map(|hook| hook.event).collect();
    let (last, rest) = events.split_last().expect("reef runs some hooks");
    format!("reef's hooks on {} and {last}", rest.join(", "))
}

/// Reads a JSON file as an object, merges reef's part into it, and plans what to write
///
/// # Arguments
///
/// * `path` - The file, which may not be there yet
/// * `holds` - What it holds of reef's, to say once it is written
/// * `merge` - What adds reef's part, saying whether anything changed
fn planned_json<F>(path: &Path, holds: String, merge: F) -> anyhow::Result<Planned>
where
    F: FnOnce(&mut Map<String, Value>) -> anyhow::Result<bool>,
{
    // the object there, else an empty one
    let kept = read_text(path)?;
    let mut object = match kept.as_deref().map(str::trim) {
        None | Some("") => Map::new(),
        Some(text) => match serde_json::from_str::<Value>(text)
            .with_context(|| format!("reading {} as JSON", path.display()))?
        {
            Value::Object(object) => object,
            _ => anyhow::bail!("{} holds no JSON object", path.display()),
        },
    };
    // merged, and written only if that changed it
    let changed = merge(&mut object).with_context(|| format!("setting {} up", path.display()))?
        || kept.is_none();
    let text = match changed {
        true => {
            let mut text = serde_json::to_string_pretty(&Value::Object(object))?;
            text.push('\n');
            Some(text)
        }
        false => None,
    };
    Ok(Planned {
        path: path.to_path_buf(),
        holds,
        text,
        executable: false,
        left: None,
    })
}

/// Plans what a git hook's file becomes with reef's block in it
///
/// # Arguments
///
/// * `path` - The hook's file, which may not be there yet
/// * `hook` - The hook
fn planned_hook(path: &Path, hook: &GitHook) -> anyhow::Result<Planned> {
    // its text as it is, then as reef leaves it
    let kept = read_text(path)?;
    let (text, left) = match hooked(kept.as_deref(), hook.line) {
        Hooked::Write(text) => (Some(text), None),
        Hooked::Kept => (None, None),
        Hooked::Left(why) => (None, Some((why, hook.line.to_string()))),
    };
    Ok(Planned {
        path: path.to_path_buf(),
        holds: format!("git's {} running `reef hook {}`", hook.name, hook.name),
        text,
        executable: kept.is_none(),
        left,
    })
}

/// Writes a planned file if it changes, and says what came of it
///
/// # Arguments
///
/// * `plan` - The file's plan
fn write_planned(plan: &Planned) -> anyhow::Result<Outcome> {
    // a file left as it is says why, and one unchanged is kept
    if let Some((why, line)) = &plan.left {
        return Ok(Outcome::Left {
            why: why.clone(),
            line: line.clone(),
        });
    }
    let Some(text) = &plan.text else {
        return Ok(Outcome::Kept);
    };
    // its directory first, since `.claude` or a hooks dir may not be there yet
    if let Some(parent) = plan.path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // writing over a file keeps its mode, so only a new hook is made executable
    std::fs::write(&plan.path, text).with_context(|| format!("writing {}", plan.path.display()))?;
    if plan.executable {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&plan.path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(Outcome::Wrote)
}

/// Reads a file's text, or `None` if it isn't there
///
/// # Arguments
///
/// * `path` - The file
fn read_text(path: &Path) -> anyhow::Result<Option<String>> {
    // a file that isn't there is one to make
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// Says what install did, a line a file, then what is left to do
///
/// # Arguments
///
/// * `installed` - What it did
/// * `out` - Where to say it
fn say<W: Write>(installed: &Installed, out: &mut W) -> anyhow::Result<()> {
    // each path from the checkout's root, where it is under it
    let shown = |path: &Path| {
        path.strip_prefix(&installed.root)
            .unwrap_or(path)
            .display()
            .to_string()
    };
    for step in &installed.steps {
        match &step.outcome {
            Outcome::Wrote => writeln!(out, "wrote {}: {}", shown(&step.path), step.holds)?,
            Outcome::Kept => writeln!(out, "{} is set up already", shown(&step.path))?,
            Outcome::Left { why, line } => writeln!(
                out,
                "left {} as it is, since {why}: add `{line}` to it",
                shown(&step.path)
            )?,
        }
    }
    // what a member's worktree checks out must be committed
    let committed = [MCP, SETTINGS, skill::PLACE];
    let wrote = installed.steps.iter().any(|step| {
        step.outcome == Outcome::Wrote
            && committed
                .iter()
                .any(|path| step.path == installed.root.join(path))
    });
    match (wrote, installed.changed()) {
        (true, _) => writeln!(
            out,
            "commit {MCP}, {SETTINGS} and {}, so every worktree checks them out",
            skill::PLACE
        )?,
        (false, true) => {}
        (false, false) => writeln!(out, "nothing to change")?,
    }
    Ok(())
}

/// Adds the reef server to an MCP entry's servers, unless one of its name is there
///
/// Returns whether it added it. A `reef` server there already is left as it is, so one pointed
/// at another config by hand stays.
///
/// # Arguments
///
/// * `mcp` - The `.mcp.json` object
fn with_server(mcp: &mut Map<String, Value>) -> anyhow::Result<bool> {
    // the servers' object, made if it isn't there
    let servers = object_at(mcp, "mcpServers")?;
    if servers.contains_key(SERVER) {
        return Ok(false);
    }
    servers.insert(
        SERVER.to_string(),
        json!({ "type": "stdio", "command": "reef", "args": ["mcp"] }),
    );
    Ok(true)
}

/// Adds each of reef's hooks to the settings' hooks, unless its event runs reef's already
///
/// Returns whether it added any. reef's group goes after the user's own.
///
/// # Arguments
///
/// * `settings` - The `.claude/settings.json` object
fn with_hooks(settings: &mut Map<String, Value>) -> anyhow::Result<bool> {
    // the hooks' object, made if it isn't there
    let events = object_at(settings, "hooks")?;
    let mut changed = false;
    for hook in HOOKS {
        // the event's groups, made if they aren't there
        let groups = events
            .entry(hook.event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("`hooks.{}` isn't a list", hook.event))?;
        if groups.iter().any(|group| runs_reefs(group, hook.sub)) {
            continue;
        }
        // reef's own group, matched on its tools where it has a matcher
        let mut group = Map::new();
        if let Some(matcher) = hook.matcher {
            group.insert("matcher".to_string(), json!(matcher));
        }
        group.insert(
            "hooks".to_string(),
            json!([{ "type": "command", "command": hook.command() }]),
        );
        groups.push(Value::Object(group));
        changed = true;
    }
    Ok(changed)
}

/// Enables the reef server, allows its tools and offers a clear at a plan's approval, each
/// unless it is set already
///
/// Returns whether it set any. A clear the user turned off stays off.
///
/// # Arguments
///
/// * `local` - The `.claude/settings.local.json` object
fn with_local(local: &mut Map<String, Value>) -> anyhow::Result<bool> {
    // the server enabled, so the MCP entry asks nothing
    let mut changed = with_listed(local, "enabledMcpjsonServers", SERVER)?;
    // its tools allowed, so none of them asks
    let permissions = object_at(local, "permissions")?;
    changed |= with_listed(permissions, "allow", ALLOW)?;
    // and the clear offered, unless the user said otherwise
    if !local.contains_key(CLEAR_ON_ACCEPT) {
        local.insert(CLEAR_ON_ACCEPT.to_string(), Value::Bool(true));
        changed = true;
    }
    Ok(changed)
}

/// Adds a string to a list under a key, unless it is in it, and says whether it added it
///
/// # Arguments
///
/// * `object` - The object the list is in
/// * `key` - The list's key, made if it isn't there
/// * `item` - The string
fn with_listed(object: &mut Map<String, Value>, key: &str, item: &str) -> anyhow::Result<bool> {
    // the list, made if it isn't there
    let list = object
        .entry(key)
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("`{key}` isn't a list"))?;
    if list.iter().any(|kept| kept.as_str() == Some(item)) {
        return Ok(false);
    }
    list.push(json!(item));
    Ok(true)
}

/// The object under a key, made if it isn't there
///
/// # Arguments
///
/// * `object` - The object it is in
/// * `key` - Its key
fn object_at<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> anyhow::Result<&'a mut Map<String, Value>> {
    // made empty if it isn't there, and refused if it is something else
    object
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("`{key}` isn't an object"))
}

/// Whether a hook group runs one of reef's hook commands
///
/// # Arguments
///
/// * `group` - The group, a matcher and its hooks
/// * `sub` - reef's hook command, after `reef hook`
fn runs_reefs(group: &Value, sub: &str) -> bool {
    // any of its hooks' commands
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| is_reefs(command, sub))
            })
        })
}

/// Whether a command runs one of reef's hook commands, however reef was named or configured
///
/// Variables set before it are passed over, the program is any path to `reef`, and its flags
/// may come before `hook <sub>`.
///
/// # Arguments
///
/// * `command` - The command, as a hook runs it
/// * `sub` - reef's hook command, after `reef hook`
///
/// # Examples
///
/// ```
/// use reef::claude::is_reefs;
///
/// assert!(is_reefs("reef hook stop", "stop"));
/// assert!(is_reefs("REEF_CONFIG=/x /home/me/.cargo/bin/reef --conf /x hook stop", "stop"));
/// assert!(!is_reefs("reef hook stop", "session-start"));
/// assert!(!is_reefs("reefer hook stop", "stop"));
/// ```
pub fn is_reefs(command: &str, sub: &str) -> bool {
    // past any variables set for it, to the program it runs
    let mut words = command
        .split_whitespace()
        .skip_while(|word| word.contains('=') && !word.starts_with('-'));
    let program = words.next().map(Path::new).and_then(Path::file_name);
    if program.is_none_or(|name| name != "reef") {
        return false;
    }
    // then `hook <sub>` somewhere after its flags
    let rest: Vec<&str> = words.collect();
    rest.windows(2)
        .any(|pair| pair[0] == "hook" && pair[1] == sub)
}

/// What a git hook's text becomes with reef's block in it
#[derive(Debug, Clone, PartialEq, Eq)]
enum Hooked {
    /// This text, new or changed
    Write(String),
    /// Set up already
    Kept,
    /// Not one reef can add to, and why
    Left(String),
}

/// The block of reef's that runs a line, each of its lines ended
///
/// # Arguments
///
/// * `line` - The line it runs
fn block(line: &str) -> String {
    format!("{BEGIN}: written by `reef claude install`, which rewrites this block\n{line}\n{END}\n")
}

/// What a git hook's text becomes with reef's block in it
///
/// # Arguments
///
/// * `kept` - Its text, if the file is there
/// * `line` - The line reef's block runs
fn hooked(kept: Option<&str>, line: &str) -> Hooked {
    // no hook, or an empty one, is reef's alone
    let Some(text) = kept.filter(|text| !text.trim().is_empty()) else {
        return Hooked::Write(format!("#!/bin/sh\n{}", block(line)));
    };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    // reef's block there already is written again in place
    if let Some(begin) = lines.iter().position(|kept| kept.starts_with(BEGIN)) {
        let Some(end) = lines[begin..]
            .iter()
            .position(|kept| kept.trim_end() == END)
        else {
            return Hooked::Left(format!("its block of reef's has no `{END}` line"));
        };
        let mut written: String = lines[..begin].concat();
        written.push_str(&block(line));
        written.push_str(&lines[begin + end + 1..].concat());
        return match written == text {
            true => Hooked::Kept,
            false => Hooked::Write(written),
        };
    }
    // else a shell script takes the block just after its shebang
    if !is_shell(lines[0]) {
        return Hooked::Left("it isn't a shell script".to_string());
    }
    let mut written = lines[0].to_string();
    if !written.ends_with('\n') {
        written.push('\n');
    }
    written.push_str(&block(line));
    written.push_str(&lines[1..].concat());
    Hooked::Write(written)
}

/// Whether a script's first line is a shebang naming a POSIX shell
///
/// # Arguments
///
/// * `first` - The script's first line
fn is_shell(first: &str) -> bool {
    // the interpreter, or the one `env` runs, past any of its flags
    let Some(shebang) = first.strip_prefix("#!") else {
        return false;
    };
    let mut words = shebang.split_whitespace();
    let mut program = words.next().map(Path::new).and_then(Path::file_name);
    if program.is_some_and(|name| name == "env") {
        program = words
            .find(|word| !word.starts_with('-'))
            .map(Path::new)
            .and_then(Path::file_name);
    }
    program
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "sh" | "bash" | "dash" | "zsh" | "ksh"))
}

#[cfg(test)]
mod tests {
    use super::{
        BEGIN, END, GIT_HOOKS, HOOKS, Hooked, hooked, is_reefs, is_shell, with_hooks, with_local,
        with_server,
    };
    use serde_json::{Map, Value, json};

    /// An object from JSON written inline
    ///
    /// # Arguments
    ///
    /// * `value` - The JSON, which must be an object
    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(object) => object,
            other => panic!("{other} is no object"),
        }
    }

    /// reef's hook commands are found however reef is named, and no others are
    #[test]
    fn reefs_commands_are_found_however_reef_is_named() {
        // bare, by path, configured and with variables set
        assert!(is_reefs("reef hook post-tool-use", "post-tool-use"));
        assert!(is_reefs("/usr/local/bin/reef hook stop", "stop"));
        assert!(is_reefs("reef --conf /tmp/r.toml hook stop", "stop"));
        assert!(is_reefs("A=1 B=2 reef hook stop --pane tmux:/s:%1", "stop"));
        // and not another command, another hook or another program
        assert!(!is_reefs("reef hook stop", "session-start"));
        assert!(!is_reefs("reef mcp", "stop"));
        assert!(!is_reefs("echo reef hook stop", "stop"));
        assert!(!is_reefs("", "stop"));
    }

    /// The server is added once, beside the others, and one of its name is left as it is
    #[test]
    fn the_server_is_added_once_beside_the_others() {
        // beside another server
        let mut mcp = object(json!({ "mcpServers": { "other": { "command": "other" } } }));
        assert!(with_server(&mut mcp).unwrap());
        assert_eq!(mcp["mcpServers"]["other"]["command"], "other");
        assert_eq!(mcp["mcpServers"]["reef"]["args"], json!(["mcp"]));
        // and not again
        assert!(!with_server(&mut mcp).unwrap());
        // one written by hand is kept
        let mut mcp = object(json!({ "mcpServers": { "reef": { "command": "/x/reef" } } }));
        assert!(!with_server(&mut mcp).unwrap());
        assert_eq!(mcp["mcpServers"]["reef"], json!({ "command": "/x/reef" }));
        // and a servers' entry that isn't an object is refused
        let mut mcp = object(json!({ "mcpServers": [] }));
        assert!(with_server(&mut mcp).is_err());
    }

    /// Each hook is added once, after the user's own, and one of reef's there already is kept
    #[test]
    fn each_hook_is_added_once_after_the_users() {
        // the user's own Stop hook, and reef's Stop hook pointed at another config by hand
        let mut settings = object(json!({
            "model": "sonnet",
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "notify-send done" }] }],
                "SessionStart": [{ "hooks": [
                    { "type": "command", "command": "reef --conf /x hook session-start" }
                ] }],
            },
        }));
        assert!(with_hooks(&mut settings).unwrap());
        // the user's kept first, reef's after, and the hand-written one alone
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "notify-send done");
        assert_eq!(stop[1]["hooks"][0]["command"], "reef hook stop");
        assert_eq!(
            settings["hooks"]["SessionStart"].as_array().unwrap().len(),
            1
        );
        assert_eq!(settings["model"], "sonnet");
        // PostToolUse matched on reef's status tool alone
        let used = &settings["hooks"]["PostToolUse"][0];
        assert_eq!(used["matcher"], "mcp__reef__status");
        // every event has one of reef's, and a second merge changes nothing
        for hook in HOOKS {
            assert!(settings["hooks"][hook.event].is_array(), "{}", hook.event);
        }
        let before = settings.clone();
        assert!(!with_hooks(&mut settings).unwrap());
        assert_eq!(settings, before);
    }

    /// The server is enabled and its tools allowed once, and a clear turned off stays off
    #[test]
    fn the_local_settings_are_set_once() {
        // beside the user's own rules
        let mut local = object(json!({ "permissions": { "allow": ["Bash(cargo test:*)"] } }));
        assert!(with_local(&mut local).unwrap());
        assert_eq!(
            local["permissions"]["allow"],
            json!(["Bash(cargo test:*)", "mcp__reef"])
        );
        assert_eq!(local["enabledMcpjsonServers"], json!(["reef"]));
        assert_eq!(local["showClearContextOnPlanAccept"], json!(true));
        assert!(!with_local(&mut local).unwrap());
        // the clear turned off by the user is left off
        let mut local = object(json!({ "showClearContextOnPlanAccept": false }));
        assert!(with_local(&mut local).unwrap());
        assert_eq!(local["showClearContextOnPlanAccept"], json!(false));
        // and a list that isn't one is refused
        let mut local = object(json!({ "enabledMcpjsonServers": "reef" }));
        assert!(with_local(&mut local).is_err());
    }

    /// Shell scripts are told by their shebang, `env`'s included
    #[test]
    fn shell_scripts_are_told_by_their_shebang() {
        for first in [
            "#!/bin/sh\n",
            "#!/bin/bash -e\n",
            "#!/usr/bin/env sh\n",
            "#!/usr/bin/env -S bash -e\n",
            "#! /bin/dash",
        ] {
            assert!(is_shell(first), "{first}");
        }
        for first in [
            "#!/usr/bin/env python3\n",
            "#!/usr/bin/perl\n",
            "echo hi\n",
            "",
        ] {
            assert!(!is_shell(first), "{first}");
        }
    }

    /// A hook gets reef's block once: alone in a new one, after a shell script's shebang, in
    /// place where it is there already, and nowhere in anything else
    #[test]
    fn a_hook_gets_reefs_block_once() {
        let line = GIT_HOOKS[0].line;
        // a new hook is reef's block alone
        let Hooked::Write(fresh) = hooked(None, line) else {
            panic!("no hook was written");
        };
        assert!(fresh.starts_with(&format!("#!/bin/sh\n{BEGIN}")), "{fresh}");
        assert!(fresh.ends_with(&format!("{line}\n{END}\n")), "{fresh}");
        assert_eq!(hooked(Some(&fresh), line), Hooked::Kept);
        // a shell script takes it just after its shebang, before its own exec
        let own = "#!/bin/bash\nset -e\nexec lefthook run post-commit \"$@\"\n";
        let Hooked::Write(added) = hooked(Some(own), line) else {
            panic!("the script took no block");
        };
        assert!(
            added.starts_with(&format!("#!/bin/bash\n{BEGIN}")),
            "{added}"
        );
        assert!(added.ends_with(&format!(
            "{END}\nset -e\nexec lefthook run post-commit \"$@\"\n"
        )));
        assert_eq!(hooked(Some(&added), line), Hooked::Kept);
        // a block of an older reef's is written again in place
        let older = added.replace(line, "reef hook post-commit");
        assert_eq!(hooked(Some(&older), line), Hooked::Write(added.clone()));
        // anything else is left
        assert!(matches!(
            hooked(Some("#!/usr/bin/env python3\nprint('hi')\n"), line),
            Hooked::Left(_)
        ));
        let broken = format!("#!/bin/sh\n{BEGIN}\n{line}\n");
        assert!(matches!(hooked(Some(&broken), line), Hooked::Left(_)));
    }
}
