//! The store suite's cases, each generic over the backend so it runs against shoal and the twin

use reef_model::{NodeSpec, PlanName};
use reef_service::{Backend, NotListed, Put, Refusal};

use super::{Fx, node};

/// A put of a new node is written at version 1, and a get reads back what was put
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn put_then_get<B: Backend>(fx: &Fx<B>) -> anyhow::Result<()> {
    // a new node goes in at version 1
    let written = node("C7", 3);
    let put = fx.store.put(&fx.plan, written.clone(), None).await?;
    assert_eq!(put, Put::Applied { version: 1 });
    // and comes back as it was written, at that version
    let got = fx
        .store
        .get(&fx.plan, "C7")
        .await?
        .expect("the node was put");
    assert_eq!(got.spec, written.spec);
    assert_eq!(got.body, written.body);
    assert_eq!(got.order, 3);
    assert_eq!(got.version, 1);
    Ok(())
}

/// A get of a node that was never put is none, not a failure
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn get_of_missing_node_is_none<B: Backend>(fx: &Fx<B>) -> anyhow::Result<()> {
    // in an empty revision, and in one holding another node
    assert_eq!(fx.store.get(&fx.plan, "C7").await?, None);
    fx.store.put(&fx.plan, node("C6", 0), None).await?;
    assert_eq!(fx.store.get(&fx.plan, "C7").await?, None);
    Ok(())
}

/// A plan just listed lists no nodes
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn list_of_new_plan_is_empty<B: Backend>(fx: &Fx<B>) -> anyhow::Result<()> {
    // its revision is current, and holds nothing yet
    let listing = fx.store.list(&fx.plan).await?;
    assert_ne!(listing.revision, 0);
    assert!(listing.nodes.is_empty(), "{listing:?}");
    Ok(())
}

/// A plan lists in the order it was written in, not the id order shoal sorts by
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn list_is_in_written_order<B: Backend>(fx: &Fx<B>) -> anyhow::Result<()> {
    // three nodes, whose ids sort the other way to their order
    for (id, order) in [("A1", 2), ("C3", 0), ("B2", 1)] {
        fx.store.put(&fx.plan, node(id, order), None).await?;
    }
    // list in written order
    let listing = fx.store.list(&fx.plan).await?;
    let ids: Vec<&str> = listing
        .nodes
        .iter()
        .map(|node| node.spec.id.as_str())
        .collect();
    assert_eq!(ids, ["C3", "B2", "A1"]);
    // and make records named in their plan, for the graph
    let records = listing.records(&fx.plan)?;
    assert_eq!(records[0].origin.to_string(), "reef/central/C3");
    Ok(())
}

/// A put that names a version the node has moved on from is refused, and changes nothing
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn stale_put_is_refused<B: Backend>(fx: &Fx<B>) -> anyhow::Result<()> {
    // a node at version 1, then edited from it to version 2
    fx.store.put(&fx.plan, node("C7", 0), None).await?;
    let mut first = node("C7", 0);
    first.spec.title = Some("The first edit".to_string());
    assert_eq!(
        fx.store.put(&fx.plan, first.clone(), Some(1)).await?,
        Put::Applied { version: 2 }
    );
    // a second edit from version 1 is stale
    let mut second = node("C7", 0);
    second.spec.title = Some("The second edit".to_string());
    assert_eq!(
        fx.store.put(&fx.plan, second, Some(1)).await?,
        Put::Stale(Refusal::Moved)
    );
    // and the first edit stands
    let got = fx
        .store
        .get(&fx.plan, "C7")
        .await?
        .expect("the node was put");
    assert_eq!(got.version, 2);
    assert_eq!(got.spec, first.spec);
    Ok(())
}

/// A put of a new node where one already is is refused
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn new_node_over_existing_is_refused<B: Backend>(fx: &Fx<B>) -> anyhow::Result<()> {
    // the first is written and the second, expecting no node, is refused
    fx.store.put(&fx.plan, node("C7", 0), None).await?;
    assert_eq!(
        fx.store.put(&fx.plan, node("C7", 1), None).await?,
        Put::Stale(Refusal::Exists)
    );
    // and the first stands
    let got = fx
        .store
        .get(&fx.plan, "C7")
        .await?
        .expect("the node was put");
    assert_eq!((got.version, got.order), (1, 0));
    Ok(())
}

