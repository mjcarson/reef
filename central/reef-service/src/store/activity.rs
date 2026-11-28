//! Each plan's activity: an entry a write to its nodes or pages, newest first (C119)
//!
//! - **Told from a write's history.** A write's history names each subject it changed, so the
//!   entries of a plan's nodes and pages say which plan it wrote and what of it: the nodes it
//!   added, changed, removed or moved, the ones it took to done, and the pages. A start writes a
//!   node's work record, not the node, so its entry is no plan's. A queue's, a loop's, a user's,
//!   a registration's and a branch's subjects aren't a node's or a page's, so they are none
//!   either.
//! - **Kept as the history is.** One row a plan the write touched, under the write's own key,
//!   in the same bundle as the first of its edit rows, once the write has landed. One that fails
//!   to keep leaves the write standing, as its edits do.
//! - **Its last update is its newest entry,** or its listing for a plan none of whose writes was
//!   kept, such as one written only before C119.
//! - **Done in a window is done now.** A window reads its entries from its end back, a page at a
//!   time, and stops at the first older than its start, since keys count down. Of the nodes they
//!   took to done, only those done now are counted, so one set back or removed since isn't.

use reef_api::{Activity, Author, Edited, Updated};
use reef_model::{PlanName, Status};
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use tracing::instrument;

use super::Store;
use super::history::Pending;
use crate::backend::Backend;
use crate::schema::ActivityRow;

/// How many of a plan's entries a window's read takes at a time
const ACTIVITY_PAGE: usize = 500;

