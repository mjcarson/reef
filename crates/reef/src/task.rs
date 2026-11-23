//! next_task: the next node taken for the loop of the pane a session runs in, started in this
//! checkout, and handed back whole
//!
//! ```text
//! reef loop take [--pane KEY] [--project NAME]   # what the MCP tool next_task does
//! ```
//!
//! - **Through the pane's loop.** A node is taken for the loop its session's pane keeps, which
//!   no other loop may hold ([`LoopEdit::Hold`]). A pane with no loop is refused, saying how to
//!   start one, since starting a loop is the user's to do.
//! - **The same node until it is done.** A loop holding a node to do or in progress hands it
//!   out again. One done, blocked or gone from its plan is passed by, and the next taken.
//! - **The queue first, then plan order,** as next orders the ready work, passing over each node
//!   another loop holds. A paused loop takes no new node.
//! - **Started before plan mode.** The node taken is started in this checkout as `reef start`
//!   starts it: pinned, recorded and marked in progress, since plan mode holds writes back. One
//!   that can't be started is let go of again.
//! - **Handed back whole:** the node as `reef node get --md` prints it, then the queue's entries
//!   passed over and why. With nothing to take, next's report says what everything waits on.
//! - **The Stop hook ends a node once** ([`ended`]). While the node the loop holds isn't done,
//!   Claude is waiting on the user, and the hook lets it stop. Once it is done, the hook acts on
//!   its end once, recorded as [`LoopEdit::Acted`], whatever `stop_hook_active` says: it names
//!   the next node, or says the loop is paused or nothing else is ready. In a zellij or tmux
//!   pane, reef then clears the session and asks for the next node itself ([`crate::clear`]).
//! - **A group at once, for a loop wider than one node.** It takes up to its width of ready
//!   nodes, held as one ([`LoopEdit::Hold`]): the loop's session works the first in its own
//!   checkout, and each member gets a session of its own in its own worktree. In zellij or tmux
//!   reef opens a pane for each, running `claude -w <id>` ([`crate::open`]), unless the loop was
//!   started with `--no-open`; elsewhere the user runs that, or `! reef loop attach` in a session
//!   they have open.
//! - **A member's session takes its member.** next_task in a pane that joined a member hands out
//!   that node, started in its worktree already, and joins a pane whose checkout is a member's
//!   worktree that no pane joined.
//! - **The group ends once, when all of it is done** ([`group_end`]). Each node's Stop says what
//!   the group still waits on, and the Stop that finds it all done acts on its end, recorded as
//!   the loop's own node's, whichever pane it runs in. Only then is the loop's pane cleared.
//! - **Merged before the next group.** A group's members are worked on branches of their own,
//!   so next_task takes no new group while a member's branch isn't in the loop's checkout's HEAD
//!   yet, and names each one to merge. A branch that is gone, as a squash and a delete leave it,
//!   is merged.

use reef_api::loops::{holder, member_of};
use reef_api::work::on_worktree;
use reef_api::{EditLoop, Loop, LoopEdit, LoopEdited, LoopState, Pane};
use reef_model::next::entry_lines;
use reef_model::{Next, NodeName, PlanName, Status};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::cli::{self, Pick, PlanPick};
use crate::client::Client;
use crate::git::Checkout;
use crate::open::Open;
use crate::{node, start, work};

/// What next_task does for a pane's loop, judged before anything is held
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// The loop holds a node not done yet, which it hands out again
    Again(NodeName),
    /// The loop's own node is done, and these members of its group aren't, so it takes none
    Waiting(Vec<NodeName>),
    /// The loop is paused, and its node is done or it holds none, so it takes no new one
    Paused,
    /// Take the first of the candidates that no other loop takes meanwhile
    Take {
        /// The ready work, in the order next gives it, that no other loop holds
        candidates: Vec<NodeName>,
        /// The ready work another loop holds, each with the pane that holds it
        elsewhere: Vec<(NodeName, Pane)>,
    },
}

/// Judges what next_task does for a pane's loop: hand out its node again, wait on its group,
/// take nothing, or take the ready work no other loop holds
///
/// # Arguments
///
/// * `feeding` - The pane's loop
/// * `held` - Where the node it holds stands, or `None` if it holds none or its plan lost it
/// * `waiting` - The members of its group that aren't finished
/// * `next` - The ready work over the loop's plans, the queue's first
/// * `loops` - Every loop of the project, the pane's included
///
/// # Examples
///
/// ```
/// use reef::task::{Choice, choose};
/// use reef_api::{Loop, LoopState, Pane};
/// use reef_model::{Next, Status};
///
/// let feeding = Loop {
///     pane: Pane::Claude { pid: 7 },
///     project: "reef".to_string(),
///     plan: Some("central".to_string()),
///     session: None,
///     node: Some("reef/central/C40".parse().unwrap()),
///     acted: None,
///     state: LoopState::Running,
///     version: 2,
///     started_at: 1,
///     width: 1,
///     opens: true,
///     pid: None,
///     members: Vec::new(),
/// };
/// let again = choose(&feeding, Some(Status::InProgress), &[], &Next::default(), &[]);
/// assert_eq!(again, Choice::Again("reef/central/C40".parse().unwrap()));
/// ```
pub fn choose(
    feeding: &Loop,
    held: Option<Status>,
    waiting: &[NodeName],
    next: &Next,
    loops: &[Loop],
) -> Choice {
    // the node it holds is the one it is on, while it is to do or under way
    if let (Some(node), Some(Status::Todo | Status::InProgress)) = (&feeding.node, held) {
        return Choice::Again(node.clone());
    }
    // its own done, the group it took is worked to its end before another is taken
    if feeding.node.is_some() && !waiting.is_empty() {
        return Choice::Waiting(waiting.to_vec());
    }
    // a paused loop stops once its node is done
    if feeding.state == LoopState::Paused {
        return Choice::Paused;
    }
    // the other loops, whose nodes this one never takes
    let others: Vec<Loop> = loops
        .iter()
        .filter(|other| other.pane != feeding.pane)
        .cloned()
        .collect();
    let mut candidates = Vec::new();
    let mut elsewhere = Vec::new();
    // the ready work in next's order, each named in full
    for ready in &next.ready {
        let Some(node) = ready_name(feeding, ready.plan.as_ref(), &ready.id) else {
            continue;
        };
        // only the loop's own project's or plan's work is its to take
        if !feeding.covers(&node) {
            continue;
        }
        match holder(&others, &node) {
            Some(by) => elsewhere.push((node, by.pane.clone())),
            None => candidates.push(node),
        }
    }
    Choice::Take {
        candidates,
        elsewhere,
    }
}

