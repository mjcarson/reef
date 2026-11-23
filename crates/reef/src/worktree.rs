//! A node's own worktree: `.claude/worktrees/<id>` on `reef/<id>`, made from the checkout of the
//! session that asks, with the node started there and the asking pane joined to it
//!
//! - **Where Claude Code puts its own.** `claude -w <id>` and EnterWorktree look for a session's
//!   worktree under the main worktree's `.claude/worktrees`, so reef's go there, named by the
//!   node's id, each on a branch of its own, `reef/<id>`.
//! - **From the asking checkout's HEAD, based on its branch.** A group's member is made from the
//!   loop's own checkout, so its start is where the loop is, and its base branch is the loop's
//!   branch, which its changes go back to. Nothing is fetched.
//! - **Out of every snapshot.** `/.claude/worktrees/` goes in the repo's `info/exclude`, so a
//!   `git add -A` in the main worktree, a start's snapshot included, never takes a linked
//!   worktree in as an embedded repo.
//! - **Started once.** The node is started in its new worktree as `reef start` starts it, which
//!   refuses a node started already, so two sessions can't make one node's worktree. A start
//!   refused takes the worktree, and the branch it made, away again.
//! - **Made again, it is the same.** A worktree there already that the node was started in is
//!   handed back as it is, and joined again.
//! - **With the asking checkout's local settings.** `.claude/settings.local.json`, which `reef
//!   claude install` writes and git ignores, isn't checked out, so a new worktree is given a copy,
//!   and its session asks no more than the asking one's. A file git doesn't ignore there is never
//!   copied, since it would be captured as the node's change.
//! - **Then the pane joins the member,** where the node is a loop's member, so the session
//!   there works it ([`reef_api::LoopEdit::Join`]). A pane running a loop of its own, or a member
//!   another pane works, is refused before anything is made.

use reef_api::loops::member_of;
use reef_api::{EditLoop, Loop, LoopEdit, LoopEdited, Pane};
use reef_model::{NodeName, PlanName};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::claude;
use crate::cli::{Pick, PlanPick};
use crate::client::Client;
use crate::git::Checkout;
use crate::start;

/// Where the worktrees go, under the main worktree's root, as Claude Code puts its own
pub const WORKTREES: &str = ".claude/worktrees";

/// The pattern that keeps them out of the main worktree's snapshots
pub const EXCLUDED: &str = "/.claude/worktrees/";

/// A node's worktree, made or found
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Made {
    /// The worktree's root
    pub path: PathBuf,
    /// The branch it is on
    pub branch: String,
    /// Whether it was there already, the node started in it
    pub reused: bool,
    /// The loop whose member the pane joined, if the node is one
    pub joined: Option<Loop>,
}

/// The branch a node's worktree is on
///
/// # Arguments
///
/// * `id` - The node's id
///
/// # Examples
///
/// ```
/// assert_eq!(reef::worktree::branch_for("C60"), "reef/C60");
/// ```
pub fn branch_for(id: &str) -> String {
    format!("reef/{id}")
}

/// The node a worktree's name names in a project: a loop's member first, else the node of that
/// id in the one plan that has one, or `None` if no plan does
///
/// # Arguments
///
/// * `client` - The service's client
/// * `project` - The project's name
/// * `name` - The worktree's name, as `claude -w` was given it
pub async fn resolve(
    client: &Client,
    project: &str,
    name: &str,
) -> anyhow::Result<Option<NodeName>> {
    // a member of one of the project's loops, which is what a group's session is opened for
    let loops = client.project_loops(project).await?;
    let member = loops
        .iter()
        .flat_map(|feeding| feeding.members.iter())
        .find(|member| member.node.id == name);
    if let Some(member) = member {
        return Ok(Some(member.node.clone()));
    }
    // else whichever of its plans has a node of that id
    let mut found = Vec::new();
    for info in client.plans(project).await? {
        let listing = client.list(&info.name).await?;
        if listing.nodes.iter().any(|stored| stored.spec.id == name) {
            found.push(info.name);
        }
    }
    match found.as_slice() {
        [] => Ok(None),
        [plan] => Ok(Some(plan.node(name)?)),
        several => {
            let names: Vec<String> = several.iter().map(PlanName::to_string).collect();
            anyhow::bail!(
                "`{name}` is a node of {}: start it with `reef start {name} --plan <NAME>` in a \
                 worktree of its own",
                names.join(" and ")
            )
        }
    }
}

