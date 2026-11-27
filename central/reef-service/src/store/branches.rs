//! The branch each plan follows: a row each, in its project's partition
//!
//! - **A plan with no row follows its project's branch**, as every plan did before plans
//!   followed branches: the branch a remote project was registered by, or a local project's
//!   working tree. So `PlanRow` needed no new field, and no fresh data directory.
//! - **A change is judged under the plan's write lock**, as every write to the plan is. The
//!   caller's check is handed the plan as it stands and both branches, builds it over the docs at
//!   each, and the change is written only if the second adds no problem.
//! - **Every checked write reads the branch under the same lock** ([`Proposal::branch`]), so a
//!   write is judged at the branch it lands on, never one a change replaced meanwhile.
//! - **Written over the version read**, or only where no row is, so two changes can't both land.
//! - **Kept in the plan's history**, under `plan/<plan>`, naming who changed it.
//! - **A plan remembers the branches it followed before**, the latest last, so a checkout still
//!   on one keeps working in it and is told where it moved.
//! - **Every row is checked against its full key on read**, so a hash collision is caught rather
//!   than read as another plan's branch.
//!
//! [`Proposal::branch`]: super::Proposal::branch

use reef_api::{Author, Edited, NodeWrite, PageWrite, records};
use reef_model::{Diagnostic, PlanName, Record};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use tracing::instrument;

use super::{Store, history, listed_order};
use crate::backend::{Backend, Expect, Written};
use crate::schema::PlanBranchRow;

/// What a plan's branch row keeps
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptBranch {
    /// The branch it follows, or `None` for its project's own
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The branches it followed before, the latest last, so a checkout still on one finds it
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub was: Vec<String>,
}

/// A plan's branch as the store keeps it: its plan, its record and the row's version
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBranch {
    /// The plan's name within its project
    pub plan: String,
    /// What it keeps
    pub kept: KeptBranch,
    /// The row's version, which a later write names
    pub version: u64,
}

/// A plan as it stands, for a check to judge at the branch it follows and the one it would
#[derive(Debug, Clone)]
pub struct BranchProposal {
    /// The plan's nodes, in written order
    pub records: Vec<Record>,
    /// The plan's pages, by path
    pub pages: Vec<PageWrite>,
    /// The branch it follows now, or `None` for its project's own
    pub from: Option<String>,
    /// The branch it would follow, or `None` for its project's own
    pub to: Option<String>,
}

/// What came of a change to a plan's branch that the store judged
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchSet {
    /// It was written, or left as it was for a plan that follows that branch already
    Applied {
        /// Whether what the plan follows changed
        changed: bool,
    },
    /// The check found problems the plan's docs at the new branch would add, so nothing changed
    Refused(Vec<Diagnostic>),
}

/// A plan's branch row read back, checked against the partition it was read from
///
/// # Arguments
///
/// * `project` - The project whose partition it was read from
/// * `row` - The row
fn read_branch(project: &str, row: PlanBranchRow) -> anyhow::Result<StoredBranch> {
    // a row of another project whose key hashed the same is a collision, not this plan's
    anyhow::ensure!(
        row.project == project,
        "plan branch row `{}/{}` was stored where `{project}`'s plans' branches are: their \
         partition keys' hashes collide",
        row.project,
        row.plan
    );
    // its plan is its key, and the rest its record
    let kept: KeptBranch = serde_json::from_str(&row.record).map_err(|error| {
        anyhow::anyhow!(
            "plan `{project}/{}`'s branch doesn't read as one: {error}",
            row.plan
        )
    })?;
    Ok(StoredBranch {
        plan: row.plan,
        kept,
        version: row.version,
    })
}

/// The row that keeps a plan's branch at a version
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `kept` - What it keeps
/// * `version` - The version it is written at
fn branch_row(plan: &PlanName, kept: &KeptBranch, version: u64) -> anyhow::Result<PlanBranchRow> {
    Ok(PlanBranchRow {
        project: plan.project.clone(),
        plan: plan.plan.clone(),
        version,
        record: serde_json::to_string(kept)?,
    })
}

impl<B: Backend> Store<B> {
    /// Reads the branch each of a project's plans follows, by plan, for each plan that has a row
    ///
    /// A plan with no row follows its project's own branch.
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Store::plan_branches", skip_all, err(Debug))]
    pub async fn plan_branches(
        &self,
        project: &str,
    ) -> anyhow::Result<HashMap<String, StoredBranch>> {
        // every plan's branch sits in its project's partition
        let rows = self.backend.plan_branches(project).await?;
        rows.into_iter()
            .map(|row| read_branch(project, row).map(|stored| (stored.plan.clone(), stored)))
            .collect()
    }

