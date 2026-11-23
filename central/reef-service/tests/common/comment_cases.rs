//! The store suite's comment cases, each generic over the backend so it runs against shoal and
//! the twin

use reef_api::{
    Anchor, COMMENT_BODY_MAX, Commented, FileStatus, FollowedThread, Hunk, Line, LineKind,
    MarkThread, Marked, NewReply, NewThread, Resolution, ResolveThread, Resolved, Resolves,
    ResolvingCommit, Side, Target, Thread, ThreadState, TrailerOutcome,
};
use reef_model::NodeName;
use reef_service::{Backend, FileDiff, Unkeepable};

use super::work_cases::{added_file, capture, drafted, kept, named, recorded, start_in};
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

/// A new file's diff, its worktree blob named by a letter, so two captures of other contents
/// hold other blobs
///
/// # Arguments
///
/// * `path` - The file's path
/// * `text` - What it holds, a line to each newline
/// * `blob` - The letter its worktree blob is made of
fn added_as(path: &str, text: &str, blob: char) -> FileDiff {
    FileDiff {
        new_id: Some(blob.to_string().repeat(40)),
        ..added_file(path, text)
    }
}

/// Keeps the next capture of a node that was started, holding the files given
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `node` - The node captured
/// * `sha` - The letter the capture's SHAs are made of, which must be new to the node
/// * `files` - The capture's files
async fn recaptured<B: Backend>(
    fx: &Fx<B>,
    node: &NodeName,
    sha: char,
    files: Vec<FileDiff>,
) -> anyhow::Result<u32> {
    // taken from the start the node is on
    let Some(record) = fx.store.work_of(node).await? else {
        panic!("`{node}` was never started");
    };
    let revision = kept(fx, node, capture(&record.start.pinned, sha, files)).await?;
    Ok(revision.revision)
}

