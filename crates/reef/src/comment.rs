//! reef's commands for a plan's comment threads
//!
//! ```text
//! reef comment ls [--node ID] [--all]                   # list the open threads, every comment
//! reef comment reply <N> <TEXT>                         # reply in thread N
//! reef comment resolve <N> [--node ID] [--revision N] [--reply TEXT]
//!                                                       # resolve thread N, linking a revision
//! ```
//!
//! - **Listed whole.** Each thread is listed with where it is, whether it is open, and every
//!   comment's markdown with who wrote it, which is what Claude needs to answer it. reef's MCP
//!   tool ends the listing with how to answer the ones that want an answer: a reply saying what
//!   changed, then the commit's `Reef-Resolves` trailer, as a node's steps say.
//! - **Resolved by a change.** A thread is resolved by the revision that answered it, which is
//!   linked to it, so the user sees which change answered which comment. A commit's
//!   `Reef-Resolves: <N>` trailer does the same when the commit is captured; `resolve` is for a
//!   change made without one.
//! - **The node this worktree is on, by default.** The revision linked is the latest of the node
//!   started here last, unless `--node` names another, a bare id in the thread's plan or a full
//!   name in any of the project's, and `--revision` one of its revisions. reef's MCP tool links
//!   the thread's own node instead.
//! - **Over the version read.** The thread is read, then resolved over its version, so a mark
//!   made between the two is never undone unseen. A reply never moves the version on, so one
//!   sent with a resolve goes first.

use reef_api::{
    Commented, NewReply, RESOLVES_TRAILER, ResolveThread, Resolved, Revision, Side, Target, Thread,
    Writer,
};
use reef_model::{NodeName, PlanName};
use std::io::Write;
use std::path::Path;

use crate::cli::{self, PlanPick};
use crate::client::Client;
use crate::git::Checkout;

/// What Claude is told of a thread that needs no change, or the user's word
pub(crate) const ANSWER_OTHERWISE: &str = "A thread that needs no change is resolved with \
                                           reef's resolve tool, saying how it was answered; one \
                                           you need the user's word on gets a question with the \
                                           reply tool, and waits on them.";

/// What Claude is told of replying in a thread before the commit that resolves it, since its
/// trailer writes nothing in the thread
pub(crate) const REPLY_FIRST: &str = "Before committing, reply in each thread you answer with a \
                                      change, with reef's reply tool: say what changed and \
                                      where, and answer any question it asks, since the \
                                      commit's trailer resolves the thread but writes nothing \
                                      in it.";

/// What Claude is told of git's output once a commit's trailer names threads
pub(crate) const CHECK_RESOLVED: &str = "git's output should show reef's `resolved #N` line for \
                                         each thread; resolve any it doesn't with the resolve \
                                         tool, and reply in any it says has no reply.";

/// The `git commit` flag whose trailer names these threads, as Claude is told it
///
/// # Arguments
///
/// * `threads` - The threads' numbers
///
/// # Examples
///
/// ```
/// assert_eq!(reef::comment::trailer_of(&[3, 5]), "--trailer 'Reef-Resolves: 3, 5'");
/// ```
pub fn trailer_of(threads: &[u32]) -> String {
    // each number, split by commas, as the trailer's reader takes them
    let numbers: Vec<String> = threads.iter().map(u32::to_string).collect();
    format!("--trailer '{RESOLVES_TRAILER}: {}'", numbers.join(", "))
}

/// How Claude answers the threads listed that want an answer, as reef's comments tool says
/// after listing them, or `None` if none of them does
///
/// A thread read mid-node through the tool comes with no node's steps, so the tool says what
/// they would: a reply, then the commit's trailer.
///
/// # Arguments
///
/// * `threads` - The threads listed
pub fn answering(threads: &[Thread]) -> Option<String> {
    // the threads that want an answer, by number
    let wanting: Vec<u32> = threads
        .iter()
        .filter(|thread| thread.wants_answer())
        .map(|thread| thread.number)
        .collect();
    if wanting.is_empty() {
        return None;
    }
    let named: Vec<String> = wanting.iter().map(|number| format!("#{number}")).collect();
    Some(format!(
        "To answer the threads that want one ({}): {REPLY_FIRST} Then name each one the change \
         answers in the commit's trailer, `{}`, leaving out any it doesn't. {CHECK_RESOLVED} \
         {ANSWER_OTHERWISE}\n",
        named.join(", "),
        trailer_of(&wanting)
    ))
}

