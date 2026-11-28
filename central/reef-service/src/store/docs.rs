//! The docs each plan names: a folder or a summary its book is read from, a row each, in its
//! project's partition (C122)
//!
//! - **A plan with no row names no docs**, so its book is its root node, then its pages, as
//!   every plan's was before plans named docs. So `PlanRow` needed no new field, and the table
//!   joined the schema last, so no data directory was replaced.
//! - **A change is judged under the plan's write lock**, as every write to the plan is. The
//!   caller's check is handed the plan as it stands and both names, reads the book at each, and
//!   the change is written only if the second adds no problem.
//! - **Every checked write reads the docs under the same lock** ([`Proposal::docs`]), so a page
//!   edit that takes a chapter away is judged against the docs the plan names as it lands.
//! - **Written over the version read**, or only where no row is, so two changes can't both land.
//! - **Kept in the plan's history**, under `plan/<plan>`, naming who changed it. Like a branch
//!   change, it keeps no activity entry, since it writes none of the plan's nodes or pages.
//! - **Every row is checked against its full key on read**, so a hash collision is caught rather
//!   than read as another plan's docs.
//!
//! [`Proposal::docs`]: super::Proposal::docs

use reef_api::{Author, Edited, NodeWrite, PageWrite, records};
use reef_model::{Diagnostic, PlanName, Record};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use tracing::instrument;

use super::{Store, history, listed_order};
use crate::backend::{Backend, Expect, Written};
use crate::schema::PlanDocsRow;

/// What a plan's docs row keeps
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptDocs {
    /// The folder or summary it names, from the root of its docs, or `None` for none
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
}

/// A plan's docs as the store keeps them: its plan, its record and the row's version
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDocs {
    /// The plan's name within its project
    pub plan: String,
    /// What it keeps
    pub kept: KeptDocs,
    /// The row's version, which a later write names
    pub version: u64,
}

/// A plan as it stands, for a check to judge with the docs it names and the ones it would
#[derive(Debug, Clone)]
pub struct DocsProposal {
    /// The plan's nodes, in written order
    pub records: Vec<Record>,
    /// The plan's pages, by path
    pub pages: Vec<PageWrite>,
    /// The branch it follows, read under the same lock, whose docs the check reads, or `None`
    /// for its project's own
    pub branch: Option<String>,
    /// The docs it names now, or `None` for none
    pub from: Option<String>,
    /// The docs it would name, or `None` for none
    pub to: Option<String>,
}

/// What came of a change to a plan's docs that the store judged
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocsSet {
    /// It was written, or left as it was for a plan that names those docs already
    Applied {
        /// Whether what the plan names changed
        changed: bool,
    },
    /// The check found problems the new docs would add, so nothing changed
    Refused(Vec<Diagnostic>),
}

/// A plan's docs row read back, checked against the partition it was read from
///
/// # Arguments
///
/// * `project` - The project whose partition it was read from
/// * `row` - The row
fn read_docs(project: &str, row: PlanDocsRow) -> anyhow::Result<StoredDocs> {
    // make sure the row is this project's, since another project's row whose key hashed the
    // same is a collision rather than this plan's docs
    anyhow::ensure!(
        row.project == project,
        "plan docs row `{}/{}` was stored where `{project}`'s plans' docs are: their partition \
         keys' hashes collide",
        row.project,
        row.plan
    );
    // parse what the row keeps from its JSON record, since the plan's name is its key
    let kept: KeptDocs = serde_json::from_str(&row.record).map_err(|error| {
        anyhow::anyhow!(
            "plan `{project}/{}`'s docs don't read as a record of them: {error}",
            row.plan
        )
    })?;
    // pair what it keeps with its plan and the version a later write names
    Ok(StoredDocs {
        plan: row.plan,
        kept,
        version: row.version,
    })
}

/// The row that keeps a plan's docs at a version
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `kept` - What it keeps
/// * `version` - The version it is written at
fn docs_row(plan: &PlanName, kept: &KeptDocs, version: u64) -> anyhow::Result<PlanDocsRow> {
    // build the row under the plan's key, with what it keeps written as JSON
    Ok(PlanDocsRow {
        project: plan.project.clone(),
        plan: plan.plan.clone(),
        version,
        record: serde_json::to_string(kept)?,
    })
}