/// What a subject of a write is of its plan: one of its nodes, or one of its pages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Touched<'a> {
    /// A node, by its id
    Node(&'a str),
    /// A page, by its path
    Page(&'a str),
}

/// The plan a subject's edits belong to, and what of it they touched, for a node's or a page's
/// subject; `None` for any other
///
/// It undoes `history::node_subject` and `history::page_subject`: a plan's name and a node's id
/// never hold a `/`, so the first two split them, and a page's path keeps the rest.
///
/// # Arguments
///
/// * `subject` - The subject's partition, such as `node/<plan>/<id>`
fn touched(subject: &str) -> Option<(&str, Touched<'_>)> {
    // its kind, its plan, then the node's id or the page's path
    let (kind, rest) = subject.split_once('/')?;
    let (plan, tail) = rest.split_once('/')?;
    match kind {
        "node" => Some((plan, Touched::Node(tail))),
        "page" => Some((plan, Touched::Page(tail))),
        _ => None,
    }
}

/// The cursor a window's read starts after: past every key stamped at or after the window's end
///
/// A key is its stamp's part, then `.`, which sorts before `/`, so every key of that stamp sorts
/// before this and every key of an earlier one after it.
///
/// # Arguments
///
/// * `end` - The window's end, in milliseconds since the epoch, not in it
fn window_cursor(end: u64) -> String {
    format!("{:020}/", u64::MAX - end)
}

/// Each plan's activity row for one write, from its subjects' parts: one a plan whose nodes or
/// pages it wrote, by plan name, none for a write that wrote neither
///
/// # Arguments
///
/// * `project` - The project every subject belongs to
/// * `key` - The write's key, its edits' own
/// * `at` - The write's stamp
/// * `author` - Who made the write
/// * `whole_plan` - Whether it wrote the whole plan at once
/// * `pending` - Each subject's part of it
pub(super) fn activity_rows(
    project: &str,
    key: &str,
    at: u64,
    author: &Author,
    whole_plan: bool,
    pending: &[Pending],
) -> Vec<ActivityRow> {
    // each plan's entry, its nodes and pages in the order the write told them
    let mut plans: BTreeMap<&str, Activity> = BTreeMap::new();
    for part in pending {
        // only a node's or a page's subject is a plan's
        let Some((plan, touched)) = touched(&part.subject) else {
            continue;
        };
        // a start writes the node's work record, not the node
        if part
            .edits
            .iter()
            .all(|edit| matches!(edit, Edited::Started { .. }))
        {
            continue;
        }
        let entry = plans.entry(plan).or_insert_with(|| Activity {
            at,
            author: author.clone(),
            whole_plan,
            ..Activity::default()
        });
        match touched {
            Touched::Node(id) => {
                entry.nodes.push(id.to_string());
                // and among the ones it took to done, if it did
                if part.went_done {
                    entry.done.push(id.to_string());
                }
            }
            Touched::Page(path) => entry.pages.push(path.to_string()),
        }
    }
    // each kept as JSON under the write's own key
    plans
        .into_iter()
        .filter_map(|(plan, entry)| match serde_json::to_string(&entry) {
            Ok(record) => Some(ActivityRow {
                project: project.to_string(),
                plan: plan.to_string(),
                key: key.to_string(),
                record,
            }),
            Err(error) => {
                tracing::warn!(project, plan, %error, "a plan's activity couldn't be kept as JSON");
                None
            }
        })
        .collect()
}

impl<B: Backend> Store<B> {
    /// Reads a page of a plan's activity, newest first, after a key if one is named: each entry
    /// with its key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `older` - The key to read the entries after, or `None` for the newest
    /// * `limit` - The most entries to read
    #[instrument(name = "Store::activity_page", skip_all, err(Debug))]
    async fn activity_page(
        &self,
        plan: &PlanName,
        older: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, Activity)>> {
        let rows = self.backend.activity(plan, older, limit).await?;
        rows.into_iter()
            .map(|row| {
                // a row of another plan whose key hashed the same is a collision, not an entry
                anyhow::ensure!(
                    row.project == plan.project && row.plan == plan.plan,
                    "activity row `{}` of `{}/{}` was stored where `{plan}`'s activity is: their \
                     partition keys' hashes collide",
                    row.key,
                    row.project,
                    row.plan
                );
                let entry = serde_json::from_str(&row.record).map_err(|error| {
                    anyhow::anyhow!("activity `{}` of `{plan}`: {error}", row.key)
                })?;
                Ok((row.key, entry))
            })
            .collect()
    }

    /// When a plan was last written, and by whom: its newest write to its nodes or pages, else
    /// its listing, for a plan none of whose writes was kept
    ///
    /// A plan written only before C119, or whose every write's history failed to keep, answers
    /// its listing until its next write. `None` means neither is kept.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::plan_updated", skip_all, err(Debug))]
    pub async fn plan_updated(&self, plan: &PlanName) -> anyhow::Result<Option<Updated>> {
        // its newest entry, if it has one
        let newest = self.activity_page(plan, None, 1).await?;
        if let Some((_, entry)) = newest.into_iter().next() {
            return Ok(Some(Updated {
                at: entry.at,
                author: entry.author,
            }));
        }
        // else its listing
        Ok(self.plan_registered(plan).await.map(Updated::from))
    }

    /// The ids of a plan's nodes done now that a write in a window took to done, in the order
    /// the plan lists them
    ///
    /// A node set back or removed since isn't counted, nor is one moved in done, since its going
    /// done is its old plan's.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `window` - The window, in milliseconds since the epoch, its start in it and its end not
    #[instrument(name = "Store::plan_done", skip_all, err(Debug))]
    pub async fn plan_done(
        &self,
        plan: &PlanName,
        window: Range<u64>,
    ) -> anyhow::Result<Vec<String>> {
        // an empty window holds nothing
        if window.is_empty() {
            return Ok(Vec::new());
        }
        // every node an entry in the window took to done, a page at a time from its end back
        let mut went: HashSet<String> = HashSet::new();
        let mut older = Some(window_cursor(window.end));
        'pages: loop {
            let page = self
                .activity_page(plan, older.as_deref(), ACTIVITY_PAGE)
                .await?;
            let ended = page.len() < ACTIVITY_PAGE;
            older = page.last().map(|(key, _)| key.clone());
            for (_, entry) in page {
                // the first older than its start ends it, since every one after is older still
                if entry.at < window.start {
                    break 'pages;
                }
                went.extend(entry.done);
            }
            if ended || older.is_none() {
                break;
            }
        }
        // nothing taken to done is nothing to read the plan for
        if went.is_empty() {
            return Ok(Vec::new());
        }
        // of those, only the ones done now
        let listing = self.list(plan).await?;
        Ok(listing
            .nodes
            .into_iter()
            .filter(|node| node.spec.status == Some(Status::Done) && went.contains(&node.spec.id))
            .map(|node| node.spec.id)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{Touched, activity_rows, touched, window_cursor};
    use crate::backend::Backend;
    use crate::schema::ActivityRow;
    use crate::store::history::{self, Pending, edit_key};
    use crate::store::{Proposal, Store};
    use reef_api::{Activity, Author, Checked, Edited, HistoryQuery, NodePut, NodeWrite, Writer};
    use reef_model::{Diagnostic, PlanName, Status};
    use std::path::Path;

    /// A check that finds no problem in any write, so a test judges only what it keeps
    ///
    /// # Arguments
    ///
    /// * `_proposal` - The plan as it stands and as the write would leave it
    async fn no_problems(_proposal: Proposal) -> anyhow::Result<Vec<Diagnostic>> {
        Ok(Vec::new())
    }

    /// A user signed in, writing for themselves
    ///
    /// # Arguments
    ///
    /// * `name` - The user's name
    fn user(name: &str) -> Author {
        Author {
            user: Some(name.to_string()),
            since: Some(1),
            by: Writer::User,
        }
    }

    /// A subject's part of a write, which took no node to done
    ///
    /// # Arguments
    ///
    /// * `subject` - The subject's partition
    /// * `edits` - What the write changed of it
    fn part(subject: &str, edits: Vec<Edited>) -> Pending {
        Pending {
            subject: subject.to_string(),
            version: None,
            edits,
            went_done: false,
        }
    }

    /// A node's and a page's subjects name their plan and what of it they are, and no other
    /// subject does
    #[test]
    fn only_nodes_and_pages_are_a_plans() -> anyhow::Result<()> {
        let plan = PlanName::new("reef", "central")?;
        // a node's and a page's, the page's path whole
        let node = history::node_subject(&plan.node("C119")?);
        assert_eq!(touched(&node), Some(("central", Touched::Node("C119"))));
        let page = history::page_subject(&plan, "design/app.md");
        assert_eq!(
            touched(&page),
            Some(("central", Touched::Page("design/app.md")))
        );
        // a queue's, a plan's listing, a project's, the loops' and the users' are none
        for subject in [
            "queue",
            "queue/central",
            "plan/central",
            "project",
            "loops",
            "users",
        ] {
            assert_eq!(touched(subject), None, "{subject}");
        }
        Ok(())
    }

    /// One write keeps an entry a plan whose nodes or pages it wrote, a start and a queue's edit
    /// being none of a plan's
    #[test]
    fn a_write_keeps_an_entry_a_plan() {
        let ada = user("ada");
        let mut done = part("node/central/C1", vec![Edited::Added]);
        done.went_done = true;
        let started = Edited::Started {
            started_at: 5,
            branch: None,
            start: "ec6614e".to_string(),
            again: false,
        };
        let pending = vec![
            done,
            part("node/central/C2", vec![Edited::Body]),
            part("node/central/C3", vec![started]),
            part("page/central/design/a.md", vec![Edited::Text]),
            part("node/side/C9", vec![Edited::Removed]),
            part("queue", vec![Edited::Removed]),
        ];
        let rows = activity_rows("reef", "k", 7, &ada, false, &pending);
        // one row a plan, by name, each under the write's key
        let plans: Vec<(&str, &str)> = rows
            .iter()
            .map(|row| (row.plan.as_str(), row.key.as_str()))
            .collect();
        assert_eq!(plans, [("central", "k"), ("side", "k")]);
        let central: Activity = serde_json::from_str(&rows[0].record).expect("it reads back");
        assert_eq!(
            central,
            Activity {
                at: 7,
                author: ada.clone(),
                whole_plan: false,
                nodes: vec!["C1".to_string(), "C2".to_string()],
                done: vec!["C1".to_string()],
                pages: vec!["design/a.md".to_string()],
            }
        );
        let side: Activity = serde_json::from_str(&rows[1].record).expect("it reads back");
        assert_eq!(side.nodes, ["C9"]);
        // and a write of none of a plan's keeps none
        let queued = activity_rows("reef", "k", 7, &ada, false, &pending[5..]);
        assert!(queued.is_empty());
    }

    /// A window's cursor sorts after every key stamped at or after its end, and before every
    /// key stamped earlier
    #[test]
    fn a_windows_cursor_starts_before_its_end() {
        let cursor = window_cursor(1_000);
        for nonce in [0, u64::MAX] {
            assert!(edit_key(1_000, nonce) < cursor);
            assert!(edit_key(1_001, nonce) < cursor);
            assert!(edit_key(999, nonce) > cursor);
        }
    }

    /// A batch put that marks two nodes done keeps one plan entry naming both, with its author
    /// and the time its edits were kept at
    #[tokio::test]
    async fn a_batch_marking_two_done_keeps_one_entry() -> anyhow::Result<()> {
        // a plan with two nodes, added by ada
        let store = Store::memory();
        store
            .add_project(
                "reef",
                Path::new(env!("CARGO_MANIFEST_DIR")),
                &reef_api::NOBODY,
            )
            .await?;
        let plan = PlanName::new("reef", "central")?;
        store.add_plan(&plan, &reef_api::NOBODY).await?;
        let (ada, bob) = (user("ada"), user("bob"));
        let added = vec![
            NodePut::new(NodeWrite::new("C1", 0), None),
            NodePut::new(NodeWrite::new("C2", 1), None),
        ];
        store.put_checked(&plan, added, &ada, no_problems).await?;
        // bob marks both done in one batch
        let done = |id: &str, order: u64| {
            let mut node = NodeWrite::new(id, order);
            node.spec.status = Some(Status::Done);
            NodePut::new(node, Some(1))
        };
        let batch = vec![done("C1", 0), done("C2", 1)];
        let checked = store.put_checked(&plan, batch, &bob, no_problems).await?;
        assert!(matches!(checked, Checked::Applied { .. }), "{checked:?}");
        // the plan keeps one entry for it, newest, beside the one that added them
        let rows = store.backend().activity(&plan, None, 10).await?;
        assert_eq!(rows.len(), 2);
        let entry: Activity = serde_json::from_str(&rows[0].record)?;
        assert_eq!(entry.author, bob);
        assert_eq!(entry.nodes, ["C1", "C2"]);
        assert_eq!(entry.done, ["C1", "C2"]);
        assert!(!entry.whole_plan && entry.pages.is_empty());
        // at the time, and under the key, its nodes' own edits were kept at
        let node = store
            .node_history(&plan.node("C1")?, &HistoryQuery::default())
            .await?;
        assert_eq!(entry.at, node.edits[0].at);
        let first = store
            .backend()
            .edits("reef", "node/central/C2", None, 1)
            .await?;
        assert_eq!(rows[0].key, first[0].key);
        // and the one that added them took neither to done
        let earlier: Activity = serde_json::from_str(&rows[1].record)?;
        assert_eq!(earlier.author, ada);
        assert_eq!(earlier.nodes, ["C1", "C2"]);
        assert!(earlier.done.is_empty());
        Ok(())
    }

    /// A row stored in a plan's activity under another plan's key is an error, never an entry
    #[tokio::test]
    async fn collided_activity_row_is_an_error() -> anyhow::Result<()> {
        // a row of another plan's, landing in this one's partition as a hash collision would
        let store = Store::memory();
        let plan = PlanName::new("reef", "central")?;
        let stray = ActivityRow {
            project: "reef".to_string(),
            plan: "side".to_string(),
            key: edit_key(5, 0),
            record: serde_json::to_string(&Activity::default())?,
        };
        store.backend().plant_activity(&plan, stray);
        // the last update and a window's done both catch it
        let updated = store
            .plan_updated(&plan)
            .await
            .expect_err("a collided row was read as an update");
        assert!(format!("{updated:#}").contains("collide"), "{updated:#}");
        let done = store
            .plan_done(&plan, 0..u64::MAX)
            .await
            .expect_err("a collided row was read as an entry");
        assert!(format!("{done:#}").contains("collide"), "{done:#}");
        Ok(())
    }
}
