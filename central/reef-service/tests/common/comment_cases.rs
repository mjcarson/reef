//! The store suite's comment cases, each generic over the backend so it runs against shoal and
//! the twin

use reef_api::{
    Anchor, COMMENT_BODY_MAX, Commented, FileStatus, Hunk, Line, LineKind, MarkThread, Marked,
    NewReply, NewThread, Side, Target, Thread, ThreadState,
};
use reef_model::NodeName;
use reef_service::{Backend, FileDiff, Unkeepable};

use super::work_cases::{added_file, capture, kept, named, recorded, start_in};
use super::{Fx, Reopen};

/// Starts a node and keeps one capture of it as its revision 1, holding the files given
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `id` - The node's id
/// * `files` - The capture's files
async fn captured<B: Backend>(
    fx: &Fx<B>,
    id: &str,
    files: Vec<FileDiff>,
) -> anyhow::Result<NodeName> {
    // a start, then one capture of it
    let node = named(fx, id);
    let record = recorded(fx, id, start_in(&fx.repo, id, 'a'), None).await?;
    let revision = kept(fx, &node, capture(&record.start.pinned, 'c', files)).await?;
    assert_eq!(revision.revision, 1);
    Ok(node)
}

/// A modified file: its first line removed, its second kept, and a line added after it
fn modified() -> FileDiff {
    // a line of a kind, numbered on the sides it is on
    let line = |kind, old, new, text: &str| Line {
        kind,
        old,
        new,
        text: text.to_string(),
        no_newline: false,
    };
    FileDiff {
        status: FileStatus::Modified,
        old_path: Some("src/m.rs".to_string()),
        new_path: Some("src/m.rs".to_string()),
        old_mode: Some("100644".to_string()),
        new_mode: Some("100644".to_string()),
        old_id: Some("1".repeat(40)),
        new_id: Some("2".repeat(40)),
        binary: false,
        too_large: false,
        hunks: vec![Hunk {
            old_start: 1,
            old_lines: 2,
            new_start: 1,
            new_lines: 2,
            lines: vec![
                line(LineKind::Removed, Some(1), None, "gone"),
                line(LineKind::Context, Some(2), Some(1), "kept"),
                line(LineKind::Added, None, Some(2), "new"),
            ],
        }],
    }
}

/// A target on a line of a node's revision
///
/// # Arguments
///
/// * `revision` - The revision's number
/// * `path` - The file's path
/// * `side` - The side the line is numbered on
/// * `line` - Its number there
fn on_line(revision: u32, path: &str, side: Side, line: u32) -> Target {
    Target::Line(Anchor {
        revision,
        path: path.to_string(),
        side,
        line,
    })
}

/// Starts a thread that must be kept, and hands back the thread and its first comment's number
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `node` - The node it is on
/// * `target` - What it is on
/// * `body` - Its first comment's body
async fn started<B: Backend>(
    fx: &Fx<B>,
    node: &NodeName,
    target: Target,
    body: &str,
) -> anyhow::Result<(Thread, u32)> {
    let new = NewThread {
        target,
        body: body.to_string(),
    };
    match fx.store.start_thread(node, new).await? {
        Commented::Added { thread, comment } => Ok((thread, comment)),
        other => panic!("a thread on `{node}` was not kept: {other:?}"),
    }
}

/// Replies in a thread, which must be kept, and hands back the thread and the reply's number
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `thread` - The thread's number
/// * `body` - The reply's body
async fn replied<B: Backend>(fx: &Fx<B>, thread: u32, body: &str) -> anyhow::Result<(Thread, u32)> {
    let reply = NewReply {
        body: body.to_string(),
    };
    match fx.store.reply(&fx.plan, thread, reply).await? {
        Commented::Added { thread, comment } => Ok((thread, comment)),
        other => panic!("a reply in thread {thread} was not kept: {other:?}"),
    }
}