/// What the Stop hook makes of a turn's end for the loop of the pane it runs in
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// Nothing to act on: no loop, no node held, the node not done, or its end acted on already
    Nothing,
    /// This pane's node is done, and the rest of its group isn't, so nothing moves on yet
    Waiting {
        /// The pane's node, which is done
        done: NodeName,
        /// The group's nodes that aren't
        on: Vec<NodeName>,
        /// Whether the pane works a member, rather than the loop's own node
        member: bool,
    },
    /// The node is done and the loop is paused, so it stops there
    Paused(NodeName),
    /// The node is done and nothing else is ready to take
    Finished(NodeName),
    /// The node is done and this one is next
    Next {
        /// The node done
        done: NodeName,
        /// The node next_task takes next, as things stand
        next: NodeName,
    },
}

impl Ending {
    /// What the hook tells the user of it, if it acted
    ///
    /// # Arguments
    ///
    /// * `typed` - Whether reef types `/clear` and the next node's kickoff into the pane, else
    ///   the user does
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::task::Ending;
    ///
    /// let paused = Ending::Paused("reef/central/C41".parse().unwrap());
    /// assert!(paused.message(false).unwrap().contains("the loop is paused"));
    /// assert_eq!(Ending::Nothing.message(true), None);
    /// ```
    pub fn message(&self, typed: bool) -> Option<String> {
        match self {
            Ending::Nothing => None,
            // a member's session has nothing more to do
            Ending::Waiting {
                done,
                on,
                member: true,
            } => Some(format!(
                "`{done}` is done. Its group waits on {}: the loop's pane moves on once they are \
                 all done, so this session can be closed",
                listed(on)
            )),
            Ending::Waiting { done, on, .. } => Some(format!(
                "`{done}` is done. Its group waits on {}: this pane moves on once they are all \
                 done",
                listed(on)
            )),
            Ending::Paused(done) => Some(format!(
                "`{done}` is done, and the loop is paused: `! reef loop start` moves on to the \
                 next node"
            )),
            Ending::Finished(done) => Some(format!(
                "`{done}` is done, and nothing else is ready to take: `reef next` says what \
                 everything waits on"
            )),
            // reef clears the session itself in a multiplexer's pane
            Ending::Next { done, next } if typed => Some(format!(
                "`{done}` is done. `{next}` is next: reef clears the context and asks for it \
                 once Claude is idle and the input box is empty"
            )),
            Ending::Next { done, next } => Some(format!(
                "`{done}` is done. `{next}` is next: `/clear`, then say `next`"
            )),
        }
    }
}

/// What a pane's node being done is for its loop's group
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupEnd {
    /// The pane's node isn't done, or the group's end was acted on already
    NotYet,
    /// The pane's node is done, and these of its group aren't finished
    Waiting {
        /// The pane's node
        done: NodeName,
        /// The group's nodes not finished
        on: Vec<NodeName>,
    },
    /// The whole group is done, its end recorded as the loop's own node's
    Done(NodeName),
}

/// Judges what the end of a pane's node is for its loop's group: nothing yet, a wait on the rest
/// of the group, or the group's end
///
/// `stop_hook_active` is never read: it stays true across a whole chain of turns, so the node
/// whose end the loop acted on is the guard. A group's end is the loop's own node's, so it is
/// acted on once, whichever of its panes ends last.
///
/// # Arguments
///
/// * `feeding` - The loop
/// * `mine` - The node the pane works: the loop's own, or a member's
/// * `status` - Where a node of the group stands, or `None` if its plan has it no more
///
/// # Examples
///
/// ```
/// use reef::task::{GroupEnd, group_end};
/// use reef_api::{Loop, LoopState, Pane};
/// use reef_model::Status;
///
/// let node = "reef/central/C41".parse().unwrap();
/// let feeding = Loop {
///     pane: Pane::Claude { pid: 7 },
///     project: "reef".to_string(),
///     plan: None,
///     session: None,
///     node: Some(node),
///     acted: None,
///     state: LoopState::Running,
///     version: 2,
///     started_at: 1,
///     width: 1,
///     opens: true,
///     pid: None,
///     members: Vec::new(),
/// };
/// let own = feeding.node.clone().unwrap();
/// assert_eq!(group_end(&feeding, &own, |_| Some(Status::Done)), GroupEnd::Done(own.clone()));
/// assert_eq!(group_end(&feeding, &own, |_| Some(Status::InProgress)), GroupEnd::NotYet);
/// ```
pub fn group_end<F>(feeding: &Loop, mine: &NodeName, status: F) -> GroupEnd
where
    F: Fn(&NodeName) -> Option<Status>,
{
    // a group's end is acted on once, recorded as the loop's own node's
    let Some(own) = feeding.node.as_ref() else {
        return GroupEnd::NotYet;
    };
    if feeding.acted.as_ref() == Some(own) {
        return GroupEnd::NotYet;
    }
    // a node under way, blocked or gone leaves its session waiting on the user
    if status(mine) != Some(Status::Done) {
        return GroupEnd::NotYet;
    }
    // the rest of the group, whatever isn't finished of it
    let on: Vec<NodeName> = feeding
        .group()
        .filter(|node| *node != mine)
        .filter(|node| unfinished(status(node)))
        .cloned()
        .collect();
    match on.is_empty() {
        true => GroupEnd::Done(own.clone()),
        false => GroupEnd::Waiting {
            done: mine.clone(),
            on,
        },
    }
}

/// Whether a node of a group keeps it waiting: to do, under way or blocked, but not done, and not
/// gone from its plan, which would never finish
///
/// # Arguments
///
/// * `status` - Where it stands, or `None` if its plan has it no more
fn unfinished(status: Option<Status>) -> bool {
    matches!(
        status,
        Some(Status::Todo | Status::InProgress | Status::Blocked)
    )
}

/// Nodes as a list a person reads, each quoted
///
/// # Arguments
///
/// * `nodes` - The nodes
fn listed(nodes: &[NodeName]) -> String {
    let quoted: Vec<String> = nodes.iter().map(|node| format!("`{node}`")).collect();
    quoted.join(", ")
}

/// What comes after a node the loop holds is done: it stops if paused, else the first ready node
/// no other loop holds is next, or nothing is
///
/// # Arguments
///
/// * `feeding` - The pane's loop
/// * `done` - The node it holds, which is done
/// * `next` - The ready work over the loop's plans, the queue's first
/// * `loops` - Every loop of the project, the pane's included
pub fn after(feeding: &Loop, done: NodeName, next: &Next, loops: &[Loop]) -> Ending {
    // next_task's own choice, as it will make it for a group done
    match choose(feeding, Some(Status::Done), &[], next, loops) {
        Choice::Paused => Ending::Paused(done),
        Choice::Take { candidates, .. } => match candidates.into_iter().next() {
            Some(next) => Ending::Next { done, next },
            None => Ending::Finished(done),
        },
        // a node done is never handed out again, and a group done waits on nothing
        Choice::Again(_) | Choice::Waiting(_) => Ending::Nothing,
    }
}

/// What the Stop hook made of a turn's end: the ending, the loop it was judged for, and the
/// member the pane works if it isn't the loop's own pane
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    /// What comes of it
    pub ending: Ending,
    /// The loop of the pane, or the loop whose member it works, if either
    pub feeding: Option<Loop>,
    /// The member the pane works, if it works one
    pub member: Option<NodeName>,
}

