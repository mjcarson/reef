//! The store suite's history cases, each generic over the backend so it runs against shoal and
//! the twin
//!
//! Each case writes a plan's nodes, its pages or its project's queue as one author or another,
//! then reads back each one's history: who made each edit, the version it left and what it
//! changed, newest first.

use reef_api::{
    Activity, Author, Edited, HISTORY_MAX, HistoryQuery, NodePut, NodeRemove, PageEdit, PageWrite,
    Place, PlanWrite, Puts, QueueEdit, Updated, Writer,
};
use reef_api::{LoopEdit, LoopEdited, Pane};
use reef_model::accept::{Criterion, Kind};
use reef_model::{Diagnostic, Met, PlanName, Status};
use reef_service::schema::{ActivityRow, LOOPS, LoopRow, PROJECTS, ProjectRow, USERS, UserRow};
use reef_service::{
    Backend, Checked, Expect, NodeWrite, PageEdited, Proposal, Queued, Rewrite, StartWork, Started,
};

use std::time::{SystemTime, UNIX_EPOCH};

use super::{Fx, Reopen, work};

/// A check that finds no problem in any write, so each case judges only what it keeps
///
/// # Arguments
///
/// * `_proposal` - The plan as it stands and as the write would leave it
async fn no_problems(_proposal: Proposal) -> anyhow::Result<Vec<Diagnostic>> {
    Ok(Vec::new())
}

/// The author of a write by a user signed in, or by Claude for them
///
/// # Arguments
///
/// * `name` - The user's name
/// * `by` - Whether the user or Claude made it
fn by(name: &str, by: Writer) -> Author {
    Author {
        user: Some(name.to_string()),
        since: Some(1),
        by,
    }
}

/// Puts one node over the version read, as an author, checking it was written
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `node` - The node
/// * `read` - The version read, or `None` for a new node
/// * `author` - Who puts it
async fn put<B: Reopen>(
    fx: &Fx<B>,
    node: NodeWrite,
    read: Option<u64>,
    author: &Author,
) -> anyhow::Result<()> {
    let puts = vec![NodePut::new(node, read)];
    let checked = fx
        .store
        .put_checked(&fx.plan, puts, author, no_problems)
        .await?;
    anyhow::ensure!(
        matches!(checked, Checked::Applied { .. }),
        "the put wasn't written: {checked:?}"
    );
    Ok(())
}

/// Writes the whole plan, as nobody known, checking it was written
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `write` - Every node of the plan, and its pages if they change
async fn write_plan<B: Reopen>(fx: &Fx<B>, write: PlanWrite) -> anyhow::Result<()> {
    let rewrite = fx
        .store
        .write_plan_checked(&fx.plan, write, &Author::default(), no_problems)
        .await?;
    anyhow::ensure!(
        matches!(rewrite, Rewrite::Applied(_)),
        "the plan wasn't written: {rewrite:?}"
    );
    Ok(())
}