/// Marks a thread over a version
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `thread` - The thread's number
/// * `state` - The state to mark it
/// * `read` - The version it was marked against
async fn mark<B: Backend>(
    fx: &Fx<B>,
    thread: u32,
    state: ThreadState,
    read: u64,
) -> anyhow::Result<Marked> {
    let mark = MarkThread { state, read };
    fx.store.mark_thread(&fx.plan, thread, mark).await
}

/// Comments on a line and on a node, their replies and a thread's mark all read back the same
/// once the store is reopened, and numbering carries on after them
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_comment_and_its_reply_survive_a_restart<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a plan never commented on has no threads
    assert!(fx.store.threads(&fx.plan).await?.is_empty());
    // a thread on a line of A's first revision, and one on A as a whole
    let a = captured(&fx, "A", vec![added_file("src/a.rs", "one\ntwo\n")]).await?;
    let line = on_line(1, "src/a.rs", Side::New, 2);
    let (first, number) = started(&fx, &a, line.clone(), "Why `two`?").await?;
    assert_eq!((first.number, number, first.version), (1, 1, 1));
    assert_eq!(first.node, a);
    assert_eq!(first.target, line);
    assert!(first.is_open());
    let (general, number) = started(&fx, &a, Target::Node, "Looks **good**").await?;
    assert_eq!((general.number, number), (2, 2));
    // a reply in each, numbered after every comment before it in the plan
    let (first, reply) = replied(&fx, 1, "Because.").await?;
    assert_eq!(reply, 3);
    let bodies: Vec<&str> = first.comments.iter().map(|c| c.body.as_str()).collect();
    assert_eq!(bodies, ["Why `two`?", "Because."]);
    assert!(first.comments.iter().all(|c| c.html.is_empty()));
    let (_, reply) = replied(&fx, 2, "Thanks").await?;
    assert_eq!(reply, 4);
    // the general thread resolved, and a thread on another node
    let Marked::Marked { thread: resolved } =
        mark(&fx, 2, ThreadState::Resolved, general.version).await?
    else {
        panic!("marking thread 2 over the version read was refused");
    };
    assert_eq!(
        (resolved.state, resolved.version),
        (ThreadState::Resolved, 2)
    );
    let b = named(&fx, "B");
    let (other, _) = started(&fx, &b, Target::Node, "And B?").await?;
    assert_eq!(other.number, 5);
    // the plan's threads are every node's, and a node's only its own
    let threads = fx.store.threads(&fx.plan).await?;
    let numbers: Vec<u32> = threads.iter().map(|thread| thread.number).collect();
    assert_eq!(numbers, [1, 2, 5]);
    let on_a: Vec<u32> = fx
        .store
        .node_threads(&a)
        .await?
        .iter()
        .map(|thread| thread.number)
        .collect();
    assert_eq!(on_a, [1, 2]);
    // all of it reads back once the store is reopened over the same rows
    let fx = fx.reopen().await?;
    assert_eq!(fx.store.threads(&fx.plan).await?, threads);
    assert_eq!(
        fx.store.thread(&fx.plan, 2).await?.as_ref(),
        Some(&threads[1])
    );
    // a reply's number names no thread
    assert!(fx.store.thread(&fx.plan, 3).await?.is_none());
    // and the next comment is numbered after the last one kept before the restart
    let (_, reply) = replied(&fx, 1, "One more thing").await?;
    assert_eq!(reply, 6);
    Ok(())
}

