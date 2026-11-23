//! reef's commands for the terminal loop: which pane feeds a project, and whether it is paused
//!
//! ```text
//! reef loop start [--plan NAME] [--pane KEY] [--session ID] [--width N] [--no-open]
//!                                                   # start, resume or move on the loop
//! reef loop pause [--pane KEY]                      # stop once its node is done
//! reef loop stop [--pane KEY]                       # end it, letting go of its node
//! reef loop ls                                      # the project's loops
//! reef loop attach [ID] [--pane KEY]                # work a group's member in this session
//! reef hook session-start [--pane KEY]              # Claude Code's SessionStart hook
//! ```
//!
//! - **A loop is its pane's.** Every `/clear` starts a new session with a new id, so a loop is
//!   kept by the pane its session runs in: zellij's or tmux's, else the Claude Code process,
//!   which Claude Code names to every command and hook it runs as `CLAUDE_PID`. So `reef loop`
//!   is run in the session's own pane, as `! reef loop start`, or names one with `--pane`.
//! - **The session is the pane's.** A start in the session's own pane records the session that
//!   ran it, `CLAUDE_CODE_SESSION_ID`, and the SessionStart hook hands the pane's loop to each
//!   session that starts there, a clear's included, so the loop follows its pane. A clear
//!   between nodes is told which node is next ([`between`]).
//! - **Every plan, or one.** A loop takes the project's ready work across its plans, unless
//!   `--plan` names one.
//! - **A start between nodes moves on.** Stop acts on a node's end once, so after a paused
//!   loop's node, or once the injector stepped aside, a start in the session's own pane hands off
//!   as Stop did: in zellij or tmux it starts the injector, and elsewhere it says `/clear`, then
//!   `next`. A start naming a pane types nothing into it, since its session isn't known.
//! - **As wide as it is started.** `--width N` has a loop take up to N ready nodes at once, its
//!   own session working the first, and each other one, a member, worked by a session of its
//!   own in its own worktree. reef opens their panes in zellij or tmux unless `--no-open` says
//!   not to ([`crate::open`]).
//! - **An open session can work a member.** `! reef loop attach [ID]` in a session with no loop
//!   of its own makes the member's worktree, starts the node there, joins the pane to it, and
//!   tells Claude to enter that worktree with EnterWorktree before it takes the node.

use reef_api::{EditLoop, Loop, LoopEdit, LoopEdited, LoopState, Pane};
use reef_model::NodeName;
use serde::Deserialize;
use std::io::Write;
use std::path::Path;

use crate::clear::{self, Clear};
use crate::cli::{self, Pick, PlanPick};
use crate::client::Client;
use crate::task::{self, Ending};
use crate::worktree;

/// The pane a loop command works on, and the session running there if it is this one
pub struct Here {
    /// The pane
    pub pane: Pane,
    /// The session running in it, when the pane is this process's own and Claude Code named it
    pub session: Option<String>,
    /// The Claude Code process the session runs in, `CLAUDE_PID`, when the pane is this
    /// process's own and Claude Code named it
    pub pid: Option<u32>,
}

impl Here {
    /// The pane named, else the one this process runs in, found from its environment
    ///
    /// A session and its Claude Code process are only known for this process's own pane, never
    /// for one named.
    ///
    /// # Arguments
    ///
    /// * `named` - The pane's key as typed, if one was
    /// * `var` - Reads one of this process's environment variables
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::loops::Here;
    ///
    /// let env = |name: &str| (name == "CLAUDE_CODE_SESSION_ID").then(|| "s1".to_string());
    /// let here = Here::find(Some("claude:4242"), env).unwrap();
    /// assert_eq!((here.pane.to_string(), here.session), ("claude:4242".to_string(), None));
    /// assert_eq!(here.pid, None);
    /// ```
    pub fn find<F: Fn(&str) -> Option<String>>(
        named: Option<&str>,
        var: F,
    ) -> anyhow::Result<Here> {
        // a pane named is that one, whose session this process can't know
        if let Some(named) = named {
            return Ok(Here {
                pane: named.parse()?,
                session: None,
                pid: None,
            });
        }
        // else this process's own, and the session Claude Code ran it from
        let pane = Pane::from_env(&var).ok_or_else(|| {
            anyhow::anyhow!(
                "reef can't tell which pane this is: no zellij or tmux pane, and no Claude Code \
                 session ran it. Run it in the session's own pane, as `! reef loop ...`, or name \
                 one with `--pane`, as `reef loop ls` prints it"
            )
        })?;
        let session = var("CLAUDE_CODE_SESSION_ID").filter(|id| !id.trim().is_empty());
        // and the Claude Code process it runs in, which the injector asks whether it is idle
        let pid = var("CLAUDE_PID").and_then(|pid| pid.trim().parse().ok());
        Ok(Here { pane, session, pid })
    }
}