/// A node's history says who made each edit, the version it left and what it changed, newest
/// first, reads back the same after a restart, and outlives the node's removal
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_nodes_history_says_who_changed_what<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let node = fx.plan.node("C1")?;
    let (ada, bob, claude) = (
        by("ada", Writer::User),
        by("bob", Writer::User),
        by("ada", Writer::Claude),
    );
    // a node never edited has no history
    assert!(
        fx.store
            .node_history(&node, &HistoryQuery::default())
            .await?
            .edits
            .is_empty()
    );
    // ada adds C1 with a criterion, bob sets it in progress, Claude for ada meets the criterion
    // naming its test, and ada edits its title and body
    let mut c1 = work("C1", 0);
    c1.spec.acceptance = vec![Criterion {
        kind: Kind::Unit,
        text: "Kept".to_string(),
        ..Criterion::default()
    }];
    put(&fx, c1.clone(), None, &ada).await?;
    c1.spec.status = Some(Status::InProgress);
    put(&fx, c1.clone(), Some(1), &bob).await?;
    c1.spec.acceptance[0].status = Some(Met::Met);
    c1.spec.acceptance[0].test = Some("tests/a.rs::kept".to_string());
    put(&fx, c1.clone(), Some(2), &claude).await?;
    c1.spec.title = Some("C1 Other work".to_string());
    c1.body = "# C1\n\nAnother body.\n".to_string();
    put(&fx, c1.clone(), Some(3), &ada).await?;
    // newest first, each with its author and the version it left
    let history = fx
        .store
        .node_history(&node, &HistoryQuery::default())
        .await?
        .edits;
    let told: Vec<(Option<u64>, Option<String>)> = history
        .iter()
        .map(|logged| (logged.version, logged.author.describe()))
        .collect();
    assert_eq!(
        told,
        vec![
            (Some(4), Some("ada".to_string())),
            (Some(3), Some("Claude, for ada".to_string())),
            (Some(2), Some("bob".to_string())),
            (Some(1), Some("ada".to_string())),
        ]
    );
    assert!(history.windows(2).all(|pair| pair[0].at > pair[1].at));
    // each saying what it changed
    assert_eq!(history[3].edits, vec![Edited::Added]);
    assert_eq!(
        history[2].edits,
        vec![Edited::Status {
            from: None,
            to: Some(Status::InProgress)
        }]
    );
    assert_eq!(
        history[1].edits,
        vec![Edited::Criterion {
            number: 1,
            text: "Kept".to_string(),
            from: None,
            to: Some(Met::Met),
            test: Some("tests/a.rs::kept".to_string()),
        }]
    );
    assert_eq!(
        history[0].edits,
        vec![
            Edited::Keys {
                keys: vec!["title".to_string()]
            },
            Edited::Body
        ]
    );
    assert!(history.iter().all(|logged| !logged.whole_plan));
    // the same once the store is reopened; a store opened again starts its clock over, so two
    // stores' edits within one millisecond sort by each store's own number, and a restart, which
    // takes longer than that, sorts its edits after the last store's
    let fx = fx.reopen().await?;
    assert_eq!(
        fx.store
            .node_history(&node, &HistoryQuery::default())
            .await?
            .edits,
        history
    );
    // a restart outlasts the last store's stamps, which writes within one millisecond push ahead
    // of the clock, so the twin, reopened at once, waits for its clock to pass them
    let newest = history[0].at;
    while std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis()
        <= u128::from(newest)
    {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    // and a node removed keeps its history, its removal newest
    let removal = Puts {
        puts: Vec::new(),
        removes: vec![NodeRemove::new("C1", 4)],
    };
    let removed = fx
        .store
        .batch_checked(&fx.plan, removal, &bob, no_problems)
        .await?;
    assert!(matches!(removed, Checked::Applied { .. }), "{removed:?}");
    let history = fx
        .store
        .node_history(&node, &HistoryQuery::default())
        .await?
        .edits;
    assert_eq!(history.len(), 5);
    assert_eq!(
        (
            history[0].version,
            history[0].author.describe(),
            history[0].edits.clone()
        ),
        (None, Some("bob".to_string()), vec![Edited::Removed])
    );
    Ok(())
}

/// A batch logs only the puts that changed their nodes, and a whole-plan write only the nodes
/// and pages it added, changed or removed, so writing the plan again as it stands logs nothing
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn writes_log_only_what_they_changed<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let node = |id: &str| fx.plan.node(id);
    let length = |id: &'static str| {
        let fx = &fx;
        async move {
            anyhow::Ok(
                fx.store
                    .node_history(&fx.plan.node(id)?, &HistoryQuery::default())
                    .await?
                    .edits
                    .len(),
            )
        }
    };
    // an import of C1 and C2 with a page, each added, as a whole plan's write
    let page = PageWrite::new("design/a.md", "# A\n");
    let import = PlanWrite {
        nodes: vec![work("C1", 0), work("C2", 1)],
        pages: Some(vec![page.clone()]),
    };
    write_plan(&fx, import.clone()).await?;
    for id in ["C1", "C2"] {
        let history = fx
            .store
            .node_history(&node(id)?, &HistoryQuery::default())
            .await?
            .edits;
        assert_eq!(history.len(), 1, "{id}: {history:?}");
        assert_eq!(history[0].edits, vec![Edited::Added]);
        assert!(history[0].whole_plan && history[0].author.is_unknown());
    }
    let pages = fx
        .store
        .page_history(&fx.plan, "design/a.md", &HistoryQuery::default())
        .await?
        .edits;
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].edits, vec![Edited::Added]);
    // the same import again changes nothing, so it logs nothing, though versions move on
    write_plan(&fx, import).await?;
    assert_eq!((length("C1").await?, length("C2").await?), (1, 1));
    assert_eq!(
        fx.store
            .page_history(&fx.plan, "design/a.md", &HistoryQuery::default())
            .await?
            .edits
            .len(),
        1
    );
    // a batch that puts C1 as it is and retitles C2 logs C2 alone
    let c1 = fx
        .store
        .get(&fx.plan, "C1")
        .await?
        .expect("C1 was imported");
    let c2 = fx
        .store
        .get(&fx.plan, "C2")
        .await?
        .expect("C2 was imported");
    let mut retitled = work("C2", 1);
    retitled.spec.title = Some("C2 Retitled".to_string());
    let batch = vec![
        NodePut::new(work("C1", 0), Some(c1.version)),
        NodePut::new(retitled.clone(), Some(c2.version)),
    ];
    let author = by("ada", Writer::User);
    let checked = fx
        .store
        .put_checked(&fx.plan, batch, &author, no_problems)
        .await?;
    assert!(matches!(checked, Checked::Applied { .. }), "{checked:?}");
    assert_eq!((length("C1").await?, length("C2").await?), (1, 2));
    // a whole-plan write that changes C1, drops C2 and adds C3 logs each of those, and the page
    // it takes out
    let mut changed = work("C1", 0);
    changed.spec.optional = true;
    let rewrite = PlanWrite {
        nodes: vec![changed, work("C3", 1)],
        pages: Some(Vec::new()),
    };
    write_plan(&fx, rewrite).await?;
    let newest = |id: &'static str| {
        let fx = &fx;
        async move {
            let history = fx
                .store
                .node_history(&fx.plan.node(id)?, &HistoryQuery::default())
                .await?
                .edits;
            anyhow::Ok(history.first().map(|logged| logged.edits.clone()))
        }
    };
    assert_eq!(
        newest("C1").await?,
        Some(vec![Edited::Keys {
            keys: vec!["optional".to_string()]
        }])
    );
    assert_eq!(newest("C2").await?, Some(vec![Edited::Removed]));
    assert_eq!(newest("C3").await?, Some(vec![Edited::Added]));
    let pages = fx
        .store
        .page_history(&fx.plan, "design/a.md", &HistoryQuery::default())
        .await?
        .edits;
    assert_eq!(pages[0].edits, vec![Edited::Removed]);
    assert_eq!(pages[0].version, None);
    Ok(())
}