/// A thread on a line its node's revisions don't hold is refused, as is a reply in no thread
/// and a body with nothing in it or too much, and none of them writes anything
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_line_comment_off_its_revision_is_refused<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // A's revision 1 modifies one file
    let a = captured(&fx, "A", vec![modified()]).await?;
    // a revision it doesn't have, a file its revision doesn't list, a line past the file's, and
    // a line named on the side it isn't on
    let refused = [
        on_line(2, "src/m.rs", Side::New, 1),
        on_line(1, "src/other.rs", Side::New, 1),
        on_line(1, "src/m.rs", Side::New, 3),
        on_line(1, "src/m.rs", Side::Old, 2),
        on_line(1, "src/m.rs", Side::New, 0),
    ];
    for target in refused {
        let new = NewThread {
            target: target.clone(),
            body: "Here?".to_string(),
        };
        let answer = fx.store.start_thread(&a, new).await?;
        assert_eq!(answer, Commented::NoLine { target });
    }
    // a node with no revisions holds no line at all
    let b = named(&fx, "B");
    let new = NewThread {
        target: on_line(1, "src/m.rs", Side::New, 1),
        body: "Here?".to_string(),
    };
    assert!(matches!(
        fx.store.start_thread(&b, new).await?,
        Commented::NoLine { .. }
    ));
    // a reply in a thread that isn't there
    let reply = NewReply {
        body: "Hm".to_string(),
    };
    let answer = fx.store.reply(&fx.plan, 1, reply.clone()).await?;
    assert_eq!(answer, Commented::NoThread { thread: 1 });
    // a body with nothing in it, or more than a comment may hold
    for body in ["", "  \n\t", &"x".repeat(COMMENT_BODY_MAX + 1)] {
        let new = NewThread {
            target: Target::Node,
            body: body.to_string(),
        };
        let error = fx
            .store
            .start_thread(&a, new)
            .await
            .expect_err("a body it can't keep was kept");
        assert!(error.downcast_ref::<Unkeepable>().is_some(), "{error:#}");
    }
    // nothing was written
    assert!(fx.store.threads(&fx.plan).await?.is_empty());
    // while the removed line on its base's side, the kept one and the added one take threads
    for (side, line) in [(Side::Old, 1), (Side::New, 1), (Side::New, 2)] {
        started(&fx, &a, on_line(1, "src/m.rs", side, line), "Here.").await?;
    }
    // and a reply naming a reply's number replies in no thread
    let (_, number) = replied(&fx, 1, "Yes").await?;
    let answer = fx.store.reply(&fx.plan, number, reply).await?;
    assert_eq!(answer, Commented::NoThread { thread: number });
    Ok(())
}

/// A mark over the version a thread is at changes it, and one over an older version, or one
/// another writer beat, is stale and changes nothing
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn marking_over_an_older_version_is_stale<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a general thread on A, at version 1
    let a = named(&fx, "A");
    let (thread, _) = started(&fx, &a, Target::Node, "Is this done?").await?;
    // resolved over version 1, it is at 2
    let Marked::Marked { thread: resolved } = mark(&fx, 1, ThreadState::Resolved, 1).await? else {
        panic!("a mark over the version read was refused");
    };
    assert_eq!(
        (resolved.state, resolved.version),
        (ThreadState::Resolved, 2)
    );
    // opened again over version 1, it is stale, and the thread is as the first mark left it
    let stale = mark(&fx, 1, ThreadState::Open, thread.version).await?;
    assert_eq!(
        stale,
        Marked::Stale {
            thread: resolved.clone()
        }
    );
    // a reply leaves the thread's version alone, so a mark over it still holds
    replied(&fx, 1, "Not yet").await?;
    let Marked::Marked { thread: opened } = mark(&fx, 1, ThreadState::Open, 2).await? else {
        panic!("a mark over the version read was refused");
    };
    assert_eq!((opened.state, opened.version), (ThreadState::Open, 3));
    assert_eq!(opened.comments.len(), 2);
    // one another writer beat to the row is stale too
    fx.faults.refuse_next_comment_write();
    let beaten = mark(&fx, 1, ThreadState::Resolved, 3).await?;
    assert_eq!(beaten, Marked::Stale { thread: opened });
    // and a thread that isn't there is named, a reply's number among them
    assert_eq!(
        mark(&fx, 9, ThreadState::Resolved, 1).await?,
        Marked::NoThread { thread: 9 }
    );
    assert_eq!(
        mark(&fx, 2, ThreadState::Resolved, 1).await?,
        Marked::NoThread { thread: 2 }
    );
    Ok(())
}
