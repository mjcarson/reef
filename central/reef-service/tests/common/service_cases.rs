//! The service suite's cases, each generic over the backend so it runs against shoal and the twin
//!
//! Each case's plan is `reef/central`, of a project registered for a fresh git repo whose working
//! tree holds the plan's docs.

use reef_api::{Change, PutNode, paths};
use reef_model::{Diagnostic, Graph, NodeSpec, PlanName, nodes_from_records};
use reef_render::{ROOT, Renderer, RepoSource, Sections};
use reef_service::{
    Backend, Checked, NodePut, NodeRemove, NodeWrite, NotListed, NotRegistered, Puts, Refusal,
    Rewrite, StalePut,
};
use std::time::Duration;

use super::{DESIGN, Sx, doc, group, member, work};

/// A problem at a node's name, as check gives a stored node's
///
/// # Arguments
///
/// * `id` - The node's id in `reef/central`
/// * `message` - What the problem is
fn problem(id: &str, message: &str) -> Diagnostic {
    Diagnostic {
        source: format!("reef/central/{id}"),
        line: None,
        message: message.to_string(),
    }
}

/// Puts of new nodes, each expecting no node of its id
///
/// # Arguments
///
/// * `nodes` - The nodes to put
fn new(nodes: Vec<NodeWrite>) -> Vec<NodePut> {
    nodes
        .into_iter()
        .map(|node| NodePut::new(node, None))
        .collect()
}

/// A node that needs what it names
///
/// # Arguments
///
/// * `node` - The node
/// * `needs` - The ids it needs
fn needing(mut node: NodeWrite, needs: &[&str]) -> NodeWrite {
    node.spec.needs = needs.iter().map(|need| need.to_string()).collect();
    node
}

/// A node that links to docs and writes no description, so its why and what come from them
///
/// # Arguments
///
/// * `node` - The node
/// * `docs` - Its doc links, from the repo's root
fn linking(mut node: NodeWrite, docs: &[&str]) -> NodeWrite {
    node.spec.docs = docs.iter().map(|link| link.to_string()).collect();
    node.spec.description = None;
    node
}

/// The ids a plan lists, in written order
///
/// # Arguments
///
/// * `sx` - The service and its plan
async fn ids<B: Backend>(sx: &Sx<B>) -> anyhow::Result<Vec<String>> {
    let listing = sx.service.list(&sx.plan).await?;
    Ok(listing.nodes.into_iter().map(|node| node.spec.id).collect())
}

/// A batch with one bad put is refused with that put's problem, and none of it is written
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn refused_batch_writes_none_of_it<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a good node, and one needing a node that isn't there
    let batch = new(vec![work("A", 0), needing(work("B", 1), &["nope"])]);
    let answer = sx.service.put(&sx.plan, batch).await?;
    // only the bad node's problem refuses it
    let expected = vec![problem("B", "needs `nope`, which is not a node")];
    assert_eq!(answer, Checked::Refused(expected));
    // and the good node wasn't written either
    assert!(ids(&sx).await?.is_empty());
    Ok(())
}

/// A plan that already has a problem takes puts that add none, and refuses one that adds one
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn fix_gets_through_on_a_broken_plan<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a node whose why and what come from a doc in the working tree
    doc(&sx.repo, "docs/a.md", DESIGN)?;
    let linked = linking(work("A", 0), &["docs/a.md#rows"]);
    let answer = sx.service.put(&sx.plan, new(vec![linked])).await?;
    assert_eq!(answer, Checked::Applied { versions: vec![1] });
    // the doc's removal breaks the plan: the link, and the why and the what it gave
    std::fs::remove_file(sx.repo.join("docs/a.md"))?;
    let broken = sx.service.check(&sx.plan).await?;
    assert_eq!(broken.len(), 2, "{broken:?}");
    assert!(
        broken
            .iter()
            .all(|problem| problem.source == "reef/central/A"),
        "{broken:?}"
    );
    // a put that adds nothing broken gets through anyway
    let answer = sx.service.put(&sx.plan, new(vec![work("B", 1)])).await?;
    assert_eq!(answer, Checked::Applied { versions: vec![1] });
    // one that adds a problem is refused with its own problem alone, not A's
    let bad = new(vec![needing(work("C", 2), &["nope"])]);
    let answer = sx.service.put(&sx.plan, bad).await?;
    let expected = vec![problem("C", "needs `nope`, which is not a node")];
    assert_eq!(answer, Checked::Refused(expected));
    // and the fix gets through, leaving the plan whole
    let fix = vec![NodePut::new(work("A", 0), Some(1))];
    let answer = sx.service.put(&sx.plan, fix).await?;
    assert_eq!(answer, Checked::Applied { versions: vec![2] });
    assert_eq!(sx.service.check(&sx.plan).await?, Vec::<Diagnostic>::new());
    Ok(())
}

