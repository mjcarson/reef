//! The store suite's move cases, each generic over the backend so it runs against shoal and the
//! twin

use reef_api::{HistoryQuery, NOBODY, NodePut, Place, Puts, QueueEdit};
use reef_model::{Diagnostic, PlanName};
use reef_service::store::{MoveProposal, MoveSet};

use super::{Fx, Reopen, node};

/// A move's check that finds no problem, for the cases that judge nothing
///
/// # Arguments
///
/// * `_proposal` - Both plans as they stand and as the move would leave them
async fn no_problems(_proposal: MoveProposal) -> anyhow::Result<Vec<Diagnostic>> {
    Ok(Vec::new())
}

/// A move killed once its node joined the other plan, before what it left behind was deleted,
/// leaves the node in both plans, and running it again finishes it: the node in the other plan
/// alone, its queue entry there, and its history saying it moved
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_move_killed_partway_is_finished_by_running_it_again<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // `central`'s A, put with a history and queued, and `side` beside it
    let side = PlanName::new("reef", "side")?;
    fx.store.add_plan(&side, &NOBODY).await?;
    let puts = Puts::new(vec![NodePut::new(node("A", 0), None)]);
    fx.store
        .batch_checked(&fx.plan, puts, &NOBODY, |_| async { Ok(Vec::new()) })
        .await?;
    let queue = QueueEdit::Add {
        node: fx.plan.node("A")?,
        at: Place::Bottom,
    };
    fx.store.edit_queue("reef", queue, &NOBODY).await?;
    // killed once it joined `side`, before it left `central`
    fx.faults.fail_next_row_delete();
    let ids = ["A".to_string()];
    let killed = fx
        .store
        .move_nodes(&fx.plan, &side, &ids, None, &NOBODY, no_problems)
        .await;
    assert!(killed.is_err(), "{killed:?}");
    assert!(fx.store.get(&fx.plan, "A").await?.is_some());
    assert!(fx.store.get(&side, "A").await?.is_some());
    // run again, it finishes, rather than finding A taken
    let finished = fx
        .store
        .move_nodes(&fx.plan, &side, &ids, None, &NOBODY, no_problems)
        .await?;
    assert!(
        matches!(&finished, MoveSet::Applied { nodes, queued: true, .. } if nodes == &ids),
        "{finished:?}"
    );
    assert!(fx.store.get(&fx.plan, "A").await?.is_none());
    assert!(fx.store.get(&side, "A").await?.is_some());
    let queued: Vec<String> = fx
        .store
        .queue("reef")
        .await?
        .into_iter()
        .map(|entry| entry.node.to_string())
        .collect();
    assert_eq!(queued, ["reef/side/A"]);
    let history = fx
        .store
        .node_history(&side.node("A")?, &HistoryQuery::default())
        .await?;
    let told: Vec<String> = history
        .edits
        .iter()
        .map(|logged| logged.describe())
        .collect();
    assert_eq!(told, ["moved from plan `central` to `side`", "added"]);
    Ok(())
}