/// Which node's revision a resolve links to a thread when none is named
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answering<'a> {
    /// The node named, a bare id in the thread's plan or a full name
    Named(&'a str),
    /// The node started last in the worktree the command runs in
    Here,
    /// The node the thread is on
    Own,
}

/// Lists a plan's comment threads, or one node's, each whole: where it is, its state, and every
/// comment with who wrote it
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which picks the project and plan when none is named
/// * `node` - The node whose threads to list, a bare id or a full name, else the plan's
/// * `all` - Whether to list resolved threads too, rather than only the open ones
/// * `steps` - Whether to end with how Claude answers the ones that want an answer, as reef's
///   comments tool does
/// * `pick` - The project and plan, if they were named
/// * `out` - Where to print them
pub async fn ls<W: Write>(
    client: &Client,
    cwd: &Path,
    node: Option<&str>,
    all: bool,
    steps: bool,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the plan's threads, or the node's
    let plan = cli::picked_plan(client, cwd, pick).await?;
    let (mut threads, of) = match node {
        Some(named) => {
            let node = named_node(&plan, named)?;
            let threads = client.node_threads(&node).await?;
            (threads, format!("`{node}`"))
        }
        None => (client.plan_threads(&plan).await?, format!("`{plan}`")),
    };
    // the open ones, unless every one was asked for
    if !all {
        threads.retain(Thread::is_open);
    }
    let kind = if all {
        "comment thread"
    } else {
        "open comment thread"
    };
    if threads.is_empty() {
        writeln!(out, "{of} has no {kind}s")?;
        return Ok(());
    }
    let count = threads.len();
    let plural = if count == 1 { "" } else { "s" };
    writeln!(out, "{of} has {count} {kind}{plural}:\n")?;
    show_threads(&threads, out)?;
    // then, for Claude, how the ones that want an answer are answered
    if let Some(answering) = steps.then(|| answering(&threads)).flatten() {
        write!(out, "{answering}")?;
    }
    Ok(())
}

/// Prints threads whole: each one's number, node, place and state, then every comment, with
/// who wrote it, its markdown indented under it
///
/// # Arguments
///
/// * `threads` - The threads, in the order to print them
/// * `out` - Where to print them
pub fn show_threads<W: Write>(threads: &[Thread], out: &mut W) -> anyhow::Result<()> {
    for thread in threads {
        // where it is and whether it still wants an answer
        let state = match (thread.is_open(), thread.wants_answer()) {
            (true, true) => "open",
            (true, false) => "open, waiting on the user's answer to Claude",
            (false, _) => "resolved",
        };
        writeln!(
            out,
            "#{} on `{}`, {}: {state}",
            thread.number,
            thread.node,
            place(thread)
        )?;
        // each comment, its markdown as written
        for comment in &thread.comments {
            // the user signed in by name, else the user or Claude, as before reef knew them
            let by = comment
                .author
                .describe()
                .unwrap_or_else(|| "the user".to_string());
            writeln!(out, "    #{} by {by}:", comment.number)?;
            for line in comment.body.lines() {
                writeln!(out, "        {line}")?;
            }
        }
        writeln!(out)?;
    }
    Ok(())
}

/// Where a thread is: its line where it was followed to last, or the node as a whole
///
/// # Arguments
///
/// * `thread` - The thread
pub fn place(thread: &Thread) -> String {
    match (&thread.target, thread.latest_anchor()) {
        // a line, where the latest revision it was followed to has it
        (Target::Line(_), Some(anchor)) => {
            let side = match anchor.side {
                Side::Old => ", a removed line",
                Side::New => "",
            };
            let outdated = match thread.outdated {
                Some(revision) => format!(", outdated: revision {revision} no longer has it"),
                None => String::new(),
            };
            format!(
                "{}:{} in revision {}{side}{outdated}",
                anchor.path, anchor.line, anchor.revision
            )
        }
        // else the node's work as a whole
        _ => "the node as a whole".to_string(),
    }
}

