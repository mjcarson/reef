//! The terminal loop's state: which session feeds a project, the node it holds, and whether it
//! is paused
//!
//! A loop feeds one Claude Code session node by node. It can't be kept by the session's id,
//! since every `/clear`, a plan approval's included, starts a new session with a new id. So a
//! loop is kept by the [`Pane`] its session runs in, which a clear keeps, and the session
//! starting there takes the pane's loop over.
//!
//! - **One loop a pane.** A pane holds one loop at most, feeding one project, or one plan of it.
//! - **One loop a node.** A node another loop holds can't be taken, so two sessions never work
//!   on the same node ([`holder`]), a node it holds as a group's member included.
//! - **A group at once.** A loop wider than one node takes a group of ready nodes: its own
//!   session works the first, and each other one, a [`Member`], is worked by a session of its
//!   own in a worktree, which joins it from its own pane ([`LoopEdit::Join`]).
//! - **Paused after the node it is on.** A paused loop keeps its node, and stops once that node
//!   is done.
//! - **The node whose end it acted on** is kept, so the Stop hook acts on each node's end once.

use reef_model::NodeName;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Where a loop's session runs: a terminal multiplexer's pane, or else the Claude Code process,
/// either of which a `/clear` keeps
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Pane {
    /// A zellij pane, which zellij's actions name by its session and its id
    Zellij {
        /// The zellij session the pane is in, `ZELLIJ_SESSION_NAME`
        session: String,
        /// The pane's id, `ZELLIJ_PANE_ID`
        pane: String,
    },
    /// A tmux pane, whose id is unique within its server
    Tmux {
        /// The server's socket, the first field of `TMUX`
        server: String,
        /// The pane's id, `TMUX_PANE`, such as `%3`
        pane: String,
    },
    /// No multiplexer: the Claude Code process the session runs in, `CLAUDE_PID`
    Claude {
        /// The process's id
        pid: u32,
    },
}

impl Pane {
    /// Finds the pane a process runs in from its environment: zellij's, then tmux's, then the
    /// Claude Code process it was started by
    ///
    /// Claude Code passes its pane's variables, and its own `CLAUDE_PID`, to every command and
    /// hook it runs.
    ///
    /// # Arguments
    ///
    /// * `var` - Reads one environment variable, `None` if it isn't set
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::loops::Pane;
    ///
    /// let env = |name: &str| match name {
    ///     "TMUX" => Some("/tmp/tmux-1000/default,4242,0".to_string()),
    ///     "TMUX_PANE" => Some("%3".to_string()),
    ///     _ => None,
    /// };
    /// let pane = Pane::from_env(env).unwrap();
    /// assert_eq!(pane.to_string(), "tmux:/tmp/tmux-1000/default:%3");
    /// ```
    pub fn from_env<F: Fn(&str) -> Option<String>>(var: F) -> Option<Pane> {
        // a variable set to nothing is as good as unset
        let set = |name: &str| var(name).filter(|value| !value.trim().is_empty());
        // zellij's actions need the session as well as the pane
        if let (Some(session), Some(pane)) = (set("ZELLIJ_SESSION_NAME"), set("ZELLIJ_PANE_ID")) {
            return Some(Pane::Zellij { session, pane });
        }
        // a tmux pane's id is unique within its server, which its socket names
        if let Some(pane) = set("TMUX_PANE") {
            let server = set("TMUX")
                .and_then(|tmux| tmux.split(',').next().map(str::to_string))
                .unwrap_or_default();
            return Some(Pane::Tmux { server, pane });
        }
        // else the Claude Code process, which outlives each of its sessions
        let pid = set("CLAUDE_PID")?.trim().parse().ok()?;
        Some(Pane::Claude { pid })
    }

    /// Whether reef can type into the pane: a multiplexer's can be written to, but a Claude Code
    /// process with none has no pane but the user's own terminal
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::loops::Pane;
    ///
    /// assert!("zellij:work:3".parse::<Pane>().unwrap().typed());
    /// assert!(!Pane::Claude { pid: 4242 }.typed());
    /// ```
    pub fn typed(&self) -> bool {
        // zellij's actions and tmux's keys reach their panes, and nothing reaches a bare terminal
        matches!(self, Pane::Zellij { .. } | Pane::Tmux { .. })
    }

