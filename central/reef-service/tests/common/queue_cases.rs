//! The store suite's queue cases, each generic over the backend so it runs against shoal and the
//! twin

use reef_model::NodeName;
use reef_service::schema::QueueRow;
use reef_service::{AddedProject, Backend, Place, QueueEdit, Queued, Refusal, Unkeepable, Written};

use super::{Fx, Reopen, git};

/// Names a node of the case's plan
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `id` - The node's id
fn named<B>(fx: &Fx<B>, id: &str) -> NodeName {
    fx.plan.node(id).expect("a test's ids are good")
}

/// Puts a node of the case's plan in the queue, at a place
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `id` - The node's id
/// * `at` - Where it goes
async fn add<B: Backend>(fx: &Fx<B>, id: &str, at: Place) -> anyhow::Result<Queued> {
    let edit = QueueEdit::Add {
        node: named(fx, id),
        at,
    };
    fx.store.edit_queue("reef", edit, &reef_api::NOBODY).await
}

/// The ids of the case's queue, in its order
///
/// # Arguments
///
/// * `fx` - The store and its plan
async fn order<B: Backend>(fx: &Fx<B>) -> anyhow::Result<Vec<String>> {
    let queue = fx.store.queue("reef").await?;
    Ok(queue.into_iter().map(|entry| entry.node.id).collect())
}

/// Every row of the case's queue as stored, in rank order, whatever a read would settle
///
/// # Arguments
///
/// * `fx` - The store and its plan
async fn rows<B: Backend>(fx: &Fx<B>) -> anyhow::Result<Vec<QueueRow>> {
    let mut rows = fx.store.backend().queue("reef").await?;
    rows.sort_by(|a, b| a.rank.cmp(&b.rank));
    Ok(rows)
}

/// A row of the case's queue for a node of its plan, written straight to the backend
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `id` - The node's id
/// * `rank` - Its rank
/// * `seq` - The write count it was written at
async fn plant<B: Backend>(fx: &Fx<B>, id: &str, rank: &str, seq: u64) -> anyhow::Result<()> {
    let row = QueueRow {
        project: "reef".to_string(),
        rank: rank.to_string(),
        plan: fx.plan.plan.clone(),
        id: id.to_string(),
        seq,
    };
    fx.store.backend().insert_queue(row).await?;
    Ok(())
}

/// A project's queue is empty before anything is queued
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn queue_of_new_project_is_empty<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // an empty partition reads as no entries
    assert!(fx.store.queue("reef").await?.is_empty());
    Ok(())
}

/// Entries read back in the order they were placed in, and adding a queued node moves it
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn queue_reads_in_the_order_entries_are_placed<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // at either end, then beside an entry
    add(&fx, "A", Place::Bottom).await?;
    add(&fx, "B", Place::Bottom).await?;
    add(&fx, "C", Place::Top).await?;
    add(&fx, "D", Place::Before(named(&fx, "A"))).await?;
    add(&fx, "E", Place::After(named(&fx, "B"))).await?;
    assert_eq!(order(&fx).await?, ["C", "D", "A", "B", "E"]);
    // a node queued again moves, keeping one row
    let Queued::Applied { queue } = add(&fx, "B", Place::Top).await? else {
        panic!("moving a queued node was refused");
    };
    let ids: Vec<&str> = queue.iter().map(|entry| entry.node.id.as_str()).collect();
    assert_eq!(ids, ["B", "C", "D", "A", "E"]);
    assert_eq!(order(&fx).await?, ids);
    // the ranks increase strictly, one row a node
    assert!(queue.windows(2).all(|w| w[0].rank < w[1].rank), "{queue:?}");
    assert_eq!(rows(&fx).await?.len(), 5);
    // and a removal takes its node off
    let remove = QueueEdit::Remove {
        node: named(&fx, "D"),
    };
    fx.store
        .edit_queue("reef", remove, &reef_api::NOBODY)
        .await?;
    assert_eq!(order(&fx).await?, ["B", "C", "A", "E"]);
    Ok(())
}

/// A move writes the one row of the node that moves, and leaves every other row as it was
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_move_writes_one_row<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // three entries
    for id in ["A", "B", "C"] {
        add(&fx, id, Place::Bottom).await?;
    }
    let before = rows(&fx).await?;
    // C to the top
    let edit = QueueEdit::Move {
        node: named(&fx, "C"),
        at: Place::Top,
    };
    fx.store.edit_queue("reef", edit, &reef_api::NOBODY).await?;
    let after = rows(&fx).await?;
    // still a row each, A's and B's untouched, C's at a rank before them
    assert_eq!(after.len(), 3);
    assert_eq!(&after[1..], &before[..2]);
    assert_eq!(after[0].id, "C");
    assert!(after[0].rank < before[0].rank && after[0].seq > before[2].seq);
    Ok(())
}