/// A group goes in with its members as one batch, where neither goes in alone
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn batch_adds_a_group_with_its_members<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a group alone is a piece of work that says neither why nor what
    let alone = sx.service.put(&sx.plan, new(vec![group("G", 0)])).await?;
    let Checked::Refused(problems) = alone else {
        panic!("a group with no members was written: {alone:?}");
    };
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0]
            .to_string()
            .starts_with("reef/central/G: `G` does not say why it is needed or what"),
        "{problems:?}"
    );
    // a member alone names a group that isn't a node
    let alone = sx
        .service
        .put(&sx.plan, new(vec![member("m1", "G", 1)]))
        .await?;
    let expected = vec![problem("m1", "group `G` is not a node")];
    assert_eq!(alone, Checked::Refused(expected));
    // together they go in, each at version 1
    let batch = new(vec![
        group("G", 0),
        member("m1", "G", 1),
        member("m2", "G", 2),
    ]);
    let answer = sx.service.put(&sx.plan, batch).await?;
    assert_eq!(
        answer,
        Checked::Applied {
            versions: vec![1, 1, 1]
        }
    );
    // in written order, and with nothing wrong
    assert_eq!(ids(&sx).await?, ["G", "m1", "m2"]);
    assert_eq!(sx.service.check(&sx.plan).await?, Vec::<Diagnostic>::new());
    Ok(())
}

/// A link to a doc or heading the working tree lacks is refused, and one it has is taken from
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn missing_doc_is_refused_and_present_doc_accepted<B: Backend>(
    sx: Sx<B>,
) -> anyhow::Result<()> {
    // a node whose why and what come from a doc the working tree has
    doc(&sx.repo, "docs/design.md", DESIGN)?;
    let linked = linking(work("A", 0), &["docs/design.md#rows"]);
    let answer = sx.service.put(&sx.plan, new(vec![linked])).await?;
    assert_eq!(answer, Checked::Applied { versions: vec![1] });
    // its brief form is taken from the linked section
    let payload = sx.service.payload(&sx.plan).await?;
    let a = &payload.nodes[0];
    assert_eq!(a.why_from.as_deref(), Some("Context"));
    assert_eq!(a.what_from.as_deref(), Some("What it does"));
    // a link to a doc that isn't there is refused with check's message
    let gone = linking(work("B", 1), &["docs/gone.md"]);
    let mut gone_written = gone.clone();
    gone_written.spec.description = work("B", 1).spec.description;
    let answer = sx.service.put(&sx.plan, new(vec![gone_written])).await?;
    let expected = vec![problem(
        "B",
        "doc `docs/gone.md` not found (looked for docs/gone.md)",
    )];
    assert_eq!(answer, Checked::Refused(expected));
    // and so is a link to a heading the doc doesn't have
    let mut nope = work("C", 2);
    nope.spec.docs = vec!["docs/design.md#nope".to_string()];
    let answer = sx.service.put(&sx.plan, new(vec![nope])).await?;
    let expected = vec![problem(
        "C",
        "doc `docs/design.md#nope`: docs/design.md has no heading with the id `nope`",
    )];
    assert_eq!(answer, Checked::Refused(expected));
    // with only the node that links what is there written
    assert_eq!(ids(&sx).await?, ["A"]);
    Ok(())
}

