//! The store suite's cases, each generic over the backend so it runs against shoal and the twin

use reef_model::{NodeSpec, PlanName};
use reef_service::{AddedProject, Backend, NotListed, NotRegistered, Put, Refusal, RevisionKey};
use std::time::Duration;

use super::{Fx, Reopen, git, ids, node};

// the queue's, the pages', the work records', the comments' and the loops' cases, run by the
// same suite, which the other suites sharing `common` don't use
#[allow(unused_imports)]
pub use super::comment_cases::*;
pub use super::decision_cases::*;
#[allow(unused_imports)]
pub use super::loop_cases::*;
#[allow(unused_imports)]
pub use super::page_cases::*;
#[allow(unused_imports)]
pub use super::queue_cases::*;
#[allow(unused_imports)]
pub use super::user_cases::*;
#[allow(unused_imports)]
pub use super::work_cases::*;

/// A put of a new node is written at version 1, and a get reads back what was put
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn put_then_get<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
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
pub async fn get_of_missing_node_is_none<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
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
pub async fn list_of_new_plan_is_empty<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
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
pub async fn list_is_in_written_order<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
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
pub async fn stale_put_is_refused<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
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
pub async fn new_node_over_existing_is_refused<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
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
pub async fn put_naming_missing_node_is_refused<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
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
pub async fn concurrent_puts_from_one_version_one_wins<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
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
pub async fn put_to_unlisted_plan_is_an_error<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
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
pub async fn nested_members_are_refused<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
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
pub async fn duplicate_plan_is_refused<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a node in the plan the fixture listed
    fx.store.put(&fx.plan, node("C7", 0), None).await?;
    let before = fx.store.list(&fx.plan).await?;
    // listing it again is refused
    assert!(!fx.store.add_plan(&fx.plan).await?);
    // and its revision and nodes are untouched
    assert_eq!(fx.store.list(&fx.plan).await?, before);
    Ok(())
}

/// A whole-plan write replaces the revision: its nodes alone, under a new revision, the old pruned
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn bulk_write_replaces_the_revision<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // two nodes put one at a time
    fx.store.put(&fx.plan, node("C1", 0), None).await?;
    fx.store.put(&fx.plan, node("C2", 1), None).await?;
    let before = fx.store.list(&fx.plan).await?.revision;
    // a whole plan that keeps one, drops one and adds one
    let nodes = vec![node("C2", 0), node("C3", 1)];
    let written = fx.store.write_plan(&fx.plan, nodes.into()).await?;
    assert_ne!(written.revision, before);
    assert_eq!((written.recovered, written.left_retired), (0, false));
    // lists as written, under the new revision
    assert_eq!(fx.store.list(&fx.plan).await?.revision, written.revision);
    assert_eq!(ids(&fx).await?, ["C2", "C3"]);
    // the old revision is pruned, and the plan's row names nothing left to prune
    let old = RevisionKey::new(&fx.plan, before);
    assert!(fx.store.backend().nodes(&old, None).await?.is_empty());
    let row = fx
        .store
        .backend()
        .plan(&fx.plan)
        .await?
        .expect("the plan is listed");
    assert_eq!(
        (row.current, row.pending, row.retired),
        (written.revision, 0, 0)
    );
    Ok(())
}

/// A whole-plan write moves each kept node's version on, so an edit read before it is refused
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn bulk_write_moves_versions_on<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a node at version 2, read there by an editor
    fx.store.put(&fx.plan, node("C1", 0), None).await?;
    fx.store.put(&fx.plan, node("C1", 0), Some(1)).await?;
    // a whole plan that keeps it and adds another
    fx.store
        .write_plan(&fx.plan, vec![node("C1", 0), node("C2", 1)].into())
        .await?;
    // the kept node moved on a version, and the new one starts at 1
    let listing = fx.store.list(&fx.plan).await?;
    let versions: Vec<(&str, u64)> = listing
        .nodes
        .iter()
        .map(|node| (node.spec.id.as_str(), node.version))
        .collect();
    assert_eq!(versions, [("C1", 3), ("C2", 1)]);
    // so the editor's put, read at 2, is refused
    let put = fx.store.put(&fx.plan, node("C1", 0), Some(2)).await?;
    assert_eq!(put, Put::Stale(Refusal::Moved));
    Ok(())
}

