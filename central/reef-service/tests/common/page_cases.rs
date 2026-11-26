//! The store suite's cases for a plan's pages, each generic over the backend so it runs against
//! shoal and the twin

use reef_api::PLAN_PAGES_MAX_BYTES;
use reef_service::{
    Backend, Checked, NodePut, PageEdit, PageEdited, PageWrite, PlanWrite, Proposal, Puts, Refusal,
    RevisionKey, Unkeepable,
};
use std::time::Duration;

use super::{Fx, Reopen, node};

/// A check that finds no problem, for writes whose judging these cases don't test
///
/// # Arguments
///
/// * `_proposal` - The plan as it stands and as the write would leave it
async fn no_problems(_proposal: Proposal) -> anyhow::Result<Vec<reef_model::Diagnostic>> {
    Ok(Vec::new())
}

/// A whole plan's nodes and its pages, to write as one revision
///
/// # Arguments
///
/// * `ids` - The ids of its nodes, in written order
/// * `pages` - Each of its pages, a path and its text
fn whole(ids: &[&str], pages: &[(&str, &str)]) -> PlanWrite {
    PlanWrite {
        nodes: ids
            .iter()
            .enumerate()
            .map(|(order, id)| node(id, order as u64))
            .collect(),
        pages: Some(
            pages
                .iter()
                .map(|(path, text)| PageWrite::new(*path, *text))
                .collect(),
        ),
    }
}

/// Each page a plan lists, its path, text and version
///
/// # Arguments
///
/// * `fx` - The store and its plan
async fn pages<B: Backend>(fx: &Fx<B>) -> anyhow::Result<Vec<(String, String, u64)>> {
    let listing = fx.store.pages(&fx.plan).await?;
    Ok(listing
        .pages
        .into_iter()
        .map(|page| (page.path, page.text, page.version))
        .collect())
}

/// The plan's row as it is stored
///
/// # Arguments
///
/// * `fx` - The store and its plan
async fn plan_row<B: Backend>(fx: &Fx<B>) -> anyhow::Result<reef_service::schema::PlanRow> {
    Ok(fx
        .store
        .backend()
        .plan(&fx.plan)
        .await?
        .expect("the plan is listed"))
}

/// A whole-plan write with pages replaces the plan's pages in the same flip as its nodes, each
/// page's version carried on, and prunes the pages it replaced
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn bulk_write_with_pages_replaces_them<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a plan's first pages, at version 1, in the revision its nodes are in
    let first = fx
        .store
        .write_plan(
            &fx.plan,
            whole(&["C1"], &[("b.md", "# B\n"), ("a.md", "# A\n")]),
        )
        .await?;
    let listing = fx.store.pages(&fx.plan).await?;
    assert_eq!(listing.revision, first.revision);
    assert_eq!(
        pages(&fx).await?,
        [
            ("a.md".to_string(), "# A\n".to_string(), 1),
            ("b.md".to_string(), "# B\n".to_string(), 1)
        ]
    );
    // a second write's pages replace them: one kept one version on, one gone, one new
    let second = fx
        .store
        .write_plan(
            &fx.plan,
            whole(&["C1"], &[("b.md", "# B2\n"), ("c.md", "# C\n")]),
        )
        .await?;
    assert!(!second.left_retired);
    assert_eq!(
        pages(&fx).await?,
        [
            ("b.md".to_string(), "# B2\n".to_string(), 2),
            ("c.md".to_string(), "# C\n".to_string(), 1)
        ]
    );
    // the pages they replaced are pruned, and nothing is left named to prune
    let old = RevisionKey::new(&fx.plan, first.revision);
    assert!(fx.store.backend().pages(&old, None).await?.is_empty());
    let row = plan_row(&fx).await?;
    assert_eq!(row.pages_current, second.revision);
    assert_eq!((row.pending, row.retired, row.pages_retired), (0, 0, 0));
    Ok(())
}

/// A whole-plan write without pages leaves the plan's pages as they are, where they are
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn bulk_write_without_pages_keeps_them<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a plan with a page
    let first = fx
        .store
        .write_plan(&fx.plan, whole(&["C1"], &[("a.md", "# A\n")]))
        .await?;
    // a write of its nodes alone makes a revision of nodes, and leaves the page in the first
    let second = fx
        .store
        .write_plan(&fx.plan, vec![node("C2", 0)].into())
        .await?;
    let row = plan_row(&fx).await?;
    assert_eq!(row.current, second.revision);
    assert_eq!(row.pages_current, first.revision);
    assert_eq!(
        pages(&fx).await?,
        [("a.md".to_string(), "# A\n".to_string(), 1)]
    );
    // its old nodes are pruned, and its pages, at that same revision, are not
    let old = RevisionKey::new(&fx.plan, first.revision);
    assert!(fx.store.backend().nodes(&old, None).await?.is_empty());
    assert_eq!(fx.store.backend().pages(&old, None).await?.len(), 1);
    Ok(())
}