/// A page's edits and a queue's each keep a history of their own, naming who made each one, and
/// an edit that changes nothing keeps none
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn pages_and_the_queue_keep_histories<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let (ada, bob) = (by("ada", Writer::User), by("bob", Writer::User));
    // ada adds a page, bob writes it again as it is and then edits it, and ada takes it out
    let edits = [
        (
            PageEdit::Put {
                page: PageWrite::new("design/a.md", "# A\n"),
                read: None,
            },
            &ada,
        ),
        (
            PageEdit::Put {
                page: PageWrite::new("design/a.md", "# A\n"),
                read: Some(1),
            },
            &bob,
        ),
        (
            PageEdit::Put {
                page: PageWrite::new("design/a.md", "# A, again\n"),
                read: Some(2),
            },
            &bob,
        ),
        (
            PageEdit::Remove {
                path: "design/a.md".to_string(),
                read: 3,
            },
            &ada,
        ),
    ];
    for (edit, author) in edits {
        let edited = fx
            .store
            .edit_page_checked(&fx.plan, edit, author, no_problems)
            .await?;
        assert!(matches!(edited, PageEdited::Applied { .. }), "{edited:?}");
    }
    let history = fx
        .store
        .page_history(&fx.plan, "design/a.md", &HistoryQuery::default())
        .await?
        .edits;
    let told: Vec<(Option<u64>, Option<String>, Vec<Edited>)> = history
        .iter()
        .map(|logged| {
            (
                logged.version,
                logged.author.describe(),
                logged.edits.clone(),
            )
        })
        .collect();
    assert_eq!(
        told,
        vec![
            (None, Some("ada".to_string()), vec![Edited::Removed]),
            (Some(3), Some("bob".to_string()), vec![Edited::Text]),
            (Some(1), Some("ada".to_string()), vec![Edited::Added]),
        ]
    );
    // ada queues C1, bob queues C2 last, ada moves C2 first and then moves it there again, and
    // bob takes C1 off
    let (c1, c2) = (fx.plan.node("C1")?, fx.plan.node("C2")?);
    let edits = [
        (
            QueueEdit::Add {
                node: c1.clone(),
                at: Place::Top,
            },
            &ada,
        ),
        (
            QueueEdit::Add {
                node: c2.clone(),
                at: Place::Bottom,
            },
            &bob,
        ),
        (
            QueueEdit::Move {
                node: c2.clone(),
                at: Place::Top,
            },
            &ada,
        ),
        (
            QueueEdit::Move {
                node: c2.clone(),
                at: Place::Top,
            },
            &ada,
        ),
        (QueueEdit::Remove { node: c1.clone() }, &bob),
    ];
    for (edit, author) in edits {
        let queued = fx.store.edit_queue("reef", edit, author).await?;
        assert!(matches!(queued, Queued::Applied { .. }), "{queued:?}");
    }
    let history = fx
        .store
        .queue_history("reef", &HistoryQuery::default())
        .await?
        .edits;
    let told: Vec<(Option<String>, Vec<Edited>)> = history
        .iter()
        .map(|logged| (logged.author.describe(), logged.edits.clone()))
        .collect();
    assert_eq!(
        told,
        vec![
            (
                Some("bob".to_string()),
                vec![Edited::Unqueued { node: c1.clone() }]
            ),
            (
                Some("ada".to_string()),
                vec![Edited::Moved {
                    node: c2.clone(),
                    at: Place::Top
                }]
            ),
            (
                Some("bob".to_string()),
                vec![Edited::Queued {
                    node: c2.clone(),
                    at: Place::Bottom
                }]
            ),
            (
                Some("ada".to_string()),
                vec![Edited::Queued {
                    node: c1.clone(),
                    at: Place::Top
                }]
            ),
        ]
    );
    assert!(history.iter().all(|logged| logged.version.is_none()));
    Ok(())
}

