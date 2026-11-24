//! The service suite's cases, each generic over the backend so it runs against shoal and the twin
//!
//! Each case's plan is `reef/central`, of a project registered for a fresh git repo whose working
//! tree holds the plan's docs.

use reef_api::PutPage;
use reef_api::{Change, PutNode, paths};
use reef_model::{Diagnostic, Graph, NodeSpec, PlanName, nodes_from_records};
use reef_model::{EntryState, Status, WaitsOn};
use reef_render::{Layered, MemSource, ROOT, Renderer, RepoSource, Sections};
use reef_service::{
    AddedProject, Asker, Backend, Checked, NodePut, NodeRemove, NodeWrite, NotListed,
    NotRegistered, PageEdit, PageEdited, PageWrite, Place, Puts, QueueEdit, Queued, Refusal,
    RevisionKey, Rewrite, StalePut,
};
use std::time::Duration;

use super::{DESIGN, Sx, doc, git, group, member, work};

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
    assert_eq!(broken.len(), 3, "{broken:?}");
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

/// A put that gives work its what alone gets through, since the why it still lacks was a
/// problem already, worded the same
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn half_brief_fix_gets_through<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a plan holding work that says neither why nor what, written as an import would be
    let mut bare = work("A", 0);
    bare.spec.description = None;
    sx.service
        .store()
        .write_plan(&sx.plan, vec![bare].into())
        .await?;
    let broken = sx.service.check(&sx.plan).await?;
    assert_eq!(broken.len(), 2, "{broken:?}");
    // a put that writes its what alone adds no problem, so it gets through
    let mut half = work("A", 0);
    if let Some(description) = &mut half.spec.description {
        description.why = None;
    }
    let answer = sx
        .service
        .put(&sx.plan, vec![NodePut::new(half, Some(1))])
        .await?;
    assert_eq!(answer, Checked::Applied { versions: vec![2] });
    // and the why's problem is left as it was, word for word
    assert_eq!(sx.service.check(&sx.plan).await?, broken[..1]);
    Ok(())
}

