//! The store suite's work record and revision cases, each generic over the backend so it runs
//! against shoal and the twin

use reef_api::work::on_worktree;
use reef_api::{FileStatus, Hunk, Line, LineKind};
use reef_model::NodeName;
use reef_service::{
    AddCapture, AddedProject, Backend, CaptureShas, Captured, FILE_ROW_MAX, FileDiff, Revision,
    StartWork, Started, Unkeepable, WorkRecord, WorkStart,
};
use std::path::{Path, PathBuf};

use super::{Fx, Reopen, git};

/// Names a node of the case's plan
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `id` - The node's id
pub fn named<B>(fx: &Fx<B>, id: &str) -> NodeName {
    fx.plan.node(id).expect("a test's ids are good")
}

/// A start read in a worktree, its SHAs made from a letter so two starts differ
///
/// # Arguments
///
/// * `worktree` - The worktree's root
/// * `id` - The node's id, which names its branch and pin
/// * `sha` - The letter its SHAs are made of
pub fn start_in(worktree: &Path, id: &str, sha: char) -> WorkStart {
    WorkStart {
        worktree: worktree.to_path_buf(),
        branch: Some(format!("reef/{id}")),
        base_branch: Some("main".to_string()),
        start: sha.to_string().repeat(40),
        start_tree: sha.to_ascii_uppercase().to_string().repeat(40),
        fork: Some("0".repeat(40)),
        pin: format!("refs/reef/start/central/{id}"),
        pinned: format!("{sha}{}", "p".repeat(39)),
    }
}

/// Starts a node of the case's plan, replacing the version given, and hands back what came of it
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `id` - The node's id
/// * `start` - What the start read
/// * `replaces` - The version of the node's record it replaces, if any
async fn start<B: Backend>(
    fx: &Fx<B>,
    id: &str,
    start: WorkStart,
    replaces: Option<u64>,
) -> anyhow::Result<Started> {
    let request = StartWork { start, replaces };
    fx.store.start_work(&named(fx, id), request).await
}

/// Starts a node that must be recorded, and hands back its record
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `id` - The node's id
/// * `read` - What the start read
/// * `replaces` - The version of the node's record it replaces, if any
pub async fn recorded<B: Backend>(
    fx: &Fx<B>,
    id: &str,
    read: WorkStart,
    replaces: Option<u64>,
) -> anyhow::Result<WorkRecord> {
    match start(fx, id, read, replaces).await? {
        Started::Recorded { record } => Ok(record),
        other => panic!("starting `{id}` was not recorded: {other:?}"),
    }
}

/// The SHAs a capture read, made from a letter
///
/// # Arguments
///
/// * `sha` - The letter its SHAs are made of
fn shas(sha: char) -> CaptureShas {
    CaptureShas {
        head: sha.to_string().repeat(40),
        tree: sha.to_ascii_uppercase().to_string().repeat(40),
        merge_base: None,
        base: "B".repeat(40),
    }
}

/// A new file's diff: one hunk adding each of its lines
///
/// # Arguments
///
/// * `path` - The file's path
/// * `text` - What it holds, a line to each newline
pub fn added_file(path: &str, text: &str) -> FileDiff {
    // every line added, numbered from 1
    let lines: Vec<Line> = text
        .lines()
        .zip(1..)
        .map(|(text, new)| Line {
            kind: LineKind::Added,
            old: None,
            new: Some(new),
            text: text.to_string(),
            no_newline: false,
        })
        .collect();
    FileDiff {
        status: FileStatus::Added,
        old_path: None,
        new_path: Some(path.to_string()),
        old_mode: None,
        new_mode: Some("100644".to_string()),
        old_id: None,
        new_id: Some("f".repeat(40)),
        binary: false,
        too_large: false,
        hunks: vec![Hunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: lines.len() as u32,
            lines,
        }],
    }
}