/// A batch of node writes leaves the plan's pages where they are, at their versions, though the
/// revision of nodes they were written with is retired
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn node_batch_leaves_pages_in_place<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a plan's nodes and its pages, in one revision
    let first = fx
        .store
        .write_plan(&fx.plan, whole(&["C1"], &[("a.md", "# A\n")]))
        .await?;
    // a batch of two puts, which is a revision of its own
    let puts = Puts::new(vec![
        NodePut::new(node("C2", 1), None),
        NodePut::new(node("C3", 2), None),
    ]);
    let checked = fx
        .store
        .batch_checked(&fx.plan, puts, &reef_api::NOBODY, no_problems)
        .await?;
    assert!(matches!(checked, Checked::Applied { .. }), "{checked:?}");
    // and a single put, which is one row's write
    let one = Puts::new(vec![NodePut::new(node("C4", 3), None)]);
    fx.store
        .batch_checked(&fx.plan, one, &reef_api::NOBODY, no_problems)
        .await?;
    // the nodes moved to a revision of their own, and the pages stayed in theirs, as they were
    let row = plan_row(&fx).await?;
    assert_ne!(row.current, first.revision);
    assert_eq!(row.pages_current, first.revision);
    assert_eq!(
        pages(&fx).await?,
        [("a.md".to_string(), "# A\n".to_string(), 1)]
    );
    // no page was copied into the nodes' revision
    let nodes = RevisionKey::new(&fx.plan, row.current);
    assert!(fx.store.backend().pages(&nodes, None).await?.is_empty());
    // and a snapshot reads the nodes of the one beside the pages of the other
    let (listing, listed) = fx.store.snapshot(&fx.plan).await?;
    assert_eq!(listing.revision, row.current);
    assert_eq!(listed.revision, first.revision);
    assert_eq!(listing.nodes.len(), 4);
    assert_eq!(listed.pages.len(), 1);
    Ok(())
}

/// A whole-plan write killed among its pages leaves the plan's nodes and pages as they were
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn killed_page_write_leaves_old_pages_whole<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a first revision with a page
    fx.store
        .write_plan(&fx.plan, whole(&["A1"], &[("a.md", "# A\n")]))
        .await?;
    let nodes = fx.store.list(&fx.plan).await?;
    let before = fx.store.pages(&fx.plan).await?;
    // a second, killed after one of its two pages
    fx.faults.fail_page_rows_after(1);
    let killed = fx
        .store
        .write_plan(
            &fx.plan,
            whole(&["B1"], &[("b.md", "# B\n"), ("c.md", "# C\n")]),
        )
        .await;
    let error = killed.expect_err("the killed write succeeded");
    assert!(format!("{error:#}").contains("killed"), "{error:#}");
    // the first is still current, its nodes and pages as they were
    assert_eq!(fx.store.list(&fx.plan).await?, nodes);
    assert_eq!(fx.store.pages(&fx.plan).await?, before);
    // and the killed one is named pending, holding its node and the one page it wrote
    let row = plan_row(&fx).await?;
    assert_ne!(row.pending, 0);
    let dead = RevisionKey::new(&fx.plan, row.pending);
    assert_eq!(fx.store.backend().nodes(&dead, None).await?.len(), 1);
    assert_eq!(fx.store.backend().pages(&dead, None).await?.len(), 1);
    Ok(())
}

/// The next write, in a restarted process, prunes the nodes and pages a killed write left
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn next_write_prunes_killed_pages<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a write killed after its two nodes and one of its pages
    fx.faults.fail_page_rows_after(1);
    let killed = whole(&["B1", "B2"], &[("b.md", "# B\n"), ("c.md", "# C\n")]);
    assert!(fx.store.write_plan(&fx.plan, killed).await.is_err());
    let dead = RevisionKey::new(&fx.plan, plan_row(&fx).await?.pending);
    // the process restarts, and its next write, a page edit, finds them by the plan's pending
    let fx = fx.reopen().await?;
    let edit = PageEdit::Put {
        page: PageWrite::new("a.md", "# A\n"),
        read: None,
    };
    let edited = fx
        .store
        .edit_page_checked(&fx.plan, edit, &reef_api::NOBODY, no_problems)
        .await?;
    assert_eq!(edited, PageEdited::Applied { version: Some(1) });
    // and prunes both tables, leaving the plan as that edit left it
    assert!(fx.store.backend().nodes(&dead, None).await?.is_empty());
    assert!(fx.store.backend().pages(&dead, None).await?.is_empty());
    assert_eq!(
        pages(&fx).await?,
        [("a.md".to_string(), "# A\n".to_string(), 1)]
    );
    let row = plan_row(&fx).await?;
    assert_eq!((row.pending, row.retired, row.pages_retired), (0, 0, 0));
    // and a whole-plan write after a killed one counts what it pruned, nodes and pages
    fx.faults.fail_page_rows_after(1);
    let killed = whole(&["B1", "B2"], &[("b.md", "# B\n"), ("c.md", "# C\n")]);
    assert!(fx.store.write_plan(&fx.plan, killed).await.is_err());
    let written = fx
        .store
        .write_plan(&fx.plan, vec![node("C1", 0)].into())
        .await?;
    assert_eq!(written.recovered, 3);
    Ok(())
}