/// A link that leaves the repo is refused, though the file it names is there
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn link_leaving_the_repo_is_refused<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a doc beside the repo, which a link from the repo's root can climb out to
    doc(&sx.dir, "outside.md", DESIGN)?;
    let out = linking(work("A", 0), &["../outside.md#rows"]);
    // refused with check's message, as a link that leaves the repo, and not briefed from it
    let answer = sx.service.put(&sx.plan, new(vec![out])).await?;
    let Checked::Refused(problems) = answer else {
        panic!("a link that leaves the repo was written: {answer:?}");
    };
    assert_eq!(
        problems[0],
        problem("A", "doc `../outside.md#rows` leaves the repo")
    );
    assert!(
        problems[1]
            .message
            .contains("does not say why it is needed"),
        "{problems:?}"
    );
    assert!(ids(&sx).await?.is_empty());
    Ok(())
}

/// A batch with a stale put is refused as stale before it is checked, and writes nothing
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn stale_put_in_a_batch_writes_nothing<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a node at version 1
    sx.service.put(&sx.plan, new(vec![work("A", 0)])).await?;
    // a fresh node beside an edit of it from a version it never had, which would break the plan
    let batch = vec![
        NodePut::new(work("B", 1), None),
        NodePut::new(needing(work("A", 0), &["nope"]), Some(7)),
    ];
    let answer = sx.service.put(&sx.plan, batch).await?;
    let moved = StalePut {
        id: "A".to_string(),
        refusal: Refusal::Moved,
    };
    assert_eq!(answer, Checked::Stale(vec![moved]));
    // a new node over one that exists, and an edit of one that doesn't, are stale too
    let batch = vec![
        NodePut::new(work("A", 0), None),
        NodePut::new(work("Z", 2), Some(1)),
    ];
    let answer = sx.service.put(&sx.plan, batch).await?;
    let stale = vec![
        StalePut {
            id: "A".to_string(),
            refusal: Refusal::Exists,
        },
        StalePut {
            id: "Z".to_string(),
            refusal: Refusal::Missing,
        },
    ];
    assert_eq!(answer, Checked::Stale(stale));
    // and neither batch wrote its fresh node
    assert_eq!(ids(&sx).await?, ["A"]);
    Ok(())
}

/// A batch the store can't keep is an error: a node put twice, or members listed inline
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn batch_it_cant_keep_is_an_error<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // one node put twice in one batch
    let twice = new(vec![work("A", 0), work("A", 1)]);
    let error = sx
        .service
        .put(&sx.plan, twice)
        .await
        .expect_err("a node put twice was written");
    assert!(format!("{error:#}").contains("put twice"), "{error:#}");
    // a group with its members inline, rather than each a node of its own
    let mut inline = group("G", 0);
    inline.spec.nodes = vec![NodeSpec {
        id: "m".to_string(),
        ..NodeSpec::default()
    }];
    let error = sx
        .service
        .put(&sx.plan, new(vec![inline]))
        .await
        .expect_err("inline members were written");
    assert!(format!("{error:#}").contains("inline"), "{error:#}");
    // and nothing was written
    assert!(ids(&sx).await?.is_empty());
    Ok(())
}