/// A capture of a start, its SHAs made from a letter, holding the files given
///
/// # Arguments
///
/// * `pinned` - The pinned commit of the start it was taken from
/// * `sha` - The letter its SHAs are made of
/// * `files` - Its files
pub fn capture(pinned: &str, sha: char, files: Vec<FileDiff>) -> AddCapture {
    AddCapture {
        pinned: pinned.to_string(),
        shas: shas(sha),
        conflicts: Vec::new(),
        files,
        resolves: Vec::new(),
    }
}

/// Keeps a capture that must add a revision, and hands back the revision
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `node` - The node captured
/// * `add` - The capture
pub async fn kept<B: Backend>(
    fx: &Fx<B>,
    node: &NodeName,
    add: AddCapture,
) -> anyhow::Result<Revision> {
    match fx.store.add_capture(node, add).await? {
        Captured::Added { revision, .. } => Ok(revision),
        other => panic!("a capture of `{node}` added no revision: {other:?}"),
    }
}

/// A project with no node started has no records, and a node not started has none
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn work_of_unstarted_node_is_none<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // an empty partition reads as no records, and an empty key as none
    assert!(fx.store.work("reef").await?.is_empty());
    assert!(fx.store.work_of(&named(&fx, "A")).await?.is_none());
    Ok(())
}

/// A start reads back as it was recorded, from its node and from its project's listing, and
/// still does after a restart
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn start_reads_back_after_a_restart<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a node started in the case's repo
    let read = start_in(&fx.repo, "A", 'a');
    let record = recorded(&fx, "A", read.clone(), None).await?;
    assert_eq!(record.node, named(&fx, "A"));
    assert_eq!((record.version, &record.start), (1, &read));
    assert!(record.captures.is_empty());
    // read back by its node and in its project's listing
    assert_eq!(
        fx.store.work_of(&named(&fx, "A")).await?,
        Some(record.clone())
    );
    assert_eq!(fx.store.work("reef").await?, std::slice::from_ref(&record));
    // and after the store is reopened over the same rows
    let fx = fx.reopen().await?;
    assert_eq!(fx.store.work_of(&named(&fx, "A")).await?, Some(record));
    Ok(())
}

/// A node started already is started again only by a start naming its record's version, which
/// replaces it, captures and all
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn second_start_is_stale_unless_it_names_the_record<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // A started, and a capture added to its start
    let first = recorded(&fx, "A", start_in(&fx.repo, "A", 'a'), None).await?;
    let add = capture(&first.start.pinned, 'c', Vec::new());
    let Captured::Added {
        record: captured, ..
    } = fx.store.add_capture(&named(&fx, "A"), add).await?
    else {
        panic!("a capture of A's start wasn't added");
    };
    // a start naming no version, or one it isn't at, is stale and names the record as it stands
    for replaces in [None, Some(first.version)] {
        let again = start(&fx, "A", start_in(&fx.repo, "A", 'b'), replaces).await?;
        assert_eq!(
            again,
            Started::Stale {
                record: Some(captured.clone())
            }
        );
    }
    // one naming the version it is at replaces it, its captures gone with the old start
    let second = recorded(
        &fx,
        "A",
        start_in(&fx.repo, "A", 'b'),
        Some(captured.version),
    )
    .await?;
    assert_eq!(second.version, captured.version + 1);
    assert_eq!(second.start, start_in(&fx.repo, "A", 'b'));
    assert!(second.captures.is_empty());
    assert!(second.started_at > first.started_at);
    assert_eq!(fx.store.work("reef").await?, [second]);
    // and a start naming a version of a node never started is stale, with no record to name
    let gone = start(&fx, "B", start_in(&fx.repo, "B", 'c'), Some(1)).await?;
    assert_eq!(gone, Started::Stale { record: None });
    assert!(fx.store.work_of(&named(&fx, "B")).await?.is_none());
    Ok(())
}

