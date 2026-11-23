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
//!   the next node, or says the loop is paused or nothing else is ready.

use reef_api::loops::holder;
use reef_api::{EditLoop, Loop, LoopEdit, LoopEdited, LoopState, Pane};
use reef_model::next::entry_lines;
use reef_model::{Next, NodeName, PlanName, Status};
use std::io::Write;
use std::path::Path;

use crate::cli::{self, Pick, PlanPick};
use crate::client::Client;
use crate::{node, start, work};

/// What next_task does for a pane's loop, judged before anything is held
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// The loop holds a node not done yet, which it hands out again
    Again(NodeName),
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

/// Judges what next_task does for a pane's loop: hand out its node again, take nothing, or take
/// the first ready node no other loop holds
///
/// # Arguments
///
/// * `feeding` - The pane's loop
/// * `held` - Where the node it holds stands, or `None` if it holds none or its plan lost it
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
/// };
/// let again = choose(&feeding, Some(Status::InProgress), &Next::default(), &[]);
/// assert_eq!(again, Choice::Again("reef/central/C40".parse().unwrap()));
/// ```
pub fn choose(feeding: &Loop, held: Option<Status>, next: &Next, loops: &[Loop]) -> Choice {
    // the node it holds is the one it is on, while it is to do or under way
    if let (Some(node), Some(Status::Todo | Status::InProgress)) = (&feeding.node, held) {
        return Choice::Again(node.clone());
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
    /// # Examples
    ///
    /// ```
    /// use reef::task::Ending;
    ///
    /// let paused = Ending::Paused("reef/central/C41".parse().unwrap());
    /// assert!(paused.message().unwrap().contains("the loop is paused"));
    /// assert_eq!(Ending::Nothing.message(), None);
    /// ```
    pub fn message(&self) -> Option<String> {
        match self {
            Ending::Nothing => None,
            Ending::Paused(done) => Some(format!(
                "`{done}` is done, and the loop is paused: `! reef loop start` sets it running \
                 again"
            )),
            Ending::Finished(done) => Some(format!(
                "`{done}` is done, and nothing else is ready to take: `reef next` says what \
                 everything waits on"
            )),
            Ending::Next { done, next } => Some(format!(
                "`{done}` is done. `{next}` is next: `/clear`, then ask for it"
            )),
        }
    }
}

/// The node whose end a turn's end is: the one the loop holds, once it is done and its end
/// wasn't acted on yet
///
/// `stop_hook_active` is never read: it stays true across a whole chain of turns, so the node
/// whose end the loop acted on is the guard.
///
/// # Arguments
///
/// * `feeding` - The pane's loop
/// * `held` - Where the node it holds stands, or `None` if it holds none or its plan lost it
///
/// # Examples
///
/// ```
/// use reef::task::end_of;
/// use reef_api::{Loop, LoopState, Pane};
/// use reef_model::Status;
///
/// let feeding = Loop {
///     pane: Pane::Claude { pid: 7 },
///     project: "reef".to_string(),
///     plan: None,
///     session: None,
///     node: Some("reef/central/C41".parse().unwrap()),
///     acted: None,
///     state: LoopState::Running,
///     version: 2,
///     started_at: 1,
/// };
/// assert_eq!(end_of(&feeding, Some(Status::Done)), feeding.node);
/// assert_eq!(end_of(&feeding, Some(Status::InProgress)), None);
/// ```
pub fn end_of(feeding: &Loop, held: Option<Status>) -> Option<NodeName> {
    // a node under way, blocked or gone leaves Claude waiting on the user
    let node = feeding.node.as_ref()?;
    if held != Some(Status::Done) {
        return None;
    }
    // and a node's end is acted on once
    (feeding.acted.as_ref() != Some(node)).then(|| node.clone())
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
    // next_task's own choice, as it will make it for a node done
    match choose(feeding, Some(Status::Done), next, loops) {
        Choice::Paused => Ending::Paused(done),
        Choice::Take { candidates, .. } => match candidates.into_iter().next() {
            Some(next) => Ending::Next { done, next },
            None => Ending::Finished(done),
        },
        // a node done is never handed out again
        Choice::Again(_) => Ending::Nothing,
    }
}

/// Judges a turn's end for the loop of the pane the Stop hook runs in, and records that it acted
/// on its node's end before saying what comes next
///
/// # Arguments
///
/// * `client` - The service's client
/// * `pane` - The pane the session runs in
pub async fn ended(client: &Client, pane: &Pane) -> anyhow::Result<Ending> {
    // the pane's loop, if it has one
    let loops = client.loops().await?;
    let Some(feeding) = loops.iter().find(|feeding| feeding.pane == *pane) else {
        return Ok(Ending::Nothing);
    };
    // a loop holding nothing, or whose node's end was acted on, reads nothing more
    let Some(node) = &feeding.node else {
        return Ok(Ending::Nothing);
    };
    if feeding.acted.as_ref() == Some(node) {
        return Ok(Ending::Nothing);
    }
    // the node's end only once it is done
    let held = status_of(client, node).await?;
    let Some(done) = end_of(feeding, held) else {
        return Ok(Ending::Nothing);
    };
    // what comes after it, over the loops of its project
    let next = next_for(client, feeding).await?;
    let project: Vec<Loop> = loops
        .iter()
        .filter(|other| other.project == feeding.project)
        .cloned()
        .collect();
    let ending = after(feeding, done.clone(), &next, &project);
    // recorded before it is told, and told only by the Stop that recorded it
    match edited(client, pane, LoopEdit::Acted { node: done }).await? {
        LoopEdited::Edited { .. } => Ok(ending),
        // another Stop acted on it first, or the loop was stopped meanwhile
        LoopEdited::ActedAlready { .. } | LoopEdited::NoLoop { .. } => Ok(Ending::Nothing),
        other => anyhow::bail!("the service answered {other:?} for the loop in {pane}"),
    }
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

/// Takes the next node for a pane's loop, or hands out the one it is on again, starts it in
/// this checkout, and prints it whole; or says why there is nothing to take
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, in a checkout of the project's repo
/// * `pane` - The pane the session runs in, if it could be found
/// * `pick` - The project named, if one was, else the working directory's
/// * `out` - Where to print the node, or why there is none
pub async fn next_task<W: Write>(
    client: &Client,
    cwd: &Path,
    pane: Option<&Pane>,
    pick: &Pick,
    out: &mut W,
) -> anyhow::Result<()> {
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
        anyhow::bail!(
            "no loop runs in {} for `{}`: `! reef loop start` starts one there, typed into the \
             session's own prompt",
            pane.describe(),
            project.name
        );
    };
    // the checkout must be the project's, before anything is held
    let named = Pick {
        project: Some(project.name.clone()),
    };
    start::checkout_in(client, cwd, &named, "take").await?;
    // where the node it holds stands, then the ready work over the loop's plans
    let held = match &feeding.node {
        Some(node) => status_of(client, node).await?,
        None => None,
    };
    let next = next_for(client, &feeding).await?;
    match choose(&feeding, held, &next, &loops) {
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
            hand_out(client, &node, &next, &[], &started, out).await
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
            mut elsewhere,
        } => {
            // each in turn, until one is held, since another loop may take one meanwhile
            for node in candidates {
                let hold = LoopEdit::Hold {
                    node: Some(node.clone()),
                };
                match edited(client, pane, hold).await? {
                    LoopEdited::Edited { .. } => {
                        let started = claimed(client, cwd, pane, &node).await?;
                        writeln!(
                            out,
                            "Took `{node}` for the loop in {}. Plan it now: enter plan mode, \
                             read its docs and needs, ask what you need to, and exit plan mode \
                             for approval.\n",
                            pane.describe()
                        )?;
                        return hand_out(client, &node, &next, &elsewhere, &started, out).await;
                    }
                    // another loop took it first, so it is passed over, saying whose it is
                    LoopEdited::Held { node, by } => elsewhere.push((node, by.pane)),
                    // gone from its plan meanwhile, or none of the loop's own
                    LoopEdited::NotInPlan { .. } | LoopEdited::NotInLoop { .. } => {}
                    LoopEdited::NoLoop { .. } => anyhow::bail!(
                        "the loop in {} was stopped meanwhile, so nothing was taken",
                        pane.describe()
                    ),
                    other => {
                        anyhow::bail!("the service answered {other:?} for the loop in {pane}")
                    }
                }
            }
            // nothing to take: every node the other loops are on, and what everything waits on
            for other in loops.iter().filter(|other| other.pane != *pane) {
                if let Some(node) = &other.node
                    && !elsewhere.iter().any(|(held, _)| held == node)
                {
                    elsewhere.push((node.clone(), other.pane.clone()));
                }
            }
            writeln!(
                out,
                "Nothing is ready to take for the loop in {}.\n",
                pane.describe()
            )?;
            held_elsewhere(&elsewhere, out)?;
            write!(out, "{next}")?;
            Ok(())
        }
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
    let release = edited(client, pane, LoopEdit::Hold { node: None }).await;
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
    use super::{Choice, Ending, after, choose, end_of};
    use reef_api::{Loop, LoopState, Pane};
    use reef_model::{Next, NodeName, PlanName, Ready, Status};

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
        }
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
                choose(&on, Some(status), &next, &loops),
                Choice::Again(named("C40"))
            );
        }
        // done, blocked or gone from its plan, the ready work is taken in next's order
        let take = Choice::Take {
            candidates: vec![named("C41"), named("C42")],
            elsewhere: Vec::new(),
        };
        for held in [Some(Status::Done), Some(Status::Blocked), None] {
            assert_eq!(choose(&on, held, &next, &loops), take);
        }
        // a paused loop keeps its node until it is done, then takes none
        let paused = Loop {
            state: LoopState::Paused,
            ..on.clone()
        };
        assert_eq!(
            choose(&paused, Some(Status::InProgress), &next, &loops),
            Choice::Again(named("C40"))
        );
        assert_eq!(
            choose(&paused, Some(Status::Done), &next, &loops),
            Choice::Paused
        );
    }

    /// The Stop hook acts on a node's end once: a node done is its end until the loop records
    /// acting on it, and an earlier node acted on leaves the new one's end to act on
    #[test]
    fn it_never_acts_twice_on_one_nodes_end() {
        // C40 done, its end not acted on yet, is the end to act on
        let on = looped(1, Some("C40"));
        assert_eq!(end_of(&on, Some(Status::Done)), Some(named("C40")));
        // once acted on, every later turn's end is no node's, however often it comes
        let acted = Loop {
            acted: Some(named("C40")),
            ..on.clone()
        };
        for _ in 0..3 {
            assert_eq!(end_of(&acted, Some(Status::Done)), None);
        }
        // the end acted on last being an earlier node's leaves C40's to act on
        let earlier = Loop {
            acted: Some(named("C39")),
            ..on
        };
        assert_eq!(end_of(&earlier, Some(Status::Done)), Some(named("C40")));
    }

    /// The Stop hook names no next node mid-node, when the loop is paused, or once nothing else
    /// is ready, and names the first ready node no other loop holds otherwise
    #[test]
    fn it_does_nothing_mid_node_when_paused_or_once_the_plan_is_finished() {
        // mid-node, blocked, gone from its plan or holding nothing, a turn's end is no node's
        let on = looped(1, Some("C40"));
        for held in [
            Some(Status::Todo),
            Some(Status::InProgress),
            Some(Status::Blocked),
            None,
        ] {
            assert_eq!(end_of(&on, held), None, "{held:?}");
        }
        assert_eq!(end_of(&looped(1, None), Some(Status::Done)), None);
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
        assert_eq!(Ending::Nothing.message(), None);
        let told = Ending::Next {
            done: named("C40"),
            next: named("C41"),
        };
        assert_eq!(
            told.message().as_deref(),
            Some(
                "`reef/central/C40` is done. `reef/central/C41` is next: `/clear`, then ask for it"
            )
        );
    }

    /// A node another loop holds is never a candidate, and is listed with the pane holding it
    #[test]
    fn another_loops_node_is_passed_over() {
        // this loop holds nothing, and another holds C41
        let here = looped(1, None);
        let there = looped(2, Some("C41"));
        let next = ready(&["C41", "C42"]);
        let chosen = choose(&here, None, &next, &[here.clone(), there.clone()]);
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
            choose(&other, None, &next, std::slice::from_ref(&other)),
            Choice::Take {
                candidates: Vec::new(),
                elsewhere: Vec::new(),
            }
        );
    }
}
