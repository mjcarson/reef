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
        width: None,
        opens: None,
        pid: None,
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
        width: None,
        opens: None,
        pid: None,
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
        width: None,
        opens: None,
        pid: None,
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
        members: Vec::new(),
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
        members: Vec::new(),
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
    edited(
        &fx,
        &holder,
        LoopEdit::Hold {
            node: None,
            members: Vec::new(),
        },
    )
    .await?;
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
        members: Vec::new(),
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
        pid: None,
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

/// A loop acts on each node's end once: the node whose end it acted on is recorded, a second
/// record of the same node is refused under the lock without a write, and the next node's end
/// is recorded over it
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_loop_acts_on_a_nodes_end_once<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a loop that acted on C39's end
    let here = pane(1);
    edited(&fx, &here, start(&fx, "s1")).await?;
    let acted = LoopEdit::Acted {
        node: node_of(&fx, "C39"),
    };
    let once = edited(&fx, &here, acted.clone()).await?;
    assert_eq!(once.acted, Some(node_of(&fx, "C39")));
    // acting on it again writes nothing, and says so with the loop as it stands
    assert_eq!(
        fx.store.edit_loop(&here, acted).await?,
        LoopEdited::ActedAlready {
            node: node_of(&fx, "C39"),
            r#loop: once.clone()
        }
    );
    assert_eq!(fx.store.loop_in(&here).await?, Some(once.clone()));
    // and the next node's end is recorded over it
    let next = LoopEdit::Acted {
        node: node_of(&fx, "C40"),
    };
    let moved = edited(&fx, &here, next).await?;
    assert_eq!(moved.acted, Some(node_of(&fx, "C40")));
    assert_eq!(moved.version, once.version + 1);
    Ok(())
}

/// Holding again the node whose end a loop acted on, for a round of its comments, clears that
/// end, and holding another keeps it
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn holding_the_acted_node_again_clears_its_end<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // a loop that held C39 and acted on its end
    let here = pane(1);
    edited(&fx, &here, start(&fx, "s1")).await?;
    let hold = |id: &str| LoopEdit::Hold {
        node: Some(node_of(&fx, id)),
        members: Vec::new(),
    };
    edited(&fx, &here, hold("C39")).await?;
    let acted = LoopEdit::Acted {
        node: node_of(&fx, "C39"),
    };
    edited(&fx, &here, acted.clone()).await?;
    // holding another node keeps the end it acted on
    let other = edited(&fx, &here, hold("C40")).await?;
    assert_eq!(other.acted, Some(node_of(&fx, "C39")));
    // holding C39 again, as a round of its comments does, clears it
    let round = edited(&fx, &here, hold("C39")).await?;
    assert_eq!(round.node, Some(node_of(&fx, "C39")));
    assert_eq!(round.acted, None);
    assert_eq!(fx.store.loop_in(&here).await?, Some(round));
    // so the round's end is acted on, once
    let ended = edited(&fx, &here, acted.clone()).await?;
    assert_eq!(ended.acted, Some(node_of(&fx, "C39")));
    assert!(matches!(
        fx.store.edit_loop(&here, acted).await?,
        LoopEdited::ActedAlready { .. }
    ));
    Ok(())
}

/// A start of a loop feeding the case's plan, as wide as named
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `width` - How many nodes it works at once
fn wide<B>(fx: &Fx<B>, width: u8) -> LoopEdit {
    LoopEdit::Start {
        project: fx.plan.project.clone(),
        plan: Some(fx.plan.plan.clone()),
        session: Some("s1".to_string()),
        width: Some(width),
        opens: Some(false),
        pid: Some(4242),
    }
}

/// A hold of a node and its group's members, by id in the case's plan
///
/// # Arguments
///
/// * `fx` - The store and its plan
/// * `ids` - The loop's own node's id, then each member's
fn group<B>(fx: &Fx<B>, ids: &[&str]) -> LoopEdit {
    LoopEdit::Hold {
        node: Some(node_of(fx, ids[0])),
        members: ids[1..].iter().map(|id| node_of(fx, id)).collect(),
    }
}