/// Each start is later than every start before it in its project, so the node a worktree is on
/// is the one started there last, across the project's plans
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_worktree_is_on_its_latest_start<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // A, then B, in the repo, and C of another plan of the project in another worktree
    let other = fx.dir.join("other");
    recorded(&fx, "A", start_in(&fx.repo, "A", 'a'), None).await?;
    recorded(&fx, "B", start_in(&fx.repo, "B", 'b'), None).await?;
    let elsewhere: NodeName = "reef/docs/C".parse()?;
    let request = StartWork {
        start: start_in(&other, "C", 'c'),
        replaces: None,
    };
    let Started::Recorded { .. } = fx.store.start_work(&elsewhere, request).await? else {
        panic!("a node of another plan of the project wasn't recorded");
    };
    // every start later than the last, even within a millisecond
    let records = fx.store.work("reef").await?;
    let names: Vec<String> = records.iter().map(|r| r.node.to_string()).collect();
    assert_eq!(names, ["reef/central/A", "reef/central/B", "reef/docs/C"]);
    assert!(
        records
            .windows(2)
            .all(|w| w[0].started_at < w[1].started_at)
    );
    // so each worktree is on its own latest start
    let on = |dir: &Path| on_worktree(&records, dir).map(|record| record.node.to_string());
    assert_eq!(on(&fx.repo).as_deref(), Some("reef/central/B"));
    assert_eq!(on(&other).as_deref(), Some("reef/docs/C"));
    assert_eq!(on(&fx.dir.join("nowhere")), None);
    Ok(())
}

/// A capture's SHAs are added to the start it was taken from, and to no other
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn captures_add_to_their_own_start<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a node not started has nowhere to add a capture
    let add = |pinned: &str, sha: char| capture(pinned, sha, Vec::new());
    let node = named(&fx, "A");
    let none = fx.store.add_capture(&node, add("x", 'c')).await?;
    assert_eq!(none, Captured::NotStarted { node: node.clone() });
    // two captures of A's start, added in order, each a version
    let first = recorded(&fx, "A", start_in(&fx.repo, "A", 'a'), None).await?;
    for sha in ['c', 'd'] {
        let Captured::Added { .. } = fx
            .store
            .add_capture(&node, add(&first.start.pinned, sha))
            .await?
        else {
            panic!("a capture of A's start wasn't added");
        };
    }
    let read = fx.store.work_of(&node).await?.expect("A was started");
    assert_eq!(read.version, 3);
    let heads: Vec<&str> = read.captures.iter().map(|c| c.shas.head.as_str()).collect();
    assert_eq!(heads, ["c".repeat(40), "d".repeat(40)]);
    assert!(read.captures[0].at <= read.captures[1].at);
    // A started again, then a capture of its first start, which is none of the second's
    let second = recorded(&fx, "A", start_in(&fx.repo, "A", 'b'), Some(read.version)).await?;
    let late = fx
        .store
        .add_capture(&node, add(&first.start.pinned, 'e'))
        .await?;
    assert_eq!(
        late,
        Captured::Restarted {
            record: second.clone()
        }
    );
    assert_eq!(fx.store.work_of(&node).await?, Some(second));
    Ok(())
}

/// A start refused under its write, as another writer's would leave it, is stale and names the
/// record as it stands, and a start naming a relative worktree is refused before anything
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn refused_and_unkeepable_starts_write_nothing<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // A's write refused under it, as one from another process between the read and the write
    fx.faults.refuse_next_work_write();
    let refused = start(&fx, "A", start_in(&fx.repo, "A", 'a'), None).await?;
    assert_eq!(refused, Started::Stale { record: None });
    assert!(fx.store.work("reef").await?.is_empty());
    // a worktree named relative to wherever the client stood is refused as unkeepable
    let relative = start_in(&PathBuf::from("reef"), "A", 'a');
    let error = start(&fx, "A", relative, None).await.unwrap_err();
    assert!(error.downcast_ref::<Unkeepable>().is_some(), "{error:#}");
    assert!(fx.store.work("reef").await?.is_empty());
    Ok(())
}

