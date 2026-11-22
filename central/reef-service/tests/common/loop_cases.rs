//! The store suite's loop cases, each generic over the backend so it runs against shoal and the
//! twin

use reef_api::{Loop, LoopEdit, LoopEdited, LoopState, Pane};
use reef_model::NodeName;
use reef_service::Backend;

use super::{Fx, Reopen};

/// A pane kept by a Claude Code process, as a session with no multiplexer runs in
///
/// # Arguments
///
/// * `pid` - The process's id
fn pane(pid: u32) -> Pane {
    Pane::Claude { pid }
}

/// A node of the case's plan
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `id` - The node's id
fn node_of<B>(fx: &Fx<B>, id: &str) -> NodeName {
    fx.plan.node(id).expect("a node's id")
}

/// A start of a loop feeding the case's plan, fed by a session
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `session` - The session's id
fn start<B>(fx: &Fx<B>, session: &str) -> LoopEdit {
    LoopEdit::Start {
        project: fx.plan.project.clone(),
        plan: Some(fx.plan.plan.clone()),
        session: Some(session.to_string()),
    }
}

/// Edits a pane's loop, which must be written, and hands back the loop as it now stands
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `pane` - The pane
/// * `edit` - The change
async fn edited<B: Backend>(fx: &Fx<B>, pane: &Pane, edit: LoopEdit) -> anyhow::Result<Loop> {
    match fx.store.edit_loop(pane, edit).await? {
        LoopEdited::Started { r#loop }
        | LoopEdited::Resumed { r#loop }
        | LoopEdited::Edited { r#loop }
        | LoopEdited::Stopped { r#loop } => Ok(r#loop),
        other => panic!("the loop in {pane} wasn't written: {other:?}"),
    }
}

/// A pane keeps one loop: a start where one of the same plan is runs it again, one of another
/// plan is busy, a pause and a stop change it, an edit with no loop finds none, and a loop reads
/// back once the store is reopened
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_pane_keeps_one_loop<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // no loop yet, anywhere
    let here = pane(1);
    assert!(fx.store.loops().await?.is_empty());
    assert_eq!(fx.store.loop_in(&here).await?, None);
    // started, running and holding nothing, at version 1
    let LoopEdited::Started { r#loop: started } =
        fx.store.edit_loop(&here, start(&fx, "s1")).await?
    else {
        panic!("no loop was started");
    };
    assert_eq!(
        (started.state, started.version, &started.node),
        (LoopState::Running, 1, &None)
    );
    assert_eq!(started.session.as_deref(), Some("s1"));
    assert_eq!(fx.store.loop_in(&here).await?, Some(started.clone()));
    // a start of another plan of the project is busy, and changes nothing
    let other = LoopEdit::Start {
        project: fx.plan.project.clone(),
        plan: None,
        session: None,
    };
    let busy = fx.store.edit_loop(&here, other).await?;
    assert_eq!(
        busy,
        LoopEdited::Busy {
            r#loop: started.clone()
        }
    );
    // paused, then started again, it runs, keeping the session it had
    let paused = edited(&fx, &here, LoopEdit::Pause).await?;
    assert_eq!((paused.state, paused.version), (LoopState::Paused, 2));
    let again = LoopEdit::Start {
        project: fx.plan.project.clone(),
        plan: Some(fx.plan.plan.clone()),
        session: None,
    };
    let LoopEdited::Resumed { r#loop: resumed } = fx.store.edit_loop(&here, again).await? else {
        panic!("the paused loop wasn't resumed");
    };
    assert_eq!((resumed.state, resumed.version), (LoopState::Running, 3));
    assert_eq!(resumed.session.as_deref(), Some("s1"));
    assert_eq!(resumed.started_at, started.started_at);
    // another pane keeps a loop of its own
    let there = pane(2);
    edited(&fx, &there, start(&fx, "s2")).await?;
    let panes: Vec<Pane> = fx
        .store
        .loops()
        .await?
        .into_iter()
        .map(|feeding| feeding.pane)
        .collect();
    assert_eq!(panes, [here.clone(), there.clone()]);
    // and every loop reads back the same once the store is reopened
    let before = fx.store.loops().await?;
    let fx = fx.reopen().await?;
    assert_eq!(fx.store.loops().await?, before);
    // stopped, the pane has no loop, and an edit of it finds none
    let stopped = edited(&fx, &here, LoopEdit::Stop).await?;
    assert_eq!(stopped, resumed);
    assert_eq!(fx.store.loop_in(&here).await?, None);
    for edit in [LoopEdit::Pause, LoopEdit::Stop] {
        let none = fx.store.edit_loop(&here, edit).await?;
        assert_eq!(none, LoopEdited::NoLoop { pane: here.clone() });
    }
    assert_eq!(fx.store.loops().await?.len(), 1);
    Ok(())
}

/// Two sessions never take the same node: a node one loop holds is refused to another, two
/// loops taking one node at once are taken one at a time, a loop holds one node at a time, and
/// a node let go of, or a loop stopped, frees it
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn two_loops_never_hold_one_node<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a loop in each of two panes
    let (one, two) = (pane(1), pane(2));
    edited(&fx, &one, start(&fx, "s1")).await?;
    edited(&fx, &two, start(&fx, "s2")).await?;
    let hold = |id: &str| LoopEdit::Hold {
        node: Some(node_of(&fx, id)),
    };
    // the first takes C39, and the second is refused it, naming the first
    let held = edited(&fx, &one, hold("C39")).await?;
    assert_eq!(held.node, Some(node_of(&fx, "C39")));
    let refused = fx.store.edit_loop(&two, hold("C39")).await?;
    assert_eq!(
        refused,
        LoopEdited::Held {
            node: node_of(&fx, "C39"),
            by: held.clone()
        }
    );
    // the first taking C39 again still holds it
    assert_eq!(edited(&fx, &one, hold("C39")).await?.node, held.node);
    // both taking C40 at once: one takes it, and the other is refused
    let (first, second) = tokio::join!(
        fx.store.edit_loop(&one, hold("C40")),
        fx.store.edit_loop(&two, hold("C40")),
    );
    let outcomes = [first?, second?];
    let took = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, LoopEdited::Edited { .. }))
        .count();
    let refused = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, LoopEdited::Held { .. }))
        .count();
    assert_eq!((took, refused), (1, 1), "{outcomes:?}");
    let holders: Vec<Pane> = fx
        .store
        .loops()
        .await?
        .into_iter()
        .filter(|feeding| feeding.node == Some(node_of(&fx, "C40")))
        .map(|feeding| feeding.pane)
        .collect();
    assert_eq!(holders.len(), 1);
    // a node of another plan is none of a loop's that feeds this one
    let elsewhere: NodeName = format!("{}/other/C40", fx.plan.project).parse()?;
    let outside = LoopEdit::Hold {
        node: Some(elsewhere.clone()),
    };
    assert!(matches!(
        fx.store.edit_loop(&one, outside).await?,
        LoopEdited::NotInLoop { node, .. } if node == elsewhere
    ));
    // the holder of C40 lets go of it, and the other takes it
    let (holder, other) = match holders[0] == one {
        true => (one.clone(), two.clone()),
        false => (two.clone(), one.clone()),
    };
    edited(&fx, &holder, LoopEdit::Hold { node: None }).await?;
    assert_eq!(
        edited(&fx, &other, hold("C40")).await?.node,
        Some(node_of(&fx, "C40"))
    );
    // a loop holds one node at a time, so taking C40 let go of anything else it held
    let loops = fx.store.loops().await?;
    let on: Vec<Option<NodeName>> = loops.iter().map(|feeding| feeding.node.clone()).collect();
    assert!(on.contains(&Some(node_of(&fx, "C40"))), "{on:?}");
    assert!(on.contains(&None), "{on:?}");
    // and a loop stopped frees what it held, for the other to take
    edited(&fx, &other, LoopEdit::Stop).await?;
    assert_eq!(
        edited(&fx, &holder, hold("C40")).await?.node,
        Some(node_of(&fx, "C40"))
    );
    Ok(())
}