/// Replies in a thread, leaving it as it is
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which picks the project and plan when none is named
/// * `thread` - The thread's number in its plan
/// * `body` - The reply, as markdown
/// * `by` - Who writes it
/// * `pick` - The project and plan the thread is in, if they were named
/// * `out` - Where to print what came of it
pub async fn reply<W: Write>(
    client: &Client,
    cwd: &Path,
    thread: u32,
    body: &str,
    by: Writer,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // kept in the thread of the plan picked
    let plan = cli::picked_plan(client, cwd, pick).await?;
    let comment = replied(client, &plan, thread, body, by).await?;
    writeln!(out, "replied in #{thread} of `{plan}`, as #{comment}")?;
    Ok(())
}

/// Keeps a reply in a thread, and gives its number
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The thread's plan
/// * `thread` - The thread's number in its plan
/// * `body` - The reply, as markdown
/// * `by` - Who writes it
async fn replied(
    client: &Client,
    plan: &PlanName,
    thread: u32,
    body: &str,
    by: Writer,
) -> anyhow::Result<u32> {
    let reply = NewReply {
        body: body.to_string(),
        by,
        review: false,
    };
    match client.reply(plan, thread, &reply).await? {
        Commented::Added { comment, .. } => Ok(comment),
        Commented::NoThread { .. } => {
            anyhow::bail!("`{plan}` has no comment thread {thread}: no thread starts at #{thread}")
        }
        Commented::NotInPlan { node } => {
            anyhow::bail!("`{node}` isn't in its plan any more, so its thread takes no reply")
        }
        Commented::NoLine { .. } => anyhow::bail!("a reply names no line, so this can't be"),
    }
}

/// Resolves a thread by hand, linking a revision of a node's changes to it as the one that
/// answered it, a reply kept in it first if one is given
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which picks the node for [`Answering::Here`]
/// * `thread` - The thread's number in its plan
/// * `node` - The node whose revision answered it
/// * `revision` - The revision that answered it, else the node's latest
/// * `reply` - A reply to keep in it first, as markdown, such as how it was answered
/// * `by` - Who writes the reply
/// * `pick` - The project and plan the thread is in, if they were named
/// * `out` - Where to print what came of it
#[allow(clippy::too_many_arguments)]
pub async fn resolve<W: Write>(
    client: &Client,
    cwd: &Path,
    thread: u32,
    node: Answering<'_>,
    revision: Option<u32>,
    reply: Option<&str>,
    by: Writer,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the thread, read for the version the resolve is made over
    let plan = cli::picked_plan(client, cwd, pick).await?;
    let Some(read) = client.thread(&plan, thread).await? else {
        anyhow::bail!("`{plan}` has no comment thread {thread}: no thread starts at #{thread}");
    };
    // the node whose revision answered it, and that revision
    let node = match node {
        Answering::Named(named) => named_node(&plan, named)?,
        Answering::Here => here(client, cwd, &plan).await?,
        Answering::Own => read.node.clone(),
    };
    let revision = match revision {
        Some(revision) => revision,
        None => latest(client, &node).await?.revision,
    };
    // the reply first, which leaves the version read as it was
    if let Some(body) = reply {
        let comment = replied(client, &plan, thread, body, by).await?;
        writeln!(out, "replied in #{thread} of `{plan}`, as #{comment}")?;
    }
    let resolve = ResolveThread {
        node: node.clone(),
        revision,
        read: read.version,
    };
    match client.resolve_thread(&plan, thread, &resolve).await? {
        Resolved::Resolved { .. } => {
            writeln!(
                out,
                "resolved #{thread} of `{plan}`, answered by revision {revision} of `{node}`"
            )?;
            Ok(())
        }
        Resolved::Stale { thread: now } => anyhow::bail!(
            "#{thread} changed while it was being resolved, and is {} now: nothing was resolved, \
             so resolve it again",
            now.state.as_str()
        ),
        Resolved::NoThread { .. } => {
            anyhow::bail!("#{thread} went from `{plan}` meanwhile: nothing was written")
        }
        Resolved::Pending { .. } => anyhow::bail!(
            "#{thread} of `{plan}` is pending in a review not yet submitted, so it can't be \
             resolved yet"
        ),
        Resolved::NoRevision { node, revision } => anyhow::bail!(
            "`{node}` has no revision {revision}: `reef node capture` keeps one, and `reef work \
             {}` shows where it is worked",
            node.id
        ),
    }
}