/// Each project's records sit in its own partition, and a node's in its own plan's key
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn each_project_has_its_own_work<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a second project, of a repo of its own, with a node of the same plan and id started
    let repo = git::repo(&fx.dir.join("app"))?;
    let AddedProject::Added(_) = fx.store.add_project("app", &repo).await? else {
        panic!("a project for a fresh repo was refused");
    };
    let theirs: NodeName = "app/central/A".parse()?;
    let request = StartWork {
        start: start_in(&repo, "A", 'b'),
        replaces: None,
    };
    let Started::Recorded { record } = fx.store.start_work(&theirs, request).await? else {
        panic!("a node of another project wasn't recorded");
    };
    // and reef's A, which the same key in another partition doesn't make stale
    let ours = recorded(&fx, "A", start_in(&fx.repo, "A", 'a'), None).await?;
    // each reads only its own
    assert_eq!(fx.store.work("reef").await?, std::slice::from_ref(&ours));
    assert_eq!(fx.store.work("app").await?, std::slice::from_ref(&record));
    assert_eq!(fx.store.work_of(&theirs).await?, Some(record));
    assert_eq!(fx.store.work_of(&named(&fx, "A")).await?, Some(ours));
    Ok(())
}

/// Captures are kept as their node's revisions, numbered from 1 and on through a restart, each
/// file read by its path, and a capture its latest revision holds adds none
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn captures_are_kept_as_numbered_revisions<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a node never captured has no revisions
    let node = named(&fx, "A");
    assert!(fx.store.revisions(&node).await?.is_empty());
    assert!(fx.store.node_revision(&node, 1).await?.is_none());
    // two captures of A's start, each a revision of the files it holds
    let first = recorded(&fx, "A", start_in(&fx.repo, "A", 'a'), None).await?;
    let pinned = first.start.pinned.clone();
    let one = vec![
        added_file("src/b.rs", "b\n"),
        added_file("src/a.rs", "a\nb\n"),
    ];
    let r1 = kept(&fx, &node, capture(&pinned, 'c', one.clone())).await?;
    let r2 = kept(
        &fx,
        &node,
        capture(&pinned, 'd', vec![added_file("c.rs", "c\n")]),
    )
    .await?;
    assert_eq!((r1.revision, r2.revision), (1, 2));
    assert_eq!(r1.node, node);
    assert_eq!(r1.shas, shas('c'));
    assert_eq!(r1.counts(), (3, 0));
    let paths: Vec<&str> = r1.files.iter().map(|file| file.path()).collect();
    assert_eq!(paths, ["src/b.rs", "src/a.rs"]);
    // each file reads by its path, whole, and a path its revision doesn't list reads as none
    for file in &one {
        let read = fx.store.revision_file(&node, 1, file.path()).await?;
        assert_eq!(read.as_ref(), Some(file));
    }
    assert!(fx.store.revision_file(&node, 1, "c.rs").await?.is_none());
    assert!(fx.store.revision_file(&node, 3, "c.rs").await?.is_none());
    // the same base and tree as the latest adds no revision, and its SHAs name the latest
    let again = fx
        .store
        .add_capture(&node, capture(&pinned, 'd', Vec::new()))
        .await?;
    let Captured::Unchanged {
        record, revision, ..
    } = again
    else {
        panic!("a capture of the latest's base and tree was {again:?}");
    };
    assert_eq!(revision, r2);
    let named: Vec<Option<u32>> = record.captures.iter().map(|c| c.revision).collect();
    assert_eq!(named, [Some(1), Some(2), Some(2)]);
    // A started again: its revisions stay, and its next is numbered after them
    let second = recorded(&fx, "A", start_in(&fx.repo, "A", 'b'), Some(record.version)).await?;
    let late = fx
        .store
        .add_capture(
            &node,
            capture(&pinned, 'e', vec![added_file("e.rs", "e\n")]),
        )
        .await?;
    assert!(matches!(late, Captured::Restarted { .. }), "{late:?}");
    let r3 = kept(&fx, &node, capture(&second.start.pinned, 'f', Vec::new())).await?;
    assert_eq!(r3.revision, 3);
    assert!(r3.files.is_empty());
    // and all of it reads back once the store is reopened over the same rows
    let fx = fx.reopen().await?;
    assert_eq!(fx.store.revisions(&node).await?, [r1, r2.clone(), r3]);
    assert_eq!(fx.store.node_revision(&node, 2).await?, Some(r2));
    let read = fx.store.revision_file(&node, 1, "src/a.rs").await?;
    assert_eq!(read.as_ref(), Some(&one[1]));
    Ok(())
}

