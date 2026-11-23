//! reef's commands for the terminal loop: which pane feeds a project, and whether it is paused
//!
//! ```text
//! reef loop start [--plan NAME] [--pane KEY] [--session ID]   # start or resume this pane's loop
//! reef loop pause [--pane KEY]                                 # stop once its node is done
//! reef loop stop [--pane KEY]                                  # end it, letting go of its node
//! reef loop ls                                                 # the project's loops
//! reef hook session-start [--pane KEY]                         # Claude Code's SessionStart hook
//! ```
//!
//! - **A loop is its pane's.** Every `/clear` starts a new session with a new id, so a loop is
//!   kept by the pane its session runs in: zellij's or tmux's, else the Claude Code process,
//!   which Claude Code names to every command and hook it runs as `CLAUDE_PID`. So `reef loop`
//!   is run in the session's own pane, as `! reef loop start`, or names one with `--pane`.
//! - **The session is the pane's.** A start in the session's own pane records the session that
//!   ran it, `CLAUDE_CODE_SESSION_ID`, and the SessionStart hook hands the pane's loop to each
//!   session that starts there, a clear's included, so the loop follows its pane.
//! - **Every plan, or one.** A loop takes the project's ready work across its plans, unless
//!   `--plan` names one.

use reef_api::{EditLoop, Loop, LoopEdit, LoopEdited, LoopState, Pane};
use serde::Deserialize;
use std::io::Write;
use std::path::Path;

use crate::cli::{self, Pick, PlanPick};
use crate::client::Client;

/// The pane a loop command works on, and the session running there if it is this one
pub struct Here {
    /// The pane
    pub pane: Pane,
    /// The session running in it, when the pane is this process's own and Claude Code named it
    pub session: Option<String>,
}

impl Here {
    /// The pane named, else the one this process runs in, found from its environment
    ///
    /// A session is only known for this process's own pane, never for one named.
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
        Ok(Here { pane, session })
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

/// What Claude Code's SessionStart hook is handed on standard input, the parts reef reads
#[derive(Debug, Deserialize)]
struct SessionStarted {
    /// The session that started, a clear's new one included
    session_id: String,
}

/// Starts a loop in a pane feeding the picked project, or one plan of it, or sets the pane's
/// paused loop running again
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which picks the project
/// * `here` - The pane, and the session running in it if known
/// * `session` - The session feeding it, named on the command line
/// * `pick` - The project and plan named, if they were
/// * `out` - Where to print what came of it
pub async fn start<W: Write>(
    client: &Client,
    cwd: &Path,
    here: &Here,
    session: Option<&str>,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the project the working directory belongs to, and a plan only if one was named
    let project = cli::picked(client, cwd, &pick.project).await?;
    let edit = LoopEdit::Start {
        project: project.name.clone(),
        plan: pick.plan.clone(),
        session: session.map(str::to_string).or(here.session.clone()),
    };
    match edited(client, &here.pane, edit).await? {
        LoopEdited::Started { r#loop } => {
            writeln!(
                out,
                "started a loop in {}, feeding it {}",
                r#loop.pane.describe(),
                feeds(&r#loop)
            )?;
            Ok(())
        }
        LoopEdited::Resumed { r#loop } => {
            writeln!(
                out,
                "the loop in {} is running, feeding it {}{}",
                r#loop.pane.describe(),
                feeds(&r#loop),
                holding(&r#loop)
            )?;
            Ok(())
        }
        LoopEdited::Busy { r#loop } => anyhow::bail!(
            "{} has a loop already, feeding it {}: `reef loop stop` ends it first",
            r#loop.pane.describe(),
            feeds(&r#loop)
        ),
        other => unexpected(&here.pane, other),
    }
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
/// hook, a clear's new session included
///
/// It prints nothing, since what a SessionStart hook prints is added to the session's context.
/// A pane with no loop is left as it is.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `input` - What the hook was handed on standard input
/// * `pane` - The pane the session runs in
/// * `out` - Where the hook's output goes, which this leaves empty
pub async fn session_start<W: Write>(
    client: &Client,
    input: &str,
    pane: &Pane,
    out: &mut W,
) -> anyhow::Result<()> {
    // the session that started
    let started: SessionStarted = serde_json::from_str(input).map_err(|error| {
        anyhow::anyhow!("the SessionStart hook's input doesn't read as Claude Code's: {error}")
    })?;
    let edit = LoopEdit::Session {
        session: started.session_id,
    };
    match edited(client, pane, edit).await? {
        // handed over, or no loop to hand: either way the session goes on
        LoopEdited::Edited { .. } | LoopEdited::NoLoop { .. } => {
            out.flush()?;
            Ok(())
        }
        other => unexpected(pane, other),
    }
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
    use super::{Here, listed, server_pane};
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
        // a pane named never takes this process's session
        let named = Here::find(Some("tmux:/tmp/t:%1"), env).unwrap();
        assert_eq!(named.session, None);
        // a key that names no pane, and no pane at all, are refused saying what to do
        assert!(Here::find(Some("nowhere"), env).is_err());
        let error = Here::find(None, |_| None).err().unwrap();
        assert!(error.to_string().contains("`! reef loop ...`"), "{error}");
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
    }
}