/// A move killed between writing its new row and deleting its old one leaves the node in two
/// rows, which a read settles as the move and the next edit prunes
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn interrupted_move_leaves_a_duplicate_dropped_on_read<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // three entries, then C moved to the top and killed before its old row is deleted
    for id in ["A", "B", "C"] {
        add(&fx, id, Place::Bottom).await?;
    }
    fx.faults.fail_next_queue_delete();
    let edit = QueueEdit::Move {
        node: named(&fx, "C"),
        at: Place::Top,
    };
    let error = fx
        .store
        .edit_queue("reef", edit, &reef_api::NOBODY)
        .await
        .expect_err("the killed move succeeded");
    assert!(format!("{error:#}").contains("killed"), "{error:#}");
    // C is stored twice, at the top and at the bottom
    let stored: Vec<String> = rows(&fx).await?.into_iter().map(|row| row.id).collect();
    assert_eq!(stored, ["C", "A", "B", "C"]);
    // a read drops the duplicate, the move made
    assert_eq!(order(&fx).await?, ["C", "A", "B"]);
    // and in a restarted process the next edit prunes it
    let fx = fx.reopen().await?;
    add(&fx, "D", Place::Bottom).await?;
    let stored: Vec<String> = rows(&fx).await?.into_iter().map(|row| row.id).collect();
    assert_eq!(stored, ["C", "A", "B", "D"]);
    assert_eq!(order(&fx).await?, ["C", "A", "B", "D"]);
    Ok(())
}

/// A placement that would make a long rank spreads the queue anew, short and in order
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn long_ranks_are_rebalanced_in_order<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // A and B so close that a rank between them is long, then C
    let close = format!("a0{}1", "0".repeat(39));
    plant(&fx, "A", "a0", 1).await?;
    plant(&fx, "B", &close, 2).await?;
    plant(&fx, "C", "a1", 3).await?;
    // D between A and B
    add(&fx, "D", Place::After(named(&fx, "A"))).await?;
    assert_eq!(order(&fx).await?, ["A", "D", "B", "C"]);
    // every rank short, a row a node
    let stored = rows(&fx).await?;
    assert_eq!(stored.len(), 4);
    assert!(stored.iter().all(|row| row.rank.len() <= 3), "{stored:?}");
    Ok(())
}

/// A rebalance killed midway leaves the queue in order, and the next edit finishes it
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn killed_rebalance_leaves_the_queue_in_order<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // as before, a gap too narrow to split short
    let close = format!("a0{}1", "0".repeat(39));
    plant(&fx, "A", "a0", 1).await?;
    plant(&fx, "B", &close, 2).await?;
    plant(&fx, "C", "a1", 3).await?;
    // the rebalance moves C, then is killed before it writes B's new row
    fx.faults.fail_queue_inserts_after(1);
    let killed = add(&fx, "D", Place::After(named(&fx, "A"))).await;
    let error = killed.expect_err("the killed rebalance succeeded");
    assert!(format!("{error:#}").contains("killed"), "{error:#}");
    // the queue still reads in order, each entry once, and D isn't in it
    assert_eq!(order(&fx).await?, ["A", "B", "C"]);
    // and in a restarted process the next edit finishes the job
    let fx = fx.reopen().await?;
    add(&fx, "D", Place::After(named(&fx, "A"))).await?;
    assert_eq!(order(&fx).await?, ["A", "D", "B", "C"]);
    let stored = rows(&fx).await?;
    assert_eq!(stored.len(), 4);
    assert!(stored.iter().all(|row| row.rank.len() <= 3), "{stored:?}");
    Ok(())
}