impl Ended {
    /// A turn's end with nothing to act on, in a pane with no loop or member
    fn nothing() -> Ended {
        Ended {
            ending: Ending::Nothing,
            feeding: None,
            member: None,
        }
    }
}

/// Judges a turn's end for the loop of the pane the Stop hook runs in, or the loop whose member
/// it works, and records that it acted on its group's end before saying what comes next
///
/// # Arguments
///
/// * `client` - The service's client
/// * `pane` - The pane the session runs in
pub async fn ended(client: &Client, pane: &Pane) -> anyhow::Result<Ended> {
    let loops = client.loops().await?;
    // the pane's own loop and its node, else the loop whose member the pane works
    let (feeding, mine, member) = match loops.iter().find(|feeding| feeding.pane == *pane) {
        Some(feeding) => match &feeding.node {
            Some(node) => (feeding, node.clone(), None),
            None => return Ok(Ended::nothing()),
        },
        None => match member_of(&loops, pane) {
            Some((feeding, member)) => (feeding, member.node.clone(), Some(member.node.clone())),
            None => return Ok(Ended::nothing()),
        },
    };
    let judged = Ended {
        ending: Ending::Nothing,
        feeding: Some(feeding.clone()),
        member: member.clone(),
    };
    // a group whose end was acted on reads nothing more
    if feeding.node.is_some() && feeding.acted == feeding.node {
        return Ok(judged);
    }
    // where each node of the group stands
    let statuses = statuses(client, feeding.group()).await?;
    let ending = match group_end(feeding, &mine, |node| stood(&statuses, node)) {
        GroupEnd::NotYet => Ending::Nothing,
        GroupEnd::Waiting { done, on } => Ending::Waiting {
            done,
            on,
            member: member.is_some(),
        },
        GroupEnd::Done(done) => {
            // what comes after it, over the loops of its project
            let ending = upcoming(client, feeding, done.clone(), &loops).await?;
            // recorded before it is told, and told only by the Stop that recorded it
            match edited(client, &feeding.pane, LoopEdit::Acted { node: done }).await? {
                LoopEdited::Edited { .. } => ending,
                // another Stop acted on it first, or the loop was stopped meanwhile
                LoopEdited::ActedAlready { .. } | LoopEdited::NoLoop { .. } => Ending::Nothing,
                other => anyhow::bail!(
                    "the service answered {other:?} for the loop in {}",
                    feeding.pane
                ),
            }
        }
    };
    Ok(Ended { ending, ..judged })
}

/// Where each of some nodes stands, each plan they are in read once
///
/// # Arguments
///
/// * `client` - The service's client
/// * `nodes` - The nodes
async fn statuses<'a, I>(
    client: &Client,
    nodes: I,
) -> anyhow::Result<HashMap<NodeName, Option<Status>>>
where
    I: IntoIterator<Item = &'a NodeName>,
{
    let mut listed: HashMap<PlanName, reef_api::Listing> = HashMap::new();
    let mut stood = HashMap::new();
    for node in nodes {
        // each plan listed once, however many of its nodes the group holds
        if !listed.contains_key(&node.plan) {
            let listing = client.list(&node.plan).await?;
            listed.insert(node.plan.clone(), listing);
        }
        let status = listed[&node.plan]
            .nodes
            .iter()
            .find(|stored| stored.spec.id == node.id)
            .map(|stored| stored.spec.status.unwrap_or_default());
        stood.insert(node.clone(), status);
    }
    Ok(stood)
}

/// Where a node stands among those read, `None` if it wasn't read or its plan lost it
///
/// # Arguments
///
/// * `statuses` - The statuses read
/// * `node` - The node
fn stood(statuses: &HashMap<NodeName, Option<Status>>, node: &NodeName) -> Option<Status> {
    statuses.get(node).copied().flatten()
}

/// What comes after a node a loop holds that is done, as next_task will take it: the pause, the
/// first ready node no other loop of its project holds, or nothing
///
/// The SessionStart hook asks it of a clear between nodes, so it reads whether the node's end
/// was acted on no more than [`after`] does.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `feeding` - The pane's loop
/// * `done` - The node it holds, which is done
/// * `loops` - Every loop the service keeps, whichever project they feed
pub async fn upcoming(
    client: &Client,
    feeding: &Loop,
    done: NodeName,
    loops: &[Loop],
) -> anyhow::Result<Ending> {
    // the ready work over the loop's plans, the queue's first
    let next = next_for(client, feeding).await?;
    // and the loops of its project, which hold what this one can't take
    let project: Vec<Loop> = loops
        .iter()
        .filter(|other| other.project == feeding.project)
        .cloned()
        .collect();
    Ok(after(feeding, done, &next, &project))
}

/// The ready work a loop takes from: its one plan's, or every plan's of its project
///
/// # Arguments
///
/// * `client` - The service's client
/// * `feeding` - The loop
async fn next_for(client: &Client, feeding: &Loop) -> anyhow::Result<Next> {
    match &feeding.plan {
        Some(plan) => {
            let plan = PlanName::new(feeding.project.as_str(), plan.as_str())?;
            client.next(&plan).await
        }
        None => client.project_next(&feeding.project).await,
    }
}

/// A ready node's full name: its plan's as next names it, else the loop's one plan's
///
/// # Arguments
///
/// * `feeding` - The loop the report was read for
/// * `plan` - The plan next names the node in, if it names one
/// * `id` - The node's id
fn ready_name(feeding: &Loop, plan: Option<&PlanName>, id: &str) -> Option<NodeName> {
    // next names each node's plan when it reads a project's plans, as it does for a loop
    let plan = match plan {
        Some(plan) => plan.clone(),
        None => PlanName::new(feeding.project.as_str(), feeding.plan.as_deref()?).ok()?,
    };
    plan.node(id).ok()
}