/// A write whose prune of the pages it replaced failed has still made them current, and the
/// next write finishes it
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn killed_page_prune_is_finished_by_the_next_put<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // a plan with a page
    let first = fx
        .store
        .write_plan(&fx.plan, whole(&["A1"], &[("a.md", "# A\n")]))
        .await?;
    // a whole-plan write whose prune of those pages fails
    fx.faults.fail_next_page_prune();
    let written = fx
        .store
        .write_plan(&fx.plan, whole(&["B1"], &[("b.md", "# B\n")]))
        .await?;
    assert!(written.left_retired);
    // its pages are current, and the first's are retired with their row still stored
    assert_eq!(
        pages(&fx).await?,
        [("b.md".to_string(), "# B\n".to_string(), 1)]
    );
    let row = plan_row(&fx).await?;
    assert_eq!(row.pages_retired, first.revision);
    let old = RevisionKey::new(&fx.plan, first.revision);
    assert_eq!(fx.store.backend().pages(&old, None).await?.len(), 1);
    // the next put prunes them, and names nothing left to prune
    fx.store.put(&fx.plan, node("B2", 1), None).await?;
    assert!(fx.store.backend().pages(&old, None).await?.is_empty());
    let row = plan_row(&fx).await?;
    assert_eq!((row.pending, row.retired, row.pages_retired), (0, 0, 0));
    assert_eq!(
        pages(&fx).await?,
        [("b.md".to_string(), "# B\n".to_string(), 1)]
    );
    Ok(())
}

/// A page edit is one row's write in the pages' current revision, by version, and a removal
/// deletes the row
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn page_edit_lands_in_place<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let before = plan_row(&fx).await?;
    // a new page at version 1, then a change to it at version 2
    let put = |text: &str, read: Option<u64>| PageEdit::Put {
        page: PageWrite::new("design/app.md", text),
        read,
    };
    let edited = fx
        .store
        .edit_page_checked(
            &fx.plan,
            put("# App\n", None),
            &reef_api::NOBODY,
            no_problems,
        )
        .await?;
    assert_eq!(edited, PageEdited::Applied { version: Some(1) });
    let edited = fx
        .store
        .edit_page_checked(
            &fx.plan,
            put("# App, again\n", Some(1)),
            &reef_api::NOBODY,
            no_problems,
        )
        .await?;
    assert_eq!(edited, PageEdited::Applied { version: Some(2) });
    let page = fx
        .store
        .page(&fx.plan, "design/app.md")
        .await?
        .expect("the page was put");
    assert_eq!((page.text.as_str(), page.version), ("# App, again\n", 2));
    // in place: no revision was made, and the plan's row is as it was
    assert_eq!(plan_row(&fx).await?, before);
    // a removal at the version read takes it out
    let remove = PageEdit::Remove {
        path: "design/app.md".to_string(),
        read: 2,
    };
    let edited = fx
        .store
        .edit_page_checked(&fx.plan, remove, &reef_api::NOBODY, no_problems)
        .await?;
    assert_eq!(edited, PageEdited::Applied { version: None });
    assert!(fx.store.page(&fx.plan, "design/app.md").await?.is_none());
    assert!(pages(&fx).await?.is_empty());
    // and a path no page can have names none
    assert!(fx.store.page(&fx.plan, "../x.md").await?.is_none());
    Ok(())
}

/// A page edit that finds the page other than its writer read it is refused as stale, saying
/// how, and writes nothing
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn stale_page_edit_is_refused<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a page at version 1
    let put = |read: Option<u64>| PageEdit::Put {
        page: PageWrite::new("a.md", "# Changed\n"),
        read,
    };
    fx.store
        .edit_page_checked(&fx.plan, put(None), &reef_api::NOBODY, no_problems)
        .await?;
    // a new page over it, an edit of another version, and edits of a page that isn't there
    let cases = [
        (put(None), Refusal::Exists),
        (put(Some(7)), Refusal::Moved),
        (
            PageEdit::Put {
                page: PageWrite::new("b.md", "# B\n"),
                read: Some(1),
            },
            Refusal::Missing,
        ),
        (
            PageEdit::Remove {
                path: "b.md".to_string(),
                read: 1,
            },
            Refusal::Missing,
        ),
    ];
    for (edit, refusal) in cases {
        let path = edit.path().to_string();
        let edited = fx
            .store
            .edit_page_checked(&fx.plan, edit, &reef_api::NOBODY, no_problems)
            .await?;
        assert_eq!(edited, PageEdited::Stale { path, refusal });
    }
    // and the page is as it was
    let page = fx.store.page(&fx.plan, "a.md").await?.expect("a.md stands");
    assert_eq!(page.version, 1);
    Ok(())
}

