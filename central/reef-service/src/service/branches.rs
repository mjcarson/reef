//! The branch each plan follows: set, changed or dropped, and checked as every write is
//!
//! - **A plan follows a branch of its project's repo**, whose docs it reads: a remote project's
//!   clone fetches it, and a local project's plan reads a worktree that has it checked out, else
//!   its tip. A plan that follows none reads its project's own, as every plan did before.
//! - **A branch is there before a plan follows it.** A remote project's clone fetches a branch
//!   it lacks first, and a branch the remote hasn't is [`Branched::NoBranch`], as is one a local
//!   project's repo hasn't. Either changes nothing.
//! - **A change that would break the plan is refused.** The plan is built over its docs at the
//!   branch it follows and at the new one, under its write lock, and a change that would add a
//!   problem is [`Branched::Refused`] with check's messages.
//! - **A change is told to the change stream** as [`Change::PlanBranched`], once it is kept in
//!   the plan's history.

use reef_api::{Branched, Change};
use reef_model::PlanName;
use tracing::instrument;

use super::{Asker, Service, added_across};
use crate::backend::Backend;
use crate::remote::{self, BadRemote};
use crate::store::{BranchSet, NotListed};
use crate::tree::{self, Docs};

impl<B: Backend> Service<B> {
    /// Sets the branch a plan follows, or drops it so the plan follows its project's, unless
    /// the repo has no such branch or the plan's docs there would add a problem
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `branch` - The branch to follow, without `refs/heads/`, or `None` for its project's own
    /// * `asker` - Who changes it, from the caller's token
    #[instrument(name = "Service::set_plan_branch", skip_all, err(Debug))]
    pub async fn set_plan_branch(
        &self,
        plan: &PlanName,
        branch: Option<String>,
        asker: &Asker<'_>,
    ) -> anyhow::Result<Branched> {
        // a plan its project lists, of a project that is registered
        let project = self.project_named(&plan.project).await?;
        let listed = self.store.plans(&plan.project).await?;
        if !listed.iter().any(|info| info.name == *plan) {
            return Err(NotListed(plan.clone()).into());
        }
        // a branch named must be one git keeps, and one the repo has
        if let Some(branch) = &branch {
            remote::check_branch(branch).await?;
            if let Some(reason) = self.lacks(&project, branch).await? {
                return Ok(Branched::NoBranch {
                    branch: branch.clone(),
                    reason,
                });
            }
        }
        // the project again, holding any branch its clone just fetched
        let project = self.project_named(&plan.project).await?;
        let sections = self.sections.clone();
        // judged under the plan's write lock, at the branch it follows and at the new one
        let set = self
            .store
            .set_plan_branch(
                plan,
                branch.clone(),
                &asker.author(),
                move |proposal| async move {
                    let from = Docs::of(&project, proposal.from.as_deref())?;
                    let to = Docs::of(&project, proposal.to.as_deref())?;
                    added_across(from, to, sections, proposal).await
                },
            )
            .await?;
        match set {
            BranchSet::Refused(problems) => Ok(Branched::Refused(problems)),
            BranchSet::Applied { changed } => {
                // told to the change stream once it is kept, if it changed what the plan follows
                if changed {
                    self.publish(Change::PlanBranched {
                        plan: plan.clone(),
                        branch: branch.clone(),
                    });
                }
                // and the plan as its listing names it now
                let info = self
                    .store
                    .plans(&plan.project)
                    .await?
                    .into_iter()
                    .find(|info| info.name == *plan)
                    .ok_or_else(|| NotListed(plan.clone()))?;
                Ok(Branched::Applied {
                    plan: info,
                    changed,
                })
            }
        }
    }

    /// The branches a project's repo has, which a plan can follow: a remote project's as its
    /// remote lists them now, a local one's as its repo keeps them
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Service::branches", skip_all, err(Debug))]
    pub async fn branches(&self, project: &str) -> anyhow::Result<Vec<String>> {
        // a project that is registered, remote or local
        let project = self.project_named(project).await?;
        match &project.remote {
            // the remote's own, asked of it, since its clone keeps only the branches followed
            Some(remote) => remote::heads(&remote.url).await,
            // the repo's own, read off the async threads
            None => {
                let repo = project.repo.clone();
                tokio::task::spawn_blocking(move || tree::local_branches(&repo)).await?
            }
        }
    }

    /// Why a project's repo can't give a plan a branch, or `None` if it can
    ///
    /// A remote project's clone that hasn't fetched the branch fetches it now, so its plan's
    /// docs can be read there at once.
    ///
    /// # Arguments
    ///
    /// * `project` - The project, as the store lists it
    /// * `branch` - The branch, its name checked already
    #[instrument(name = "Service::lacks", skip_all, err(Debug))]
    async fn lacks(
        &self,
        project: &reef_api::Project,
        branch: &str,
    ) -> anyhow::Result<Option<String>> {
        match &project.remote {
            // a remote project's branch is fetched into its clone, if it isn't there already
            Some(remote) => {
                if remote.commit_of(Some(branch)).is_some() {
                    return Ok(None);
                }
                match self.fetch_branch(&project.name, branch).await {
                    Ok(()) => Ok(None),
                    Err(error) if error.is::<BadRemote>() => Err(error),
                    // git's own failure already names the branch and the remote
                    Err(error) => Ok(Some(format!("{error:#}"))),
                }
            }
            // a local project's is a ref its repo has
            None => {
                let repo = project.repo.clone();
                let wanted = branch.to_string();
                let has =
                    tokio::task::spawn_blocking(move || tree::has_branch(&repo, &wanted)).await??;
                Ok((!has).then(|| {
                    format!(
                        "the repo at {} has no branch `{branch}`",
                        project.repo.display()
                    )
                }))
            }
        }
    }
}