/// Takes the next node for a pane's loop, or the next group for a loop wider than one, or hands
/// out the one it is on again, starts it in this checkout, and prints it whole; or hands a
/// member's session its member; or says why there is nothing to take
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, in a checkout of the project's repo
/// * `pane` - The pane the session runs in, if it could be found
/// * `pick` - The project named, if one was, else the working directory's
/// * `open` - Opens a member's pane beside the loop's, as [`crate::open::open`] does
/// * `out` - Where to print the node, or why there is none
pub async fn next_task<W, O>(
    client: &Client,
    cwd: &Path,
    pane: Option<&Pane>,
    pick: &Pick,
    mut open: O,
    out: &mut W,
) -> anyhow::Result<()>
where
    W: Write,
    O: FnMut(&Open) -> anyhow::Result<()>,
{
    // a session whose pane is unknown has no loop to take a node for
    let Some(pane) = pane else {
        anyhow::bail!(
            "reef can't tell which pane this session runs in: no zellij or tmux pane, and no \
             Claude Code process named. Start Claude Code in zellij or tmux, or take a node by \
             hand with `reef start <ID>`"
        );
    };
    // the project, and the loop this pane keeps for it
    let project = cli::picked(client, cwd, pick).await?;
    let loops = client.project_loops(&project.name).await?;
    let Some(feeding) = loops.iter().find(|feeding| feeding.pane == *pane).cloned() else {
        // a pane with no loop of its own may work a group's member
        return member_task(client, cwd, pane, &project.name, &loops, out).await;
    };
    // the checkout must be the project's, before anything is held
    let named = Pick {
        project: Some(project.name.clone()),
    };
    let (checkout, _) = start::checkout_in(client, cwd, &named, "take").await?;
    // where each node of its group stands, then the ready work over the loop's plans
    let statuses = statuses(client, feeding.group()).await?;
    let held = feeding
        .node
        .as_ref()
        .and_then(|node| stood(&statuses, node));
    let waiting: Vec<NodeName> = feeding
        .members
        .iter()
        .map(|member| member.node.clone())
        .filter(|node| unfinished(stood(&statuses, node)))
        .collect();
    let next = next_for(client, &feeding).await?;
    match choose(&feeding, held, &waiting, &next, &loops) {
        Choice::Again(node) => {
            // the node it is on, started here if it never was
            let mut started = Vec::new();
            started_here(client, cwd, &node, &mut started).await?;
            let status = held.unwrap_or_default();
            writeln!(
                out,
                "`{node}` is still this loop's, {}: carry on with it. A call before it is done \
                 hands it out again.\n",
                status.as_str()
            )?;
            hand_out(client, &node, &next, &[], &started, out).await?;
            members_stand(&feeding, &statuses, checkout.root(), out)
        }
        Choice::Waiting(on) => {
            // its own done, the rest of its group is the members' sessions' to finish
            let own = feeding.node.clone().unwrap_or_else(|| on[0].clone());
            writeln!(
                out,
                "`{own}` is done, and its group waits on {}: the loop takes its next group once \
                 they are all done, and this pane is cleared then. Nothing is taken now.",
                listed(&on)
            )?;
            members_stand(&feeding, &statuses, checkout.root(), out)
        }
        Choice::Paused => {
            writeln!(
                out,
                "the loop in {} is paused, so it takes no new node: `! reef loop start` sets it \
                 running again",
                pane.describe()
            )?;
            Ok(())
        }
        Choice::Take {
            candidates,
            elsewhere,
        } => {
            // the last group's members back in this checkout before another group is taken
            let unmerged = unmerged(client, &checkout, &feeding).await?;
            if !unmerged.is_empty() {
                return merge_first(&feeding, &checkout, &unmerged, out);
            }
            let taking = Taking {
                client,
                cwd,
                pane,
                feeding: &feeding,
                root: checkout.root(),
                next: &next,
                loops: &loops,
            };
            taking.take(candidates, elsewhere, &mut open, out).await
        }
    }
}

/// What a take for a pane's loop works from
struct Taking<'a> {
    /// The service's client
    client: &'a Client,
    /// The working directory, in the loop's checkout
    cwd: &'a Path,
    /// The loop's pane
    pane: &'a Pane,
    /// The loop, as read
    feeding: &'a Loop,
    /// The loop's checkout's root, which members' sessions are opened from
    root: &'a Path,
    /// The ready work it takes from
    next: &'a Next,
    /// Every loop of the project
    loops: &'a [Loop],
}

impl Taking<'_> {
    /// Holds the first ready nodes no other loop holds, up to the loop's width, starts the first
    /// here and opens a pane for each member; or says why there is nothing to take
    ///
    /// # Arguments
    ///
    /// * `candidates` - The ready work no other loop holds, in next's order
    /// * `elsewhere` - The ready work another loop holds, each with its pane
    /// * `open` - Opens a member's pane beside the loop's
    /// * `out` - Where to print what came of it
    async fn take<W, O>(
        &self,
        mut candidates: Vec<NodeName>,
        mut elsewhere: Vec<(NodeName, Pane)>,
        open: &mut O,
        out: &mut W,
    ) -> anyhow::Result<()>
    where
        W: Write,
        O: FnMut(&Open) -> anyhow::Result<()>,
    {
        let width = usize::from(self.feeding.width.max(1));
        // the first of them, as many as fit, held as one, since another loop may take one
        while !candidates.is_empty() {
            let group: Vec<NodeName> = candidates.iter().take(width).cloned().collect();
            let hold = LoopEdit::Hold {
                node: Some(group[0].clone()),
                members: group[1..].to_vec(),
            };
            match edited(self.client, self.pane, hold).await? {
                LoopEdited::Edited { r#loop } => {
                    return self.took(&r#loop, &group[0], &elsewhere, open, out).await;
                }
                // another loop took one first, so it is passed over, saying whose it is
                LoopEdited::Held { node, by } => {
                    candidates.retain(|candidate| *candidate != node);
                    elsewhere.push((node, by.pane));
                }
                // gone from its plan meanwhile, or none of the loop's own
                LoopEdited::NotInPlan { node } | LoopEdited::NotInLoop { node, .. } => {
                    candidates.retain(|candidate| *candidate != node);
                }
                LoopEdited::NoLoop { .. } => anyhow::bail!(
                    "the loop in {} was stopped meanwhile, so nothing was taken",
                    self.pane.describe()
                ),
                other => {
                    anyhow::bail!(
                        "the service answered {other:?} for the loop in {}",
                        self.pane
                    )
                }
            }
        }
        // nothing to take: every node the other loops are on, and what everything waits on
        for other in self.loops.iter().filter(|other| other.pane != *self.pane) {
            for node in other.group() {
                if !elsewhere.iter().any(|(held, _)| held == node) {
                    elsewhere.push((node.clone(), other.pane.clone()));
                }
            }
        }
        writeln!(
            out,
            "Nothing is ready to take for the loop in {}.\n",
            self.pane.describe()
        )?;
        held_elsewhere(&elsewhere, out)?;
        write!(out, "{}", self.next)?;
        Ok(())
    }

    /// Starts the node a hold just took here, opens its members' panes, and prints it whole with
    /// its group
    ///
    /// # Arguments
    ///
    /// * `held` - The loop as the hold left it
    /// * `node` - The loop's own node, just held
    /// * `elsewhere` - The ready work another loop holds, each with its pane
    /// * `open` - Opens a member's pane beside the loop's
    /// * `out` - Where to print it
    async fn took<W, O>(
        &self,
        held: &Loop,
        node: &NodeName,
        elsewhere: &[(NodeName, Pane)],
        open: &mut O,
        out: &mut W,
    ) -> anyhow::Result<()>
    where
        W: Write,
        O: FnMut(&Open) -> anyhow::Result<()>,
    {
        let started = claimed(self.client, self.cwd, self.pane, node).await?;
        // each member's session opened beside the loop's, where reef can
        let opened = open_members(held, self.root, open);
        let beside = match held.members.len() {
            0 => String::new(),
            count => format!(
                ", with {count} more of its group beside it for sessions of their own. Work only \
                 `{node}` here"
            ),
        };
        writeln!(
            out,
            "Took `{node}` for the loop in {}{beside}. Plan it now: enter plan mode, read its \
             docs and needs, ask what you need to, and exit plan mode for approval.\n",
            self.pane.describe()
        )?;
        hand_out(self.client, node, self.next, elsewhere, &started, out).await?;
        group_said(&opened, self.root, out)
    }
}

