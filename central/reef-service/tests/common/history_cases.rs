//! The store suite's history cases, each generic over the backend so it runs against shoal and
//! the twin
//!
//! Each case writes a plan's nodes, its pages or its project's queue as one author or another,
//! then reads back each one's history: who made each edit, the version it left and what it
//! changed, newest first.

use reef_api::{
    Author, Edited, HISTORY_MAX, HistoryQuery, NodePut, NodeRemove, PageEdit, PageWrite, Place,
    PlanWrite, Puts, QueueEdit, Writer,
};
use reef_model::accept::{Criterion, Kind};
use reef_model::{Diagnostic, Met, Status};
use reef_service::{Checked, NodeWrite, PageEdited, Proposal, Queued, Rewrite};

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