/// A line of a node's revision, as an anchor
///
/// # Arguments
///
/// * `revision` - The revision's number
/// * `path` - The file's path
/// * `line` - Its number in the worktree
fn new_line(revision: u32, path: &str, line: u32) -> Anchor {
    Anchor {
        revision,
        path: path.to_string(),
        side: Side::New,
        line,
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

/// A capture that deletes a commented line outdates its thread from that revision on, as does
/// one that no longer changes its file, and an outdated thread is followed no further
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn deleting_a_commented_line_outdates_its_thread<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // A's revision 1 adds two files, and a thread is on a line of each
    let files = vec![
        added_as("src/a.rs", "one\ntwo\nthree\n", '1'),
        added_as("src/b.rs", "only\n", '2'),
    ];
    let a = captured(&fx, "A", files).await?;
    let (on_two, _) = started(&fx, &a, on_line(1, "src/a.rs", Side::New, 2), "Why two?").await?;
    started(&fx, &a, on_line(1, "src/b.rs", Side::New, 1), "And b?").await?;
    // revision 2 deletes `two`, and leaves `b.rs` out of its changes
    let two = vec![added_as("src/a.rs", "one\nthree\n", '3')];
    assert_eq!(recaptured(&fx, &a, 'd', two).await?, 2);
    let Some(followed) = fx.store.follow_threads(&a).await? else {
        panic!("a capture that lost two lines followed no thread");
    };
    let outdated = |thread| FollowedThread {
        thread,
        version: 2,
        outdated: true,
    };
    assert_eq!(followed.revision, 2);
    assert_eq!(followed.threads, [outdated(1), outdated(2)]);
    // each thread is outdated from revision 2, and still on its line in revision 1
    let threads = fx.store.threads(&fx.plan).await?;
    for thread in &threads {
        assert_eq!(thread.outdated, Some(2));
        assert!(thread.followed.is_empty());
        assert_eq!(thread.anchor_in(1), thread.target.anchor());
        assert_eq!(thread.anchor_in(2), None);
        assert!(thread.is_open());
    }
    assert_eq!(threads[0].comments, on_two.comments);
    assert_eq!(threads[0].latest_anchor(), on_two.target.anchor());
    // a later revision follows them no further, even one with `two` back
    let three = vec![added_as("src/a.rs", "one\ntwo\nthree\n", '4')];
    recaptured(&fx, &a, 'e', three).await?;
    assert_eq!(fx.store.follow_threads(&a).await?, None);
    // and they read back so once the store is reopened
    let fx = fx.reopen().await?;
    assert_eq!(fx.store.threads(&fx.plan).await?, threads);
    Ok(())
}

/// A follow moves each of a node's line threads to where its line went, resolved ones too,
/// leaves general threads and other nodes' alone, and a thread started on an older revision
/// is followed at once
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn captures_follow_their_nodes_threads<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // A's revision 1 and B's each add the same file
    let file = || vec![added_as("src/a.rs", "one\ntwo\nthree\n", '1')];
    let a = captured(&fx, "A", file()).await?;
    let b = captured(&fx, "B", file()).await?;
    // a thread on A's `two`, one on its `three` resolved, a general one, and one on B's `two`
    let (on_two, _) = started(&fx, &a, on_line(1, "src/a.rs", Side::New, 2), "Two?").await?;
    let (on_three, _) = started(&fx, &a, on_line(1, "src/a.rs", Side::New, 3), "Three?").await?;
    let Marked::Marked { .. } = mark(&fx, 2, ThreadState::Resolved, on_three.version).await? else {
        panic!("marking thread 2 over the version read was refused");
    };
    let (general, _) = started(&fx, &a, Target::Node, "Overall?").await?;
    let (on_b, _) = started(&fx, &b, on_line(1, "src/a.rs", Side::New, 2), "B's two?").await?;
    // A's revision 2 adds two lines above them
    let two = vec![added_as("src/a.rs", "zero\nhalf\none\ntwo\nthree\n", '2')];
    recaptured(&fx, &a, 'd', two).await?;
    let Some(followed) = fx.store.follow_threads(&a).await? else {
        panic!("a capture that moved two lines followed no thread");
    };
    let moved = |thread, version| FollowedThread {
        thread,
        version,
        outdated: false,
    };
    assert_eq!(followed.revision, 2);
    assert_eq!(followed.threads, [moved(1, 2), moved(2, 3)]);
    // each moved down by two, the resolved one still resolved, and the others as they were
    let threads = fx.store.threads(&fx.plan).await?;
    assert_eq!(threads[0].followed, [new_line(2, "src/a.rs", 4)]);
    assert_eq!(threads[1].followed, [new_line(2, "src/a.rs", 5)]);
    assert_eq!(threads[1].state, ThreadState::Resolved);
    assert_eq!((&threads[2], &threads[3]), (&general, &on_b));
    assert!(threads.iter().all(|thread| !thread.is_outdated()));
    // the follow moved its threads' versions on, so a mark over the one read before is stale
    let stale = mark(&fx, 1, ThreadState::Resolved, on_two.version).await?;
    assert!(matches!(stale, Marked::Stale { .. }), "{stale:?}");
    // a follow with no newer revision writes nothing
    assert_eq!(fx.store.follow_threads(&a).await?, None);
    // revision 3 drops `zero`, and a thread started on revision 1's `one` is followed at once
    let three = vec![added_as("src/a.rs", "half\none\ntwo\nthree\n", '3')];
    recaptured(&fx, &a, 'e', three).await?;
    let (late, _) = started(&fx, &a, on_line(1, "src/a.rs", Side::New, 1), "One?").await?;
    assert_eq!(
        late.followed,
        [new_line(2, "src/a.rs", 3), new_line(3, "src/a.rs", 2)]
    );
    assert_eq!(late.anchor_in(1), late.target.anchor());
    assert_eq!(late.outdated, None);
    // then the follow takes the others on to revision 3, and leaves the late one where it is
    let Some(followed) = fx.store.follow_threads(&a).await? else {
        panic!("a capture that moved two lines followed no thread");
    };
    assert_eq!(followed.threads, [moved(1, 3), moved(2, 4)]);
    let threads = fx.store.threads(&fx.plan).await?;
    assert_eq!(
        threads[0].latest_anchor(),
        Some(&new_line(3, "src/a.rs", 3))
    );
    assert_eq!(threads[0].anchor_in(2), Some(&new_line(2, "src/a.rs", 4)));
    assert_eq!(threads[4], late);
    // and all of it reads back once the store is reopened
    let fx = fx.reopen().await?;
    assert_eq!(fx.store.threads(&fx.plan).await?, threads);
    Ok(())
}