/// An edit whose history fails to keep still stands, and the next edit's history is kept
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn an_unkept_history_leaves_its_edit<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let node = fx.plan.node("C1")?;
    let ada = by("ada", Writer::User);
    // the history's write fails, and the put stands
    fx.faults.fail_next_edit_write();
    put(&fx, work("C1", 0), None, &ada).await?;
    let stored = fx.store.get(&fx.plan, "C1").await?.expect("the put stands");
    assert_eq!(stored.version, 1);
    assert!(
        fx.store
            .node_history(&node, &HistoryQuery::default())
            .await?
            .edits
            .is_empty()
    );
    // and the next edit's is kept, a gap where the first's would be
    let mut c1 = work("C1", 0);
    c1.spec.status = Some(Status::Done);
    put(&fx, c1, Some(1), &ada).await?;
    let history = fx
        .store
        .node_history(&node, &HistoryQuery::default())
        .await?
        .edits;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].version, Some(2));
    Ok(())
}

/// A history pages back past its newest 200: each page starts after the one before's cursor, so
/// together they hold every edit once, newest first, and the oldest page names no cursor
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_history_pages_past_its_newest<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let node = fx.plan.node("C1")?;
    let ada = by("ada", Writer::User);
    // 205 edits of C1: its addition, then 204 of its body
    let total = HISTORY_MAX as u64 + 5;
    let mut c1 = work("C1", 0);
    put(&fx, c1.clone(), None, &ada).await?;
    for version in 1..total {
        c1.body = format!("# C1\n\nEdit {version}.\n");
        put(&fx, c1.clone(), Some(version), &ada).await?;
    }
    // the versions a page's entries left, newest first
    let versions = |edits: &[reef_api::Logged]| -> Vec<Option<u64>> {
        edits.iter().map(|logged| logged.version).collect()
    };
    let newest_first =
        |from: u64, to: u64| -> Vec<Option<u64>> { (from..=to).rev().map(Some).collect() };
    // the newest page holds 200 of them, and names the cursor onto the rest
    let newest = fx
        .store
        .node_history(&node, &HistoryQuery::default())
        .await?;
    assert_eq!(versions(&newest.edits), newest_first(6, total));
    let older = newest.older.clone().expect("older edits are kept");
    // the page older than it holds the other five, ending with C1's addition, and names none
    let rest = fx
        .store
        .node_history(&node, &HistoryQuery::older(older.clone()))
        .await?;
    assert_eq!(versions(&rest.edits), newest_first(1, 5));
    assert_eq!(rest.edits[4].edits, vec![Edited::Added]);
    assert_eq!(rest.older, None);
    // a page its limit fills exactly names none either, since nothing older is kept
    let exact = HistoryQuery {
        older: Some(older),
        limit: Some(5),
    };
    let exact = fx.store.node_history(&node, &exact).await?;
    assert_eq!((exact.edits.len(), exact.older), (5, None));
    // smaller pages walk the same history, every edit once and in order
    let mut walked = Vec::new();
    let mut query = HistoryQuery {
        older: None,
        limit: Some(7),
    };
    loop {
        let page = fx.store.node_history(&node, &query).await?;
        assert!(page.edits.len() <= 7, "{}", page.edits.len());
        walked.extend(versions(&page.edits));
        match page.older {
            Some(older) => query.older = Some(older),
            None => break,
        }
    }
    assert_eq!(walked, newest_first(1, total));
    // a page asking for more than 200 gets 200
    let greedy = HistoryQuery {
        older: None,
        limit: Some(5000),
    };
    let greedy = fx.store.node_history(&node, &greedy).await?;
    assert_eq!(greedy.edits.len(), HISTORY_MAX);
    // and the same pages read back once the store is reopened
    let fx = fx.reopen().await?;
    let reread = fx
        .store
        .node_history(&node, &HistoryQuery::default())
        .await?;
    assert_eq!(reread, newest);
    Ok(())
}

