//! Each node's, page's and queue's history: an edit row a write, newest first
//!
//! - **One table for every subject.** A node's edits sit in the partition `node/<plan>/<id>`, a
//!   page's in `page/<plan>/<path>` and a project's queue's in `queue`, so each history is one
//!   read. A node removed and added again keeps one history.
//! - **Kept after the write it records, under the same lock.** A write's entries are kept once
//!   the write has landed, and before the service tells the change stream of it, so a client
//!   that reads again on the change finds them. A history that fails to keep leaves its write
//!   standing, warned of, with a gap where its entries would be: an entry kept first would
//!   outlive a write that then failed, and a write failed after it landed would be tried again
//!   into a stale refusal.
//! - **Keys count down, unique without a read.** Each write takes a stamp from the store's
//!   clock, its time in milliseconds but always past the last one taken, and its key is
//!   `u64::MAX` less that, so a partition reads newest first and a limit takes the newest. The
//!   key ends with a number each store draws at random when it opens, so a store opened again
//!   after a restart, whose clock starts over, never writes over an edit the last one kept in
//!   the same millisecond; two stores' edits within one millisecond sort by those numbers. One
//!   write's entries share a key, each in a partition of its own.
//! - **Read a page at a time, by key.** A page reads one row past its limit, so it knows whether
//!   any older one is kept, and names the key of its last row as the cursor onto the next page,
//!   which reads past it. Keys are unique, so a page never skips or repeats an edit, as a time
//!   could when two stores' edits share a millisecond.
//! - **What changed, rather than the node again.** A node's entry tells its status changed, a
//!   criterion was met or named another test, other keys were edited, or its body was. A put that
//!   left a node as it was keeps none, and nor does a whole-plan write for the nodes it left so.

use reef_api::{
    Author, Edited, HistoryPage, HistoryQuery, Logged, LoopAct, NodePut, NodeRemove, NodeWrite,
    PageEdit, PageWrite, Pane, Registered, UserAct,
};
use reef_model::accept::Criterion;
use reef_model::{Description, NodeName, NodeSpec, PlanName};
use std::collections::{BTreeSet, HashMap};
use tracing::instrument;

use super::{Store, bundles, work::now_ms};
use crate::backend::Backend;
use crate::schema::{
    EditRow, LOOPS_SUBJECT, PROJECT_SUBJECT, QUEUE_SUBJECT, SERVER_PROJECT, USERS_SUBJECT,
};

/// The keys of a node's spec in the order a spec writes them, which an entry names its edited
/// keys in; a key a newer spec adds comes after them
const KEY_ORDER: &[&str] = &[
    "title",
    "optional",
    "group",
    "needs",
    "helps",
    "docs",
    "collapsed",
    "description",
    "decisions",
    "performance",
    "acceptance",
];

/// Picks one part of a node's description: its why, its what or its details
type Part = fn(&Description) -> &Option<String>;

/// One subject's part of a write, to keep in its history once the write lands
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The subject's partition: `node/<plan>/<id>`, `page/<plan>/<path>` or `queue`
    pub subject: String,
    /// The version the write leaves the subject at, or `None` for a removal or a queue's edit
    pub version: Option<u64>,
    /// What the write changed of it
    pub edits: Vec<Edited>,
}

/// The subject a node's edits are kept under, which a node removed and added again shares
///
/// # Arguments
///
/// * `node` - The node's full name
pub fn node_subject(node: &NodeName) -> String {
    format!("node/{}/{}", node.plan.plan, node.id)
}

/// The subject a page's edits are kept under
///
/// # Arguments
///
/// * `plan` - The plan the page is one of
/// * `path` - The page's path
pub fn page_subject(plan: &PlanName, path: &str) -> String {
    format!("page/{}/{path}", plan.plan)
}

/// The subject a plan's listing is kept under
///
/// # Arguments
///
/// * `plan` - The plan's full name
pub fn plan_subject(plan: &PlanName) -> String {
    format!("plan/{}", plan.plan)
}

/// An edit row's sort key: `u64::MAX` less its stamp, padded so the partition sorts by it,
/// newest first, then the store's own random number, which tells two stores' edits apart
///
/// # Arguments
///
/// * `stamp` - The write's stamp
/// * `nonce` - The store's random number
fn edit_key(stamp: u64, nonce: u64) -> String {
    format!("{:020}.{nonce:016x}", u64::MAX - stamp)
}