/// The pane an MCP server Claude Code started runs in: its multiplexer's, else the Claude Code
/// process, named by `CLAUDE_PID` or else by being the server's parent
///
/// Claude Code hands the processes it starts its pane's variables, but `CLAUDE_PID` only to its
/// Bash commands and hooks. It starts each MCP server itself, so the server's parent is the
/// process `CLAUDE_PID` names to the session's Bash.
///
/// # Arguments
///
/// * `var` - Reads one of the server's environment variables
/// * `parent` - The server's parent process's id
///
/// # Examples
///
/// ```
/// use reef::loops::server_pane;
/// use reef_api::Pane;
///
/// let pane = server_pane(|_| None, 4242);
/// assert_eq!(pane, Some(Pane::Claude { pid: 4242 }));
/// ```
pub fn server_pane<F: Fn(&str) -> Option<String>>(var: F, parent: u32) -> Option<Pane> {
    // the multiplexer's pane, or the process its variables name
    if let Some(pane) = Pane::from_env(var) {
        return Some(pane);
    }
    // else the process that started the server, unless the server was orphaned to init
    (parent > 1).then_some(Pane::Claude { pid: parent })
}

/// What a start asks of a pane's loop beyond the project it feeds
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Starts {
    /// The session feeding it, named on the command line
    pub session: Option<String>,
    /// How many nodes it works at once, or `None` to keep its width, one for a new loop
    pub width: Option<u8>,
    /// Whether reef opens its members' panes, or `None` to keep what it does
    pub opens: Option<bool>,
}

/// What Claude Code's SessionStart hook is handed on standard input, the parts reef reads
#[derive(Debug, Deserialize)]
struct SessionStarted {
    /// The session that started, a clear's new one included
    session_id: String,
    /// Why it started: `startup`, `resume`, `clear` or `compact`
    #[serde(default)]
    source: String,
}