/// A whole-plan write naming a node twice, or nesting one, is refused before anything is written
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn bulk_write_refuses_what_it_cant_keep<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // the same id twice
    let twice = vec![node("C1", 0), node("C1", 1)];
    let error = fx
        .store
        .write_plan(&fx.plan, twice.into())
        .await
        .expect_err("C1 was written twice");
    assert!(format!("{error:#}").contains("twice"), "{error:#}");
    // a member inside its group
    let mut group = node("M2", 0);
    group.spec.nodes = vec![NodeSpec {
        id: "C1".to_string(),
        ..NodeSpec::default()
    }];
    assert!(
        fx.store
            .write_plan(&fx.plan, vec![group].into())
            .await
            .is_err()
    );
    // and the plan's row names no revision either left
    let row = fx
        .store
        .backend()
        .plan(&fx.plan)
        .await?
        .expect("the plan is listed");
    assert_eq!((row.pending, row.retired, row.version), (0, 0, 1));
    Ok(())
}

/// A write killed midway leaves the old revision current and whole
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn killed_write_leaves_old_revision_whole<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a first revision of three nodes
    let first = vec![node("A1", 0), node("A2", 1), node("A3", 2)];
    fx.store.write_plan(&fx.plan, first.into()).await?;
    let before = fx.store.list(&fx.plan).await?;
    // a second, killed after one of its three rows
    fx.faults.fail_rows_after(1);
    let second = vec![node("B1", 0), node("B2", 1), node("B3", 2)];
    let killed = fx.store.write_plan(&fx.plan, second.into()).await;
    let error = killed.expect_err("the killed write succeeded");
    assert!(format!("{error:#}").contains("killed"), "{error:#}");
    // the first is still current, every node as it was
    assert_eq!(fx.store.list(&fx.plan).await?, before);
    // and the killed one is named pending, holding the one row it wrote
    let row = fx
        .store
        .backend()
        .plan(&fx.plan)
        .await?
        .expect("the plan is listed");
    assert_eq!(row.current, before.revision);
    assert_ne!(row.pending, 0);
    let dead = RevisionKey::new(&fx.plan, row.pending);
    assert_eq!(fx.store.backend().nodes(&dead, None).await?.len(), 1);
    Ok(())
}

/// The next write, in a restarted process, prunes the rows a killed write left
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn next_write_prunes_what_a_killed_write_left<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // a write killed after two of its three rows
    fx.faults.fail_rows_after(2);
    let killed = vec![node("B1", 0), node("B2", 1), node("B3", 2)];
    assert!(fx.store.write_plan(&fx.plan, killed.into()).await.is_err());
    let row = fx
        .store
        .backend()
        .plan(&fx.plan)
        .await?
        .expect("the plan is listed");
    let dead = RevisionKey::new(&fx.plan, row.pending);
    // the process restarts, and its next write finds them by the plan's pending revision
    let fx = fx.reopen().await?;
    let written = fx
        .store
        .write_plan(&fx.plan, vec![node("C1", 0)].into())
        .await?;
    assert_eq!(written.recovered, 2);
    // and prunes them, leaving the plan as that write wrote it
    assert!(fx.store.backend().nodes(&dead, None).await?.is_empty());
    assert_eq!(ids(&fx).await?, ["C1"]);
    let row = fx
        .store
        .backend()
        .plan(&fx.plan)
        .await?
        .expect("the plan is listed");
    assert_eq!((row.pending, row.retired), (0, 0));
    Ok(())
}