/// A node named on the command line: a full name in any of the project's plans, which the
/// service checks, or a bare id in the plan picked
///
/// # Arguments
///
/// * `plan` - The plan a bare id is in
/// * `named` - The node as typed
fn named_node(plan: &PlanName, named: &str) -> anyhow::Result<NodeName> {
    match named.contains('/') {
        true => Ok(named.parse()?),
        false => Ok(plan.node(named)?),
    }
}

/// The node started last in the worktree a command runs in, in the plan its branch picks, else
/// in any of the project's plans
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `plan` - The thread's plan, whose project the node is in
async fn here(client: &Client, cwd: &Path, plan: &PlanName) -> anyhow::Result<NodeName> {
    // the checkout the command runs in
    let checkout = Checkout::open(cwd).map_err(|error| {
        anyhow::anyhow!("{error:#}: name the node whose revision answered the thread with `--node`")
    })?;
    // the thread's project, and the node whose work was started there last
    let pick = cli::Pick {
        project: Some(plan.project.clone()),
    };
    let project = cli::picked(client, cwd, &pick).await?;
    cli::worktree_record(client, &project, checkout.root())
        .await?
        .map(|record| record.node.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no node of `{}` has been started in {}: name the node whose revision answered \
                 the thread with `--node`",
                plan.project,
                checkout.root().display()
            )
        })
}

/// A node's latest revision, which it must have
///
/// # Arguments
///
/// * `client` - The service's client
/// * `node` - The node's full name
async fn latest(client: &Client, node: &NodeName) -> anyhow::Result<Revision> {
    // the last of its revisions, oldest first
    let mut revisions = client.revisions(node).await?;
    revisions.pop().ok_or_else(|| {
        anyhow::anyhow!(
            "`{node}` has no captured revision to link: `reef node capture` keeps one, or name \
             another with `--node`"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::answering;
    use reef_api::{Author, Comment, Target, Thread, ThreadState, Writer};

    /// A thread of `reef/central/C130` whose comments are written by these writers in turn
    ///
    /// # Arguments
    ///
    /// * `number` - The thread's number
    /// * `state` - Whether it is open or resolved
    /// * `by` - Who wrote each of its comments, the first first
    fn thread(number: u32, state: ThreadState, by: &[Writer]) -> Thread {
        // each comment numbered after the thread's first
        let comments = by
            .iter()
            .zip(number..)
            .map(|(by, at)| Comment {
                number: at,
                at: u64::from(at),
                author: Author {
                    by: *by,
                    ..Author::default()
                },
                review: None,
                body: "Is it slower?".to_string(),
                html: String::new(),
            })
            .collect();
        Thread {
            node: "reef/central/C130".parse().unwrap(),
            number,
            version: 1,
            target: Target::Node,
            followed: Vec::new(),
            outdated: None,
            state,
            resolutions: Vec::new(),
            marks: Vec::new(),
            pending: false,
            comments,
        }
    }

    /// The comments tool says how to answer only the threads that want an answer: a reply
    /// first, then each named in the commit's trailer
    #[test]
    fn the_listing_says_how_to_answer_the_threads_that_want_one() {
        // one the user spoke last in, one Claude answered last, and one resolved
        let threads = [
            thread(3, ThreadState::Open, &[Writer::User]),
            thread(5, ThreadState::Open, &[Writer::User, Writer::Claude]),
            thread(7, ThreadState::Resolved, &[Writer::User]),
        ];
        let said = answering(&threads).expect("#3 wants an answer");
        assert!(
            said.starts_with(
                "To answer the threads that want one (#3): Before committing, reply in each \
                 thread you answer with a change, with reef's reply tool"
            ),
            "{said}"
        );
        assert!(
            said.contains("in the commit's trailer, `--trailer 'Reef-Resolves: 3'`"),
            "{said}"
        );
        assert!(said.contains("resolved with reef's resolve tool"), "{said}");
        // and nothing while none wants one
        assert_eq!(answering(&threads[1..]), None);
    }
}