/// How a member came to have a session, or how it can get one
#[derive(Debug, Clone, PartialEq, Eq)]
enum Opened {
    /// reef opened a pane for it beside the loop's
    Pane,
    /// reef couldn't open one, for this reason
    Failed(String),
    /// reef opens none: the loop doesn't, or its pane is no multiplexer's
    Waits,
}

/// Opens a pane beside the loop's for each member of the group it holds, where it opens them
///
/// # Arguments
///
/// * `held` - The loop, holding its group
/// * `root` - The loop's checkout, which each member's session starts from
/// * `open` - Opens a member's pane
fn open_members<O>(held: &Loop, root: &Path, open: &mut O) -> Vec<(NodeName, Opened)>
where
    O: FnMut(&Open) -> anyhow::Result<()>,
{
    // only a loop that opens them, in a pane reef can open one beside
    let opens = held.opens && held.pane.typed();
    held.members
        .iter()
        .map(|member| {
            if !opens {
                return (member.node.clone(), Opened::Waits);
            }
            let order = Open {
                beside: held.pane.clone(),
                root: root.to_path_buf(),
                id: member.node.id.clone(),
            };
            let opened = match open(&order) {
                Ok(()) => Opened::Pane,
                Err(error) => Opened::Failed(format!("{error:#}")),
            };
            (member.node.clone(), opened)
        })
        .collect()
}

/// Prints a group just taken: each member, and how it got a session or can get one
///
/// # Arguments
///
/// * `opened` - Each member, and what came of opening its pane
/// * `root` - The loop's checkout
/// * `out` - Where to print it
fn group_said<W: Write>(
    opened: &[(NodeName, Opened)],
    root: &Path,
    out: &mut W,
) -> anyhow::Result<()> {
    if opened.is_empty() {
        return Ok(());
    }
    writeln!(out, "\nIts group ({} beside it):", opened.len())?;
    for (node, how) in opened {
        let said = match how {
            Opened::Pane => format!(
                "a pane of its own was opened, running `claude -w {}`",
                node.id
            ),
            Opened::Failed(why) => {
                format!("reef couldn't open its pane ({why}): {}", ways(node, root))
            }
            Opened::Waits => ways(node, root),
        };
        writeln!(out, "  {node}  {said}")?;
    }
    Ok(())
}

/// The ways a member waiting for a session gets one
///
/// # Arguments
///
/// * `node` - The member's node
/// * `root` - The loop's checkout
fn ways(node: &NodeName, root: &Path) -> String {
    format!(
        "waits for a session of its own: `claude -w {id}` run from {}, or `! reef loop attach \
         {id}` in a Claude session already open",
        root.display(),
        id = node.id
    )
}

/// Prints where each member of a loop's group stands, if it has any
///
/// # Arguments
///
/// * `feeding` - The loop
/// * `statuses` - Where each node of its group stands
/// * `root` - The loop's checkout
/// * `out` - Where to print it
fn members_stand<W: Write>(
    feeding: &Loop,
    statuses: &HashMap<NodeName, Option<Status>>,
    root: &Path,
    out: &mut W,
) -> anyhow::Result<()> {
    if feeding.members.is_empty() {
        return Ok(());
    }
    writeln!(out, "\nIts group ({} beside it):", feeding.members.len())?;
    for member in &feeding.members {
        let status = stood(statuses, &member.node).map_or("gone from its plan", Status::as_str);
        let session = match &member.pane {
            Some(pane) => format!("worked in {}", pane.describe()),
            None => ways(&member.node, root),
        };
        writeln!(out, "  {}  {status}, {session}", member.node)?;
    }
    Ok(())
}

/// The members of a loop's last group whose branches aren't in its checkout's HEAD, each with its
/// branch
///
/// A member never started, or started with no branch, has nothing to merge, and a branch gone,
/// as a squash and a delete leave it, is merged.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `checkout` - The loop's checkout
/// * `feeding` - The loop, holding its last group
async fn unmerged(
    client: &Client,
    checkout: &Checkout,
    feeding: &Loop,
) -> anyhow::Result<Vec<(NodeName, String)>> {
    // a checkout with no commit has nothing merged into it, and nothing to take a group from
    let Some(head) = checkout.head()? else {
        return Ok(Vec::new());
    };
    let mut unmerged = Vec::new();
    for member in &feeding.members {
        // the branch it was started on, as its record says
        let Some(record) = client.work_of(&member.node).await? else {
            continue;
        };
        let Some(branch) = record.start.branch else {
            continue;
        };
        let Some(tip) = checkout.commit(&format!("refs/heads/{branch}"))? else {
            continue;
        };
        if !checkout.is_ancestor(&tip, &head)? {
            unmerged.push((member.node.clone(), branch));
        }
    }
    Ok(unmerged)
}

/// Says a loop's last group must be merged before the next is taken, and how
///
/// # Arguments
///
/// * `feeding` - The loop
/// * `checkout` - Its checkout
/// * `unmerged` - Each member whose branch isn't in its HEAD, with that branch
/// * `out` - Where to say it
fn merge_first<W: Write>(
    feeding: &Loop,
    checkout: &Checkout,
    unmerged: &[(NodeName, String)],
    out: &mut W,
) -> anyhow::Result<()> {
    // the branch it merges into, or the checkout's HEAD where none is checked out
    let into = match checkout.branch()? {
        Some(branch) => format!("`{branch}`"),
        None => "this checkout's HEAD".to_string(),
    };
    let own = feeding
        .node
        .as_ref()
        .map_or_else(|| "The loop's".to_string(), |node| format!("`{node}`'s"));
    writeln!(
        out,
        "{own} group is done, but {} of its members' branches aren't in {into} yet, so no new \
         group is taken. Merge each into this checkout, resolving any conflict as the repo's own \
         instructions say, then call next_task again:\n",
        unmerged.len()
    )?;
    for (node, branch) in unmerged {
        writeln!(out, "  git merge {branch}    # {node}")?;
    }
    Ok(())
}