/// A write whose prune failed has still made its revision current, and the next put finishes it
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn killed_prune_is_finished_by_the_next_put<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a node in the plan's first revision
    fx.store.put(&fx.plan, node("A1", 0), None).await?;
    let first = fx.store.list(&fx.plan).await?.revision;
    // a whole-plan write whose prune of that revision fails
    fx.faults.fail_next_prune();
    let written = fx
        .store
        .write_plan(&fx.plan, vec![node("B1", 0)].into())
        .await?;
    assert!(written.left_retired);
    // its revision is current, and the first is retired with its row still stored
    assert_eq!(ids(&fx).await?, ["B1"]);
    let row = fx
        .store
        .backend()
        .plan(&fx.plan)
        .await?
        .expect("the plan is listed");
    assert_eq!(row.retired, first);
    let old = RevisionKey::new(&fx.plan, first);
    assert_eq!(fx.store.backend().nodes(&old, None).await?.len(), 1);
    // the next put prunes it, and names nothing left to prune
    fx.store.put(&fx.plan, node("B2", 1), None).await?;
    assert!(fx.store.backend().nodes(&old, None).await?.is_empty());
    let row = fx
        .store
        .backend()
        .plan(&fx.plan)
        .await?
        .expect("the plan is listed");
    assert_eq!((row.pending, row.retired), (0, 0));
    assert_eq!(ids(&fx).await?, ["B1", "B2"]);
    Ok(())
}

/// A put made during a whole-plan write waits for its flip, and is then refused as stale
///
/// Without the plan's lock it would land in the revision the write is about to retire, be
/// acknowledged, and be lost (X2).
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn put_during_bulk_write_waits_and_is_refused<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // a node at version 1, read there by an editor
    fx.store.put(&fx.plan, node("C1", 0), None).await?;
    let mut edit = node("C1", 0);
    edit.spec.title = Some("An edit".to_string());
    let mut import = node("C1", 0);
    import.spec.title = Some("Imported".to_string());
    // a whole-plan write that stops between its rows and its flip
    let pause = fx.faults.pause_next_flip();
    let write = fx.store.write_plan(&fx.plan, vec![import.clone()].into());
    let racer = async {
        pause.reached.notified().await;
        // the editor's put waits on the plan's lock, rather than writing into the old revision
        let put = fx.store.put(&fx.plan, edit, Some(1));
        tokio::pin!(put);
        let waited = tokio::time::timeout(Duration::from_millis(200), &mut put)
            .await
            .is_err();
        // let the write flip, and the put through after it
        pause.resume.notify_one();
        (waited, put.await)
    };
    let (written, (waited, put)) = tokio::join!(write, racer);
    written?;
    assert!(waited, "the put didn't wait for the write");
    assert_eq!(put?, Put::Stale(Refusal::Moved));
    // and the write's node stands, one version on
    let got = fx
        .store
        .get(&fx.plan, "C1")
        .await?
        .expect("the node was written");
    assert_eq!((got.version, got.spec), (2, import.spec));
    Ok(())
}

/// A delete of a row that isn't stored deletes nothing and fails nothing, in shoal as in the twin
///
/// Shoal answers it as `QueryDidNotSucceed`, a query that worked and found nothing, which the
/// store reads as done, so a prune tried again is harmless.
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn deleting_a_missing_row_deletes_nothing<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a node in the plan's current revision
    fx.store.put(&fx.plan, node("C1", 0), None).await?;
    let current = RevisionKey::new(&fx.plan, fx.store.list(&fx.plan).await?.revision);
    // a delete of another id there, and one in a revision that never was
    let backend = fx.store.backend();
    backend
        .delete_nodes(&current, vec!["C2".to_string()])
        .await?;
    backend
        .delete_nodes(&RevisionKey::new(&fx.plan, 7), vec!["C1".to_string()])
        .await?;
    // leave the node as it was
    assert_eq!(ids(&fx).await?, ["C1"]);
    Ok(())
}

