//! The store suite's decision cases, each generic over the backend so it runs against shoal and
//! the twin

use reef_api::{Decided, DecisionKind, Outcome, Via};

use super::{Fx, Reopen};

/// A plan asked for approval in a session, answered or not
///
/// # Arguments
///
/// * `call` - The tool call's id
/// * `outcome` - What came of it, if anything yet
fn plan(call: &str, outcome: Option<Outcome>) -> Decided {
    Decided {
        call: call.to_string(),
        session: "s1".to_string(),
        via: Via::Terminal,
        asked_at: 1_000,
        answered_at: outcome.as_ref().map(|_| 2_000),
        kind: DecisionKind::Plan {
            plan: format!("# Plan {call}"),
            html: String::new(),
        },
        outcome,
        author: reef_api::Author::default(),
    }
}

/// A node's decisions are kept by their calls, numbered in turn: one read again is kept once, an
/// answer read later fills it in and moves its version, a read that missed the answer never
/// takes it away, and all of it reads back once the store is reopened, numbering carrying on
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn decisions_are_kept_by_call<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let node = fx.plan.node("C75")?;
    // a node no session decided on has none
    assert!(fx.store.decisions(&node).await?.is_empty());
    // two plans, the first rejected, the second not answered yet
    let rejected = Some(Outcome::Rejected {
        feedback: "Name the test.".to_string(),
    });
    let first = vec![plan("p1", rejected.clone()), plan("p2", None)];
    assert_eq!(
        fx.store.record_decisions(&node, first.clone()).await?,
        (2, 0)
    );
    // the same read again keeps nothing more
    assert_eq!(fx.store.record_decisions(&node, first).await?, (0, 0));
    // the second answered since, and a read that missed the first's answer
    let later = vec![plan("p1", None), plan("p2", Some(Outcome::Approved))];
    assert_eq!(fx.store.record_decisions(&node, later).await?, (0, 1));
    let kept = fx.store.decisions(&node).await?;
    assert_eq!(kept.len(), 2);
    assert_eq!((kept[0].number, kept[0].version), (1, 1));
    assert_eq!(kept[0].decided.outcome, rejected);
    assert_eq!((kept[1].number, kept[1].version), (2, 2));
    assert_eq!(kept[1].decided.outcome, Some(Outcome::Approved));
    assert_eq!(kept[1].decided.answered_at, Some(2_000));
    // all of it reads back once the store is reopened, and the next is numbered after it
    let fx = fx.reopen().await?;
    assert_eq!(fx.store.decisions(&node).await?, kept);
    assert_eq!(
        fx.store
            .record_decisions(&node, vec![plan("p3", None)])
            .await?,
        (1, 0)
    );
    assert_eq!(fx.store.decisions(&node).await?[2].number, 3);
    // and another node's are its own
    assert!(fx.store.decisions(&fx.plan.node("C76")?).await?.is_empty());
    Ok(())
}