/// Makes a node's worktree from the asking checkout, or finds the one it has, starts the node in
/// it, and joins the pane to it where the node is a loop's member
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - A directory in the checkout of the session that asks
/// * `node` - The node
/// * `pane` - The pane of the session that will work it, if it could be found
/// * `out` - Where to print what came of it
pub async fn for_node<W: Write>(
    client: &Client,
    cwd: &Path,
    node: &NodeName,
    pane: Option<&Pane>,
    out: &mut W,
) -> anyhow::Result<Made> {
    // the asking checkout, which must be the project's
    let pick = Pick {
        project: Some(node.plan.project.clone()),
    };
    let (checkout, _) = start::checkout_in(client, cwd, &pick, "make worktrees for").await?;
    // a pane that can't join is refused before anything is made
    if let Some(pane) = pane {
        joinable(client, node, pane).await?;
    }
    let path = path_for(&checkout, &node.id)?;
    let branch = branch_for(&node.id);
    // linked worktrees stay out of the main worktree's snapshots
    checkout.exclude(EXCLUDED)?;
    let reused = path.exists();
    match reused {
        // there already: the node's own, or one it is started in now
        true => {
            let there = Checkout::open(&path)?;
            match client.work_of(node).await? {
                Some(record) if record.start.worktree == there.root() => {
                    writeln!(out, "`{}` is in {} already", node.id, path.display())?;
                }
                Some(record) => anyhow::bail!(
                    "{} is there, but `{}` was started {}: work it there",
                    path.display(),
                    node.id,
                    start::where_started(&record)
                ),
                None => start_in(client, &path, node, None, out).await?,
            }
        }
        // else made from where the asking session is, on the node's own branch
        false => {
            let from = checkout.head()?.ok_or_else(|| {
                anyhow::anyhow!(
                    "{} has no commit to make a worktree from: commit something first",
                    checkout.root().display()
                )
            })?;
            let made_branch = !checkout.has_ref(&format!("refs/heads/{branch}"))?;
            checkout.add_worktree(&path, &branch, &from)?;
            // based on the branch it was made from, which its changes go back to
            let base = checkout.branch()?;
            if let Err(error) = start_in(client, &path, node, base.as_deref(), out).await {
                // a start refused takes back what was made for it
                let undone = undo(&checkout, &path, made_branch.then_some(branch.as_str()));
                return Err(match undone {
                    Ok(()) => error,
                    Err(undo) => error.context(format!(
                        "and {} couldn't be taken away again: {undo:#}",
                        path.display()
                    )),
                });
            }
            // started, it gets the asking checkout's local settings
            if carry_local(&checkout, &path)? {
                writeln!(out, "copied {} into {}", claude::LOCAL, path.display())?;
            }
        }
    }
    // the pane then works it, where it is a loop's member
    let joined = match pane {
        Some(pane) => join(client, node, pane).await?,
        None => None,
    };
    Ok(Made {
        path: path.canonicalize()?,
        branch,
        reused,
        joined,
    })
}

/// Makes a worktree for a name that is no node, as Claude Code would make its own: on
/// `worktree-<name>`, from the asking checkout's HEAD, or finds the one there
///
/// # Arguments
///
/// * `cwd` - A directory in the checkout of the session that asks
/// * `name` - The worktree's name
pub fn plain(cwd: &Path, name: &str) -> anyhow::Result<PathBuf> {
    // the asking checkout, whatever repo it is
    let checkout = Checkout::open(cwd)?;
    let path = path_for(&checkout, name)?;
    checkout.exclude(EXCLUDED)?;
    if !path.exists() {
        let from = checkout.head()?.ok_or_else(|| {
            anyhow::anyhow!(
                "{} has no commit to make a worktree from: commit something first",
                checkout.root().display()
            )
        })?;
        checkout.add_worktree(&path, &format!("worktree-{name}"), &from)?;
        carry_local(&checkout, &path)?;
    }
    Ok(path.canonicalize()?)
}

/// Copies the asking checkout's local Claude Code settings into a worktree just made from it,
/// where git ignores them, and says whether it did
///
/// A checkout with none, or a worktree that has its own, is left as it is.
///
/// # Arguments
///
/// * `from` - The asking checkout
/// * `to` - The new worktree's root
fn carry_local(from: &Checkout, to: &Path) -> anyhow::Result<bool> {
    // the asking checkout's, if it has them and the worktree doesn't
    let source = from.root().join(claude::LOCAL);
    let target = to.join(claude::LOCAL);
    if !source.is_file() || target.exists() {
        return Ok(false);
    }
    // never a file git would take in as the node's change
    if !Checkout::open(to)?.ignores(claude::LOCAL)? {
        return Ok(false);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&source, &target)?;
    Ok(true)
}