/// Moving or taking off a node that isn't queued, or placing one beside such a node, is
/// refused, and writes nothing
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn edits_of_entries_not_queued_are_refused<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    add(&fx, "A", Place::Bottom).await?;
    let before = rows(&fx).await?;
    // a move or a removal of X, which isn't queued
    let not_x = Queued::NotQueued {
        node: named(&fx, "X"),
    };
    let edit = QueueEdit::Move {
        node: named(&fx, "X"),
        at: Place::Top,
    };
    assert_eq!(
        fx.store.edit_queue("reef", edit, &reef_api::NOBODY).await?,
        not_x
    );
    let edit = QueueEdit::Remove {
        node: named(&fx, "X"),
    };
    assert_eq!(
        fx.store.edit_queue("reef", edit, &reef_api::NOBODY).await?,
        not_x
    );
    // and B placed beside X, or beside itself while not queued
    assert_eq!(add(&fx, "B", Place::Before(named(&fx, "X"))).await?, not_x);
    assert_eq!(
        add(&fx, "B", Place::After(named(&fx, "B"))).await?,
        Queued::NotQueued {
            node: named(&fx, "B")
        }
    );
    // nothing was written
    assert_eq!(rows(&fx).await?, before);
    // while A placed beside itself, or where it is, is applied and writes nothing either
    add(&fx, "A", Place::Before(named(&fx, "A"))).await?;
    add(&fx, "A", Place::Top).await?;
    assert_eq!(rows(&fx).await?, before);
    Ok(())
}

/// Two edits at once both land, each at a rank of its own
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn concurrent_edits_both_land<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // both at the bottom, at once
    let (a, b) = tokio::join!(add(&fx, "A", Place::Bottom), add(&fx, "B", Place::Bottom));
    assert!(matches!(a?, Queued::Applied { .. }));
    assert!(matches!(b?, Queued::Applied { .. }));
    // both queued, under ranks of their own
    let stored = rows(&fx).await?;
    assert_eq!(stored.len(), 2);
    assert_ne!(stored[0].rank, stored[1].rank);
    let mut ids = order(&fx).await?;
    ids.sort();
    assert_eq!(ids, ["A", "B"]);
    Ok(())
}

/// Each project has a queue of its own, and an edit names only its own project's nodes
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn each_project_has_its_own_queue<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a second project, of a repo of its own
    let repo = git::repo(&fx.dir.join("app"))?;
    let AddedProject::Added(_) = fx.store.add_project("app", &repo).await? else {
        panic!("a project for a fresh repo was refused");
    };
    let app: NodeName = "app/main/A".parse()?;
    // a node queued in each
    add(&fx, "A", Place::Bottom).await?;
    let edit = QueueEdit::Add {
        node: app.clone(),
        at: Place::Bottom,
    };
    fx.store.edit_queue("app", edit, &reef_api::NOBODY).await?;
    // each reads only its own
    assert_eq!(order(&fx).await?, ["A"]);
    let theirs = fx.store.queue("app").await?;
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].node, app);
    // and a node of one can't be queued in the other's
    let edit = QueueEdit::Add {
        node: app,
        at: Place::Bottom,
    };
    let error = fx
        .store
        .edit_queue("reef", edit, &reef_api::NOBODY)
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<Unkeepable>().is_some(), "{error:#}");
    Ok(())
}

/// A queue row is deleted only while it is the row read: another row under its rank is moved,
/// and no row there is missing, as shoal judges `if_matches`
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn queue_deletes_only_the_row_read<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // A's row, written once
    plant(&fx, "A", "a0", 1).await?;
    let stored = rows(&fx).await?.remove(0);
    // a delete naming another write of it, or another node, is refused as moved
    let backend = fx.store.backend();
    let later = QueueRow {
        seq: 2,
        ..stored.clone()
    };
    assert_eq!(
        backend.delete_queue(&later).await?,
        Written::Refused(Refusal::Moved)
    );
    let other = QueueRow {
        id: "B".to_string(),
        ..stored.clone()
    };
    assert_eq!(
        backend.delete_queue(&other).await?,
        Written::Refused(Refusal::Moved)
    );
    // one of a rank holding nothing as missing
    let elsewhere = QueueRow {
        rank: "a1".to_string(),
        ..stored.clone()
    };
    assert_eq!(
        backend.delete_queue(&elsewhere).await?,
        Written::Refused(Refusal::Missing)
    );
    // and the row as read is deleted, after which it is missing too
    assert_eq!(backend.delete_queue(&stored).await?, Written::Applied);
    assert!(rows(&fx).await?.is_empty());
    assert_eq!(
        backend.delete_queue(&stored).await?,
        Written::Refused(Refusal::Missing)
    );
    // while a second insert under a taken rank is refused as existing
    plant(&fx, "A", "a0", 1).await?;
    assert_eq!(
        backend.insert_queue(stored).await?,
        Written::Refused(Refusal::Exists)
    );
    Ok(())
}