/// Hands a session in a pane with no loop of its own the member it works: the one its pane
/// joined, else the one its checkout's worktree is on that no pane joined, joined from here
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `pane` - The session's pane
/// * `project` - The project's name
/// * `loops` - Every loop of the project
/// * `out` - Where to print the node, or why there is none
async fn member_task<W: Write>(
    client: &Client,
    cwd: &Path,
    pane: &Pane,
    project: &str,
    loops: &[Loop],
    out: &mut W,
) -> anyhow::Result<()> {
    // the member this pane joined, else one this worktree is on, joined now
    let (by, node) = match member_of(loops, pane) {
        Some((by, member)) => (by.clone(), member.node.clone()),
        None => match unjoined_here(client, cwd, project, loops).await? {
            Some(node) => (joined_here(client, pane, &node).await?, node),
            None => anyhow::bail!(
                "no loop runs in {} for `{project}`: `! reef loop start` starts one there, typed \
                 into the session's own prompt, or `! reef loop attach` works a member of \
                 another loop's group here",
                pane.describe()
            ),
        },
    };
    // started in its own worktree already, never in whatever checkout this is
    let Some(record) = client.work_of(&node).await? else {
        anyhow::bail!(
            "`{node}` hasn't been started: `claude -w {id}` run from the loop's checkout, or `! \
             reef loop attach {id}` here, starts it in a worktree of its own",
            id = node.id
        );
    };
    let mut said = Vec::new();
    match status_of(client, &node).await? {
        // a member done has nothing more for its session
        Some(Status::Done) => {
            writeln!(
                out,
                "`{node}`, this session's member of the loop in {}'s group, is done: this \
                 session can be closed. The loop takes its next group once the whole group is \
                 done.",
                by.pane.describe()
            )?;
            return Ok(());
        }
        Some(Status::Todo) => {
            work::status(client, &node.plan, &node.id, Status::InProgress, &mut said).await?;
        }
        _ => {}
    }
    writeln!(
        out,
        "`{node}` is this session's, a member of the loop in {}'s group, started in {} on `{}`. \
         Work it there and nothing else. Plan it now: enter plan mode, read its docs and needs, \
         ask what you need to, and exit plan mode for approval. Commit its work on that branch: \
         the loop merges it once the group is done.\n",
        by.pane.describe(),
        record.start.worktree.display(),
        record.start.branch.as_deref().unwrap_or("its own branch")
    )?;
    hand_out(client, &node, &Next::default(), &[], &said, out).await
}

/// The member a checkout's worktree is on, if it is one no pane joined
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `project` - The project's name
/// * `loops` - Every loop of the project
async fn unjoined_here(
    client: &Client,
    cwd: &Path,
    project: &str,
    loops: &[Loop],
) -> anyhow::Result<Option<NodeName>> {
    // a directory outside git is no member's worktree
    let Ok(checkout) = Checkout::open(cwd) else {
        return Ok(None);
    };
    let records = client.work(project).await?;
    let Some(record) = on_worktree(&records, checkout.root()) else {
        return Ok(None);
    };
    // a member of some loop's group, that no session has joined
    let unjoined = loops
        .iter()
        .find_map(|feeding| feeding.member(&record.node))
        .is_some_and(|member| member.pane.is_none());
    Ok(unjoined.then(|| record.node.clone()))
}