/// A put that names a version of a node that doesn't exist is refused, and writes nothing
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn put_naming_missing_node_is_refused<B: Backend>(fx: &Fx<B>) -> anyhow::Result<()> {
    // an edit of a node no one put
    let put = fx.store.put(&fx.plan, node("C7", 0), Some(1)).await?;
    assert_eq!(put, Put::Stale(Refusal::Missing));
    assert_eq!(fx.store.get(&fx.plan, "C7").await?, None);
    Ok(())
}

/// Two puts made at once from the same version: one is written, the other refused
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn concurrent_puts_from_one_version_one_wins<B: Backend>(
    fx: &Fx<B>,
) -> anyhow::Result<()> {
    // a node at version 1, and two writers who both read it there
    fx.store.put(&fx.plan, node("C7", 0), None).await?;
    let mut left = node("C7", 0);
    left.spec.title = Some("Left".to_string());
    let mut right = node("C7", 0);
    right.spec.title = Some("Right".to_string());
    // both put at once
    let (a, b) = tokio::join!(
        fx.store.put(&fx.plan, left.clone(), Some(1)),
        fx.store.put(&fx.plan, right.clone(), Some(1))
    );
    let outcomes = [a?, b?];
    // exactly one is written, at version 2, and the other is refused as stale
    let applied = outcomes
        .iter()
        .filter(|put| **put == Put::Applied { version: 2 })
        .count();
    let stale = outcomes
        .iter()
        .filter(|put| **put == Put::Stale(Refusal::Moved))
        .count();
    assert_eq!((applied, stale), (1, 1), "{outcomes:?}");
    // and the node holds the winner's edit
    let got = fx
        .store
        .get(&fx.plan, "C7")
        .await?
        .expect("the node was put");
    let winner = if outcomes[0] == (Put::Applied { version: 2 }) {
        left
    } else {
        right
    };
    assert_eq!(got.spec, winner.spec);
    Ok(())
}

/// A plan that was never listed is an error a caller can tell apart, not an empty plan
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn put_to_unlisted_plan_is_an_error<B: Backend>(fx: &Fx<B>) -> anyhow::Result<()> {
    // a plan in the same project that no one listed
    let unlisted = PlanName::new("reef", "unlisted")?;
    for error in [
        fx.store
            .put(&unlisted, node("C7", 0), None)
            .await
            .expect_err("a put was written"),
        fx.store
            .get(&unlisted, "C7")
            .await
            .expect_err("a get answered"),
        fx.store.list(&unlisted).await.expect_err("a list answered"),
    ] {
        // each says which plan
        let not_listed = error
            .downcast_ref::<NotListed>()
            .expect("the error is NotListed");
        assert_eq!(not_listed.0, unlisted);
    }
    Ok(())
}

/// A node that lists members inline is refused: each member is a node of its own
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn nested_members_are_refused<B: Backend>(fx: &Fx<B>) -> anyhow::Result<()> {
    // a group with a member written inside it
    let mut group = node("M2", 0);
    group.spec.nodes = vec![NodeSpec {
        id: "C7".to_string(),
        ..NodeSpec::default()
    }];
    let error = fx
        .store
        .put(&fx.plan, group, None)
        .await
        .expect_err("a nested node was put");
    assert!(format!("{error:#}").contains("inline"), "{error:#}");
    // and nothing was written
    assert!(fx.store.list(&fx.plan).await?.nodes.is_empty());
    Ok(())
}

/// Listing a plan already listed is refused, and leaves its nodes as they were
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn duplicate_plan_is_refused<B: Backend>(fx: &Fx<B>) -> anyhow::Result<()> {
    // a node in the plan the fixture listed
    fx.store.put(&fx.plan, node("C7", 0), None).await?;
    let before = fx.store.list(&fx.plan).await?;
    // listing it again is refused
    assert!(!fx.store.add_plan(&fx.plan).await?);
    // and its revision and nodes are untouched
    assert_eq!(fx.store.list(&fx.plan).await?, before);
    Ok(())
}