/// A group goes in with its members as one batch, where neither goes in alone
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn batch_adds_a_group_with_its_members<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a group alone is a piece of work that says neither why nor what, a problem for each
    let alone = sx.service.put(&sx.plan, new(vec![group("G", 0)])).await?;
    let Checked::Refused(problems) = alone else {
        panic!("a group with no members was written: {alone:?}");
    };
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(
        problems[0]
            .to_string()
            .starts_with("reef/central/G: `G` does not say why it is needed: "),
        "{problems:?}"
    );
    assert!(
        problems[1]
            .to_string()
            .starts_with("reef/central/G: `G` does not say what finishing it will do: "),
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
    let write = sx.service.write_plan(&sx.plan, vec![work("W", 0)].into());
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
    let answer = sx.service.write_plan(&sx.plan, broken.into()).await?;
    let expected = vec![problem("B", "needs `nope`, which is not a node")];
    assert_eq!(answer, Rewrite::Refused(expected));
    // leaving the plan's row as it was: no revision reserved, none made current
    assert_eq!(sx.service.store().backend().plan(&sx.plan).await?, row);
    assert!(ids(&sx).await?.is_empty());
    // and a whole import goes in
    let whole = vec![work("A", 0), needing(work("B", 1), &["A"])];
    let answer = sx.service.write_plan(&sx.plan, whole.into()).await?;
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
    // a group, two members, one taking its why and what from a doc, one from a page of the
    // plan's, and a node after them
    doc(&sx.repo, "docs/design.md", DESIGN)?;
    put_page(&sx, "pages/rows.md", DESIGN, None).await?;
    let mut c11 = linking(member("C11", "M3", 1), &["docs/design.md#rows"]);
    c11.body = "# C11\n\nSee [the rows](docs/design.md#rows).\n".to_string();
    let written = vec![
        group("M3", 0),
        c11,
        needing(
            linking(member("C53", "M3", 2), &["pages/rows.md#rows"]),
            &["C11"],
        ),
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
    // the oracle: the graph their records build, briefed from the plan's pages over the working
    // tree and rendered
    let records = listing.records(&sx.plan)?;
    let (mut graph, mut problems) = Graph::build(nodes_from_records(records));
    let pages: MemSource = sx
        .service
        .pages(&sx.plan)
        .await?
        .pages
        .into_iter()
        .map(|page| (page.path, page.text))
        .collect();
    let renderer = Renderer::new(
        Layered::new(pages, RepoSource::new(&sx.repo)),
        Sections::default(),
    );
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

/// A single put's write refused by the backend is an error, as another process having written
/// the plan would make it
///
/// One write lands whole, so it is written in place, only over the version its writer read.
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn node_write_refused_in_place_is_an_error<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // the write is refused, as another process having written would refuse it
    sx.faults.refuse_node_writes_after(0);
    let error = sx
        .service
        .put(&sx.plan, new(vec![work("A", 0)]))
        .await
        .expect_err("a refused write was answered as written");
    // it names the node and says another process wrote the plan
    let message = format!("{error:#}");
    assert!(message.contains("node `A`"), "{message}");
    assert!(message.contains("another process"), "{message}");
    assert!(ids(&sx).await?.is_empty());
    Ok(())
}

/// A batch killed partway leaves the plan as it was, and the next write prunes what it left
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn killed_batch_leaves_the_plan_as_it_was<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a plan of two nodes, then a batch that changes one, adds one and removes the other
    sx.service
        .put(&sx.plan, new(vec![work("A", 0), work("B", 1)]))
        .await?;
    let before = sx.service.list(&sx.plan).await?;
    let mut changed = work("A", 0);
    changed.spec.title = Some("A Changed".to_string());
    let batch = Puts {
        puts: vec![
            NodePut::new(changed, Some(1)),
            NodePut::new(work("C", 2), None),
        ],
        removes: vec![NodeRemove::new("B", 1)],
    };
    // killed after one of its rows is written
    sx.faults.fail_rows_after(1);
    let error = sx
        .service
        .batch(&sx.plan, batch)
        .await
        .expect_err("the killed batch was answered as written");
    assert!(format!("{error:#}").contains("killed"), "{error:#}");
    // none of it shows: every node as it was, at the version it was
    assert_eq!(sx.service.list(&sx.plan).await?, before);
    // what it wrote is named pending, and the next write prunes it
    let backend = sx.service.store().backend();
    let row = backend.plan(&sx.plan).await?.expect("the plan is listed");
    assert_ne!(row.pending, 0);
    let dead = RevisionKey::new(&sx.plan, row.pending);
    assert_eq!(backend.nodes(&dead, None).await?.len(), 1);
    let answer = sx.service.put(&sx.plan, new(vec![work("D", 3)])).await?;
    assert_eq!(answer, Checked::Applied { versions: vec![1] });
    assert!(backend.nodes(&dead, None).await?.is_empty());
    let row = backend.plan(&sx.plan).await?.expect("the plan is listed");
    assert_eq!((row.pending, row.retired), (0, 0));
    assert_eq!(ids(&sx).await?, ["A", "B", "D"]);
    assert_eq!(sx.service.check(&sx.plan).await?, Vec::<Diagnostic>::new());
    Ok(())
}

/// A batch of several nodes moves only their versions on, so an edit read before it of a node
/// it doesn't touch still gets in after it
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn untouched_nodes_keep_their_versions_through_a_batch<B: Backend>(
    sx: Sx<B>,
) -> anyhow::Result<()> {
    // three nodes, and an edit of the last read at its version now
    sx.service
        .put(
            &sx.plan,
            new(vec![work("A", 0), work("B", 1), work("C", 2)]),
        )
        .await?;
    let read = sx.service.get(&sx.plan, "C").await?.expect("C was put");
    assert_eq!(read.version, 1);
    // a batch elsewhere: A changed, D added and B removed, as one revision
    let revision = sx.service.list(&sx.plan).await?.revision;
    let batch = Puts {
        puts: vec![
            NodePut::new(work("A", 0), Some(1)),
            NodePut::new(work("D", 3), None),
        ],
        removes: vec![NodeRemove::new("B", 1)],
    };
    let answer = sx.service.batch(&sx.plan, batch).await?;
    assert_eq!(
        answer,
        Checked::Applied {
            versions: vec![2, 1]
        }
    );
    // it landed as a revision of its own, which C is in at the version it was read at
    let listing = sx.service.list(&sx.plan).await?;
    assert_ne!(listing.revision, revision);
    let versions: Vec<(&str, u64)> = listing
        .nodes
        .iter()
        .map(|node| (node.spec.id.as_str(), node.version))
        .collect();
    assert_eq!(versions, [("A", 2), ("C", 1), ("D", 1)]);
    // so the edit of C read before the batch gets in after it
    let mut edit = NodeWrite::from(read);
    edit.spec.title = Some("C Edited".to_string());
    let answer = sx
        .service
        .put(&sx.plan, vec![NodePut::new(edit, Some(1))])
        .await?;
    assert_eq!(answer, Checked::Applied { versions: vec![2] });
    Ok(())
}