/// Where a worktree of a name goes: under the main worktree's `.claude/worktrees`
///
/// # Arguments
///
/// * `checkout` - The asking checkout
/// * `name` - The worktree's name, which must stay under that directory
fn path_for(checkout: &Checkout, name: &str) -> anyhow::Result<PathBuf> {
    // each segment a plain name, so the path never leaves the worktrees' directory
    let plain = !name.is_empty()
        && name
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    if !plain {
        anyhow::bail!("`{name}` can't name a worktree: give it a name such as a node's id");
    }
    Ok(checkout.main_root()?.join(WORKTREES).join(name))
}

/// Starts a node in its worktree, as `reef start` does there
///
/// # Arguments
///
/// * `client` - The service's client
/// * `path` - The worktree's root
/// * `node` - The node
/// * `base` - The base branch, if the worktree was made from one
/// * `out` - Where to print what came of it
async fn start_in<W: Write>(
    client: &Client,
    path: &Path,
    node: &NodeName,
    base: Option<&str>,
    out: &mut W,
) -> anyhow::Result<()> {
    // the node's own plan, wherever the project has several
    let pick = PlanPick {
        project: Pick {
            project: Some(node.plan.project.clone()),
        },
        plan: Some(node.plan.plan.clone()),
    };
    start::start(client, path, &node.id, &pick, base, false, out).await
}

/// Takes a worktree made for a start that was refused away again, with the branch made for it
///
/// # Arguments
///
/// * `checkout` - The checkout it was made from
/// * `path` - The worktree's root
/// * `branch` - The branch made for it, if it was new
fn undo(checkout: &Checkout, path: &Path, branch: Option<&str>) -> anyhow::Result<()> {
    // the worktree first, since git keeps a branch a worktree has checked out
    checkout.remove_worktree(path)?;
    if let Some(branch) = branch {
        checkout.delete_branch(branch)?;
    }
    Ok(())
}

/// Refuses a pane that can't join a node: one running a loop of its own, or any pane while
/// another works the node as a member
///
/// # Arguments
///
/// * `client` - The service's client
/// * `node` - The node
/// * `pane` - The pane
async fn joinable(client: &Client, node: &NodeName, pane: &Pane) -> anyhow::Result<()> {
    let loops = client.project_loops(&node.plan.project).await?;
    // a loop's own pane works the loop's own node
    if let Some(own) = loops.iter().find(|feeding| feeding.pane == *pane) {
        anyhow::bail!(
            "{} runs a loop of its own, feeding it {}: a member's session runs in a pane of its \
             own",
            pane.describe(),
            own.project
        );
    }
    // and a member another pane works is that session's
    let worked = loops
        .iter()
        .find_map(|feeding| feeding.member(node))
        .and_then(|member| member.pane.as_ref())
        .filter(|joined| *joined != pane);
    if let Some(joined) = worked {
        anyhow::bail!(
            "`{}` has a session already, in {}: work it there",
            node.id,
            joined.describe()
        );
    }
    Ok(())
}

/// Joins a pane to a node where it is a loop's member, and says which loop's, or `None` if it is
/// none
///
/// # Arguments
///
/// * `client` - The service's client
/// * `node` - The node
/// * `pane` - The pane
async fn join(client: &Client, node: &NodeName, pane: &Pane) -> anyhow::Result<Option<Loop>> {
    let edit = EditLoop {
        pane: pane.clone(),
        edit: LoopEdit::Join { node: node.clone() },
    };
    match client.edit_loop(&edit).await? {
        LoopEdited::Joined { r#loop } => Ok(Some(r#loop)),
        // a node no loop holds as a member is worked without one
        LoopEdited::NotMember { .. } => Ok(None),
        // another pane joined it meanwhile, or this one started a loop
        LoopEdited::Held { by, .. } => {
            let joined = by
                .member(node)
                .and_then(|member| member.pane.as_ref())
                .map_or_else(|| by.pane.describe(), Pane::describe);
            anyhow::bail!(
                "`{}` has a session already, in {joined}: work it there",
                node.id
            )
        }
        LoopEdited::Busy { r#loop } => anyhow::bail!(
            "{} runs a loop of its own, or works another loop's member, so it joined none: the \
             loop in {} holds it",
            pane.describe(),
            r#loop.pane.describe()
        ),
        other => anyhow::bail!("the service answered {other:?} for {pane} joining `{node}`"),
    }
}

/// The member a pane works, if it joined one: its loop and node
///
/// # Arguments
///
/// * `client` - The service's client
/// * `project` - The project's name
/// * `pane` - The pane
pub async fn worked_in(
    client: &Client,
    project: &str,
    pane: &Pane,
) -> anyhow::Result<Option<(Loop, NodeName)>> {
    let loops = client.project_loops(project).await?;
    Ok(member_of(&loops, pane).map(|(by, member)| (by.clone(), member.node.clone())))
}