    /// The pane as a person reads it
    pub fn describe(&self) -> String {
        match self {
            Pane::Zellij { session, pane } => format!("zellij session `{session}`, pane {pane}"),
            Pane::Tmux { server, pane } => format!("tmux pane {pane} on {server}"),
            Pane::Claude { pid } => format!("Claude Code process {pid}"),
        }
    }
}

impl fmt::Display for Pane {
    /// The pane as its key: `zellij:<session>:<pane>`, `tmux:<server>:<pane>` or `claude:<pid>`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pane::Zellij { session, pane } => write!(f, "zellij:{session}:{pane}"),
            Pane::Tmux { server, pane } => write!(f, "tmux:{server}:{pane}"),
            Pane::Claude { pid } => write!(f, "claude:{pid}"),
        }
    }
}

/// A pane's key that doesn't read as one
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadPane(pub String);

impl fmt::Display for BadPane {
    /// Says what a pane's key looks like
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is no pane: one is `zellij:<session>:<pane>`, `tmux:<server>:<pane>` or \
             `claude:<pid>`, as `reef loop ls` prints it",
            self.0
        )
    }
}

impl std::error::Error for BadPane {}

impl FromStr for Pane {
    type Err = BadPane;

    /// Reads a pane's key, as [`Pane`]'s `Display` writes it
    ///
    /// A pane's id holds no `:`, so the key's last `:` splits it from its session or server.
    ///
    /// # Arguments
    ///
    /// * `key` - The key
    fn from_str(key: &str) -> Result<Pane, BadPane> {
        let bad = || BadPane(key.to_string());
        // the kind first, then what that kind names a pane by
        let (kind, rest) = key.split_once(':').ok_or_else(bad)?;
        let split = |rest: &str| {
            rest.rsplit_once(':')
                .filter(|(within, pane)| !within.is_empty() && !pane.is_empty())
                .map(|(within, pane)| (within.to_string(), pane.to_string()))
        };
        match kind {
            "zellij" => {
                let (session, pane) = split(rest).ok_or_else(bad)?;
                Ok(Pane::Zellij { session, pane })
            }
            "tmux" => {
                let (server, pane) = split(rest).ok_or_else(bad)?;
                Ok(Pane::Tmux { server, pane })
            }
            "claude" => Ok(Pane::Claude {
                pid: rest.parse().map_err(|_| bad())?,
            }),
            _ => Err(bad()),
        }
    }
}

/// Whether a loop takes its next node once the one it is on is done
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoopState {
    /// It takes the next node once the one it is on is done
    Running,
    /// It stops once the node it is on is done
    Paused,
}

impl LoopState {
    /// The name this state takes in JSON and as the CLI prints it
    pub fn as_str(self) -> &'static str {
        match self {
            LoopState::Running => "running",
            LoopState::Paused => "paused",
        }
    }
}

/// A node of a loop's group past the loop's own, worked by a session of its own in a worktree
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// The node
    pub node: NodeName,
    /// The pane of the session that joined it, if one has
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<Pane>,
}

/// A loop's width before groups: one node at a time
fn one() -> u8 {
    1
}

/// Whether a loop works one node at a time, so its width needn't be written
///
/// # Arguments
///
/// * `width` - The loop's width
fn is_one(width: &u8) -> bool {
    *width == 1
}

/// Whether reef opens a pane for each member of a loop's group, unless told not to
fn yes() -> bool {
    true
}

/// Whether a flag is set, so a set one needn't be written
///
/// # Arguments
///
/// * `flag` - The flag
fn is_set(flag: &bool) -> bool {
    *flag
}

/// One loop: the pane it feeds, the project it feeds it from, and where it is
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loop {
    /// The pane its session runs in, which keeps it
    pub pane: Pane,
    /// The project it takes nodes from
    pub project: String,
    /// The one plan of the project it takes nodes from, or `None` for all of them
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The Claude Code session feeding it now, if one has said
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The node it holds, which no other loop may take
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeName>,
    /// The node whose end it acted on last, so it never acts on one twice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acted: Option<NodeName>,
    /// Whether it takes the next node once this one is done
    pub state: LoopState,
    /// Bumped by every write of it
    pub version: u64,
    /// When it was started, in milliseconds since the epoch
    pub started_at: u64,
    /// How many nodes it works at once: its own, and as many members as fit beside it
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub width: u8,
    /// Whether reef opens a pane for each member in the loop's own zellij or tmux session
    #[serde(default = "yes", skip_serializing_if = "is_set")]
    pub opens: bool,
    /// The Claude Code process its session runs in, `CLAUDE_PID`, so a member's Stop can clear
    /// its pane
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// The group's nodes past its own, each worked by a session of its own in a worktree
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<Member>,
}