/// Queues a node at the bottom of project `reef`'s queue
///
/// # Arguments
///
/// * `sx` - The service and its plan
/// * `plan` - The node's plan
/// * `id` - The node's id
async fn queue_last<B: Backend>(sx: &Sx<B>, plan: &PlanName, id: &str) -> anyhow::Result<Queued> {
    let edit = QueueEdit::Add {
        node: plan.node(id)?,
        at: Place::Bottom,
    };
    sx.service.edit_queue("reef", edit).await
}

/// next takes the queue's ready entries first across a project's plans, then each plan's ready
/// work in plan order, and narrowed to one plan, that plan's entries alone
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn next_follows_the_queue_across_plans<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // two plans, each with work of its own
    let other = PlanName::new("reef", "other")?;
    sx.service.add_plan(&other).await?;
    sx.service
        .put(&sx.plan, new(vec![work("A", 0), work("B", 1)]))
        .await?;
    sx.service.put(&other, new(vec![work("X", 0)])).await?;
    // X then B queued
    queue_last(&sx, &other, "X").await?;
    queue_last(&sx, &sx.plan, "B").await?;
    // across the plans, the queue's first, then each plan's in plan order, plan by plan
    let next = sx.service.project_next("reef").await?;
    let ready: Vec<(String, bool)> = next
        .ready
        .iter()
        .map(|r| (format!("{}/{}", r.plan.as_ref().unwrap(), r.id), r.queued))
        .collect();
    let expected = [
        ("reef/other/X".to_string(), true),
        ("reef/central/B".to_string(), true),
        ("reef/central/A".to_string(), false),
    ];
    assert_eq!(ready, expected);
    assert_eq!(next.plans, [sx.plan.clone(), other.clone()]);
    // narrowed to one plan, the other's entries are left out
    let central = sx.service.next(&sx.plan).await?;
    let ready: Vec<(&str, bool)> = central
        .ready
        .iter()
        .map(|r| (r.id.as_str(), r.queued))
        .collect();
    assert_eq!(ready, [("B", true), ("A", false)]);
    assert_eq!(central.plans, std::slice::from_ref(&sx.plan));
    Ok(())
}