/// A clear's new session takes its pane's loop over: its node, the node it acted on and its
/// pause kept, its session the new one, and a session in a pane with no loop takes none
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_clears_session_takes_its_panes_loop<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a paused loop holding C39, which acted on C37's end, fed by the session before the clear
    let here = pane(1);
    edited(&fx, &here, start(&fx, "before")).await?;
    let hold = LoopEdit::Hold {
        node: Some(node_of(&fx, "C39")),
    };
    edited(&fx, &here, hold).await?;
    let acted = LoopEdit::Acted {
        node: node_of(&fx, "C37"),
    };
    edited(&fx, &here, acted).await?;
    let cleared = edited(&fx, &here, LoopEdit::Pause).await?;
    // the clear's new session takes it, keeping everything it held
    let session = LoopEdit::Session {
        session: "after".to_string(),
    };
    let handed = edited(&fx, &here, session.clone()).await?;
    assert_eq!(handed.session.as_deref(), Some("after"));
    assert_eq!(
        Loop {
            session: cleared.session.clone(),
            version: cleared.version,
            ..handed.clone()
        },
        cleared
    );
    assert_eq!(handed.version, cleared.version + 1);
    assert_eq!(fx.store.loop_in(&here).await?, Some(handed));
    // and a session starting where no loop is takes none
    let elsewhere = pane(2);
    assert_eq!(
        fx.store.edit_loop(&elsewhere, session).await?,
        LoopEdited::NoLoop { pane: elsewhere }
    );
    Ok(())
}

/// A loop write another writer beat is an error, and leaves the loop as that writer left it
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_beaten_loop_write_is_an_error<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a loop, then a pause another process beat to its row
    let here = pane(1);
    let started = edited(&fx, &here, start(&fx, "s1")).await?;
    fx.faults.refuse_next_loop_write();
    let error = fx
        .store
        .edit_loop(&here, LoopEdit::Pause)
        .await
        .expect_err("a beaten write was taken as written");
    assert!(format!("{error:#}").contains("another writer"), "{error:#}");
    assert_eq!(fx.store.loop_in(&here).await?, Some(started));
    Ok(())
}