impl Loop {
    /// The nodes of its group: its own first, then each member's
    pub fn group(&self) -> impl Iterator<Item = &NodeName> {
        // its own, then the members' in the order they were taken
        self.node
            .iter()
            .chain(self.members.iter().map(|member| &member.node))
    }

    /// Whether it holds a node, as its own or as a member's
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    pub fn holds(&self, node: &NodeName) -> bool {
        self.group().any(|held| held == node)
    }

    /// The member a node is, if it is one of this loop's
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    pub fn member(&self, node: &NodeName) -> Option<&Member> {
        self.members.iter().find(|member| member.node == *node)
    }

    /// The member whose session runs in a pane, if one of this loop's does
    ///
    /// # Arguments
    ///
    /// * `pane` - The pane
    pub fn member_in(&self, pane: &Pane) -> Option<&Member> {
        self.members
            .iter()
            .find(|member| member.pane.as_ref() == Some(pane))
    }

    /// Whether a node is one this loop takes nodes from: its project's, and its plan's if it
    /// names one
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    pub fn covers(&self, node: &NodeName) -> bool {
        // the project, then the plan it narrows to
        node.plan.project == self.project
            && self
                .plan
                .as_ref()
                .is_none_or(|plan| *plan == node.plan.plan)
    }

    /// The loop handed to the session that now runs in its pane, as a clear's new session takes
    /// it: everything it holds kept, its session the new one
    ///
    /// # Arguments
    ///
    /// * `session` - The session's id
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::loops::{Loop, LoopState, Pane};
    ///
    /// let cleared = Loop {
    ///     pane: Pane::Claude { pid: 4242 },
    ///     project: "reef".to_string(),
    ///     plan: None,
    ///     session: Some("before".to_string()),
    ///     node: Some("reef/central/C39".parse().unwrap()),
    ///     acted: None,
    ///     state: LoopState::Running,
    ///     version: 3,
    ///     started_at: 1,
    ///     width: 1,
    ///     opens: true,
    ///     pid: None,
    ///     members: Vec::new(),
    /// };
    /// let handed = cleared.clone().handed_to("after");
    /// assert_eq!(handed.session.as_deref(), Some("after"));
    /// assert_eq!(handed.node, cleared.node);
    /// ```
    pub fn handed_to(mut self, session: &str) -> Loop {
        self.session = Some(session.to_string());
        self
    }
}

/// The loop holding a node, if one does, so no other takes it
///
/// # Arguments
///
/// * `loops` - Every loop of the node's project
/// * `node` - The node's full name
///
/// # Examples
///
/// ```
/// use reef_api::loops::holder;
///
/// let node = "reef/central/C39".parse().unwrap();
/// assert!(holder(&[], &node).is_none());
/// ```
pub fn holder<'a>(loops: &'a [Loop], node: &NodeName) -> Option<&'a Loop> {
    // its own node, or one of its group's members
    loops.iter().find(|held| held.holds(node))
}

/// The loop a pane works a member of, and that member, if it works one
///
/// # Arguments
///
/// * `loops` - Every loop
/// * `pane` - The pane
///
/// # Examples
///
/// ```
/// use reef_api::loops::{Pane, member_of};
///
/// assert!(member_of(&[], &Pane::Claude { pid: 7 }).is_none());
/// ```
pub fn member_of<'a>(loops: &'a [Loop], pane: &Pane) -> Option<(&'a Loop, &'a Member)> {
    // a pane joins one member at a time
    loops
        .iter()
        .find_map(|feeding| feeding.member_in(pane).map(|member| (feeding, member)))
}