/// A draft's lines take no thread, and a thread follows past a draft only once it is sealed,
/// to where its line is in the sealed revision
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn drafts_hold_no_thread_until_sealed<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // A's revision 1, sealed, with a thread on its `two`
    let a = captured(&fx, "A", vec![added_as("src/a.rs", "one\ntwo\n", '1')]).await?;
    let (on_two, _) = started(&fx, &a, on_line(1, "src/a.rs", Side::New, 2), "Two?").await?;
    // revision 2, a draft adding a line above it
    let pinned = fx
        .store
        .work_of(&a)
        .await?
        .expect("A was started")
        .start
        .pinned;
    let draft = vec![added_as("src/a.rs", "zero\none\ntwo\n", '2')];
    let r2 = kept(&fx, &a, drafted(&pinned, 'd', draft.clone())).await?;
    assert!(r2.draft);
    // a thread on the draft's lines names no line a comment can stay on
    let new = NewThread {
        target: on_line(2, "src/a.rs", Side::New, 1),
        body: "Zero?".to_string(),
    };
    let refused = fx.store.start_thread(&a, new).await?;
    assert!(matches!(refused, Commented::NoLine { .. }), "{refused:?}");
    // the thread on revision 1 stays there, since no sealed revision follows it
    assert_eq!(fx.store.follow_threads(&a).await?, None);
    assert_eq!(thread_of(&fx, on_two.number).await?, on_two);
    // and a thread started on revision 1 now is followed no further than it
    let (late, _) = started(&fx, &a, on_line(1, "src/a.rs", Side::New, 1), "One?").await?;
    assert!(late.followed.is_empty(), "{late:?}");
    // once the draft is sealed, both follow to their lines in it
    let sealed = fx
        .store
        .add_capture(&a, capture(&pinned, 'd', Vec::new()))
        .await?;
    assert!(
        matches!(sealed, reef_service::Captured::Replaced { .. }),
        "{sealed:?}"
    );
    let Some(followed) = fx.store.follow_threads(&a).await? else {
        panic!("sealing the draft followed no thread");
    };
    assert_eq!(followed.revision, 2);
    let threads = fx.store.threads(&fx.plan).await?;
    assert_eq!(threads[0].followed, [new_line(2, "src/a.rs", 3)]);
    assert_eq!(threads[1].followed, [new_line(2, "src/a.rs", 2)]);
    // and a sealed revision's lines take threads
    let (on_zero, _) = started(&fx, &a, on_line(2, "src/a.rs", Side::New, 1), "Zero?").await?;
    assert_eq!(on_zero.target.anchor(), Some(&new_line(2, "src/a.rs", 1)));
    Ok(())
}

/// A commit by the suite's author, its id made of a letter, authored at a time
///
/// # Arguments
///
/// * `sha` - The letter its id is made of
/// * `authored` - When it was authored, in seconds since the epoch
fn commit_by(sha: char, authored: i64) -> ResolvingCommit {
    ResolvingCommit {
        commit: sha.to_string().repeat(40),
        summary: format!("Answer {sha}"),
        author: "dev@example.invalid".to_string(),
        authored,
    }
}

/// Resolves the threads named by one commit's trailers, as a capture of a node's revision does,
/// and hands back what came of each
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `node` - The node captured
/// * `revision` - The revision the capture kept
/// * `commit` - The commit whose trailers named them
/// * `threads` - The threads named
async fn trailed<B: Backend>(
    fx: &Fx<B>,
    node: &NodeName,
    revision: u32,
    commit: &ResolvingCommit,
    threads: &[u32],
) -> anyhow::Result<Vec<TrailerOutcome>> {
    // each thread named by the one commit
    let resolves = threads
        .iter()
        .map(|thread| Resolves {
            thread: *thread,
            commit: commit.clone(),
        })
        .collect();
    let read = fx
        .store
        .resolve_by_trailers(node, revision, resolves)
        .await?;
    Ok(read.into_iter().map(|(read, _)| read.outcome).collect())
}

/// Reads a thread that must be there
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `number` - The thread's number
async fn thread_of<B: Backend>(fx: &Fx<B>, number: u32) -> anyhow::Result<Thread> {
    match fx.store.thread(&fx.plan, number).await? {
        Some(thread) => Ok(thread),
        None => panic!("thread {number} isn't there"),
    }
}