/// A linked worktree, and any directory in one, finds its main repo's project
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn linked_worktree_finds_its_project<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a project for a repo of its own
    let main = git::repo(&fx.dir.join("app"))?;
    let AddedProject::Added(app) = fx.store.add_project("app", &main).await? else {
        panic!("a project for a fresh repo was refused");
    };
    assert_eq!(app.repo, main);
    // a linked worktree of it beside it, and a directory inside that
    let linked = git::worktree(&main, &fx.dir.join("app-linked"))?;
    let inside = linked.join("src").join("deep");
    std::fs::create_dir_all(&inside)?;
    // both find the main repo's project
    assert_eq!(fx.store.find_project(&linked).await?, Some(app.clone()));
    assert_eq!(fx.store.find_project(&inside).await?, Some(app));
    // and registering the worktree is registering its main repo, which is taken
    let again = fx.store.add_project("app-linked", &linked).await?;
    assert_eq!(
        again,
        AddedProject::RepoTaken {
            project: "app".to_string()
        }
    );
    Ok(())
}

/// A directory deep in a project's repo finds that project
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn subdirectory_finds_its_project<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a directory two levels into the fixture's repo
    let inside = fx.repo.join("crates").join("model");
    std::fs::create_dir_all(&inside)?;
    // finds the project registered for the repo
    let found = fx
        .store
        .find_project(&inside)
        .await?
        .expect("the repo's project is found");
    assert_eq!((found.name.as_str(), &found.repo), ("reef", &fx.repo));
    Ok(())
}

/// A directory in a repo no project is registered for finds none
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn unregistered_directory_finds_none<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a repo beside the registered one, never registered
    let other = git::repo(&fx.dir.join("other"))?;
    assert_eq!(fx.store.find_project(&other).await?, None);
    // and the case's directory itself, which is in this checkout's repo, not the fixture's
    assert_eq!(fx.store.find_project(&fx.dir).await?, None);
    Ok(())
}

/// Projects list by name, each with its repo
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn projects_list_by_name<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // two more projects, registered out of name order
    for name in ["zeta", "alpha"] {
        let repo = git::repo(&fx.dir.join(name))?;
        let added = fx.store.add_project(name, &repo).await?;
        assert!(matches!(added, AddedProject::Added(_)), "{added:?}");
    }
    // list in name order, each with the repo it was registered for
    let projects = fx.store.projects().await?;
    let names: Vec<&str> = projects
        .iter()
        .map(|project| project.name.as_str())
        .collect();
    assert_eq!(names, ["alpha", "reef", "zeta"]);
    assert_eq!(projects[1].repo, fx.repo);
    Ok(())
}

/// A project's plans list by name, each with a revision to load
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn plans_list_by_name<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // two more plans, listed out of name order
    for name in ["zz", "aa"] {
        assert!(fx.store.add_plan(&PlanName::new("reef", name)?).await?);
    }
    // list in name order, each named in its project and holding a current revision
    let plans = fx.store.plans("reef").await?;
    let names: Vec<String> = plans.iter().map(|plan| plan.name.to_string()).collect();
    assert_eq!(names, ["reef/aa", "reef/central", "reef/zz"]);
    assert!(plans.iter().all(|plan| plan.revision != 0), "{plans:?}");
    // and a project with no plans lists none
    assert!(fx.store.plans("nobody").await?.is_empty());
    Ok(())
}

/// A plan of a project that isn't registered is refused, naming the project
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn plan_of_unknown_project_is_refused<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // no project named `nobody` is registered
    let orphan = PlanName::new("nobody", "central")?;
    let error = fx
        .store
        .add_plan(&orphan)
        .await
        .expect_err("an orphan plan was listed");
    let not_registered = error
        .downcast_ref::<NotRegistered>()
        .expect("the error is NotRegistered");
    assert_eq!(not_registered.0, "nobody");
    // and nothing was listed for it
    assert!(fx.store.plans("nobody").await?.is_empty());
    Ok(())
}