/// A loop holds its group as one: as wide as it is and no wider, none of it taken by another
/// loop, a member another loop holds refused whole, its next group in place of the last, and
/// everything let go of by a stop
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_loop_holds_its_group_as_one<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a loop two wide, opening no panes, its process known, and one a node wide
    let (two, one) = (pane(1), pane(2));
    let started = edited(&fx, &two, wide(&fx, 2)).await?;
    assert_eq!(
        (started.width, started.opens, started.pid),
        (2, false, Some(4242))
    );
    edited(&fx, &one, start(&fx, "s2")).await?;
    // it holds C39 with C40 beside it, neither joined yet
    let held = edited(&fx, &two, group(&fx, &["C39", "C40"])).await?;
    assert_eq!(held.node, Some(node_of(&fx, "C39")));
    let members: Vec<(NodeName, Option<Pane>)> = held
        .members
        .iter()
        .map(|member| (member.node.clone(), member.pane.clone()))
        .collect();
    assert_eq!(members, vec![(node_of(&fx, "C40"), None)]);
    // a group wider than the loop is refused, and writes nothing
    assert!(matches!(
        fx.store
            .edit_loop(&two, group(&fx, &["C41", "C42", "C43"]))
            .await?,
        LoopEdited::TooWide { named: 3, .. }
    ));
    assert_eq!(fx.store.loop_in(&two).await?, Some(held.clone()));
    // its member is refused to another loop, naming the loop that holds it
    assert_eq!(
        fx.store.edit_loop(&one, group(&fx, &["C40"])).await?,
        LoopEdited::Held {
            node: node_of(&fx, "C40"),
            by: held.clone()
        }
    );
    // and a group with a member another loop holds is refused whole
    edited(&fx, &one, group(&fx, &["C41"])).await?;
    assert!(matches!(
        fx.store.edit_loop(&two, group(&fx, &["C42", "C41"])).await?,
        LoopEdited::Held { node, .. } if node == node_of(&fx, "C41")
    ));
    assert_eq!(fx.store.loop_in(&two).await?, Some(held));
    // its next group replaces the last, members and all
    let next = edited(&fx, &two, group(&fx, &["C42", "C43"])).await?;
    let ids: Vec<&str> = next.group().map(|node| node.id.as_str()).collect();
    assert_eq!(ids, ["C42", "C43"]);
    // and the width, the process and the members read back once the store is reopened
    let fx = fx.reopen().await?;
    assert_eq!(fx.store.loop_in(&two).await?, Some(next));
    // a stop lets go of all of it, for the other loop to take
    edited(&fx, &two, LoopEdit::Stop).await?;
    edited(&fx, &one, group(&fx, &["C43"])).await?;
    Ok(())
}

/// A pane joins a group's member: once, from one pane, never from a loop's own pane or to a
/// node that is no member, moving off any member it joined before, and a pane working a member
/// starts no loop of its own
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_pane_joins_a_member<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a loop three wide holding C39, with C40 and C41 beside it
    let (lead, member, other) = (pane(1), pane(5), pane(6));
    edited(&fx, &lead, wide(&fx, 3)).await?;
    edited(&fx, &lead, group(&fx, &["C39", "C40", "C41"])).await?;
    let plan = fx.plan.clone();
    let join = |id: &str| LoopEdit::Join {
        node: plan.node(id).expect("a node's id"),
    };
    let joined_by = |feeding: &Loop, id: &str| {
        feeding
            .member(&plan.node(id).expect("a node's id"))
            .and_then(|member| member.pane.clone())
    };
    // a pane with no loop joins C40, and joining it again changes nothing
    let LoopEdited::Joined { r#loop: joined } = fx.store.edit_loop(&member, join("C40")).await?
    else {
        panic!("the pane didn't join C40");
    };
    assert_eq!(joined.pane, lead);
    assert_eq!(joined_by(&joined, "C40"), Some(member.clone()));
    assert_eq!(
        fx.store.edit_loop(&member, join("C40")).await?,
        LoopEdited::Joined {
            r#loop: joined.clone()
        }
    );
    // another pane is refused it, naming the loop that holds it
    assert!(matches!(
        fx.store.edit_loop(&other, join("C40")).await?,
        LoopEdited::Held { node, .. } if node == node_of(&fx, "C40")
    ));
    // the loop's own node, or one no loop holds, is no member to join
    for id in ["C39", "C42"] {
        assert_eq!(
            fx.store.edit_loop(&other, join(id)).await?,
            LoopEdited::NotMember {
                node: node_of(&fx, id)
            }
        );
    }
    // the loop's own pane works its own node, not a member
    assert!(matches!(
        fx.store.edit_loop(&lead, join("C41")).await?,
        LoopEdited::Busy { r#loop } if r#loop.pane == lead
    ));
    // a pane working a member starts no loop of its own
    assert!(matches!(
        fx.store.edit_loop(&member, start(&fx, "s9")).await?,
        LoopEdited::Busy { r#loop } if r#loop.pane == lead
    ));
    assert_eq!(fx.store.loop_in(&member).await?, None);
    // joining C41, it leaves C40 for another session
    let LoopEdited::Joined { r#loop: moved } = fx.store.edit_loop(&member, join("C41")).await?
    else {
        panic!("the pane didn't join C41");
    };
    assert_eq!(joined_by(&moved, "C41"), Some(member.clone()));
    assert_eq!(joined_by(&moved, "C40"), None);
    assert!(matches!(
        fx.store.edit_loop(&other, join("C40")).await?,
        LoopEdited::Joined { .. }
    ));
    // and who joined what reads back once the store is reopened
    let fx = fx.reopen().await?;
    let read = fx.store.loop_in(&lead).await?.expect("the loop reads back");
    assert_eq!(joined_by(&read, "C40"), Some(other));
    assert_eq!(joined_by(&read, "C41"), Some(member));
    Ok(())
}