/// A trailer resolves its open thread and links its revision, once for each change: its change
/// read again, or rewritten by a rebase, writes nothing, so a thread reopened stays open, while
/// a new change resolves it again, and one naming a thread resolved already only links
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn trailers_resolve_each_thread_once_a_change<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // A captured once, with a general thread and a line thread on it, and a reply in the first
    let a = captured(&fx, "A", vec![added_file("src/a.rs", "one\n")]).await?;
    started(&fx, &a, Target::Node, "Take the lock first").await?;
    started(&fx, &a, on_line(1, "src/a.rs", Side::New, 1), "Why one?").await?;
    replied(&fx, 1, "Which lock?").await?;
    // one commit naming both threads, the reply and a number no comment has
    let first = commit_by('f', 100);
    let read = trailed(&fx, &a, 1, &first, &[1, 2, 3, 9]).await?;
    use TrailerOutcome::{Linked, NoThread, Reopened, Seen};
    assert_eq!(
        read,
        [
            TrailerOutcome::Resolved,
            TrailerOutcome::Resolved,
            NoThread,
            NoThread
        ]
    );
    // each thread resolved, a version on, the commit's revision linked
    let one = thread_of(&fx, 1).await?;
    assert_eq!((one.state, one.version), (ThreadState::Resolved, 2));
    let linked = Resolution {
        node: a.clone(),
        revision: 1,
        at: one.resolutions[0].at,
        commit: Some(first.clone()),
    };
    assert_eq!(one.resolutions, std::slice::from_ref(&linked));
    assert_eq!(one.resolved_by(), Some(&linked));
    assert_eq!(one.comments.len(), 2);
    // the same change read again, by the next capture or rewritten by a rebase, writes nothing
    let rebased = ResolvingCommit {
        commit: "r".repeat(40),
        ..first.clone()
    };
    assert_eq!(trailed(&fx, &a, 1, &first, &[1]).await?, [Seen]);
    assert_eq!(trailed(&fx, &a, 2, &rebased, &[1]).await?, [Seen]);
    assert_eq!(thread_of(&fx, 1).await?, one);
    // reopened, the same change still writes nothing, and the thread stays open
    let Marked::Marked { thread: reopened } = mark(&fx, 1, ThreadState::Open, 2).await? else {
        panic!("a mark over the version read was refused");
    };
    assert_eq!(trailed(&fx, &a, 2, &rebased, &[1]).await?, [Reopened]);
    assert_eq!(thread_of(&fx, 1).await?, reopened);
    // a new change resolves it again, its revision linked after the first
    let second = commit_by('s', 200);
    let read = trailed(&fx, &a, 2, &second, &[1, 2]).await?;
    assert_eq!(read, [TrailerOutcome::Resolved, Linked]);
    let one = thread_of(&fx, 1).await?;
    assert_eq!((one.state, one.version), (ThreadState::Resolved, 4));
    let revisions: Vec<(u32, &str)> = one
        .resolutions
        .iter()
        .map(|linked| {
            (
                linked.revision,
                linked.commit.as_ref().unwrap().commit.as_str(),
            )
        })
        .collect();
    assert_eq!(
        revisions,
        [(1, first.commit.as_str()), (2, second.commit.as_str())]
    );
    // the thread resolved already only had the revision linked, a version on
    let two = thread_of(&fx, 2).await?;
    assert_eq!((two.state, two.version), (ThreadState::Resolved, 3));
    assert_eq!(two.resolutions.len(), 2);
    // a capture of another node of the plan resolves a thread on this one, linking its own
    let b = captured(&fx, "B", vec![added_file("src/b.rs", "two\n")]).await?;
    started(&fx, &a, Target::Node, "And the docs?").await?;
    let third = commit_by('t', 300);
    let read = trailed(&fx, &b, 1, &third, &[4]).await?;
    assert_eq!(read, [TrailerOutcome::Resolved]);
    let four = thread_of(&fx, 4).await?;
    assert_eq!(four.node, a);
    assert_eq!(four.resolutions[0].node, b);
    // and everything reads back the same once the store is reopened
    let before = fx.store.threads(&fx.plan).await?;
    let fx = fx.reopen().await?;
    assert_eq!(fx.store.threads(&fx.plan).await?, before);
    Ok(())
}

/// A thread resolved by hand over the version it is at links the revision named, and one over
/// an older version, or one another writer beat, is stale and changes nothing
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn resolving_by_hand_is_over_the_version_read<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // a general thread on A, and B's revision 1 that answered it
    let a = named(&fx, "A");
    let (thread, _) = started(&fx, &a, Target::Node, "Is this done?").await?;
    let b = captured(&fx, "B", vec![added_file("src/b.rs", "two\n")]).await?;
    let resolve = |read| ResolveThread {
        node: b.clone(),
        revision: 1,
        read,
    };
    // over the version read, it is resolved, the revision linked with no commit
    let Resolved::Resolved { thread: resolved } = fx
        .store
        .resolve_thread(&fx.plan, 1, resolve(thread.version))
        .await?
    else {
        panic!("a resolve over the version read was refused");
    };
    assert_eq!(
        (resolved.state, resolved.version),
        (ThreadState::Resolved, 2)
    );
    let linked = &resolved.resolutions[0];
    assert_eq!(
        (&linked.node, linked.revision, &linked.commit),
        (&b, 1, &None)
    );
    // over the version it was at before, it is stale, and nothing changes
    let stale = fx.store.resolve_thread(&fx.plan, 1, resolve(1)).await?;
    assert_eq!(
        stale,
        Resolved::Stale {
            thread: resolved.clone()
        }
    );
    // one another writer beat to the row is stale too
    fx.faults.refuse_next_comment_write();
    let beaten = fx.store.resolve_thread(&fx.plan, 1, resolve(2)).await?;
    assert_eq!(beaten, Resolved::Stale { thread: resolved });
    // and a thread that isn't there is named
    let missing = fx.store.resolve_thread(&fx.plan, 9, resolve(1)).await?;
    assert_eq!(missing, Resolved::NoThread { thread: 9 });
    Ok(())
}