/// Each start keeps an entry in its node's history, naming who started it, on what branch and
/// at what commit, and a start again says so, its version left as it was
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_start_is_kept_in_its_nodes_history<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let node = fx.plan.node("C1")?;
    let (ada, bob) = (by("ada", Writer::User), by("bob", Writer::Claude));
    put(&fx, work("C1", 0), None, &ada).await?;
    // ada starts C1, then Claude, for bob, starts it again over her record
    let worktree = std::path::Path::new("/work/reef");
    let first = StartWork {
        start: super::work_cases::start_in(worktree, "C1", 'a'),
        replaces: None,
    };
    let Started::Recorded { record, .. } = fx.store.start_work(&node, first, &ada).await? else {
        anyhow::bail!("the first start wasn't recorded");
    };
    let again = StartWork {
        start: super::work_cases::start_in(worktree, "C1", 'b'),
        replaces: Some(record.version),
    };
    let Started::Recorded { record: again, .. } = fx.store.start_work(&node, again, &bob).await?
    else {
        anyhow::bail!("the start again wasn't recorded");
    };
    // newest first: the start again, the first start, then the node's addition
    let history = fx
        .store
        .node_history(&node, &HistoryQuery::default())
        .await?
        .edits;
    let told: Vec<(Option<u64>, Option<String>, String)> = history
        .iter()
        .map(|logged| (logged.version, logged.author.describe(), logged.describe()))
        .collect();
    assert_eq!(
        told,
        vec![
            (
                None,
                Some("Claude, for bob".to_string()),
                "started again on reef/C1 at bbbbbbb".to_string()
            ),
            (
                None,
                Some("ada".to_string()),
                "started on reef/C1 at aaaaaaa".to_string()
            ),
            (Some(1), Some("ada".to_string()), "added".to_string()),
        ]
    );
    // each naming when its record says it started, which a timeline matches it by
    assert_eq!(
        history[0].edits,
        vec![Edited::Started {
            started_at: again.started_at,
            branch: Some("reef/C1".to_string()),
            start: "b".repeat(40),
            again: true,
        }]
    );
    assert!(matches!(
        history[1].edits[..],
        [Edited::Started { started_at, again: false, .. }] if started_at == record.started_at
    ));
    // and a stale start keeps nothing
    let stale = StartWork {
        start: super::work_cases::start_in(worktree, "C1", 'c'),
        replaces: Some(record.version),
    };
    let refused = fx.store.start_work(&node, stale, &ada).await?;
    assert!(matches!(refused, Started::Stale { .. }), "{refused:?}");
    let after = fx
        .store
        .node_history(&node, &HistoryQuery::default())
        .await?
        .edits;
    assert_eq!(after, history);
    Ok(())
}