/// Starts a loop in a pane feeding the picked project, or one plan of it, or sets the pane's
/// paused loop running again, moving on to the next node when it is between nodes
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which picks the project
/// * `here` - The pane, and the session and Claude Code process running in it if known
/// * `starts` - The session named, the width and whether it opens its members' panes
/// * `pick` - The project and plan named, if they were
/// * `spawn` - Starts the injector in the background, as [`crate::clear::spawn`] does
/// * `out` - Where to print what came of it
pub async fn start<W, S>(
    client: &Client,
    cwd: &Path,
    here: &Here,
    starts: &Starts,
    pick: &PlanPick,
    spawn: S,
    out: &mut W,
) -> anyhow::Result<()>
where
    W: Write,
    S: FnOnce(&Clear) -> anyhow::Result<()>,
{
    // the project the working directory belongs to, and a plan only if one was named
    let project = cli::picked(client, cwd, &pick.project).await?;
    if starts.width == Some(0) {
        anyhow::bail!("a loop works at least one node at a time: `--width` takes 1 or more");
    }
    let edit = LoopEdit::Start {
        project: project.name.clone(),
        plan: pick.plan.clone(),
        session: starts.session.clone().or(here.session.clone()),
        width: starts.width,
        opens: starts.opens,
        pid: here.pid,
    };
    match edited(client, &here.pane, edit).await? {
        LoopEdited::Started { r#loop } => {
            writeln!(
                out,
                "started a loop in {}, feeding it {}{}",
                r#loop.pane.describe(),
                feeds(&r#loop),
                wide(&r#loop)
            )?;
            Ok(())
        }
        LoopEdited::Resumed { r#loop } => {
            writeln!(
                out,
                "the loop in {} is running, feeding it {}{}{}",
                r#loop.pane.describe(),
                feeds(&r#loop),
                wide(&r#loop),
                holding(&r#loop)
            )?;
            // between nodes, it moves on as the Stop that ended its node would have
            move_on(client, &r#loop, here, spawn, out).await
        }
        // a pane working another loop's member runs that session, not a loop
        LoopEdited::Busy { r#loop } if r#loop.pane != here.pane => anyhow::bail!(
            "{} works a member of the group of the loop in {}: a loop starts in a pane of its \
             own",
            here.pane.describe(),
            r#loop.pane.describe()
        ),
        LoopEdited::Busy { r#loop } => anyhow::bail!(
            "{} has a loop already, feeding it {}: `reef loop stop` ends it first",
            r#loop.pane.describe(),
            feeds(&r#loop)
        ),
        other => unexpected(&here.pane, other),
    }
}

/// Moves a loop that a start set running on to its next node, if it is between nodes, as the
/// Stop that ended its node does: the injector started where reef can type into the session,
/// else the user told to `/clear`, then say `next`
///
/// A loop mid-node, or one holding nothing, is left to its session.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `feeding` - The pane's loop, set running
/// * `here` - The pane, and the session and Claude Code process running in it if known
/// * `spawn` - Starts the injector in the background
/// * `out` - Where to print what came of it
async fn move_on<W, S>(
    client: &Client,
    feeding: &Loop,
    here: &Here,
    spawn: S,
    out: &mut W,
) -> anyhow::Result<()>
where
    W: Write,
    S: FnOnce(&Clear) -> anyhow::Result<()>,
{
    // only a loop whose node's end was acted on is between nodes
    let Some(done) = between(feeding) else {
        return Ok(());
    };
    // what comes after its node, over the loops of its project
    let loops = client.loops().await?;
    let ending = task::upcoming(client, feeding, done, &loops).await?;
    if let Some(message) = handed_off(&ending, here, spawn) {
        writeln!(out, "{message}")?;
    }
    Ok(())
}

/// Hands a start's ending off as Stop does, from the pane, session and process it ran in, and
/// says what the user is told of it
///
/// # Arguments
///
/// * `ending` - What comes after the loop's node, which is done
/// * `here` - The pane, and the session and Claude Code process running in it if known
/// * `spawn` - Starts the injector in the background
fn handed_off<S>(ending: &Ending, here: &Here, spawn: S) -> Option<String>
where
    S: FnOnce(&Clear) -> anyhow::Result<()>,
{
    // a pane named has no session or process known, so it is never typed into
    clear::hand_off(
        ending,
        Some(&here.pane),
        here.session.clone(),
        here.pid,
        spawn,
    )
}

/// Works a member of a loop's group in the session this runs in: makes the member's worktree,
/// starts the node there, joins the pane to it, and tells Claude to enter it
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, in a checkout of the project's repo
/// * `here` - The pane, which must run no loop of its own
/// * `id` - The member's id, else the first member no session has joined
/// * `pick` - The project named, if one was
/// * `out` - Where to print what came of it
pub async fn attach<W: Write>(
    client: &Client,
    cwd: &Path,
    here: &Here,
    id: Option<&str>,
    pick: &Pick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the project, and the member named or the first waiting for a session
    let project = cli::picked(client, cwd, pick).await?;
    let loops = client.project_loops(&project.name).await?;
    let mut members = loops
        .iter()
        .flat_map(|feeding| feeding.members.iter().map(move |member| (feeding, member)));
    let found = match id {
        Some(id) => members.find(|(_, member)| member.node.id == id),
        None => members.find(|(_, member)| member.pane.is_none()),
    };
    let Some((by, member)) = found else {
        match id {
            Some(id) => anyhow::bail!(
                "no loop of `{}` holds `{id}` as a member of its group: `reef loop ls` lists \
                 each loop's members",
                project.name
            ),
            None => anyhow::bail!(
                "no member of a loop of `{}` waits for a session: `reef loop ls` lists each \
                 loop's members",
                project.name
            ),
        }
    };
    // its worktree, made from this checkout, with the node started and this pane joined
    let node = member.node.clone();
    let by = by.pane.clone();
    let made = worktree::for_node(client, cwd, &node, Some(&here.pane), out).await?;
    writeln!(
        out,
        "\n`{node}` is this session's, a member of the group of the loop in {}, in {} on \
         `{}`. Enter that worktree with EnterWorktree (path: {}), then take the node with \
         next_task and plan it as the reef skill's loop says. Work it there and nothing else.",
        by.describe(),
        made.path.display(),
        made.branch,
        made.path.display()
    )?;
    Ok(())
}

/// Pauses a pane's loop, so it stops once the node it is on is done
///
/// # Arguments
///
/// * `client` - The service's client
/// * `here` - The pane
/// * `out` - Where to print what came of it
pub async fn pause<W: Write>(client: &Client, here: &Here, out: &mut W) -> anyhow::Result<()> {
    match edited(client, &here.pane, LoopEdit::Pause).await? {
        LoopEdited::Edited { r#loop } => {
            // what it stops after, if it holds a node
            let after = match &r#loop.node {
                Some(node) => format!("it stops once `{node}` is done"),
                None => "it holds no node, so it takes none".to_string(),
            };
            writeln!(
                out,
                "paused the loop in {}: {after}",
                r#loop.pane.describe()
            )?;
            Ok(())
        }
        other => unexpected(&here.pane, other),
    }
}

/// Stops a pane's loop, letting go of the node it holds
///
/// # Arguments
///
/// * `client` - The service's client
/// * `here` - The pane
/// * `out` - Where to print what came of it
pub async fn stop<W: Write>(client: &Client, here: &Here, out: &mut W) -> anyhow::Result<()> {
    match edited(client, &here.pane, LoopEdit::Stop).await? {
        LoopEdited::Stopped { r#loop } => {
            // the node it let go of keeps its status
            let freed = match &r#loop.node {
                Some(node) => format!(", letting go of `{node}`"),
                None => String::new(),
            };
            writeln!(
                out,
                "stopped the loop in {}, which fed it {}{freed}",
                r#loop.pane.describe(),
                feeds(&r#loop)
            )?;
            Ok(())
        }
        other => unexpected(&here.pane, other),
    }
}

/// Lists the loops feeding the picked project, this pane's marked
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which picks the project
/// * `here` - This process's pane, if it has one
/// * `pick` - The project named, if one was
/// * `out` - Where to print them
pub async fn ls<W: Write>(
    client: &Client,
    cwd: &Path,
    here: Option<&Pane>,
    pick: &Pick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the project's loops, by pane
    let project = cli::picked(client, cwd, pick).await?;
    let loops = client.project_loops(&project.name).await?;
    if loops.is_empty() {
        writeln!(
            out,
            "no loop feeds `{}`: `reef loop start` starts one in this pane",
            project.name
        )?;
        return Ok(());
    }
    let rows = loops.iter().map(|feeding| {
        // this pane's is marked, and each says where it is
        let mark = match Some(&feeding.pane) == here {
            true => " (this pane)",
            false => "",
        };
        (format!("{}{mark}", feeding.pane), listed(feeding))
    });
    cli::columns(out, rows)
}

/// Hands the pane's loop to the session that just started there, as Claude Code's SessionStart
/// hook, a clear's new session included, and names the next node to a clear's between nodes
///
/// What a SessionStart hook prints is added to the session's context. So it prints nothing,
/// except after a clear while the loop is between nodes: its node done and its end acted on, and
/// another next. Then it names that node and how to take it, so `next`, or the kickoff reef
/// types, starts it. A plan approval's clear, mid-node, is told nothing. A pane with no loop is
/// left as it is.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `input` - What the hook was handed on standard input
/// * `pane` - The pane the session runs in
/// * `pid` - The Claude Code process the session runs in, `CLAUDE_PID`, if it was named
/// * `out` - Where the hook's output goes
pub async fn session_start<W: Write>(
    client: &Client,
    input: &str,
    pane: &Pane,
    pid: Option<u32>,
    out: &mut W,
) -> anyhow::Result<()> {
    // the session that started
    let started: SessionStarted = serde_json::from_str(input).map_err(|error| {
        anyhow::anyhow!("the SessionStart hook's input doesn't read as Claude Code's: {error}")
    })?;
    let edit = LoopEdit::Session {
        session: started.session_id,
        pid,
    };
    let feeding = match edited(client, pane, edit).await? {
        // handed over, or no loop to hand: either way the session goes on
        LoopEdited::Edited { r#loop } => r#loop,
        // a pane with no loop of its own may work a group's member, which it is told of
        LoopEdited::NoLoop { .. } => return member_started(client, pane, out).await,
        other => return unexpected(pane, other),
    };
    // only a clear between nodes is told what is next, never a plan approval's mid-node
    if started.source != "clear" {
        out.flush()?;
        return Ok(());
    }
    let Some(done) = between(&feeding) else {
        out.flush()?;
        return Ok(());
    };
    let loops = client.loops().await?;
    let Ending::Next { next, .. } = task::upcoming(client, &feeding, done.clone(), &loops).await?
    else {
        out.flush()?;
        return Ok(());
    };
    // added to the new session's context, as Claude Code reads a SessionStart's JSON
    let context = format!(
        "reef's loop in this pane is between nodes: `{done}` is done, and `{next}` is next. When \
         the user says next, or asks for the next node, take it with reef's next_task tool \
         (`mcp__reef__next_task`, or `reef loop take` where reef's tools aren't served), then \
         plan it in plan mode as the reef skill's loop says."
    );
    let said = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": context,
        }
    });
    writeln!(out, "{said}")?;
    Ok(())
}

/// Tells a session that started in a pane working a group's member which node it works, so the
/// user's `next` takes it; a pane working none is told nothing
///
/// # Arguments
///
/// * `client` - The service's client
/// * `pane` - The pane the session runs in
/// * `out` - Where the hook's output goes
async fn member_started<W: Write>(client: &Client, pane: &Pane, out: &mut W) -> anyhow::Result<()> {
    // the member this pane joined, if it joined one
    let loops = client.loops().await?;
    let Some((by, member)) = reef_api::loops::member_of(&loops, pane) else {
        out.flush()?;
        return Ok(());
    };
    // added to the session's context, as Claude Code reads a SessionStart's JSON
    let context = format!(
        "This session works `{}`, a member of the group of reef's loop in {}, in a worktree of \
         its own. When the user says next, or asks for its node, take it with reef's next_task \
         tool (`mcp__reef__next_task`, or `reef loop take` where reef's tools aren't served), \
         then plan it in plan mode as the reef skill's loop says.",
        member.node,
        by.pane.describe()
    );
    let said = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": context,
        }
    });
    writeln!(out, "{said}")?;
    Ok(())
}

/// The node a loop holds when it is between nodes: running, its node done and that node's end
/// acted on by the Stop hook, which acts only on a node that is done
///
/// A clear's new session, and a start, read it to move on to the next node.
///
/// # Arguments
///
/// * `feeding` - The pane's loop
///
/// # Examples
///
/// ```
/// use reef::loops::between;
/// use reef_api::{Loop, LoopState, Pane};
///
/// let node = Some("reef/central/C61".parse().unwrap());
/// let feeding = Loop {
///     pane: Pane::Claude { pid: 7 },
///     project: "reef".to_string(),
///     plan: None,
///     session: Some("s2".to_string()),
///     node: node.clone(),
///     acted: node.clone(),
///     state: LoopState::Running,
///     version: 5,
///     started_at: 1,
///     width: 1,
///     opens: true,
///     pid: None,
///     members: Vec::new(),
/// };
/// assert_eq!(between(&feeding), node);
/// // mid-node, its node's end not acted on yet, it isn't
/// let mid = Loop { acted: None, ..feeding };
/// assert_eq!(between(&mid), None);
/// ```
pub fn between(feeding: &Loop) -> Option<NodeName> {
    // only a running loop moves on
    if feeding.state != LoopState::Running {
        return None;
    }
    // whose node's end was acted on, so a plan approval's clear mid-node is passed by
    let node = feeding.node.as_ref()?;
    (feeding.acted.as_ref() == Some(node)).then(|| node.clone())
}

/// Sends a change to a pane's loop
///
/// # Arguments
///
/// * `client` - The service's client
/// * `pane` - The pane
/// * `edit` - The change
async fn edited(client: &Client, pane: &Pane, edit: LoopEdit) -> anyhow::Result<LoopEdited> {
    let edit = EditLoop {
        pane: pane.clone(),
        edit,
    };
    client.edit_loop(&edit).await
}

/// Fails on an outcome the command didn't ask for, a pane with no loop said plainly
///
/// # Arguments
///
/// * `pane` - The pane the command named
/// * `edited` - What came of it
fn unexpected(pane: &Pane, edited: LoopEdited) -> anyhow::Result<()> {
    match edited {
        LoopEdited::NoLoop { .. } => anyhow::bail!(
            "no loop runs in {}: `reef loop start` starts one there",
            pane.describe()
        ),
        other => anyhow::bail!("the service answered {other:?} for the loop in {pane}"),
    }
}

/// What a loop feeds its session: one plan's nodes, or every plan's of its project
///
/// # Arguments
///
/// * `feeding` - The loop
fn feeds(feeding: &Loop) -> String {
    match &feeding.plan {
        Some(plan) => format!("`{}/{plan}`", feeding.project),
        None => format!("every plan of `{}`", feeding.project),
    }
}

/// How many nodes a loop works at once, as a clause after what it feeds, or nothing for one
///
/// # Arguments
///
/// * `feeding` - The loop
fn wide(feeding: &Loop) -> String {
    match feeding.width {
        0 | 1 => String::new(),
        width => format!(", {width} nodes at once"),
    }
}

/// The node a loop holds, as a clause after it, or nothing
///
/// # Arguments
///
/// * `feeding` - The loop
fn holding(feeding: &Loop) -> String {
    match &feeding.node {
        Some(node) => format!(", holding `{node}`"),
        None => String::new(),
    }
}

/// One loop as `ls` lists it: whether it runs, what it feeds, its node, the node it acted on
/// and its session
///
/// # Arguments
///
/// * `feeding` - The loop
fn listed(feeding: &Loop) -> String {
    let mut parts = vec![
        feeding.state.as_str().to_string(),
        format!("feeding {}", feeds(feeding)),
    ];
    // what it holds, or that it holds nothing
    parts.push(match &feeding.node {
        Some(node) => format!("on `{node}`"),
        None => "on no node".to_string(),
    });
    if feeding.width > 1 {
        let opens = match feeding.opens {
            true => "",
            false => ", opening no panes",
        };
        parts.push(format!("{} at once{opens}", feeding.width));
    }
    // each member of its group, and the session working it if one is
    for member in &feeding.members {
        parts.push(match &member.pane {
            Some(pane) => format!("with `{}` in {pane}", member.node),
            None => format!("with `{}` waiting for a session", member.node),
        });
    }
    if let Some(acted) = &feeding.acted {
        parts.push(format!("acted on `{acted}`'s end"));
    }
    if let Some(session) = &feeding.session {
        parts.push(format!("session {session}"));
    }
    // a paused loop says what pausing means
    if feeding.state == LoopState::Paused && feeding.node.is_some() {
        parts.push("stops once its node is done".to_string());
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::{Here, handed_off, listed, server_pane};
    use crate::task::Ending;
    use reef_api::{Loop, LoopState, Pane};

    /// An MCP server's pane is its multiplexer's, then `CLAUDE_PID`'s, then its parent's
    #[test]
    fn a_servers_pane_falls_back_to_its_parent() {
        // zellij's pane wins over the parent
        let zellij = |name: &str| match name {
            "ZELLIJ_SESSION_NAME" => Some("reef".to_string()),
            "ZELLIJ_PANE_ID" => Some("0".to_string()),
            _ => None,
        };
        assert_eq!(
            server_pane(zellij, 4242),
            Some(Pane::Zellij {
                session: "reef".to_string(),
                pane: "0".to_string()
            })
        );
        // `CLAUDE_PID` names the process when it is passed
        let named = |name: &str| (name == "CLAUDE_PID").then(|| "7".to_string());
        assert_eq!(server_pane(named, 4242), Some(Pane::Claude { pid: 7 }));
        // else the parent, unless the server was orphaned to init
        assert_eq!(
            server_pane(|_| None, 4242),
            Some(Pane::Claude { pid: 4242 })
        );
        assert_eq!(server_pane(|_| None, 1), None);
    }

    /// A pane named is that one with no session, and this process's own comes with the session
    /// Claude Code named, or with none
    #[test]
    fn here_is_the_pane_named_or_this_ones() {
        // a session and a pane in the environment
        let env = |name: &str| match name {
            "CLAUDE_PID" => Some("4242".to_string()),
            "CLAUDE_CODE_SESSION_ID" => Some("s1".to_string()),
            _ => None,
        };
        let own = Here::find(None, env).unwrap();
        assert_eq!(own.pane, Pane::Claude { pid: 4242 });
        assert_eq!(own.session.as_deref(), Some("s1"));
        assert_eq!(own.pid, Some(4242));
        // a pane named never takes this process's session or process
        let named = Here::find(Some("tmux:/tmp/t:%1"), env).unwrap();
        assert_eq!((named.session, named.pid), (None, None));
        // a key that names no pane, and no pane at all, are refused saying what to do
        assert!(Here::find(Some("nowhere"), env).is_err());
        let error = Here::find(None, |_| None).err().unwrap();
        assert!(error.to_string().contains("`! reef loop ...`"), "{error}");
    }

    /// A start between nodes in its session's own zellij pane starts the injector once, handed
    /// the session and its process, while one with no multiplexer, or naming a pane, starts none
    /// and says `/clear`, then `next`
    #[test]
    fn a_start_between_nodes_says_who_clears() {
        let next = Ending::Next {
            done: "reef/central/C61".parse().unwrap(),
            next: "reef/central/C71".parse().unwrap(),
        };
        // a session in zellij, as Claude Code runs `! reef loop start` there
        let zellij = |name: &str| match name {
            "ZELLIJ_SESSION_NAME" => Some("work".to_string()),
            "ZELLIJ_PANE_ID" => Some("3".to_string()),
            "CLAUDE_PID" => Some("4242".to_string()),
            "CLAUDE_CODE_SESSION_ID" => Some("s1".to_string()),
            _ => None,
        };
        let here = Here::find(None, zellij).unwrap();
        let mut started = Vec::new();
        let said = handed_off(&next, &here, |order| {
            started.push(order.args().join(" "));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            started,
            vec![
                "hook clear --pane zellij:work:3 --session s1 --pid 4242 --after reef/central/C61"
            ]
        );
        assert!(said.contains("reef clears the context"), "{said}");
        // a session with no multiplexer is the user's to clear
        let bare = |name: &str| match name {
            "CLAUDE_PID" => Some("4242".to_string()),
            "CLAUDE_CODE_SESSION_ID" => Some("s1".to_string()),
            _ => None,
        };
        let here = Here::find(None, bare).unwrap();
        let said = handed_off(&next, &here, |_| unreachable!()).unwrap();
        assert_eq!(
            said,
            "`reef/central/C61` is done. `reef/central/C71` is next: `/clear`, then say `next`"
        );
        // as is a zellij pane named from elsewhere, whose session isn't known
        let here = Here::find(Some("zellij:work:3"), zellij).unwrap();
        let said = handed_off(&next, &here, |_| unreachable!()).unwrap();
        assert!(said.ends_with("`/clear`, then say `next`"), "{said}");
    }

    /// A loop is listed by its state, what it feeds, its node, what it acted on and its session
    #[test]
    fn loops_list_where_they_are() {
        // a paused loop of one plan, on a node, having acted on another
        let paused = Loop {
            pane: Pane::Claude { pid: 1 },
            project: "reef".to_string(),
            plan: Some("central".to_string()),
            session: Some("s1".to_string()),
            node: Some("reef/central/C39".parse().unwrap()),
            acted: Some("reef/central/C37".parse().unwrap()),
            state: LoopState::Paused,
            version: 3,
            started_at: 1,
            width: 1,
            opens: true,
            pid: None,
            members: Vec::new(),
        };
        assert_eq!(
            listed(&paused),
            "paused, feeding `reef/central`, on `reef/central/C39`, acted on \
             `reef/central/C37`'s end, session s1, stops once its node is done"
        );
        // a running one spanning the project, holding nothing
        let running = Loop {
            plan: None,
            session: None,
            node: None,
            acted: None,
            state: LoopState::Running,
            ..paused
        };
        assert_eq!(
            listed(&running),
            "running, feeding every plan of `reef`, on no node"
        );
        // a group, one member joined and one waiting, from a loop that opens no panes
        let grouped = Loop {
            width: 3,
            opens: false,
            node: Some("reef/central/C40".parse().unwrap()),
            members: vec![
                reef_api::Member {
                    node: "reef/central/C41".parse().unwrap(),
                    pane: Some(Pane::Claude { pid: 9 }),
                },
                reef_api::Member {
                    node: "reef/central/C42".parse().unwrap(),
                    pane: None,
                },
            ],
            ..running
        };
        assert_eq!(
            listed(&grouped),
            "running, feeding every plan of `reef`, on `reef/central/C40`, 3 at once, opening \
             no panes, with `reef/central/C41` in claude:9, with `reef/central/C42` waiting for \
             a session"
        );
    }
}