/// A capture killed after its files and before its revision keeps nothing a reader sees, and
/// the next capture is kept under the same number, serving only its own files
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn killed_capture_serves_none_of_its_files<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // A's capture killed between its files and its revision
    let node = named(&fx, "A");
    let first = recorded(&fx, "A", start_in(&fx.repo, "A", 'a'), None).await?;
    let pinned = first.start.pinned.clone();
    fx.faults.fail_next_revision_write();
    let files = vec![added_file("a.rs", "a\n"), added_file("b.rs", "b\n")];
    let error = fx
        .store
        .add_capture(&node, capture(&pinned, 'c', files))
        .await
        .expect_err("a killed capture succeeded");
    assert!(format!("{error:#}").contains("killed"), "{error:#}");
    // no revision, and the record has none of its SHAs
    assert!(fx.store.revisions(&node).await?.is_empty());
    assert!(fx.store.revision_file(&node, 1, "a.rs").await?.is_none());
    let record = fx.store.work_of(&node).await?.expect("A was started");
    assert!(record.captures.is_empty());
    // the next capture is revision 1, and the killed one's other file is none of its
    let next = vec![added_file("a.rs", "a, again\n")];
    let revision = kept(&fx, &node, capture(&pinned, 'd', next.clone())).await?;
    assert_eq!(revision.revision, 1);
    let read = fx.store.revision_file(&node, 1, "a.rs").await?;
    assert_eq!(read.as_ref(), Some(&next[0]));
    assert!(fx.store.revision_file(&node, 1, "b.rs").await?.is_none());
    Ok(())
}

/// A file too large to keep is kept without its hunks, its counts still said, beside the files
/// kept whole, and a capture naming one path twice, or none, is refused before anything
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn captures_keep_what_fits_and_refuse_what_cant_be_read<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let node = named(&fx, "A");
    let first = recorded(&fx, "A", start_in(&fx.repo, "A", 'a'), None).await?;
    let pinned = first.start.pinned.clone();
    // one file past a row's limit, and one well under it
    let line = format!("{}\n", "x".repeat(1023));
    let lines = FILE_ROW_MAX / line.len() + 1;
    let big = added_file("big.txt", &line.repeat(lines));
    let small = added_file("small.txt", "small\n");
    let revision = kept(&fx, &node, capture(&pinned, 'c', vec![big, small.clone()])).await?;
    let big = &revision.files[0];
    assert!(big.too_large, "{big:?}");
    assert_eq!(big.added, lines as u32);
    assert!(!revision.files[1].too_large);
    // the big one reads back without its hunks, and the small one whole
    let read = fx.store.revision_file(&node, 1, "big.txt").await?;
    let read = read.expect("the big file is listed");
    assert!(read.too_large && read.hunks.is_empty(), "{read:?}");
    let read = fx.store.revision_file(&node, 1, "small.txt").await?;
    assert_eq!(read, Some(small.clone()));
    // two files under one path, or one under none, are refused, keeping nothing
    let twice = vec![small.clone(), small.clone()];
    let mut nameless = small;
    nameless.new_path = None;
    for files in [twice, vec![nameless]] {
        let error = fx
            .store
            .add_capture(&node, capture(&pinned, 'd', files))
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<Unkeepable>().is_some(), "{error:#}");
    }
    assert_eq!(fx.store.revisions(&node).await?.len(), 1);
    Ok(())
}