/// A loop's acts each keep an entry in its project's loops' history, naming who did them: a
/// start, a take, a join, a hand-off to another session, a pause and a stop, which says what it
/// let go of; a node's end acted on, a hand-off to the session it had and a join again keep none,
/// and the loop says who started it and who changed it last
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn loop_acts_keep_a_history<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let (ada, claude) = (by("ada", Writer::User), by("ada", Writer::Claude));
    let (c1, c2) = (fx.plan.node("C1")?, fx.plan.node("C2")?);
    let (pane, other) = (Pane::Claude { pid: 1 }, Pane::Claude { pid: 2 });
    // ada starts a loop two wide, fed by session s1
    let start = LoopEdit::Start {
        project: "reef".to_string(),
        plan: Some("central".to_string()),
        session: Some("s1".to_string()),
        width: Some(2),
        opens: None,
        pid: None,
    };
    fx.store.edit_loop(&pane, start, &ada).await?;
    // Claude, for her, takes C1 with C2 beside it
    let hold = LoopEdit::Hold {
        node: Some(c1.clone()),
        members: vec![c2.clone()],
    };
    fx.store.edit_loop(&pane, hold, &claude).await?;
    // another pane joins C2, then joins it again, which changes nothing
    for _ in 0..2 {
        let join = LoopEdit::Join { node: c2.clone() };
        let joined = fx.store.edit_loop(&other, join, &ada).await?;
        assert!(matches!(joined, LoopEdited::Joined { .. }), "{joined:?}");
    }
    // the join changed the loop too, so it names ada, who joined, as changing it last
    let joined = fx.store.loop_in(&pane).await?.expect("the loop is there");
    assert_eq!(joined.started_by.describe().as_deref(), Some("ada"));
    assert_eq!(joined.changed_by.describe().as_deref(), Some("ada"));
    // the Stop hook acts on C1's end, as Claude, which leaves who changed it last
    let acted = LoopEdit::Acted { node: c1.clone() };
    fx.store.edit_loop(&pane, acted, &claude).await?;
    let held = fx.store.loop_in(&pane).await?.expect("the loop is there");
    assert_eq!(held.changed_by.describe().as_deref(), Some("ada"));
    assert_eq!(held.acted, Some(c1.clone()));
    // handed to the session it has, which keeps nothing, then to another
    for session in ["s1", "s2"] {
        let handed = LoopEdit::Session {
            session: session.to_string(),
            pid: None,
        };
        fx.store.edit_loop(&pane, handed, &ada).await?;
    }
    // ada pauses it, which it keeps saying, then stops it
    fx.store.edit_loop(&pane, LoopEdit::Pause, &ada).await?;
    let paused = fx.store.loop_in(&pane).await?.expect("the loop is there");
    assert_eq!(paused.changed_by.describe().as_deref(), Some("ada"));
    assert_eq!(paused.started_by.describe().as_deref(), Some("ada"));
    fx.store.edit_loop(&pane, LoopEdit::Stop, &ada).await?;
    assert!(fx.store.loop_in(&pane).await?.is_none());
    // newest first, each with who did it, none with a version
    let history = fx
        .store
        .loop_history("reef", &HistoryQuery::default())
        .await?
        .edits;
    let told: Vec<(Option<String>, String)> = history
        .iter()
        .map(|logged| (logged.author.describe(), logged.describe()))
        .collect();
    let (one, two) = ("Claude Code process 1", "Claude Code process 2");
    let ada_said = |what: String| (Some("ada".to_string()), what);
    assert_eq!(
        told,
        vec![
            ada_said(format!(
                "stopped the loop in {one}, letting go of `reef/central/C1`, `reef/central/C2`"
            )),
            ada_said(format!("paused the loop in {one}")),
            ada_said(format!("handed the loop in {one} to session `s2`")),
            ada_said(format!(
                "{two} joined `reef/central/C2` of the loop in {one}"
            )),
            (
                Some("Claude, for ada".to_string()),
                format!("the loop in {one} took `reef/central/C1`, with `reef/central/C2`")
            ),
            ada_said(format!(
                "started a loop in {one}, feeding plan `central`, 2 nodes at once"
            )),
        ]
    );
    assert!(history.iter().all(|logged| logged.version.is_none()));
    // and the same once the store is reopened
    let fx = fx.reopen().await?;
    let reread = fx
        .store
        .loop_history("reef", &HistoryQuery::default())
        .await?
        .edits;
    assert_eq!(reread, history);
    Ok(())
}

/// Users and loops a reef before C89 kept read back naming nobody, and keep naming nobody
/// through later writes, which name only who made them; and a project and a plan registered
/// before say no registration
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn rows_kept_before_c89_name_nobody<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let ada = by("ada", Writer::User);
    // a user's row as a reef before C89 wrote it, with no adder
    let user = UserRow {
        list: USERS.to_string(),
        name: "old".to_string(),
        version: 1,
        record: r#"{"password":"$argon2id$x","admin":false,"created_at":5}"#.to_string(),
    };
    fx.store.backend().write_user(user, Expect::Absent).await?;
    let old = fx.store.user("old").await?.expect("the user reads back");
    assert!(old.user.added_by.is_unknown());
    // a password set over it leaves its adder as it was
    assert!(fx.store.rewrite_user(&old, "$argon2id$y", false).await?);
    let rewritten = fx.store.user("old").await?.expect("the user reads back");
    assert!(rewritten.user.added_by.is_unknown());
    // a loop's row as a reef before C89 wrote it, with no starter
    let pane = Pane::Claude { pid: 9 };
    let row = LoopRow {
        list: LOOPS.to_string(),
        pane: pane.to_string(),
        version: 1,
        record: r#"{"project":"reef","state":"running","started_at":5}"#.to_string(),
    };
    fx.store.backend().write_loop(row, Expect::Absent).await?;
    let read = fx.store.loop_in(&pane).await?.expect("the loop reads back");
    assert!(read.started_by.is_unknown() && read.changed_by.is_unknown());
    // a pause names who paused it, and leaves who started it unknown
    fx.store.edit_loop(&pane, LoopEdit::Pause, &ada).await?;
    let paused = fx.store.loop_in(&pane).await?.expect("the loop reads back");
    assert!(paused.started_by.is_unknown());
    assert_eq!(paused.changed_by.describe().as_deref(), Some("ada"));
    // and a project registered before, its row with no history beside it, says no registration
    let project = ProjectRow {
        list: PROJECTS.to_string(),
        name: "older".to_string(),
        repo: "/srv/older".to_string(),
    };
    fx.store.backend().insert_project(project).await?;
    assert!(fx.store.project("older").await?.is_some());
    assert!(fx.store.registered("older").await.is_none());
    let plan = reef_model::PlanName::new("older", "main")?;
    assert!(fx.store.plan_registered(&plan).await.is_none());
    Ok(())
}