impl<B: Backend> Store<B> {
    /// Reads the docs each of a project's plans names, by plan, for each plan that has a row
    ///
    /// A plan with no row names no docs.
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Store::plans_docs", skip_all, err(Debug))]
    pub async fn plans_docs(&self, project: &str) -> anyhow::Result<HashMap<String, StoredDocs>> {
        // read every docs row in the project's partition, where all of its plans' docs sit
        let rows = self.backend.plan_docs(project).await?;
        // check each row against the project and key it by its plan's name
        rows.into_iter()
            .map(|row| read_docs(project, row).map(|stored| (stored.plan.clone(), stored)))
            .collect()
    }

    /// Reads the docs one plan names, its row and all, if it has a row
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::stored_docs", skip_all, err(Debug))]
    async fn stored_docs(&self, plan: &PlanName) -> anyhow::Result<Option<StoredDocs>> {
        // read the whole partition and take this plan's row, since a project's partition is small
        Ok(self.plans_docs(&plan.project).await?.remove(&plan.plan))
    }

    /// The docs a plan names, or `None` for none
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::plan_docs", skip_all, err(Debug))]
    pub async fn plan_docs(&self, plan: &PlanName) -> anyhow::Result<Option<String>> {
        // take the docs the plan's row names, since a plan with no row names none
        Ok(self
            .stored_docs(plan)
            .await?
            .and_then(|stored| stored.kept.docs))
    }

    /// Names a plan's docs, or clears them, unless reading the new ones would add a problem
    ///
    /// The check is handed the plan as it stands and both names, under the plan's write lock,
    /// so no write to the plan lands between the check and the change. A change keeps an entry
    /// in the plan's history, naming its author.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `docs` - The folder or summary to name, or `None` for none
    /// * `author` - Who changes it, from the caller's token
    /// * `check` - Finds the problems the new docs would add
    #[instrument(name = "Store::set_plan_docs", skip_all, err(Debug))]
    pub async fn set_plan_docs<F, Fut>(
        &self,
        plan: &PlanName,
        docs: Option<String>,
        author: &Author,
        check: F,
    ) -> anyhow::Result<DocsSet>
    where
        F: FnOnce(DocsProposal) -> Fut + Send,
        Fut: Future<Output = anyhow::Result<Vec<Diagnostic>>> + Send,
    {
        // take the plan's write lock so the check judges the plan this change lands on
        let _write = self.writing(plan).await;
        // read the plan's row for the revisions of its nodes and pages
        let row = self.plan_row(plan).await?;
        // read what the plan names now, and stop early if the change names those same docs
        let stored = self.stored_docs(plan).await?;
        let from = stored.as_ref().and_then(|stored| stored.kept.docs.clone());
        if from == docs {
            return Ok(DocsSet::Applied { changed: false });
        }
        // read the plan's nodes as they stand, in written order, and its pages
        let mut nodes = self.revision(plan, row.current, None).await?;
        listed_order(&mut nodes);
        let pages = self.page_revision(plan, row.pages_current, None).await?;
        // hand the check the plan, the branch its docs are read at, and both names
        let proposal = DocsProposal {
            records: records(plan, nodes.into_iter().map(NodeWrite::from).collect())?,
            pages: pages.into_iter().map(PageWrite::from).collect(),
            branch: self.plan_branch(plan).await?,
            from: from.clone(),
            to: docs.clone(),
        };
        // ask the check what problems the new docs add, and refuse the change if they add any
        let problems = check(proposal).await?;
        if !problems.is_empty() {
            return Ok(DocsSet::Refused(problems));
        }
        // keep the new docs at the next version, expecting no row or the version read so two
        // changes can't both land
        let kept = KeptDocs { docs: docs.clone() };
        let (version, expect) = match &stored {
            Some(stored) => (stored.version + 1, Expect::Version(stored.version)),
            None => (1, Expect::Absent),
        };
        // write the row if what is stored under its key is still what was read
        let written = self
            .backend
            .write_plan_docs(docs_row(plan, &kept, version)?, expect)
            .await?;
        // fail if another process wrote the row after it was read
        if let Written::Refused(refusal) = written {
            anyhow::bail!(
                "plan `{plan}`'s docs were written by another process while they were changed \
                 here ({refusal:?})"
            );
        }
        // describe the change as an edit to the plan, naming what it named and names now
        let pending = history::Pending {
            subject: history::plan_subject(plan),
            version: None,
            edits: vec![Edited::DocsNamed { from, to: docs }],
            went_done: false,
        };
        // keep that edit in the plan's history under its author while still holding the lock
        self.keep_edits(&plan.project, author, false, vec![pending])
            .await;
        Ok(DocsSet::Applied { changed: true })
    }
}