/// One change to a pane's loop
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "kebab-case")]
pub enum LoopEdit {
    /// Start a loop in the pane, or set its paused loop running again
    Start {
        /// The project it takes nodes from
        project: String,
        /// The one plan of it to take nodes from, or `None` for all of them
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<String>,
        /// The session feeding it, if the starter knows
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        /// How many nodes it works at once, or `None` to keep its width, one for a new loop
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u8>,
        /// Whether reef opens the members' panes, or `None` to keep what it does, which a new
        /// loop does
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opens: Option<bool>,
        /// The Claude Code process its session runs in, if the starter knows
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
    },
    /// Pause it, so it stops once the node it is on is done
    Pause,
    /// End it, letting go of the node it holds
    Stop,
    /// Hand it to the session that now runs in its pane
    Session {
        /// The session's id
        session: String,
        /// The Claude Code process the session runs in, if the hook was told
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
    },
    /// Take a node, and a group's members beside it, none of which another loop may hold, or
    /// let go of everything it holds
    Hold {
        /// The node to take, or `None` to hold none
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<NodeName>,
        /// The group's other nodes, each to be worked by a session of its own
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        members: Vec<NodeName>,
    },
    /// Join the pane, which runs no loop of its own, to the member of whichever loop holds the
    /// node, moving it off any member it worked before
    Join {
        /// The member's node
        node: NodeName,
    },
    /// Record the node whose end it acted on, unless it acted on that node's end already
    Acted {
        /// The node
        node: NodeName,
    },
}

/// A change to the loop of a pane
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditLoop {
    /// The pane whose loop it changes
    pub pane: Pane,
    /// The change
    pub edit: LoopEdit,
}

/// What came of a change to a pane's loop
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoopEdited {
    /// A loop was started in the pane
    Started {
        /// The loop as it now stands
        r#loop: Loop,
    },
    /// The pane's loop, of the same project and plan, runs again, or ran already
    Resumed {
        /// The loop as it now stands
        r#loop: Loop,
    },
    /// The pane's loop was changed
    Edited {
        /// The loop as it now stands
        r#loop: Loop,
    },
    /// The pane's loop was ended, letting go of its node
    Stopped {
        /// The loop as it was
        r#loop: Loop,
    },
    /// The pane has no loop, so nothing was written
    NoLoop {
        /// The pane
        pane: Pane,
    },
    /// The pane's loop feeds another project or plan, so no loop was started
    Busy {
        /// The pane's loop as it stands
        r#loop: Loop,
    },
    /// Another loop holds the node, so it wasn't taken
    Held {
        /// The node
        node: NodeName,
        /// The loop that holds it
        by: Loop,
    },
    /// The node is none of the loop's project or plan, so it wasn't taken
    NotInLoop {
        /// The node
        node: NodeName,
        /// The pane's loop as it stands
        r#loop: Loop,
    },
    /// The node isn't in its plan, so it wasn't taken
    NotInPlan {
        /// The node
        node: NodeName,
    },
    /// The loop acted on the node's end already, so nothing was written
    ActedAlready {
        /// The node
        node: NodeName,
        /// The pane's loop as it stands
        r#loop: Loop,
    },
    /// The pane joined a member of this loop, or had joined it already
    Joined {
        /// The loop whose member it joined, as it now stands
        r#loop: Loop,
    },
    /// No loop holds the node as a member, so the pane joined nothing
    NotMember {
        /// The node
        node: NodeName,
    },
    /// The group held is wider than the loop, so nothing was taken
    TooWide {
        /// How many nodes the hold named
        named: usize,
        /// The pane's loop as it stands
        r#loop: Loop,
    },
}

#[cfg(test)]
mod tests {
    use super::{EditLoop, Loop, LoopEdit, LoopEdited, LoopState, Member, Pane, holder, member_of};
    use reef_model::NodeName;

    /// Reads an environment out of pairs
    ///
    /// # Arguments
    ///
    /// * `pairs` - Each variable and its value
    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    /// A loop in a pane, holding a node or none
    ///
    /// # Arguments
    ///
    /// * `pid` - The Claude Code process it is kept by
    /// * `node` - The node it holds, a bare id in `reef/central`
    fn looped(pid: u32, node: Option<&str>) -> Loop {
        Loop {
            pane: Pane::Claude { pid },
            project: "reef".to_string(),
            plan: Some("central".to_string()),
            session: Some(format!("session-{pid}")),
            node: node.map(|id| format!("reef/central/{id}").parse().unwrap()),
            acted: None,
            state: LoopState::Running,
            version: 1,
            started_at: 1,
            width: 1,
            opens: true,
            pid: None,
            members: Vec::new(),
        }
    }

