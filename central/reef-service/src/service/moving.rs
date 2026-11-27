//! Nodes moved from one plan of a project to another, checked on both plans
//!
//! - **A move is checked as every write is**, on both plans, each at the branch it follows:
//!   refused if it adds a problem to either, such as a need left across the two, so a subtree
//!   of needs moves whole or not at all.
//! - **Both plans' followers hear of it** as a whole plan written, and both queues' as their
//!   queues' changes, where a node moved was queued.

use reef_api::{Change, MoveNodes, Moved, Renumbered};
use reef_model::PlanName;
use tracing::instrument;

use super::{Asker, Service, added_at};
use crate::backend::Backend;
use crate::store::{MoveProposal, MoveSet};

impl<B: Backend> Service<B> {
    /// Moves nodes of a plan to another plan of its project, everything of theirs with them,
    /// unless that would add a problem to either plan
    ///
    /// # Arguments
    ///
    /// * `from` - The plan the nodes leave
    /// * `request` - The nodes, the plan they join, and the group a node joins there whose own
    ///   stays behind
    /// * `asker` - Who moves them, from the caller's token
    #[instrument(name = "Service::move_nodes", skip_all, err(Debug))]
    pub async fn move_nodes(
        &self,
        from: &PlanName,
        request: MoveNodes,
        asker: &Asker<'_>,
    ) -> anyhow::Result<Moved> {
        // the plan they join is the same project's
        let to = PlanName::new(from.project.clone(), request.to)?;
        let project = self.project_named(&from.project).await?;
        let sections = self.sections.clone();
        // judged on both plans, each at the branch it follows as its lock reads it
        let check = move |proposal: MoveProposal| async move {
            let mut problems = added_at(project.clone(), sections.clone(), proposal.from).await?;
            problems.extend(added_at(project, sections, proposal.to).await?);
            Ok(problems)
        };
        let set = self
            .store
            .move_nodes(
                from,
                &to,
                &request.ids,
                request.group.as_deref(),
                &asker.author(),
                check,
            )
            .await?;
        Ok(match set {
            MoveSet::Applied {
                nodes,
                regrouped,
                threads,
                from_revision,
                to_revision,
                queued,
            } => {
                // both plans' followers read them again, and both queues' where one moved
                for (plan, revision) in [(from, from_revision), (&to, to_revision)] {
                    self.publish(Change::PlanWritten {
                        plan: plan.clone(),
                        revision,
                    });
                    if queued {
                        self.publish(Change::QueueChanged {
                            project: plan.project.clone(),
                            plan: Some(plan.clone()),
                        });
                    }
                }
                Moved::Applied {
                    nodes,
                    regrouped,
                    threads: threads
                        .into_iter()
                        .map(|(from, to)| Renumbered { from, to })
                        .collect(),
                }
            }
            MoveSet::NotInPlan(ids) => Moved::NotInPlan { ids },
            MoveSet::Taken(ids) => Moved::Taken { ids },
            MoveSet::Held { node, pane } => Moved::Held { node, pane },
            MoveSet::Refused(problems) => Moved::Refused(problems),
        })
    }
}