/// A cursor this store never handed out, which a history page can't be read past
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadCursor(pub String);

impl std::fmt::Display for BadCursor {
    /// Says which cursor was refused, and where a good one comes from
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` is no history's cursor: hand back the `older` a page named",
            self.0
        )
    }
}

impl std::error::Error for BadCursor {}

/// Whether some text is shaped as [`edit_key`] makes a key: twenty digits, a dot, and sixteen
/// lowercase hex digits
///
/// # Arguments
///
/// * `text` - The text a client sent as a cursor
fn is_cursor(text: &str) -> bool {
    // the stamp's part, then the store's number's
    let Some((stamp, nonce)) = text.split_once('.') else {
        return false;
    };
    stamp.len() == 20
        && stamp.bytes().all(|byte| byte.is_ascii_digit())
        && nonce.len() == 16
        && nonce
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// What a put changed of a node: added if it is new, else each part it changed, or nothing for
/// a put that left it as it was
///
/// Where the node falls in its plan's order is no edit of it.
///
/// # Arguments
///
/// * `before` - The node as it stood, or `None` for a node the put adds
/// * `after` - The node as the put writes it
pub fn node_edits(before: Option<&NodeWrite>, after: &NodeWrite) -> Vec<Edited> {
    // a node not there before was added
    let Some(before) = before else {
        return vec![Edited::Added];
    };
    let mut edits = Vec::new();
    // its status, from one to the other
    if before.spec.status != after.spec.status {
        edits.push(Edited::Status {
            from: before.spec.status,
            to: after.spec.status,
        });
    }
    // each criterion marked or naming another test, unless the list itself changed
    let (criteria, listed) = criteria_edits(&before.spec.acceptance, &after.spec.acceptance);
    edits.extend(criteria);
    // every other key edited, in the order a spec writes them
    let keys = spec_keys(&before.spec, &after.spec, listed);
    if !keys.is_empty() {
        edits.push(Edited::Keys { keys });
    }
    // and its body, a file node's page or an inline node's own
    if before.body != after.body || before.spec.body != after.spec.body {
        edits.push(Edited::Body);
    }
    edits
}

/// What changed of a node's criteria: each one marked or naming another test, and whether the
/// list itself changed, which an entry tells as an edited key instead
///
/// A list that gained, lost, reworded or reordered a criterion, or changed one in any other way
/// than its mark and its test, is told as a whole.
///
/// # Arguments
///
/// * `before` - The criteria as they stood
/// * `after` - The criteria as a put writes them
fn criteria_edits(before: &[Criterion], after: &[Criterion]) -> (Vec<Edited>, bool) {
    // criteria added, dropped, reworded or reordered make the list itself an edit
    let same_list = before.len() == after.len()
        && before
            .iter()
            .zip(after)
            .all(|(old, new)| old.kind == new.kind && old.text == new.text);
    if !same_list {
        return (Vec::new(), true);
    }
    let mut edits = Vec::new();
    let mut listed = false;
    for (index, (old, new)) in before.iter().zip(after).enumerate() {
        // one left as it was is no edit
        if old == new {
            continue;
        }
        // only its mark and a test named for it are told as the criterion's own
        let rest_kept = Criterion {
            status: new.status,
            test: new.test.clone(),
            ..old.clone()
        } == *new;
        if !rest_kept || new.test.is_none() && old.test.is_some() {
            listed = true;
            continue;
        }
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        edits.push(Edited::Criterion {
            number,
            text: new.text.clone(),
            from: old.status,
            to: new.status,
            test: (old.test != new.test).then(|| new.test.clone()).flatten(),
        });
    }
    (edits, listed)
}

/// The keys of a node's spec a put edited, other than its status, its criteria and its body,
/// in the order a spec writes them, its description by part
///
/// Each key is compared as JSON, so a key a newer spec adds is told too, after the known ones.
///
/// # Arguments
///
/// * `before` - The spec as it stood
/// * `after` - The spec as a put writes it
/// * `listed` - Whether the criteria's list itself changed
fn spec_keys(before: &NodeSpec, after: &NodeSpec, listed: bool) -> Vec<String> {
    // each spec without what an entry tells apart, as JSON keyed by name
    let bare = |spec: &NodeSpec| {
        let spec = NodeSpec {
            status: None,
            acceptance: Vec::new(),
            body: None,
            description: None,
            nodes: Vec::new(),
            ..spec.clone()
        };
        match serde_json::to_value(spec) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        }
    };
    let (old, new) = (bare(before), bare(after));
    // the known keys in order, then any newer one either side writes, by name
    let newer: BTreeSet<&str> = old
        .keys()
        .chain(new.keys())
        .map(String::as_str)
        .filter(|key| *key != "id" && !KEY_ORDER.contains(key))
        .collect();
    let mut keys = Vec::new();
    for key in KEY_ORDER.iter().copied().chain(newer) {
        match key {
            // a description by its parts, why first
            "description" => {
                let part = |spec: &NodeSpec, pick: Part| {
                    spec.description
                        .as_ref()
                        .and_then(|text| pick(text).clone())
                };
                let parts: [(&str, Part); 3] = [
                    ("why", |text| &text.why),
                    ("what", |text| &text.what),
                    ("details", |text| &text.details),
                ];
                for (name, pick) in parts {
                    if part(before, pick) != part(after, pick) {
                        keys.push(format!("description.{name}"));
                    }
                }
            }
            // the criteria as a whole, only when the list itself changed
            "acceptance" => {
                if listed {
                    keys.push(key.to_string());
                }
            }
            // any other key as JSON
            _ => {
                if old.get(key) != new.get(key) {
                    keys.push(key.to_string());
                }
            }
        }
    }
    keys
}

/// What each put and removal of a batch changes, to keep once the batch lands
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `before` - Each node the batch touches, as it stood, by id
/// * `puts` - The batch's puts
/// * `versions` - The version each put leaves its node at, in the puts' order
/// * `removes` - The batch's removals
pub fn batch_edits(
    plan: &PlanName,
    before: &HashMap<String, NodeWrite>,
    puts: &[NodePut],
    versions: &[u64],
    removes: &[NodeRemove],
) -> anyhow::Result<Vec<Pending>> {
    let mut pending = Vec::with_capacity(puts.len() + removes.len());
    // each put that changed its node, at the version it leaves it at
    for (put, version) in puts.iter().zip(versions) {
        let edits = node_edits(before.get(&put.node.spec.id), &put.node);
        if edits.is_empty() {
            continue;
        }
        let node = NodeName::new(plan.clone(), &put.node.spec.id)?;
        pending.push(Pending {
            subject: node_subject(&node),
            version: Some(*version),
            edits,
        });
    }
    // and each removal
    for remove in removes {
        let node = NodeName::new(plan.clone(), &remove.id)?;
        pending.push(Pending {
            subject: node_subject(&node),
            version: None,
            edits: vec![Edited::Removed],
        });
    }
    Ok(pending)
}

/// What a whole-plan write changes of its nodes, and of its pages if it carries them: each node
/// or page added, changed or removed, and none it left as it was
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `before` - The plan's nodes as they stood, by id
/// * `after` - Every node the write leaves, with the version it leaves it at
/// * `pages_before` - The plan's pages as they stood, by path
/// * `pages_after` - Every page the write leaves with its version, or `None` for a write that
///   leaves the pages as they are
pub fn plan_edits(
    plan: &PlanName,
    before: &HashMap<String, NodeWrite>,
    after: &[(NodeWrite, u64)],
    pages_before: &HashMap<String, PageWrite>,
    pages_after: Option<&[(PageWrite, u64)]>,
) -> anyhow::Result<Vec<Pending>> {
    let mut pending = Vec::new();
    // each node the write adds or changes
    for (node, version) in after {
        let edits = node_edits(before.get(&node.spec.id), node);
        if edits.is_empty() {
            continue;
        }
        let name = NodeName::new(plan.clone(), &node.spec.id)?;
        pending.push(Pending {
            subject: node_subject(&name),
            version: Some(*version),
            edits,
        });
    }
    // each node it leaves out, by id so the history reads the same each time
    let kept: BTreeSet<&str> = after
        .iter()
        .map(|(node, _)| node.spec.id.as_str())
        .collect();
    let mut removed: Vec<&String> = before
        .keys()
        .filter(|id| !kept.contains(id.as_str()))
        .collect();
    removed.sort();
    for id in removed {
        let name = NodeName::new(plan.clone(), id)?;
        pending.push(Pending {
            subject: node_subject(&name),
            version: None,
            edits: vec![Edited::Removed],
        });
    }
    // and its pages the same way, if it carries them
    if let Some(pages) = pages_after {
        for (page, version) in pages {
            let Some(edit) = page_edit(
                pages_before.get(&page.path).map(|old| old.text.as_str()),
                Some(&page.text),
            ) else {
                continue;
            };
            pending.push(Pending {
                subject: page_subject(plan, &page.path),
                version: Some(*version),
                edits: vec![edit],
            });
        }
        let kept: BTreeSet<&str> = pages.iter().map(|(page, _)| page.path.as_str()).collect();
        let mut removed: Vec<&String> = pages_before
            .keys()
            .filter(|path| !kept.contains(path.as_str()))
            .collect();
        removed.sort();
        for path in removed {
            pending.push(Pending {
                subject: page_subject(plan, path),
                version: None,
                edits: vec![Edited::Removed],
            });
        }
    }
    Ok(pending)
}

/// What an edit of a page changes of it: added, its text edited or removed, or nothing for one
/// written again as it was
///
/// # Arguments
///
/// * `before` - The page's text as it stood, or `None` for a page not there
/// * `after` - The page's text as the edit leaves it, or `None` for one taken out
pub fn page_edit(before: Option<&str>, after: Option<&str>) -> Option<Edited> {
    match (before, after) {
        (None, Some(_)) => Some(Edited::Added),
        (Some(old), Some(new)) => (old != new).then_some(Edited::Text),
        (Some(_), None) => Some(Edited::Removed),
        (None, None) => None,
    }
}

/// What a page edit leaves the page's text at, or `None` for one that takes it out
///
/// # Arguments
///
/// * `edit` - The page's edit
pub fn page_text(edit: &PageEdit) -> Option<&str> {
    match edit {
        PageEdit::Put { page, .. } => Some(page.text.as_str()),
        PageEdit::Remove { .. } => None,
    }
}

impl<B: Backend> Store<B> {
    /// Takes a write's stamp: now in milliseconds, but always past the last one taken
    fn stamp(&self) -> u64 {
        // the clock is only held to read and move it, never across a wait
        let mut last = self
            .edit_clock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stamp = now_ms().max(last.saturating_add(1));
        *last = stamp;
        stamp
    }

    /// Keeps a write's entries in their subjects' histories, once the write has landed
    ///
    /// A history that fails to keep is warned of and leaves the write standing, so this never
    /// fails the write it records. It answers the stamp the entries were kept at, if any were.
    ///
    /// # Arguments
    ///
    /// * `project` - The project every subject belongs to
    /// * `author` - Who made the write
    /// * `whole_plan` - Whether it wrote the whole plan at once
    /// * `pending` - Each subject's part of it
    #[instrument(name = "Store::keep_edits", skip_all)]
    pub(super) async fn keep_edits(
        &self,
        project: &str,
        author: &Author,
        whole_plan: bool,
        pending: Vec<Pending>,
    ) -> Option<u64> {
        // a write that changed nothing keeps nothing
        if pending.is_empty() {
            return None;
        }
        // one stamp for the whole write, each subject in a partition of its own
        let at = self.stamp();
        let key = edit_key(at, self.edit_nonce);
        let mut rows = Vec::with_capacity(pending.len());
        for Pending {
            subject,
            version,
            edits,
        } in pending
        {
            let logged = Logged {
                at,
                author: author.clone(),
                version,
                whole_plan,
                edits,
            };
            match serde_json::to_string(&logged) {
                Ok(record) => rows.push(EditRow {
                    project: project.to_string(),
                    subject,
                    key: key.clone(),
                    record,
                }),
                Err(error) => {
                    tracing::warn!(project, %subject, %error, "an edit couldn't be kept as JSON");
                }
            }
        }
        // a bundle at a time, the first failure ending it with the rest left out
        for bundle in bundles(rows, |row| row.subject.len() + row.record.len()) {
            if let Err(error) = self.backend.write_edits(bundle).await {
                tracing::warn!(
                    project,
                    error = %format!("{error:#}"),
                    "keeping a write's history failed, so it stands with a gap in its history"
                );
                return None;
            }
        }
        Some(at)
    }

    /// Keeps an act on a project's loops or sessions in its loops' history, once it has landed
    ///
    /// # Arguments
    ///
    /// * `project` - The project whose loop or session it was
    /// * `author` - Who did it
    /// * `pane` - The pane whose loop or session it was
    /// * `act` - What was done
    #[instrument(name = "Store::keep_loop_act", skip_all)]
    pub(crate) async fn keep_loop_act(
        &self,
        project: &str,
        author: &Author,
        pane: &Pane,
        act: LoopAct,
    ) {
        // one entry in the project's loops' subject, with no version to leave
        let pending = Pending {
            subject: LOOPS_SUBJECT.to_string(),
            version: None,
            edits: vec![Edited::Loop {
                pane: pane.clone(),
                act,
            }],
        };
        self.keep_edits(project, author, false, vec![pending]).await;
    }

    /// Keeps an act on one of reef's users in the users' log, once it has landed
    ///
    /// # Arguments
    ///
    /// * `author` - Who did it, nobody known for a recovery on the service's host
    /// * `name` - The user's name
    /// * `act` - What was done
    #[instrument(name = "Store::keep_user_act", skip_all)]
    pub(crate) async fn keep_user_act(&self, author: &Author, name: &str, act: UserAct) {
        // one entry in the users' subject, under the partition no project has
        let pending = Pending {
            subject: USERS_SUBJECT.to_string(),
            version: None,
            edits: vec![Edited::User {
                name: name.to_string(),
                act,
            }],
        };
        self.keep_edits(SERVER_PROJECT, author, false, vec![pending])
            .await;
    }

    /// Keeps a project's registration, or a plan's listing, in its history, answering when and
    /// by whom it was kept
    ///
    /// # Arguments
    ///
    /// * `project` - The project
    /// * `subject` - The project's subject, or the plan's
    /// * `author` - Who registered it
    pub(super) async fn keep_registered(
        &self,
        project: &str,
        subject: String,
        author: &Author,
    ) -> Option<Registered> {
        let pending = Pending {
            subject,
            version: None,
            edits: vec![Edited::Added],
        };
        // when it was kept, if it was
        let at = self
            .keep_edits(project, author, false, vec![pending])
            .await?;
        Some(Registered {
            at,
            author: author.clone(),
        })
    }

    /// Reads a page of a subject's history, newest first: the newest, or the one older than a
    /// cursor, with the cursor onto the page after it when older entries are kept
    ///
    /// # Arguments
    ///
    /// * `project` - The project the subject belongs to
    /// * `subject` - The subject's partition
    /// * `query` - Which page to read, and how many entries it may hold
    #[instrument(name = "Store::history", skip_all, err(Debug))]
    async fn history(
        &self,
        project: &str,
        subject: &str,
        query: &HistoryQuery,
    ) -> anyhow::Result<HistoryPage> {
        // a cursor this store never handed out is refused, rather than read as a key
        let older = query.older.as_deref();
        if let Some(older) = older
            && !is_cursor(older)
        {
            return Err(BadCursor(older.to_string()).into());
        }
        // one row past the page, which says whether any older one is kept
        let limit = query.page_size();
        let mut rows = self
            .backend
            .edits(project, subject, older, limit + 1)
            .await?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        // the page's last key is the cursor onto the next, if there is a next
        let older = match more {
            true => rows.last().map(|row| row.key.clone()),
            false => None,
        };
        let edits = rows
            .into_iter()
            .map(|row| {
                // a row of another subject whose key hashed the same is a collision, not an edit
                anyhow::ensure!(
                    row.project == project && row.subject == subject,
                    "edit row `{}` of `{}/{}` was stored where `{project}/{subject}`'s history is: \
                     their partition keys' hashes collide",
                    row.key,
                    row.project,
                    row.subject
                );
                serde_json::from_str(&row.record).map_err(|error| {
                    anyhow::anyhow!("edit `{}` of `{project}/{subject}`: {error}", row.key)
                })
            })
            .collect::<anyhow::Result<Vec<Logged>>>()?;
        Ok(HistoryPage { edits, older })
    }

    /// Reads a page of a node's history, newest first, a node removed included
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `query` - Which page to read
    #[instrument(name = "Store::node_history", skip_all, err(Debug))]
    pub async fn node_history(
        &self,
        node: &NodeName,
        query: &HistoryQuery,
    ) -> anyhow::Result<HistoryPage> {
        self.history(&node.plan.project, &node_subject(node), query)
            .await
    }

    /// Reads a page of a page's history, newest first, a page removed included
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan the page is one of
    /// * `path` - The page's path
    /// * `query` - Which page of its history to read
    #[instrument(name = "Store::page_history", skip_all, err(Debug))]
    pub async fn page_history(
        &self,
        plan: &PlanName,
        path: &str,
        query: &HistoryQuery,
    ) -> anyhow::Result<HistoryPage> {
        self.history(&plan.project, &page_subject(plan, path), query)
            .await
    }

    /// Reads a page of a project's queue's history, newest first
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `query` - Which page to read
    #[instrument(name = "Store::queue_history", skip_all, err(Debug))]
    pub async fn queue_history(
        &self,
        project: &str,
        query: &HistoryQuery,
    ) -> anyhow::Result<HistoryPage> {
        self.history(project, QUEUE_SUBJECT, query).await
    }

    /// Reads a page of a project's loops' and sessions' history, newest first
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `query` - Which page to read
    #[instrument(name = "Store::loop_history", skip_all, err(Debug))]
    pub async fn loop_history(
        &self,
        project: &str,
        query: &HistoryQuery,
    ) -> anyhow::Result<HistoryPage> {
        self.history(project, LOOPS_SUBJECT, query).await
    }

    /// Reads a page of the log of acts on reef's users, newest first
    ///
    /// # Arguments
    ///
    /// * `query` - Which page to read
    #[instrument(name = "Store::user_history", skip_all, err(Debug))]
    pub async fn user_history(&self, query: &HistoryQuery) -> anyhow::Result<HistoryPage> {
        self.history(SERVER_PROJECT, USERS_SUBJECT, query).await
    }

    /// When a subject was registered, and by whom: its newest `added` entry on its newest page
    ///
    /// A history that can't be read is warned of, and reads as none, so a listing never fails
    /// for it.
    ///
    /// # Arguments
    ///
    /// * `project` - The project
    /// * `subject` - The project's subject, or a plan's
    async fn registration(&self, project: &str, subject: &str) -> Option<Registered> {
        let page = match self
            .history(project, subject, &HistoryQuery::default())
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(project, subject, error = %format!("{error:#}"), "a registration's history wasn't read");
                return None;
            }
        };
        // the newest that registered it, since it may have been registered again
        page.edits
            .into_iter()
            .find(|logged| logged.edits.contains(&Edited::Added))
            .map(|logged| Registered {
                at: logged.at,
                author: logged.author,
            })
    }

    /// When a project was registered, and by whom, if its history says
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Store::registered", skip_all)]
    pub async fn registered(&self, project: &str) -> Option<Registered> {
        self.registration(project, PROJECT_SUBJECT).await
    }

    /// When a plan was listed, and by whom, if its history says
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::plan_registered", skip_all)]
    pub async fn plan_registered(&self, plan: &PlanName) -> Option<Registered> {
        self.registration(&plan.project, &plan_subject(plan)).await
    }
}

#[cfg(test)]
mod tests {
    use super::{edit_key, is_cursor, node_edits, page_edit};
    use reef_api::{Edited, NodeWrite};
    use reef_model::accept::{Criterion, Kind};
    use reef_model::{Description, Met, NodeSpec, Status};

    /// A node with a spec and a body
    ///
    /// # Arguments
    ///
    /// * `spec` - Its spec
    /// * `body` - Its body
    fn node(spec: NodeSpec, body: &str) -> NodeWrite {
        NodeWrite {
            spec,
            body: body.to_string(),
            order: 0,
        }
    }

    /// A criterion of a kind, with its text and its mark
    ///
    /// # Arguments
    ///
    /// * `text` - What it says
    /// * `status` - Whether it is met
    fn criterion(text: &str, status: Option<Met>) -> Criterion {
        Criterion {
            kind: Kind::Integration,
            text: text.to_string(),
            status,
            ..Criterion::default()
        }
    }

    /// A node's edit is told by its status, its criteria, its other keys and its body, and a
    /// put that changed nothing but its order tells nothing
    #[test]
    fn node_edits_say_what_changed() {
        let spec = NodeSpec {
            id: "C50".to_string(),
            title: Some("C50 Who did what".to_string()),
            needs: vec!["C48".to_string()],
            description: Some(Description {
                why: Some("No authors".to_string()),
                what: Some("Authors".to_string()),
                details: None,
            }),
            acceptance: vec![criterion("A", None), criterion("B", None)],
            ..NodeSpec::default()
        };
        let before = node(spec.clone(), "Body");
        // a new node is added
        assert_eq!(node_edits(None, &before), vec![Edited::Added]);
        // its order alone is no edit
        let moved = NodeWrite {
            order: 9,
            ..before.clone()
        };
        assert!(node_edits(Some(&before), &moved).is_empty());
        // its status, a criterion met with a test, a key and a description's part, and its body
        let mut acceptance = spec.acceptance.clone();
        acceptance[1].status = Some(Met::Met);
        acceptance[1].test = Some("tests/a.rs::b".to_string());
        let after = node(
            NodeSpec {
                status: Some(Status::Done),
                needs: vec!["C48".to_string(), "C34".to_string()],
                description: Some(Description {
                    why: Some("Nobody named".to_string()),
                    ..spec.description.clone().unwrap()
                }),
                acceptance,
                ..spec.clone()
            },
            "Body, longer",
        );
        assert_eq!(
            node_edits(Some(&before), &after),
            vec![
                Edited::Status {
                    from: None,
                    to: Some(Status::Done)
                },
                Edited::Criterion {
                    number: 2,
                    text: "B".to_string(),
                    from: None,
                    to: Some(Met::Met),
                    test: Some("tests/a.rs::b".to_string()),
                },
                Edited::Keys {
                    keys: vec!["needs".to_string(), "description.why".to_string()]
                },
                Edited::Body,
            ]
        );
        // a criterion added is the list edited, told as a key
        let added = node(
            NodeSpec {
                acceptance: vec![
                    criterion("A", None),
                    criterion("B", None),
                    criterion("C", None),
                ],
                ..spec.clone()
            },
            "Body",
        );
        assert_eq!(
            node_edits(Some(&before), &added),
            vec![Edited::Keys {
                keys: vec!["acceptance".to_string()]
            }]
        );
    }

    /// A page is added, its text edited or removed, and one written again as it was tells
    /// nothing
    #[test]
    fn page_edits_say_what_changed() {
        assert_eq!(page_edit(None, Some("a")), Some(Edited::Added));
        assert_eq!(page_edit(Some("a"), Some("b")), Some(Edited::Text));
        assert_eq!(page_edit(Some("a"), Some("a")), None);
        assert_eq!(page_edit(Some("a"), None), Some(Edited::Removed));
    }

    /// Keys count down as stamps go up, so a partition sorts newest first whatever store wrote
    /// them, and two stores' keys of one stamp differ
    #[test]
    fn keys_sort_newest_first() {
        let older = edit_key(1_700_000_000_000, u64::MAX);
        let newer = edit_key(1_700_000_000_001, 0);
        assert!(newer < older);
        assert_eq!(older.len(), 37);
        assert_ne!(edit_key(5, 1), edit_key(5, 2));
    }

    /// Only a key shaped as the store makes one is a cursor
    #[test]
    fn only_keys_are_cursors() {
        // every key the store makes is one
        assert!(is_cursor(&edit_key(1_700_000_000_000, 0xab)));
        assert!(is_cursor(&edit_key(0, u64::MAX)));
        // anything else is not
        for bad in [
            "",
            "nope",
            "00000000000000000001",
            "00000000000000000001.",
            "0000000000000000001.00000000000000ab",
            "00000000000000000001.00000000000000AB",
            "00000000000000000001.00000000000000ab.",
            "0000000000000000000x.00000000000000ab",
        ] {
            assert!(!is_cursor(bad), "{bad}");
        }
    }
}