    /// zellij's pane wins over tmux's, tmux's over Claude Code's process, and none is none
    #[test]
    fn panes_are_found_from_the_environment() {
        // every one set: zellij, by its session and pane
        let all = [
            ("ZELLIJ_SESSION_NAME", "reef"),
            ("ZELLIJ_PANE_ID", "0"),
            ("TMUX", "/tmp/tmux-1000/default,4242,0"),
            ("TMUX_PANE", "%3"),
            ("CLAUDE_PID", "492329"),
        ];
        assert_eq!(
            Pane::from_env(vars(&all)),
            Some(Pane::Zellij {
                session: "reef".to_string(),
                pane: "0".to_string()
            })
        );
        // tmux by its socket and pane, the session's name alone not being zellij's
        let tmux = [("ZELLIJ_SESSION_NAME", "reef"), ("TMUX_PANE", "%3")];
        assert_eq!(
            Pane::from_env(vars(&tmux)),
            Some(Pane::Tmux {
                server: String::new(),
                pane: "%3".to_string()
            })
        );
        // Claude Code's process with no multiplexer, and nothing at all is no pane
        let claude = [("CLAUDE_PID", " 492329 "), ("TMUX_PANE", "")];
        assert_eq!(
            Pane::from_env(vars(&claude)),
            Some(Pane::Claude { pid: 492329 })
        );
        assert_eq!(Pane::from_env(vars(&[("CLAUDE_PID", "x")])), None);
        assert_eq!(Pane::from_env(vars(&[])), None);
    }

    /// A pane's key reads back as the pane, and anything else is refused saying what one is
    #[test]
    fn pane_keys_read_back() {
        // each kind, a session holding a `:` included
        let panes = [
            Pane::Zellij {
                session: "work:two".to_string(),
                pane: "12".to_string(),
            },
            Pane::Tmux {
                server: "/tmp/tmux-1000/default".to_string(),
                pane: "%3".to_string(),
            },
            Pane::Claude { pid: 492329 },
        ];
        for pane in panes {
            let key = pane.to_string();
            assert_eq!(key.parse::<Pane>(), Ok(pane), "{key}");
        }
        // keys that name no pane
        for bad in [
            "",
            "zellij:only",
            "tmux::%3",
            "claude:me",
            "screen:1:2",
            "zellij:a:",
        ] {
            let error = bad.parse::<Pane>().expect_err(bad);
            assert!(error.to_string().contains("`reef loop ls`"), "{error}");
        }
        // and a pane says what it is
        assert_eq!(Pane::Claude { pid: 7 }.describe(), "Claude Code process 7");
    }

    /// Two loops never hold the same node: the one holding it is found, whichever it is
    #[test]
    fn two_sessions_never_take_the_same_node() {
        // one loop on C39, another on C40, and one on none
        let loops = [
            looped(1, Some("C39")),
            looped(2, Some("C40")),
            looped(3, None),
        ];
        let node = |id: &str| -> NodeName { format!("reef/central/{id}").parse().unwrap() };
        // each held node names its one loop, and a free node none
        assert_eq!(
            holder(&loops, &node("C39")).map(|l| &l.pane),
            Some(&loops[0].pane)
        );
        assert_eq!(
            holder(&loops, &node("C40")).map(|l| &l.pane),
            Some(&loops[1].pane)
        );
        assert!(holder(&loops, &node("C41")).is_none());
        // a loop covers its own plan's nodes, and no other project's or plan's
        assert!(loops[0].covers(&node("C41")));
        assert!(!loops[0].covers(&"reef/other/C41".parse().unwrap()));
        assert!(!loops[0].covers(&"shoal/central/C41".parse().unwrap()));
        let spanning = Loop {
            plan: None,
            ..looped(4, None)
        };
        assert!(spanning.covers(&"reef/other/C41".parse().unwrap()));
    }

    /// A clear's new session keeps the loop it cleared: its pane, node, the node it acted on,
    /// and whether it is paused, its session the new one
    #[test]
    fn a_clears_new_session_keeps_the_loop_it_cleared() {
        // a paused loop on C39 that acted on C37's end, fed by the session before the clear
        let cleared = Loop {
            acted: Some("reef/central/C37".parse().unwrap()),
            state: LoopState::Paused,
            ..looped(1, Some("C39"))
        };
        let handed = cleared.clone().handed_to("after-the-clear");
        assert_eq!(handed.session.as_deref(), Some("after-the-clear"));
        assert_eq!(
            Loop {
                session: cleared.session.clone(),
                ..handed
            },
            cleared
        );
    }