/// The queue's listing says what each entry is now: ready, waiting on its needs, done, a
/// group's ready members, or missing once its node is gone
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn queue_lists_each_entry_with_its_state<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // work, work needing it, work done, a group of one, and work about to go
    let mut done = work("D", 2);
    done.spec.status = Some(Status::Done);
    let nodes = vec![
        work("A", 0),
        needing(work("B", 1), &["A"]),
        done,
        group("G", 3),
        member("m1", "G", 4),
        work("E", 5),
    ];
    sx.service.put(&sx.plan, new(nodes)).await?;
    for id in ["B", "A", "D", "G", "E"] {
        queue_last(&sx, &sx.plan, id).await?;
    }
    // E goes from its plan, and stays queued
    let gone = Puts::removing(vec![NodeRemove::new("E", 1)]);
    assert!(matches!(
        sx.service.batch(&sx.plan, gone).await?,
        Checked::Applied { .. }
    ));
    // each entry's state, in the queue's order, under ranks that sort it
    let listing = sx.service.queue("reef").await?;
    assert_eq!(listing.plans, std::slice::from_ref(&sx.plan));
    let states: Vec<(&str, &EntryState)> = listing
        .entries
        .iter()
        .map(|listed| (listed.entry.node.id.as_str(), &listed.entry.state))
        .collect();
    let needs = vec![WaitsOn {
        id: "A".to_string(),
        status: Status::Todo,
        through: None,
    }];
    assert_eq!(
        states,
        [
            ("B", &EntryState::Waiting(needs)),
            ("A", &EntryState::Ready(vec!["A".to_string()])),
            ("D", &EntryState::Done),
            ("G", &EntryState::Ready(vec!["m1".to_string()])),
            ("E", &EntryState::Missing),
        ]
    );
    let ranks: Vec<&str> = listing.entries.iter().map(|l| l.rank.as_str()).collect();
    assert!(ranks.windows(2).all(|w| w[0] < w[1]), "{ranks:?}");
    // a missing entry can still be taken off
    let off = QueueEdit::Remove {
        node: sx.plan.node("E")?,
    };
    assert!(matches!(
        sx.service.edit_queue("reef", off).await?,
        Queued::Applied { .. }
    ));
    // while a node not in its plan isn't queued, and an unlisted plan or unknown project is an
    // error
    let nope = sx.plan.node("nope")?;
    assert_eq!(
        queue_last(&sx, &sx.plan, "nope").await?,
        Queued::NotInPlan { node: nope }
    );
    let unlisted = PlanName::new("reef", "unlisted")?;
    let error = queue_last(&sx, &unlisted, "A").await.unwrap_err();
    assert!(error.downcast_ref::<NotListed>().is_some(), "{error:#}");
    let error = sx.service.queue("nobody").await.unwrap_err();
    assert!(error.downcast_ref::<NotRegistered>().is_some(), "{error:#}");
    Ok(())
}

/// A queue edit made is a change to its project, and one refused is none
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn queue_edit_is_a_change_and_a_refusal_none<B: Backend>(
    sx: Sx<B>,
) -> anyhow::Result<()> {
    sx.service.put(&sx.plan, new(vec![work("A", 0)])).await?;
    let mut changes = sx.service.changes();
    // an entry added tells the stream its project's queue changed
    queue_last(&sx, &sx.plan, "A").await?;
    let expected = Change::QueueChanged {
        project: "reef".to_string(),
    };
    assert_eq!(changes.try_recv()?, expected);
    // a move of a node not queued, and an add of one not in its plan, tell it nothing
    let stray = QueueEdit::Move {
        node: sx.plan.node("X")?,
        at: Place::Top,
    };
    assert!(matches!(
        sx.service.edit_queue("reef", stray).await?,
        Queued::NotQueued { .. }
    ));
    queue_last(&sx, &sx.plan, "nope").await?;
    assert!(changes.try_recv().is_err());
    Ok(())
}

/// Puts a page into a plan, at the version its writer read, failing unless it is written
///
/// # Arguments
///
/// * `sx` - The service and its plan
/// * `path` - The page's path
/// * `text` - Its markdown
/// * `read` - The version its writer read, or `None` for a new page
async fn put_page<B: Backend>(
    sx: &Sx<B>,
    path: &str,
    text: &str,
    read: Option<u64>,
) -> anyhow::Result<u64> {
    let edit = PageEdit::Put {
        page: PageWrite::new(path, text),
        read,
    };
    match sx.service.edit_page(&sx.plan, edit).await? {
        PageEdited::Applied {
            version: Some(version),
        } => Ok(version),
        other => anyhow::bail!("the page wasn't put: {other:?}"),
    }
}

/// A page with a why and a what of its own under its `Rows` heading
const PAGE: &str = "# The page\n\n## Rows\n\n### Context\n\nPages are kept.\n\n### What it does\n\nPages go with the plan.\n";

