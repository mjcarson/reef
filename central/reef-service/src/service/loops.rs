//! The service's terminal loops: which pane feeds a project, the node its loop holds, and
//! whether it is paused
//!
//! - **A loop feeds a registered project**, or one listed plan of it. A start naming a project
//!   that isn't registered, or a plan it doesn't list, is refused before anything is read.
//! - **A node held is in its plan.** A hold naming a node its plan doesn't have is
//!   [`LoopEdited::NotInPlan`], and writes nothing. Whether the node is the loop's own project's
//!   or plan's, and whether another loop holds it, the store judges under the loops' lock.
//! - **Each loop written is a change**, [`Change::LoopChanged`], to the project it feeds. An edit
//!   that wrote nothing sends none.

use reef_api::{Change, EditLoop, Loop, LoopEdit, LoopEdited};
use reef_model::PlanName;
use tracing::instrument;

use super::Service;
use crate::backend::Backend;
use crate::store::NotRegistered;

impl<B: Backend> Service<B> {
    /// Lists every loop, by pane
    #[instrument(name = "Service::loops", skip_all, err(Debug))]
    pub async fn loops(&self) -> anyhow::Result<Vec<Loop>> {
        self.store.loops().await
    }

    /// Lists the loops feeding one project, by pane
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Service::project_loops", skip_all, err(Debug))]
    pub async fn project_loops(&self, project: &str) -> anyhow::Result<Vec<Loop>> {
        // a project that isn't registered has no loops
        if self.store.project(project).await?.is_none() {
            return Err(NotRegistered(project.to_string()).into());
        }
        let mut loops = self.store.loops().await?;
        loops.retain(|feeding| feeding.project == project);
        Ok(loops)
    }

    /// Changes a pane's loop: starts, pauses or stops it, hands it to a session, has it take or
    /// let go of a node, or records the node whose end it acted on
    ///
    /// # Arguments
    ///
    /// * `edit` - The pane, and the change to its loop
    #[instrument(name = "Service::edit_loop", skip_all, err(Debug))]
    pub async fn edit_loop(&self, edit: EditLoop) -> anyhow::Result<LoopEdited> {
        match &edit.edit {
            // a start names a registered project, and a plan it lists if it names one
            LoopEdit::Start {
                project,
                plan: Some(plan),
                ..
            } => self.listed(&PlanName::new(project, plan)?).await?,
            LoopEdit::Start {
                project,
                plan: None,
                ..
            } if self.store.project(project).await?.is_none() => {
                return Err(NotRegistered(project.clone()).into());
            }
            // a node held is one its plan has
            LoopEdit::Hold { node: Some(node) }
                if self.store.get(&node.plan, &node.id).await?.is_none() =>
            {
                return Ok(LoopEdited::NotInPlan { node: node.clone() });
            }
            _ => {}
        }
        // changed under the loops' lock, and told to the change stream once it is
        let edited = self.store.edit_loop(&edit.pane, edit.edit).await?;
        match &edited {
            LoopEdited::Started { r#loop }
            | LoopEdited::Resumed { r#loop }
            | LoopEdited::Edited { r#loop }
            | LoopEdited::Stopped { r#loop } => self.publish(Change::LoopChanged {
                project: r#loop.project.clone(),
                pane: r#loop.pane.clone(),
            }),
            // nothing was written, so nothing is told
            LoopEdited::NoLoop { .. }
            | LoopEdited::Busy { .. }
            | LoopEdited::Held { .. }
            | LoopEdited::NotInLoop { .. }
            | LoopEdited::NotInPlan { .. }
            | LoopEdited::ActedAlready { .. } => {}
        }
        Ok(edited)
    }
}