/// Removals are judged with their batch: a node another needs can't go alone, nor a group
/// without its members, a stale removal writes nothing, and the batch's change names each node
/// removed
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn removes_are_checked_with_the_batch<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a group of two, and work that needs one of them
    let batch = new(vec![
        group("G", 0),
        member("m1", "G", 1),
        member("m2", "G", 2),
        needing(work("W", 3), &["m1"]),
    ]);
    let applied = sx.service.put(&sx.plan, batch).await?;
    assert_eq!(
        applied,
        Checked::Applied {
            versions: vec![1, 1, 1, 1]
        }
    );
    let mut changes = sx.service.changes();
    // a node another needs can't be removed alone
    let needed = Puts::removing(vec![NodeRemove::new("m1", 1)]);
    let refused = sx.service.batch(&sx.plan, needed).await?;
    let expected = vec![problem("W", "needs `m1`, which is not a node")];
    assert_eq!(refused, Checked::Refused(expected));
    // nor a group without its members
    let alone = Puts::removing(vec![NodeRemove::new("G", 1)]);
    let refused = sx.service.batch(&sx.plan, alone).await?;
    let expected = vec![
        problem("m1", "group `G` is not a node"),
        problem("m2", "group `G` is not a node"),
    ];
    assert_eq!(refused, Checked::Refused(expected));
    // a removal of a node that has moved on is stale, and so is one of a node that isn't there
    let stale = Puts::removing(vec![NodeRemove::new("m2", 7), NodeRemove::new("nope", 1)]);
    let refused = sx.service.batch(&sx.plan, stale).await?;
    let expected = vec![
        StalePut {
            id: "m2".to_string(),
            refusal: Refusal::Moved,
        },
        StalePut {
            id: "nope".to_string(),
            refusal: Refusal::Missing,
        },
    ];
    assert_eq!(refused, Checked::Stale(expected));
    // a node put and removed in one batch is one the store can't keep
    let both = Puts {
        puts: new(vec![work("X", 4)]),
        removes: vec![NodeRemove::new("X", 1)],
    };
    let error = sx
        .service
        .batch(&sx.plan, both)
        .await
        .expect_err("a node put and removed was written");
    assert!(
        format!("{error:#}").contains("put and removed in one batch"),
        "{error:#}"
    );
    // nothing refused was removed, and nothing was sent
    assert_eq!(ids(&sx).await?, ["G", "m1", "m2", "W"]);
    assert!(changes.try_recv().is_err());
    // the need moved off in the same batch as the group and its members go
    let together = Puts {
        puts: vec![NodePut::new(work("W", 3), Some(1))],
        removes: vec![
            NodeRemove::new("G", 1),
            NodeRemove::new("m1", 1),
            NodeRemove::new("m2", 1),
        ],
    };
    let applied = sx.service.batch(&sx.plan, together).await?;
    assert_eq!(applied, Checked::Applied { versions: vec![2] });
    assert_eq!(ids(&sx).await?, ["W"]);
    assert_eq!(sx.service.check(&sx.plan).await?, Vec::<Diagnostic>::new());
    // and its one change names the node written and each node removed
    let expected = Change::NodesPut {
        plan: sx.plan.clone(),
        nodes: vec![PutNode {
            id: "W".to_string(),
            version: 2,
        }],
        removed: vec!["G".to_string(), "m1".to_string(), "m2".to_string()],
    };
    assert_eq!(changes.try_recv()?, expected);
    Ok(())
}

/// Two puts that each leave the plan whole, but together make a cycle, can't both get in
///
/// The first is held mid-write, under the plan's lock. The second waits for the lock, then is
/// judged against the plan the first left, rather than the one it read.
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn concurrent_puts_that_together_cycle_one_is_refused<B: Backend>(
    sx: Sx<B>,
) -> anyhow::Result<()> {
    // two nodes at version 1, each fine needing the other alone
    let batch = new(vec![work("Y", 0), work("Z", 1)]);
    sx.service.put(&sx.plan, batch).await?;
    let first = vec![NodePut::new(needing(work("Y", 0), &["Z"]), Some(1))];
    let second = vec![NodePut::new(needing(work("Z", 1), &["Y"]), Some(1))];
    // the first put stops once it is judged, just before its write
    let pause = sx.faults.pause_next_node_write();
    let write = sx.service.put(&sx.plan, first);
    let racer = async {
        pause.reached.notified().await;
        // the second waits on the plan's lock, rather than being judged beside the first
        let put = sx.service.put(&sx.plan, second);
        tokio::pin!(put);
        let waited = tokio::time::timeout(Duration::from_millis(200), &mut put)
            .await
            .is_err();
        // let the first write, and the second be judged after it
        pause.resume.notify_one();
        (waited, put.await)
    };
    let (written, (waited, put)) = tokio::join!(write, racer);
    assert_eq!(written?, Checked::Applied { versions: vec![2] });
    assert!(waited, "the second put didn't wait for the first");
    // the second would close a cycle, so it is refused
    let Checked::Refused(problems) = put? else {
        panic!("both halves of a cycle were written");
    };
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].message.starts_with("dependency cycle"),
        "{problems:?}"
    );
    // the plan is whole, and Z wasn't written
    assert_eq!(sx.service.check(&sx.plan).await?, Vec::<Diagnostic>::new());
    let z = sx.service.get(&sx.plan, "Z").await?.expect("Z was put");
    assert_eq!(z.version, 1);
    Ok(())
}