/// A doc link naming a plan's page is checked, briefed and served from the page, which hides
/// the repo's file at its path
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn plan_pages_are_docs_over_the_repo<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // the repo's doc, and a page of the plan's at the same path, and one the repo hasn't
    doc(&sx.repo, "docs/design.md", DESIGN)?;
    put_page(&sx, "docs/design.md", PAGE, None).await?;
    put_page(&sx, "plan/only.md", PAGE, None).await?;
    // nodes linking each are checked against the pages, and written
    let a = linking(work("A", 0), &["docs/design.md#rows"]);
    let b = linking(work("B", 1), &["plan/only.md#rows"]);
    let answer = sx.service.put(&sx.plan, new(vec![a, b])).await?;
    assert_eq!(
        answer,
        Checked::Applied {
            versions: vec![1, 1]
        }
    );
    assert_eq!(sx.service.check(&sx.plan).await?, Vec::<Diagnostic>::new());
    // each briefed from its page, the page hiding the repo's doc
    let payload = sx.service.payload(&sx.plan).await?;
    for node in &payload.nodes {
        let why = &node.why;
        assert!(why.contains("Pages are kept."), "{}: {why}", node.id);
        assert_eq!(node.why_from.as_deref(), Some("Context"));
    }
    // their links lead to the pages, served from the plan
    let docs = paths::docs(&sx.plan);
    assert_eq!(
        payload.nodes[1].docs[0].href,
        format!("{docs}plan/only.html#rows")
    );
    let page = sx
        .service
        .doc_page(&sx.plan, "docs/design.html")
        .await?
        .expect("the page is served");
    assert!(page.contains("Pages are kept."), "{page}");
    assert!(!page.contains("Rows are slow."), "{page}");
    let only = sx.service.doc_page(&sx.plan, "plan/only.html").await?;
    assert!(only.is_some_and(|page| page.contains("<h2 id=\"rows\">")));
    // a heading the page lacks is refused, though the repo's doc has it
    let mut nope = work("C", 2);
    nope.spec.docs = vec!["docs/design.md#design-choices".to_string()];
    let answer = sx.service.put(&sx.plan, new(vec![nope])).await?;
    let expected = vec![problem(
        "C",
        "doc `docs/design.md#design-choices`: docs/design.md has no heading with the id \
         `design-choices`",
    )];
    assert_eq!(answer, Checked::Refused(expected));
    Ok(())
}

/// A page edit is refused only if it adds a problem, a stale one as stale, and an applied one
/// is a change while a refused one is none
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn page_edits_are_checked<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a node briefed from a page's section
    put_page(&sx, "design.md", PAGE, None).await?;
    let linked = linking(work("A", 0), &["design.md#rows"]);
    sx.service.put(&sx.plan, new(vec![linked])).await?;
    let mut changes = sx.service.changes();
    // taking the page out leaves the link nowhere to lead
    let remove = PageEdit::Remove {
        path: "design.md".to_string(),
        read: 1,
    };
    let answer = sx.service.edit_page(&sx.plan, remove.clone()).await?;
    let PageEdited::Refused(problems) = answer else {
        panic!("a linked page was taken out: {answer:?}");
    };
    assert_eq!(
        problems[0],
        problem("A", "doc `design.md#rows` not found (looked for design.md)")
    );
    // dropping its heading does too, and so does dropping the section the node's why is from
    let headless = PageEdit::Put {
        page: PageWrite::new("design.md", "# The page\n\nNo rows.\n"),
        read: Some(1),
    };
    let answer = sx.service.edit_page(&sx.plan, headless).await?;
    let PageEdited::Refused(problems) = answer else {
        panic!("a linked heading was dropped: {answer:?}");
    };
    assert_eq!(
        problems[0],
        problem(
            "A",
            "doc `design.md#rows`: design.md has no heading with the id `rows`"
        )
    );
    let whyless = PageEdit::Put {
        page: PageWrite::new(
            "design.md",
            "# The page\n\n## Rows\n\n### What it does\n\nKept.\n",
        ),
        read: Some(1),
    };
    let answer = sx.service.edit_page(&sx.plan, whyless).await?;
    let PageEdited::Refused(problems) = answer else {
        panic!("a page the node's why is from lost it: {answer:?}");
    };
    assert!(
        problems[0]
            .message
            .contains("does not say why it is needed"),
        "{problems:?}"
    );
    // a stale edit is refused as stale, before it is judged
    let stale = PageEdit::Put {
        page: PageWrite::new("design.md", PAGE),
        read: Some(9),
    };
    let answer = sx.service.edit_page(&sx.plan, stale).await?;
    assert_eq!(
        answer,
        PageEdited::Stale {
            path: "design.md".to_string(),
            refusal: Refusal::Moved
        }
    );
    // none of them wrote anything, or told the stream
    let page = sx
        .service
        .page(&sx.plan, "design.md")
        .await?
        .expect("the page stands");
    assert_eq!((page.text.as_str(), page.version), (PAGE, 1));
    assert!(changes.try_recv().is_err());
    // an edit that keeps what the node needs gets through, and is a change
    let more = format!("{PAGE}\nMore.\n");
    assert_eq!(put_page(&sx, "design.md", &more, Some(1)).await?, 2);
    let expected = Change::PagesPut {
        plan: sx.plan.clone(),
        pages: vec![PutPage {
            path: "design.md".to_string(),
            version: 2,
        }],
        removed: Vec::new(),
    };
    assert_eq!(changes.try_recv()?, expected);
    // and once nothing links it, it comes out, which is a change too
    let gone = Puts::removing(vec![NodeRemove::new("A", 1)]);
    sx.service.batch(&sx.plan, gone).await?;
    let _ = changes.try_recv()?;
    let remove = PageEdit::Remove {
        path: "design.md".to_string(),
        read: 2,
    };
    let answer = sx.service.edit_page(&sx.plan, remove).await?;
    assert_eq!(answer, PageEdited::Applied { version: None });
    let expected = Change::PagesPut {
        plan: sx.plan.clone(),
        pages: Vec::new(),
        removed: vec!["design.md".to_string()],
    };
    assert_eq!(changes.try_recv()?, expected);
    Ok(())
}