/// A page edit made during a whole-plan write waits for its flip, and is then refused as stale,
/// the write having moved the page's version on
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn page_edit_during_import_waits_and_is_refused<B: Reopen>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // a page at version 1, read there by an editor
    fx.store
        .write_plan(&fx.plan, whole(&["C1"], &[("a.md", "# A\n")]))
        .await?;
    let edit = PageEdit::Put {
        page: PageWrite::new("a.md", "# An edit\n"),
        read: Some(1),
    };
    // a whole-plan write that stops between its rows and its flip
    let pause = fx.faults.pause_next_flip();
    let write = fx
        .store
        .write_plan(&fx.plan, whole(&["C1"], &[("a.md", "# Imported\n")]));
    let racer = async {
        pause.reached.notified().await;
        // the edit waits on the plan's lock, rather than writing into the old pages
        let edited = fx
            .store
            .edit_page_checked(&fx.plan, edit, &reef_api::NOBODY, no_problems);
        tokio::pin!(edited);
        let waited = tokio::time::timeout(Duration::from_millis(200), &mut edited)
            .await
            .is_err();
        // let the write flip, and the edit through after it
        pause.resume.notify_one();
        (waited, edited.await)
    };
    let (written, (waited, edited)) = tokio::join!(write, racer);
    written?;
    assert!(waited, "the edit didn't wait for the write");
    assert_eq!(
        edited?,
        PageEdited::Stale {
            path: "a.md".to_string(),
            refusal: Refusal::Moved
        }
    );
    // and the write's page stands, one version on
    assert_eq!(
        pages(&fx).await?,
        [("a.md".to_string(), "# Imported\n".to_string(), 2)]
    );
    Ok(())
}

/// Pages the store can't keep are refused before anything is written: a path no page can
/// have, a path twice, a page a book would read as a node, and more text than a plan keeps
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn pages_it_cant_keep_are_refused<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    let before = plan_row(&fx).await?;
    let too_big = "x".repeat(PLAN_PAGES_MAX_BYTES);
    let refused: [(&[(&str, &str)], &str); 4] = [
        (
            &[("../a.md", "# A\n")],
            "`../a.md` has a part starting with `.`",
        ),
        (
            &[("a.md", "# A\n"), ("a.md", "# A\n")],
            "page `a.md` is written twice",
        ),
        (
            &[("n.md", "---\nid: n\n---\n")],
            "page `n.md` opens with a node's frontmatter",
        ),
        (&[("big.md", too_big.as_str())], "more than the"),
    ];
    for (pages, why) in refused {
        let error = fx
            .store
            .write_plan(&fx.plan, whole(&["C1"], pages))
            .await
            .expect_err("a page the store can't keep was written");
        assert!(error.downcast_ref::<Unkeepable>().is_some(), "{error:#}");
        assert!(format!("{error:#}").contains(why), "{error:#}");
    }
    // and an edit of one is refused the same way
    let edit = PageEdit::Put {
        page: PageWrite::new("SUMMARY.md", "# Summary\n"),
        read: None,
    };
    let error = fx
        .store
        .edit_page_checked(&fx.plan, edit, &reef_api::NOBODY, no_problems)
        .await
        .expect_err("a summary was kept as a page");
    assert!(error.downcast_ref::<Unkeepable>().is_some(), "{error:#}");
    // none of them wrote anything
    assert_eq!(plan_row(&fx).await?, before);
    Ok(())
}

/// A page write refused under the plan's lock, once the page was seen as read, is an error:
/// another process wrote the plan
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn page_write_refused_in_place_is_an_error<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // the write is refused as another process's write would make it
    fx.faults.refuse_next_page_write();
    let edit = PageEdit::Put {
        page: PageWrite::new("a.md", "# A\n"),
        read: None,
    };
    let error = fx
        .store
        .edit_page_checked(&fx.plan, edit, &reef_api::NOBODY, no_problems)
        .await
        .expect_err("a refused write was taken as written");
    assert!(
        format!("{error:#}").contains("another process wrote the plan"),
        "{error:#}"
    );
    assert!(fx.store.page(&fx.plan, "a.md").await?.is_none());
    Ok(())
}