/// A put that waits on a whole-plan write is judged against the plan that write leaves
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn put_during_checked_import_is_judged_against_it<B: Backend>(
    sx: Sx<B>,
) -> anyhow::Result<()> {
    // a node a put will need, which a whole-plan write drops
    sx.service.put(&sx.plan, new(vec![work("Y", 0)])).await?;
    let pause = sx.faults.pause_next_flip();
    let write = sx.service.write_plan(&sx.plan, vec![work("W", 0)]);
    let racer = async {
        pause.reached.notified().await;
        // the put waits on the plan's lock while the write holds it
        let put = sx
            .service
            .put(&sx.plan, new(vec![needing(work("X", 1), &["Y"])]));
        tokio::pin!(put);
        let waited = tokio::time::timeout(Duration::from_millis(200), &mut put)
            .await
            .is_err();
        // let the write flip, and the put be judged after it
        pause.resume.notify_one();
        (waited, put.await)
    };
    let (written, (waited, put)) = tokio::join!(write, racer);
    assert!(
        matches!(written?, Rewrite::Applied(_)),
        "the whole-plan write was refused"
    );
    assert!(waited, "the put didn't wait for the write");
    // the node it needs is gone, so it is refused
    let expected = vec![problem("X", "needs `Y`, which is not a node")];
    assert_eq!(put?, Checked::Refused(expected));
    assert_eq!(ids(&sx).await?, ["W"]);
    Ok(())
}

/// A whole-plan write that would add a problem is refused before it reserves a revision
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn checked_import_refuses_a_broken_plan<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // the plan's row as it stands
    let row = sx.service.store().backend().plan(&sx.plan).await?;
    // an import into the empty plan with one broken node is refused with its problem
    let broken = vec![work("A", 0), needing(work("B", 1), &["nope"])];
    let answer = sx.service.write_plan(&sx.plan, broken).await?;
    let expected = vec![problem("B", "needs `nope`, which is not a node")];
    assert_eq!(answer, Rewrite::Refused(expected));
    // leaving the plan's row as it was: no revision reserved, none made current
    assert_eq!(sx.service.store().backend().plan(&sx.plan).await?, row);
    assert!(ids(&sx).await?.is_empty());
    // and a whole import goes in
    let whole = vec![work("A", 0), needing(work("B", 1), &["A"])];
    let answer = sx.service.write_plan(&sx.plan, whole).await?;
    assert!(matches!(answer, Rewrite::Applied(_)), "{answer:?}");
    assert_eq!(ids(&sx).await?, ["A", "B"]);
    assert_eq!(sx.service.check(&sx.plan).await?, Vec::<Diagnostic>::new());
    Ok(())
}