    /// A loop holds its group's members as its own node, so no other loop takes one, and a pane
    /// is found working the member it joined
    #[test]
    fn a_groups_members_are_held_by_its_loop() {
        // a loop on C39, with C40 joined from a tmux pane and C41 waiting for a session
        let node = |id: &str| -> NodeName { format!("reef/central/{id}").parse().unwrap() };
        let joined = Pane::Tmux {
            server: "/tmp/t".to_string(),
            pane: "%5".to_string(),
        };
        let wide = Loop {
            width: 3,
            members: vec![
                Member {
                    node: node("C40"),
                    pane: Some(joined.clone()),
                },
                Member {
                    node: node("C41"),
                    pane: None,
                },
            ],
            ..looped(1, Some("C39"))
        };
        let loops = [wide.clone(), looped(2, Some("C42"))];
        // its own node first, then its members, all held by it
        let group: Vec<String> = wide.group().map(|held| held.id.clone()).collect();
        assert_eq!(group, ["C39", "C40", "C41"]);
        for id in ["C39", "C40", "C41"] {
            assert_eq!(holder(&loops, &node(id)).map(|l| &l.pane), Some(&wide.pane));
        }
        // the joined pane works C40, and a pane that joined nothing works none
        let (by, member) = member_of(&loops, &joined).unwrap();
        assert_eq!((&by.pane, &member.node), (&wide.pane, &node("C40")));
        assert!(member_of(&loops, &Pane::Claude { pid: 9 }).is_none());
        assert!(wide.member(&node("C41")).unwrap().pane.is_none());
        assert!(wide.member(&node("C39")).is_none());
    }

    /// A loop written before groups reads as one node wide, opening panes, with no members, and
    /// one that is one wide writes none of what groups added
    #[test]
    fn loops_from_before_groups_read_one_wide() {
        // a loop as it was written before C60
        let before = r#"{"pane":{"kind":"claude","pid":7},"project":"reef","state":"running","version":2,"started_at":1}"#;
        let read: Loop = serde_json::from_str(before).unwrap();
        assert_eq!(read.width, 1);
        assert!(read.opens);
        assert_eq!(read.pid, None);
        assert!(read.members.is_empty());
        let value = serde_json::to_value(&read).unwrap();
        for key in ["width", "opens", "pid", "members"] {
            assert!(value.get(key).is_none(), "{key} was written");
        }
        // and a hold or a session from before reads as one with no members or process
        let hold: LoopEdit =
            serde_json::from_str(r#"{"edit":"hold","node":"reef/central/C39"}"#).unwrap();
        assert!(matches!(hold, LoopEdit::Hold { members, .. } if members.is_empty()));
        let session: LoopEdit =
            serde_json::from_str(r#"{"edit":"session","session":"s"}"#).unwrap();
        assert!(matches!(session, LoopEdit::Session { pid: None, .. }));
    }

    /// Edits and their outcomes are tagged in kebab-case, a loop under its keyword's name
    #[test]
    fn edits_and_outcomes_are_tagged() {
        // an edit names its kind beside what it carries, and the pane its kind
        let edit = EditLoop {
            pane: Pane::Claude { pid: 7 },
            edit: LoopEdit::Hold {
                node: None,
                members: Vec::new(),
            },
        };
        assert_eq!(
            serde_json::to_string(&edit).unwrap(),
            r#"{"pane":{"kind":"claude","pid":7},"edit":{"edit":"hold"}}"#
        );
        let start: LoopEdit = serde_json::from_str(r#"{"edit":"start","project":"reef"}"#).unwrap();
        assert_eq!(
            start,
            LoopEdit::Start {
                project: "reef".to_string(),
                plan: None,
                session: None,
                width: None,
                opens: None,
                pid: None,
            }
        );
        // an outcome carries its loop as `loop`, which reads back
        let started = LoopEdited::Started {
            r#loop: looped(1, None),
        };
        let value = serde_json::to_value(&started).unwrap();
        assert_eq!(value["started"]["loop"]["state"], "running");
        assert!(value["started"]["loop"].get("node").is_none());
        assert_eq!(
            serde_json::from_value::<LoopEdited>(value).unwrap(),
            started
        );
        assert_eq!(LoopState::Paused.as_str(), "paused");
    }
}