/// A day in milliseconds, the window the front page counts done nodes in
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Now, in milliseconds since the epoch
fn now_ms() -> u64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past the epoch");
    u64::try_from(since.as_millis()).expect("the time fits in 64 bits")
}

/// Writes a batch of puts to a plan as an author, checking it was written
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `plan` - The plan to write
/// * `puts` - The puts, each with the version its writer read
/// * `author` - Who writes them
async fn put_all<B: Reopen>(
    fx: &Fx<B>,
    plan: &PlanName,
    puts: Vec<NodePut>,
    author: &Author,
) -> anyhow::Result<()> {
    let checked = fx
        .store
        .put_checked(plan, puts, author, no_problems)
        .await?;
    anyhow::ensure!(
        matches!(checked, Checked::Applied { .. }),
        "the puts weren't written: {checked:?}"
    );
    Ok(())
}

/// A node at a status, read at a version
///
/// # Arguments
///
/// * `id` - Its id
/// * `order` - Where it falls in its plan's order
/// * `status` - Its status
/// * `read` - The version read, or `None` for a new node
fn at_status(id: &str, order: u64, status: Status, read: Option<u64>) -> NodePut {
    let mut node = work(id, order);
    node.spec.status = Some(status);
    NodePut::new(node, read)
}

/// When a plan was last written and by whom, as the history of the write it names keeps it
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `node` - The id of a node the newest write changed
async fn newest_edit<B: Reopen>(fx: &Fx<B>, node: &str) -> anyhow::Result<(u64, Option<String>)> {
    let history = fx
        .store
        .node_history(&fx.plan.node(node)?, &HistoryQuery::default())
        .await?;
    let newest = history.edits.first().expect("the node has a history");
    Ok((newest.at, newest.author.describe()))
}

/// When the case's plan was last written and by whom, as the plan answers it
///
/// # Arguments
///
/// * `fx` - The store and its plan
async fn last_update<B: Reopen>(fx: &Fx<B>) -> anyhow::Result<(u64, Option<String>)> {
    let updated = fx
        .store
        .plan_updated(&fx.plan)
        .await?
        .expect("the plan was updated");
    Ok((updated.at, updated.author.describe()))
}