/// A plan's payload is what `Graph::build` and the renderer make of its stored nodes
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn payload_equals_graph_build_of_stored_nodes<B: Backend>(
    sx: Sx<B>,
) -> anyhow::Result<()> {
    // a group, two members, one taking its why and what from a doc, and a node after them
    doc(&sx.repo, "docs/design.md", DESIGN)?;
    let mut c11 = linking(member("C11", "M3", 1), &["docs/design.md#rows"]);
    c11.body = "# C11\n\nSee [the rows](docs/design.md#rows).\n".to_string();
    let written = vec![
        group("M3", 0),
        c11,
        needing(member("C53", "M3", 2), &["C11"]),
        needing(work("C14", 3), &["C53"]),
    ];
    let answer = sx.service.put(&sx.plan, new(written.clone())).await?;
    assert_eq!(
        answer,
        Checked::Applied {
            versions: vec![1, 1, 1, 1]
        }
    );
    // the stored nodes read back as they were written
    let listing = sx.service.list(&sx.plan).await?;
    let specs: Vec<NodeSpec> = listing.nodes.iter().map(|node| node.spec.clone()).collect();
    let expected: Vec<NodeSpec> = written.iter().map(|node| node.spec.clone()).collect();
    assert_eq!(specs, expected);
    // the oracle: the graph their records build, briefed from the working tree and rendered
    let records = listing.records(&sx.plan)?;
    let (mut graph, mut problems) = Graph::build(nodes_from_records(records));
    let renderer = Renderer::new(RepoSource::new(&sx.repo), Sections::default());
    problems.extend(renderer.prepare(&mut graph));
    assert_eq!(problems, Vec::<Diagnostic>::new());
    let oracle = renderer.payload(&graph, |_| None);
    // the service's payload is that, field for field, its links under the plan's docs
    let docs = paths::docs(&sx.plan);
    let oracle = serde_json::to_string(&oracle)?.replace(ROOT, &docs);
    let payload = sx.service.payload(&sx.plan).await?;
    assert_eq!(
        serde_json::to_value(&payload)?,
        serde_json::from_str::<serde_json::Value>(&oracle)?
    );
    // in written order, each member under its group
    let ids: Vec<&str> = payload.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(ids, ["M3", "C11", "C53", "C14"]);
    let c11 = &payload.nodes[1];
    assert_eq!(c11.parent.as_deref(), Some("M3"));
    // named in its plan, its brief form taken from the doc, and its links resolved
    assert_eq!(c11.origin.to_string(), "reef/central/C11");
    assert_eq!(c11.why_from.as_deref(), Some("Context"));
    let page = format!("{docs}docs/design.html#rows");
    assert_eq!(c11.docs[0].href, page);
    assert!(
        c11.html.contains(&format!("href=\"{page}\"")),
        "{}",
        c11.html
    );
    assert!(!c11.html.contains(ROOT), "{}", c11.html);
    Ok(())
}

/// A put to a plan no one listed, or of a project no one registered, is an error that says so
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn put_to_unlisted_plan_is_an_error<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a plan of the registered project that no one listed
    let unlisted = PlanName::new("reef", "unlisted")?;
    let error = sx
        .service
        .put(&unlisted, new(vec![work("A", 0)]))
        .await
        .expect_err("a put to an unlisted plan was written");
    let not_listed = error
        .downcast_ref::<NotListed>()
        .expect("the error is NotListed");
    assert_eq!(not_listed.0, unlisted);
    // and a plan of a project no one registered, whether put to, checked or built
    let unknown = PlanName::new("nobody", "central")?;
    for error in [
        sx.service
            .put(&unknown, new(vec![work("A", 0)]))
            .await
            .expect_err("a put to an unknown project was written"),
        sx.service
            .check(&unknown)
            .await
            .expect_err("an unknown project's plan was checked"),
        sx.service
            .payload(&unknown)
            .await
            .expect_err("an unknown project's plan was built"),
    ] {
        let not_registered = error
            .downcast_ref::<NotRegistered>()
            .expect("the error is NotRegistered");
        assert_eq!(not_registered.0, "nobody");
    }
    Ok(())
}

/// A node write refused partway through a batch is an error naming what was written
///
/// A batch isn't atomic: only another process can refuse a write the batch was judged fit for,
/// and the puts before it stay written.
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn node_write_refused_midway_is_an_error<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // the second write of the batch is refused, as another process having written would
    sx.faults.refuse_node_writes_after(1);
    let batch = new(vec![work("A", 0), work("B", 1)]);
    let error = sx
        .service
        .put(&sx.plan, batch)
        .await
        .expect_err("a refused write was answered as written");
    // it names the node refused and the one written before it
    let message = format!("{error:#}");
    assert!(message.contains("node `B`"), "{message}");
    assert!(message.contains("after `A` were written"), "{message}");
    // which stays written
    assert_eq!(ids(&sx).await?, ["A"]);
    Ok(())
}
