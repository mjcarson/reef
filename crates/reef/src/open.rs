//! A group's member opened in a pane of its own: `claude -w <id>` beside the loop's, in its zellij
//! session or on its tmux server
//!
//! - **Opened as the group is taken.** A loop wider than one node, in a pane reef can type into,
//!   opens a pane for each member once next_task holds the group, unless it was started with
//!   `--no-open`. The new session's WorktreeCreate hook makes the member's worktree, starts the
//!   node there and joins its pane to the member ([`crate::worktree`]).
//! - **From the loop's checkout,** so `claude -w` makes the worktree from where the loop is.
//! - **Asked for its node at once.** The session is started with the member's kickoff as its
//!   first prompt, so it takes its node with next_task without a word from the user.
//! - **Nothing of the loop's session goes with it.** Every `CLAUDE*` variable the opener has is
//!   unset in the new pane's command, since what a session starts inherits its variables (X4).
//! - **Elsewhere the user gives each one a session**, with `claude -w <id>` or `! reef loop
//!   attach <id>` in a session they have open.

use reef_api::Pane;
use std::path::PathBuf;
use std::process::Stdio;

/// What a member's session is asked first
pub const MEMBER_KICKOFF: &str = "Take your node from reef's loop with next_task";

/// One member's pane to open: beside which pane, from where, and for which node
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    /// The loop's pane, whose zellij session or tmux server the new pane opens in
    pub beside: Pane,
    /// The loop's checkout, which `claude -w` runs from
    pub root: PathBuf,
    /// The member's id, which names its worktree
    pub id: String,
}

impl Open {
    /// The multiplexer's command line that opens it, the program first, or `None` for a pane
    /// reef can't open beside
    ///
    /// # Arguments
    ///
    /// * `scrub` - The variables to unset in the new pane's command
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::open::Open;
    ///
    /// let open = Open {
    ///     beside: "tmux:/tmp/t:%3".parse().unwrap(),
    ///     root: "/src/reef".into(),
    ///     id: "C60".to_string(),
    /// };
    /// let line = open.command(&["CLAUDE_PID".to_string()]).unwrap().join(" ");
    /// assert_eq!(
    ///     line,
    ///     "tmux -S /tmp/t split-window -d -t %3 -c /src/reef env -u CLAUDE_PID claude -w C60 \
    ///      Take your node from reef's loop with next_task"
    /// );
    /// ```
    pub fn command(&self, scrub: &[String]) -> Option<Vec<String>> {
        // the session's command: `claude -w <id>`, asked for its node, with nothing of this one's
        let mut session = vec!["env".to_string()];
        for name in scrub {
            session.push("-u".to_string());
            session.push(name.clone());
        }
        session.extend([
            "claude".to_string(),
            "-w".to_string(),
            self.id.clone(),
            MEMBER_KICKOFF.to_string(),
        ]);
        let root = self.root.to_string_lossy().into_owned();
        let mut line = match &self.beside {
            // zellij names the new pane for its node, and closes it once Claude Code exits
            Pane::Zellij {
                session: zellij, ..
            } => vec![
                "zellij".to_string(),
                "--session".to_string(),
                zellij.clone(),
                "action".to_string(),
                "new-pane".to_string(),
                "--cwd".to_string(),
                root,
                "--name".to_string(),
                self.id.clone(),
                "--close-on-exit".to_string(),
                "--".to_string(),
            ],
            // tmux splits the loop's pane, leaving the loop's focused
            Pane::Tmux { server, pane } => {
                let mut line = vec!["tmux".to_string()];
                if !server.is_empty() {
                    line.extend(["-S".to_string(), server.clone()]);
                }
                line.extend([
                    "split-window".to_string(),
                    "-d".to_string(),
                    "-t".to_string(),
                    pane.clone(),
                    "-c".to_string(),
                    root,
                ]);
                line
            }
            // a Claude Code process with no multiplexer has no pane to open beside
            Pane::Claude { .. } | Pane::Reef { .. } => return None,
        };
        line.extend(session);
        Some(line)
    }
}

/// The `CLAUDE*` variables this process has, which a member's pane must not inherit
pub fn scrubbed() -> Vec<String> {
    // every one Claude Code set, sorted so the command reads the same each time
    let mut names: Vec<String> = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| name.starts_with("CLAUDE"))
        .collect();
    names.sort();
    names
}

/// Opens a member's pane, waiting only for the multiplexer to say it opened
///
/// # Arguments
///
/// * `order` - Which member, from where, and beside which pane
pub fn open(order: &Open) -> anyhow::Result<()> {
    // the multiplexer's command, which returns once the pane is open
    let Some(line) = order.command(&scrubbed()) else {
        anyhow::bail!(
            "{} is no zellij or tmux pane, so reef can't open one beside it",
            order.beside.describe()
        );
    };
    let output = std::process::Command::new(&line[0])
        .args(&line[1..])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| anyhow::anyhow!("running {}: {error}", line[0]))?;
    // a failure carries the multiplexer's own message
    if !output.status.success() {
        anyhow::bail!(
            "{} couldn't open a pane: {}",
            line[0],
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Open;

    /// A member's pane opens in the loop's zellij session or beside its tmux pane, running
    /// `claude -w` with the kickoff and nothing of the loop's session, and nowhere else
    #[test]
    fn a_members_pane_opens_beside_the_loops() {
        // a zellij loop
        let open = Open {
            beside: "zellij:work:3".parse().unwrap(),
            root: "/src/reef".into(),
            id: "C60".to_string(),
        };
        let scrub = ["CLAUDECODE".to_string(), "CLAUDE_PID".to_string()];
        assert_eq!(
            open.command(&scrub).unwrap(),
            [
                "zellij",
                "--session",
                "work",
                "action",
                "new-pane",
                "--cwd",
                "/src/reef",
                "--name",
                "C60",
                "--close-on-exit",
                "--",
                "env",
                "-u",
                "CLAUDECODE",
                "-u",
                "CLAUDE_PID",
                "claude",
                "-w",
                "C60",
                "Take your node from reef's loop with next_task",
            ]
        );
        // tmux with no socket named, its default server
        let tmux = Open {
            beside: reef_api::Pane::Tmux {
                server: String::new(),
                pane: "%3".to_string(),
            },
            ..open.clone()
        };
        let line = tmux.command(&[]).unwrap().join(" ");
        assert!(line.starts_with("tmux split-window -d -t %3 -c /src/reef env claude -w C60"));
        // a bare Claude Code process has no pane beside it
        let bare = Open {
            beside: reef_api::Pane::Claude { pid: 7 },
            ..open
        };
        assert_eq!(bare.command(&[]), None);
    }
}