/// A plan answers when it was last written and by whom, and counts its nodes done in the last
/// day: one set back since isn't counted, one added done is, and one that went done two days ago
/// counts in a window that holds then alone
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_plan_answers_its_last_update_and_its_nodes_done<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let (ada, bob) = (by("ada", Writer::User), by("bob", Writer::User));
    let plan = fx.plan.clone();
    // ada adds C1, C2 and C4
    let added = vec![
        NodePut::new(work("C1", 0), None),
        NodePut::new(work("C2", 1), None),
        NodePut::new(work("C4", 2), None),
    ];
    put_all(&fx, &plan, added, &ada).await?;
    assert_eq!(last_update(&fx).await?, newest_edit(&fx, "C4").await?);
    // C4 went done two days ago: its put keeps no history, and an entry then says so
    fx.faults.fail_next_edit_write();
    put_all(
        &fx,
        &plan,
        vec![at_status("C4", 2, Status::Done, Some(1))],
        &ada,
    )
    .await?;
    let then = now_ms() - 2 * DAY_MS;
    let old = Activity {
        at: then,
        author: ada.clone(),
        nodes: vec!["C4".to_string()],
        done: vec!["C4".to_string()],
        ..Activity::default()
    };
    let row = ActivityRow {
        project: plan.project.clone(),
        plan: plan.plan.clone(),
        key: format!("{:020}.{:016x}", u64::MAX - then, 0),
        record: serde_json::to_string(&old)?,
    };
    fx.store
        .backend()
        .write_edits(Vec::new(), vec![row])
        .await?;
    // bob marks C1 and C2 done in one batch, the plan's last update
    let done = vec![
        at_status("C1", 0, Status::Done, Some(1)),
        at_status("C2", 1, Status::Done, Some(1)),
    ];
    put_all(&fx, &plan, done, &bob).await?;
    let (at, author) = last_update(&fx).await?;
    assert_eq!(
        (at, author.as_deref()),
        (newest_edit(&fx, "C1").await?.0, Some("bob"))
    );
    // the whole plan is written again as ada, as it stands with C3 added done
    let nodes = fx.store.list(&plan).await?.nodes;
    let mut write: Vec<NodeWrite> = nodes.into_iter().map(NodeWrite::from).collect();
    let mut c3 = work("C3", 3);
    c3.spec.status = Some(Status::Done);
    write.push(c3);
    let rewrite = fx
        .store
        .write_plan_checked(
            &plan,
            PlanWrite {
                nodes: write,
                pages: None,
            },
            &ada,
            no_problems,
        )
        .await?;
    assert!(matches!(rewrite, Rewrite::Applied(_)), "{rewrite:?}");
    assert_eq!(last_update(&fx).await?, newest_edit(&fx, "C3").await?);
    // and ada sets C2 back in progress
    let c2 = fx.store.get(&plan, "C2").await?.expect("C2 is there");
    let back = at_status("C2", 1, Status::InProgress, Some(c2.version));
    put_all(&fx, &plan, vec![back], &ada).await?;
    assert_eq!(last_update(&fx).await?, newest_edit(&fx, "C2").await?);
    // the last day's done are C1 and C3, in the plan's order: C2 is set back, and C4 went done
    // before it
    let now = now_ms();
    let last_day = fx.store.plan_done(&plan, now - DAY_MS..u64::MAX).await?;
    assert_eq!(last_day, ["C1", "C3"]);
    // three days hold C4 too, and the day two days ago holds it alone
    let three_days = fx
        .store
        .plan_done(&plan, now - 3 * DAY_MS..u64::MAX)
        .await?;
    assert_eq!(three_days, ["C1", "C4", "C3"]);
    let that_day = fx
        .store
        .plan_done(&plan, then - DAY_MS / 2..then + DAY_MS / 2)
        .await?;
    assert_eq!(that_day, ["C4"]);
    // a window with nothing in it, or none at all, counts none
    assert!(
        fx.store
            .plan_done(&plan, 1..then - DAY_MS)
            .await?
            .is_empty()
    );
    assert!(fx.store.plan_done(&plan, now..now).await?.is_empty());
    // a page's edit is an update too, and the store opened again reads the same
    let page = PageEdit::Put {
        page: PageWrite::new("design/a.md", "# A\n"),
        read: None,
    };
    let edited = fx
        .store
        .edit_page_checked(&plan, page, &bob, no_problems)
        .await?;
    assert!(matches!(edited, PageEdited::Applied { .. }), "{edited:?}");
    let paged = fx
        .store
        .page_history(&plan, "design/a.md", &HistoryQuery::default())
        .await?;
    let told = (paged.edits[0].at, Some("bob".to_string()));
    assert_eq!(last_update(&fx).await?, told);
    let fx = fx.reopen().await?;
    assert_eq!(last_update(&fx).await?, told);
    let reopened = fx.store.plan_done(&plan, now - DAY_MS..u64::MAX).await?;
    assert_eq!(reopened, ["C1", "C3"]);
    Ok(())
}

/// A plan none of whose writes was kept answers its listing as its last update, until a write
/// of it is kept
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_plan_never_written_answers_its_listing<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let (ada, bob) = (by("ada", Writer::User), by("bob", Writer::User));
    // ada lists a plan, which answers its listing
    let plan = PlanName::new(&fx.plan.project, "fresh")?;
    assert!(fx.store.add_plan(&plan, &ada).await?);
    let listed = fx
        .store
        .plan_registered(&plan)
        .await
        .expect("its listing is kept");
    assert_eq!(listed.author.describe().as_deref(), Some("ada"));
    let listing = Some(Updated::from(listed));
    assert_eq!(fx.store.plan_updated(&plan).await?, listing);
    // a write whose history failed to keep, as one before C119 kept none, leaves it so
    fx.faults.fail_next_edit_write();
    put_all(
        &fx,
        &plan,
        vec![at_status("C1", 0, Status::Done, None)],
        &bob,
    )
    .await?;
    assert_eq!(fx.store.plan_updated(&plan).await?, listing);
    assert!(fx.store.plan_done(&plan, 0..u64::MAX).await?.is_empty());
    // and its next write that is kept answers in its place
    put_all(&fx, &plan, vec![NodePut::new(work("C2", 1), None)], &bob).await?;
    let history = fx
        .store
        .node_history(&plan.node("C2")?, &HistoryQuery::default())
        .await?;
    let updated = fx.store.plan_updated(&plan).await?.expect("it was written");
    assert_eq!(updated.at, history.edits[0].at);
    assert_eq!(updated.author.describe().as_deref(), Some("bob"));
    // while a plan neither listed nor written answers none
    let never = PlanName::new(&fx.plan.project, "never")?;
    assert_eq!(fx.store.plan_updated(&never).await?, None);
    Ok(())
}