/// Each plan of a project reads its own pages, in check and in next across the project
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn each_plan_reads_its_own_pages<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // two plans, each with a page at the same path under a heading of its own
    let other = PlanName::new("reef", "other")?;
    sx.service.add_plan(&other).await?;
    put_page(&sx, "design.md", PAGE, None).await?;
    let columns = PAGE.replace("## Rows", "## Columns");
    let edit = PageEdit::Put {
        page: PageWrite::new("design.md", columns),
        read: None,
    };
    sx.service.edit_page(&other, edit).await?;
    // each plan's node links its own page's heading
    sx.service
        .put(
            &sx.plan,
            new(vec![linking(work("A", 0), &["design.md#rows"])]),
        )
        .await?;
    let answer = sx
        .service
        .put(
            &other,
            new(vec![linking(work("B", 0), &["design.md#columns"])]),
        )
        .await?;
    assert_eq!(answer, Checked::Applied { versions: vec![1] });
    // and not the other plan's
    let answer = sx
        .service
        .put(
            &other,
            new(vec![linking(work("C", 1), &["design.md#rows"])]),
        )
        .await?;
    assert!(matches!(answer, Checked::Refused(_)), "{answer:?}");
    // so each checks clean, and next across the project finds no problem in either
    assert_eq!(sx.service.check(&other).await?, Vec::<Diagnostic>::new());
    let next = sx.service.project_next("reef").await?;
    assert_eq!(next.problems, 0, "{next:?}");
    assert_eq!(next.ready.len(), 2, "{next:?}");
    Ok(())
}

/// The first user is added only from the service's own host, and is an admin; after that, only
/// an admin adds one
///
/// # Arguments
///
/// * `sx` - The service and its plan
pub async fn the_first_user_needs_this_host<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    use reef_api::{NewUser, UserAdded};
    use reef_service::{NotAllowed, NotSignedIn};
    let new = |name: &str| NewUser {
        name: name.to_string(),
        password: "correct horse battery".to_string(),
        admin: false,
    };
    // from another host, nobody adds the first user
    let remote = Asker {
        user: None,
        local: false,
    };
    let refused = sx
        .service
        .add_user(new("ada"), remote)
        .await
        .expect_err("added from afar");
    assert!(
        refused.downcast_ref::<NotAllowed>().is_some(),
        "{refused:#}"
    );
    assert!(sx.service.store().users().await?.is_empty());
    // from this host, the first is an admin
    let local = Asker {
        user: None,
        local: true,
    };
    let UserAdded::Added(ada) = sx.service.add_user(new("ada"), local).await? else {
        anyhow::bail!("ada wasn't added from this host");
    };
    assert!(ada.admin);
    // and then this host alone adds nobody, but its admin does
    let refused = sx
        .service
        .add_user(new("eve"), local)
        .await
        .expect_err("added by nobody");
    assert!(
        refused.downcast_ref::<NotSignedIn>().is_some(),
        "{refused:#}"
    );
    let by_ada = Asker {
        user: Some(&ada),
        local: false,
    };
    let added = sx.service.add_user(new("bob"), by_ada).await?;
    assert!(
        matches!(&added, UserAdded::Added(bob) if !bob.admin),
        "{added:?}"
    );
    Ok(())
}