/// Joins a pane to a member, returning the loop whose member it is
///
/// # Arguments
///
/// * `client` - The service's client
/// * `pane` - The pane
/// * `node` - The member's node
async fn joined_here(client: &Client, pane: &Pane, node: &NodeName) -> anyhow::Result<Loop> {
    match edited(client, pane, LoopEdit::Join { node: node.clone() }).await? {
        LoopEdited::Joined { r#loop } => Ok(r#loop),
        LoopEdited::Held { .. } => {
            anyhow::bail!("`{node}` has a session already, in another pane: work it there")
        }
        other => anyhow::bail!("the service answered {other:?} for {pane} joining `{node}`"),
    }
}

/// Where a node stands, or `None` if its plan has it no more
///
/// # Arguments
///
/// * `client` - The service's client
/// * `node` - The node's full name
async fn status_of(client: &Client, node: &NodeName) -> anyhow::Result<Option<Status>> {
    // the plan as it stands, the node's status unset meaning to do
    let listing = client.list(&node.plan).await?;
    Ok(listing
        .nodes
        .iter()
        .find(|stored| stored.spec.id == node.id)
        .map(|stored| stored.spec.status.unwrap_or_default()))
}

/// Starts a node just held, letting go of it if it can't be, and returns what the start printed
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, in the node's checkout
/// * `pane` - The pane whose loop holds it
/// * `node` - The node held
async fn claimed(
    client: &Client,
    cwd: &Path,
    pane: &Pane,
    node: &NodeName,
) -> anyhow::Result<Vec<u8>> {
    let mut started = Vec::new();
    let Err(error) = started_here(client, cwd, node, &mut started).await else {
        return Ok(started);
    };
    // a node that can't be started is let go of, so the loop isn't stuck on it
    let release = edited(
        client,
        pane,
        LoopEdit::Hold {
            node: None,
            members: Vec::new(),
        },
    )
    .await;
    match release {
        Ok(_) => Err(error.context(format!(
            "`{node}` couldn't be started here, so the loop let go of it"
        ))),
        Err(release) => Err(error.context(format!(
            "`{node}` couldn't be started here, and letting go of it failed too: {release:#}"
        ))),
    }
}

/// Starts a node in this checkout as `reef start` does, unless it was started already, and
/// marks it in progress
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, in the node's checkout
/// * `node` - The node's full name
/// * `out` - Where to print what came of it
async fn started_here<W: Write>(
    client: &Client,
    cwd: &Path,
    node: &NodeName,
    out: &mut W,
) -> anyhow::Result<()> {
    // a node started already keeps its record, wherever it was started
    if let Some(record) = client.work_of(node).await? {
        if status_of(client, node).await? == Some(Status::Todo) {
            work::status(client, &node.plan, &node.id, Status::InProgress, out).await?;
        }
        writeln!(
            out,
            "`{}` was started already, {}",
            node.id,
            start::where_started(&record)
        )?;
        return Ok(());
    }
    // else started here, pinned and recorded, and marked in progress
    let pick = PlanPick {
        project: Pick {
            project: Some(node.plan.project.clone()),
        },
        plan: Some(node.plan.plan.clone()),
    };
    start::start(client, cwd, &node.id, &pick, None, false, out).await
}

/// Prints a node handed out: the node as a node file, then the queue's entries passed over,
/// the ready work another loop holds, and what its start printed
///
/// # Arguments
///
/// * `client` - The service's client
/// * `node` - The node handed out
/// * `next` - The report it was chosen from
/// * `elsewhere` - The ready work another loop holds, each with its pane
/// * `started` - What starting it printed
/// * `out` - Where to print it all
async fn hand_out<W: Write>(
    client: &Client,
    node: &NodeName,
    next: &Next,
    elsewhere: &[(NodeName, Pane)],
    started: &[u8],
    out: &mut W,
) -> anyhow::Result<()> {
    // the node whole, as `reef node get --md` prints it
    node::get(client, &node.plan, &node.id, true, out).await?;
    // the queue's entries passed over, each with why
    if !next.skipped.is_empty() {
        writeln!(out, "\nSkipped in the queue ({}):", next.skipped.len())?;
        for entry in &next.skipped {
            writeln!(out, "  {}  {}", entry.node, entry.title)?;
            for why in entry_lines(&entry.state) {
                writeln!(out, "      {why}")?;
            }
        }
    }
    if !elsewhere.is_empty() {
        writeln!(out)?;
        held_elsewhere(elsewhere, out)?;
    }
    // then what its start said
    if !started.is_empty() {
        writeln!(out)?;
        out.write_all(started)?;
    }
    Ok(())
}

/// Prints the ready work another loop holds, each with the pane holding it, if there is any
///
/// # Arguments
///
/// * `elsewhere` - The ready work another loop holds, each with its pane
/// * `out` - Where to print it
fn held_elsewhere<W: Write>(elsewhere: &[(NodeName, Pane)], out: &mut W) -> anyhow::Result<()> {
    if elsewhere.is_empty() {
        return Ok(());
    }
    writeln!(out, "Held by other loops ({}):", elsewhere.len())?;
    for (node, pane) in elsewhere {
        writeln!(out, "  {node}  by the loop in {}", pane.describe())?;
    }
    writeln!(out)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{Choice, Ending, GroupEnd, after, choose, group_end};
    use reef_api::{Loop, LoopState, Member, Pane};
    use reef_model::{Next, NodeName, PlanName, Ready, Status};
    use std::collections::HashMap;

    /// Names a node of `reef/central`
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    fn named(id: &str) -> NodeName {
        format!("reef/central/{id}").parse().unwrap()
    }

    /// A running loop of `reef/central` in a Claude Code process, holding a node or none
    ///
    /// # Arguments
    ///
    /// * `pid` - The process it is kept by
    /// * `node` - The id of the node it holds
    fn looped(pid: u32, node: Option<&str>) -> Loop {
        Loop {
            pane: Pane::Claude { pid },
            project: "reef".to_string(),
            plan: Some("central".to_string()),
            session: None,
            node: node.map(named),
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

    /// Judges a group's end from statuses given by id, a node not named being gone
    ///
    /// # Arguments
    ///
    /// * `feeding` - The loop
    /// * `mine` - The id of the node the pane works
    /// * `statuses` - Each node's id and where it stands
    fn judged(feeding: &Loop, mine: &str, statuses: &[(&str, Status)]) -> GroupEnd {
        let stood: HashMap<NodeName, Status> = statuses
            .iter()
            .map(|(id, status)| (named(id), *status))
            .collect();
        group_end(feeding, &named(mine), |node| stood.get(node).copied())
    }

    /// A report whose ready work is these nodes of `reef/central`, in this order
    ///
    /// # Arguments
    ///
    /// * `ids` - The ready nodes' ids
    fn ready(ids: &[&str]) -> Next {
        let plan = PlanName::new("reef", "central").unwrap();
        Next {
            ready: ids
                .iter()
                .map(|id| Ready {
                    id: id.to_string(),
                    title: format!("{id} Some work"),
                    plan: Some(plan.clone()),
                    queued: false,
                })
                .collect(),
            plans: vec![plan],
            ..Next::default()
        }
    }

    /// A second call before the node is done returns the same node, and one once it is done or
    /// blocked takes the next
    #[test]
    fn a_second_call_before_the_node_is_done_returns_the_same_node() {
        // a loop on C40, with C41 and C42 ready
        let on = looped(1, Some("C40"));
        let next = ready(&["C41", "C42"]);
        let loops = [on.clone()];
        // to do or under way, C40 is handed out again, however much else is ready
        for status in [Status::Todo, Status::InProgress] {
            assert_eq!(
                choose(&on, Some(status), &[], &next, &loops),
                Choice::Again(named("C40"))
            );
        }
        // done, blocked or gone from its plan, the ready work is taken in next's order
        let take = Choice::Take {
            candidates: vec![named("C41"), named("C42")],
            elsewhere: Vec::new(),
        };
        for held in [Some(Status::Done), Some(Status::Blocked), None] {
            assert_eq!(choose(&on, held, &[], &next, &loops), take);
        }
        // a paused loop keeps its node until it is done, then takes none
        let paused = Loop {
            state: LoopState::Paused,
            ..on.clone()
        };
        assert_eq!(
            choose(&paused, Some(Status::InProgress), &[], &next, &loops),
            Choice::Again(named("C40"))
        );
        assert_eq!(
            choose(&paused, Some(Status::Done), &[], &next, &loops),
            Choice::Paused
        );
    }

    /// The Stop hook acts on a node's end once: a node done is its end until the loop records
    /// acting on it, and an earlier node acted on leaves the new one's end to act on
    #[test]
    fn it_never_acts_twice_on_one_nodes_end() {
        // C40 done, its end not acted on yet, is the end to act on
        let on = looped(1, Some("C40"));
        let done = [("C40", Status::Done)];
        assert_eq!(judged(&on, "C40", &done), GroupEnd::Done(named("C40")));
        // once acted on, every later turn's end is no node's, however often it comes
        let acted = Loop {
            acted: Some(named("C40")),
            ..on.clone()
        };
        for _ in 0..3 {
            assert_eq!(judged(&acted, "C40", &done), GroupEnd::NotYet);
        }
        // the end acted on last being an earlier node's leaves C40's to act on
        let earlier = Loop {
            acted: Some(named("C39")),
            ..on
        };
        assert_eq!(judged(&earlier, "C40", &done), GroupEnd::Done(named("C40")));
    }

    /// The Stop hook names no next node mid-node, when the loop is paused, or once nothing else
    /// is ready, and names the first ready node no other loop holds otherwise
    #[test]
    fn it_does_nothing_mid_node_when_paused_or_once_the_plan_is_finished() {
        // mid-node, blocked, gone from its plan or holding nothing, a turn's end is no node's
        let on = looped(1, Some("C40"));
        for held in [Status::Todo, Status::InProgress, Status::Blocked] {
            assert_eq!(
                judged(&on, "C40", &[("C40", held)]),
                GroupEnd::NotYet,
                "{held:?}"
            );
        }
        assert_eq!(judged(&on, "C40", &[]), GroupEnd::NotYet);
        let none = looped(1, None);
        assert_eq!(
            judged(&none, "C40", &[("C40", Status::Done)]),
            GroupEnd::NotYet
        );
        // a paused loop stops at C40's end, however much is ready
        let next = ready(&["C41", "C42"]);
        let paused = Loop {
            state: LoopState::Paused,
            ..on.clone()
        };
        assert_eq!(
            after(&paused, named("C40"), &next, std::slice::from_ref(&paused)),
            Ending::Paused(named("C40"))
        );
        // nothing ready, or all of it another loop's, is the plan finished for this loop
        assert_eq!(
            after(&on, named("C40"), &ready(&[]), std::slice::from_ref(&on)),
            Ending::Finished(named("C40"))
        );
        let there = looped(2, Some("C41"));
        let loops = [on.clone(), there];
        assert_eq!(
            after(&on, named("C40"), &ready(&["C41"]), &loops),
            Ending::Finished(named("C40"))
        );
        // else the first ready node no other loop holds is next
        assert_eq!(
            after(&on, named("C40"), &next, &loops),
            Ending::Next {
                done: named("C40"),
                next: named("C42")
            }
        );
        assert_eq!(
            after(&on, named("C40"), &next, std::slice::from_ref(&on)),
            Ending::Next {
                done: named("C40"),
                next: named("C41")
            }
        );
        // and only an ending that acted says anything
        assert_eq!(Ending::Nothing.message(false), None);
        assert_eq!(Ending::Nothing.message(true), None);
        let told = Ending::Next {
            done: named("C40"),
            next: named("C41"),
        };
        assert_eq!(
            told.message(false).as_deref(),
            Some(
                "`reef/central/C40` is done. `reef/central/C41` is next: `/clear`, then say `next`"
            )
        );
    }

    /// The next node's message says reef clears the context where it types into the pane, and
    /// that the user does elsewhere, while a pause or a finished plan reads the same either way
    #[test]
    fn the_next_nodes_message_says_who_clears() {
        // reef types into a multiplexer's pane
        let next = Ending::Next {
            done: named("C40"),
            next: named("C41"),
        };
        assert_eq!(
            next.message(true).as_deref(),
            Some(
                "`reef/central/C40` is done. `reef/central/C41` is next: reef clears the context \
                 and asks for it once Claude is idle and the input box is empty"
            )
        );
        // and the user clears and says one word anywhere else
        assert!(
            next.message(false)
                .unwrap()
                .ends_with("is next: `/clear`, then say `next`")
        );
        // a pause or a finished plan types nothing, so it reads the same
        for ending in [Ending::Paused(named("C40")), Ending::Finished(named("C40"))] {
            assert_eq!(ending.message(true), ending.message(false));
        }
    }

    /// A node another loop holds is never a candidate, and is listed with the pane holding it
    #[test]
    fn another_loops_node_is_passed_over() {
        // this loop holds nothing, and another holds C41
        let here = looped(1, None);
        let there = looped(2, Some("C41"));
        let next = ready(&["C41", "C42"]);
        let chosen = choose(&here, None, &[], &next, &[here.clone(), there.clone()]);
        assert_eq!(
            chosen,
            Choice::Take {
                candidates: vec![named("C42")],
                elsewhere: vec![(named("C41"), there.pane.clone())],
            }
        );
        // and a loop of another plan takes none of this one's work
        let other = Loop {
            plan: Some("other".to_string()),
            ..looped(3, None)
        };
        assert_eq!(
            choose(&other, None, &[], &next, std::slice::from_ref(&other)),
            Choice::Take {
                candidates: Vec::new(),
                elsewhere: Vec::new(),
            }
        );
    }

    /// A loop of width two in zellij, on C40 with C41 beside it
    ///
    /// # Arguments
    ///
    /// * `joined` - Whether a session has joined C41, from tmux pane `%5`
    fn grouped(joined: bool) -> Loop {
        let pane = joined.then(|| Pane::Tmux {
            server: "/tmp/t".to_string(),
            pane: "%5".to_string(),
        });
        Loop {
            width: 2,
            members: vec![Member {
                node: named("C41"),
                pane,
            }],
            ..looped(1, Some("C40"))
        }
    }

    /// A group's end comes once all of it is done, whichever of its nodes ends last, and is the
    /// loop's own node's; until then each node's end waits on the rest
    #[test]
    fn a_group_ends_once_all_of_it_is_done() {
        let wide = grouped(true);
        // the loop's own node done first waits on its member
        let own_first = [("C40", Status::Done), ("C41", Status::InProgress)];
        assert_eq!(
            judged(&wide, "C40", &own_first),
            GroupEnd::Waiting {
                done: named("C40"),
                on: vec![named("C41")]
            }
        );
        // and the member's session, its node not done, waits on its user
        assert_eq!(judged(&wide, "C41", &own_first), GroupEnd::NotYet);
        // the member done first waits on the loop's own, a blocked one included
        for own in [Status::InProgress, Status::Blocked] {
            let member_first = [("C40", own), ("C41", Status::Done)];
            assert_eq!(
                judged(&wide, "C41", &member_first),
                GroupEnd::Waiting {
                    done: named("C41"),
                    on: vec![named("C40")]
                }
            );
        }
        // all done, either pane's end is the group's, named as the loop's own node
        let all = [("C40", Status::Done), ("C41", Status::Done)];
        for mine in ["C40", "C41"] {
            assert_eq!(judged(&wide, mine, &all), GroupEnd::Done(named("C40")));
        }
        // a member gone from its plan never finishes, so it isn't waited on
        assert_eq!(
            judged(&wide, "C40", &[("C40", Status::Done)]),
            GroupEnd::Done(named("C40"))
        );
        // and once acted on, no pane's end is the group's again
        let acted = Loop {
            acted: Some(named("C40")),
            ..wide
        };
        for mine in ["C40", "C41"] {
            assert_eq!(judged(&acted, mine, &all), GroupEnd::NotYet);
        }
    }

    /// A loop whose own node is done takes nothing while its group's members aren't, and takes
    /// as next does once they are; a pause still stops it
    #[test]
    fn a_loop_takes_no_new_group_while_its_members_work() {
        let wide = grouped(true);
        let next = ready(&["C42", "C43"]);
        let loops = [wide.clone()];
        // its own done and C41 still under way, it waits on C41
        assert_eq!(
            choose(&wide, Some(Status::Done), &[named("C41")], &next, &loops),
            Choice::Waiting(vec![named("C41")])
        );
        // its own under way, it is handed out again whatever the members do
        assert_eq!(
            choose(
                &wide,
                Some(Status::InProgress),
                &[named("C41")],
                &next,
                &loops
            ),
            Choice::Again(named("C40"))
        );
        // all of it done, the ready work is taken in next's order
        assert_eq!(
            choose(&wide, Some(Status::Done), &[], &next, &loops),
            Choice::Take {
                candidates: vec![named("C42"), named("C43")],
                elsewhere: Vec::new(),
            }
        );
        // a member another loop's group holds is never a candidate for this one
        let here = looped(2, None);
        let both = [wide.clone(), here.clone()];
        assert_eq!(
            choose(&here, None, &[], &ready(&["C41", "C42"]), &both),
            Choice::Take {
                candidates: vec![named("C42")],
                elsewhere: vec![(named("C41"), wide.pane.clone())],
            }
        );
        // and a group's end, all of it done, comes after as one node's would
        assert_eq!(
            after(&wide, named("C40"), &next, &loops),
            Ending::Next {
                done: named("C40"),
                next: named("C42")
            }
        );
    }

    /// A node done while its group isn't says what the group waits on, and a member's session
    /// is told it can be closed
    #[test]
    fn a_wait_on_the_group_says_who_moves_on() {
        let waiting = |member: bool| Ending::Waiting {
            done: named("C41"),
            on: vec![named("C40")],
            member,
        };
        assert_eq!(
            waiting(true).message(true).as_deref(),
            Some(
                "`reef/central/C41` is done. Its group waits on `reef/central/C40`: the loop's \
                 pane moves on once they are all done, so this session can be closed"
            )
        );
        assert!(
            waiting(false)
                .message(false)
                .unwrap()
                .ends_with("this pane moves on once they are all done")
        );
    }
}