    /// Reads the branch one plan follows, its row and all, if it has a row
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::stored_branch", skip_all, err(Debug))]
    async fn stored_branch(&self, plan: &PlanName) -> anyhow::Result<Option<StoredBranch>> {
        // the project's partition is small, so it is read whole
        Ok(self.plan_branches(&plan.project).await?.remove(&plan.plan))
    }

    /// The branch a plan follows, or `None` for its project's own
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::plan_branch", skip_all, err(Debug))]
    pub async fn plan_branch(&self, plan: &PlanName) -> anyhow::Result<Option<String>> {
        // a plan with no row follows its project's
        Ok(self
            .stored_branch(plan)
            .await?
            .and_then(|stored| stored.kept.branch))
    }

    /// Sets the branch a plan follows, or drops it so the plan follows its project's, unless its
    /// docs at the new branch would add a problem
    ///
    /// The check is handed the plan as it stands and both branches, under the plan's write
    /// lock, so no write to the plan lands between the check and the change. A change keeps an
    /// entry in the plan's history, naming its author.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `branch` - The branch to follow, or `None` for its project's own
    /// * `author` - Who changes it, from the caller's token
    /// * `check` - Finds the problems the plan's docs at the new branch would add
    #[instrument(name = "Store::set_plan_branch", skip_all, err(Debug))]
    pub async fn set_plan_branch<F, Fut>(
        &self,
        plan: &PlanName,
        branch: Option<String>,
        author: &Author,
        check: F,
    ) -> anyhow::Result<BranchSet>
    where
        F: FnOnce(BranchProposal) -> Fut + Send,
        Fut: Future<Output = anyhow::Result<Vec<Diagnostic>>> + Send,
    {
        // one write to this plan at a time, so the check judges the plan the change lands on
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // what it follows now, and a change to that same branch changes nothing
        let stored = self.stored_branch(plan).await?;
        let from = stored
            .as_ref()
            .and_then(|stored| stored.kept.branch.clone());
        if from == branch {
            return Ok(BranchSet::Applied { changed: false });
        }
        // the plan as it stands, in written order, and its pages
        let mut nodes = self.revision(plan, row.current, None).await?;
        listed_order(&mut nodes);
        let pages = self.page_revision(plan, row.pages_current, None).await?;
        let proposal = BranchProposal {
            records: records(plan, nodes.into_iter().map(NodeWrite::from).collect())?,
            pages: pages.into_iter().map(PageWrite::from).collect(),
            from: from.clone(),
            to: branch.clone(),
        };
        // judged at both branches, and nothing written if the new one adds a problem
        let problems = check(proposal).await?;
        if !problems.is_empty() {
            return Ok(BranchSet::Refused(problems));
        }
        // the branch it leaves joins those it followed before, and the one it follows now leaves
        // them, so a checkout still on the old one finds the plan
        let mut was = stored
            .as_ref()
            .map(|stored| stored.kept.was.clone())
            .unwrap_or_default();
        if let Some(old) = &from {
            was.retain(|kept| kept != old);
            was.push(old.clone());
        }
        if let Some(new) = &branch {
            was.retain(|kept| kept != new);
        }
        // written where no row is, or over the version read
        let kept = KeptBranch {
            branch: branch.clone(),
            was,
        };
        let (version, expect) = match &stored {
            Some(stored) => (stored.version + 1, Expect::Version(stored.version)),
            None => (1, Expect::Absent),
        };
        let written = self
            .backend
            .write_plan_branch(branch_row(plan, &kept, version)?, expect)
            .await?;
        if let Written::Refused(refusal) = written {
            anyhow::bail!(
                "plan `{plan}`'s branch was written by another process while it was changed here \
                 ({refusal:?})"
            );
        }
        // what it followed and follows now, kept in its history under the same lock
        let pending = history::Pending {
            subject: history::plan_subject(plan),
            version: None,
            edits: vec![Edited::Branched { from, to: branch }],
        };
        self.keep_edits(&plan.project, author, false, vec![pending])
            .await;
        Ok(BranchSet::Applied { changed: true })
    }
}