/// A service given an interval fetches every remote project on it, so a pushed doc is read
/// without anyone asking for a fetch
///
/// # Arguments
///
/// * `sx` - The service, its project and plan
pub async fn remotes_are_fetched_on_their_interval<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    // a service that fetches every remote project often
    let service = sx.service.fetching_every(Some(Duration::from_millis(100)));
    // and a remote project of a published repo, with a plan
    let before = "# Design\n\nThe old words.\n";
    let (bare, checkout) = git::published(&sx.dir.join("up"), &[("docs/design.md", before)])?;
    let url = format!("file://{}", bare.display());
    let added = service
        .add_remote_project("up", &url, None, Asker::default())
        .await?;
    assert!(matches!(added, AddedProject::Added(_)), "{added:?}");
    let plan = PlanName::new("up", "main")?;
    assert!(service.add_plan(&plan).await?);
    // a change pushed, and no fetch asked for
    git::write(&checkout, "docs/design.md", "# Design\n\nThe new words.\n")?;
    git::run(&checkout, &["commit", "-qam", "revised"])?;
    let tip = git::push(&checkout)?;
    let page = service.doc_page(&plan, "docs/design.html").await?;
    assert!(page.is_some_and(|page| page.contains("The old words")));
    // read once the service has followed its remotes for a while
    let seen = async {
        loop {
            let page = service.doc_page(&plan, "docs/design.html").await?;
            if page.is_some_and(|page| page.contains("The new words")) {
                return anyhow::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::select! {
        () = service.follow_remotes() => anyhow::bail!("following the remotes stopped"),
        seen = tokio::time::timeout(Duration::from_secs(30), seen) => seen??,
    }
    // and the project names the commit it fetched
    let up = service
        .projects()
        .await?
        .into_iter()
        .find(|project| project.name == "up")
        .expect("the remote project is listed");
    assert_eq!(up.remote.map(|remote| remote.commit), Some(tip));
    Ok(())
}

/// A service given hosts to clone from refuses a remote that names another, however its URL
/// is written, and still takes a path on its own host, which has none
///
/// # Arguments
///
/// * `sx` - The service, its project and plan
pub async fn remotes_come_from_the_hosts_listed<B: Backend>(sx: Sx<B>) -> anyhow::Result<()> {
    use reef_service::remote::BadRemote;
    // a service that clones from one host alone, named in another case
    let service = sx.service.cloning_from(vec!["Git.Example".to_string()]);
    // a remote on any other host is refused before anything is fetched, naming the key
    for url in [
        "https://elsewhere.example/up.git",
        "ssh://git@ELSEWHERE.example/up.git",
        "git@elsewhere.example:up.git",
        "git://git.example.elsewhere.example/up.git",
    ] {
        let refused = service
            .add_remote_project("up", url, None, Asker::default())
            .await
            .expect_err(url);
        let bad = refused
            .downcast_ref::<BadRemote>()
            .unwrap_or_else(|| panic!("{url}: {refused}"));
        assert!(bad.0.contains("`remotes.hosts`"), "{url}: {bad}");
        assert!(bad.0.contains("git.example"), "{url}: {bad}");
    }
    // a path on this host names no host, so the list leaves it be
    let (bare, _) = git::published(&sx.dir.join("up"), &[("README.md", "# Up\n")])?;
    let url = format!("file://{}", bare.display());
    let added = service
        .add_remote_project("up", &url, None, Asker::default())
        .await?;
    assert!(matches!(added, AddedProject::Added(_)), "{added:?}");
    Ok(())
}
