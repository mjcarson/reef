//! The service's work records: a node started where its work happens, and each capture's SHAs
//!
//! - **A node started is in its plan.** A start of a node its plan doesn't have is
//!   [`Started::NotInPlan`], and writes nothing. A record outlives its node, so a node removed
//!   since it started can still be read, and captured.
//! - **The service reads no checkout.** What a start or a capture read comes from the client
//!   that read it, in the node's own checkout, so the service is the same once it is shared.
//! - **Each record written is a change**, [`Change::WorkRecorded`], naming its node and the
//!   version it is now at. A start or a capture that wrote nothing sends none. A capture kept as
//!   a revision is [`Change::RevisionCaptured`] too, naming the revision.
//! - **A node's revisions are read where its plan is listed,** as its record is, whether or not
//!   the node is still in its plan.

use reef_api::Change;
use reef_model::{NodeName, PlanName};
use tracing::instrument;

use super::Service;
use crate::backend::Backend;
use crate::store::{
    AddCapture, Captured, FileDiff, NotListed, NotRegistered, Revision, StartWork, Started,
    WorkRecord,
};

impl<B: Backend> Service<B> {
    /// Lists the work records of a project's nodes, by node, across its plans
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Service::work", skip_all, err(Debug))]
    pub async fn work(&self, project: &str) -> anyhow::Result<Vec<WorkRecord>> {
        // a project that isn't registered has no records
        if self.store.project(project).await?.is_none() {
            return Err(NotRegistered(project.to_string()).into());
        }
        self.store.work(project).await
    }

    /// Gets one node's work record, if it was started
    ///
    /// The node need not be in its plan any more, but its plan must be listed.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Service::work_of", skip_all, err(Debug))]
    pub async fn work_of(&self, node: &NodeName) -> anyhow::Result<Option<WorkRecord>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(&node.plan).await?;
        self.store.work_of(node).await
    }

    /// Records a node's start: where its work happens and what it started from
    ///
    /// The node must be in its plan, or the start is [`Started::NotInPlan`]. One started
    /// already is started again only by a start naming its record's version.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `start` - What the start read in the node's checkout, and the version it replaces
    #[instrument(name = "Service::start_work", skip_all, err(Debug))]
    pub async fn start_work(&self, node: &NodeName, start: StartWork) -> anyhow::Result<Started> {
        // a node its plan doesn't have can't be started, and a plan not listed is refused
        if self.store.get(&node.plan, &node.id).await?.is_none() {
            return Ok(Started::NotInPlan { node: node.clone() });
        }
        // written under the project's work lock, and told to the change stream once it is
        let started = self.store.start_work(node, start).await?;
        if let Started::Recorded { record } = &started {
            self.publish(Change::WorkRecorded {
                node: node.clone(),
                version: record.version,
            });
        }
        Ok(started)
    }

    /// Keeps a capture as its node's next revision, if it was taken from the node's start and
    /// changed anything, and adds the SHAs it read to the node's record
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `add` - The start the capture was taken from, what it read, and its files
    #[instrument(name = "Service::add_capture", skip_all, err(Debug))]
    pub async fn add_capture(&self, node: &NodeName, add: AddCapture) -> anyhow::Result<Captured> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(&node.plan).await?;
        // written under the project's work lock, and told to the change stream once it is
        let captured = self.store.add_capture(node, add).await?;
        match &captured {
            Captured::Added { record, revision } => {
                self.publish(Change::RevisionCaptured {
                    node: node.clone(),
                    revision: revision.revision,
                });
                self.publish(Change::WorkRecorded {
                    node: node.clone(),
                    version: record.version,
                });
            }
            Captured::Unchanged { record, .. } => self.publish(Change::WorkRecorded {
                node: node.clone(),
                version: record.version,
            }),
            Captured::NotStarted { .. } | Captured::Restarted { .. } => {}
        }
        Ok(captured)
    }

    /// Lists a node's revisions, oldest first, each with its files but not their hunks
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Service::revisions", skip_all, err(Debug))]
    pub async fn revisions(&self, node: &NodeName) -> anyhow::Result<Vec<Revision>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(&node.plan).await?;
        self.store.revisions(node).await
    }

    /// Gets one of a node's revisions, if it has one of that number
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `revision` - The revision's number
    #[instrument(name = "Service::node_revision", skip_all, err(Debug))]
    pub async fn node_revision(
        &self,
        node: &NodeName,
        revision: u32,
    ) -> anyhow::Result<Option<Revision>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(&node.plan).await?;
        self.store.node_revision(node, revision).await
    }

    /// Gets one file of a node's revision, its hunks and all, if the revision lists it
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `revision` - The revision's number
    /// * `path` - The path the file is shown under
    #[instrument(name = "Service::revision_file", skip_all, err(Debug))]
    pub async fn revision_file(
        &self,
        node: &NodeName,
        revision: u32,
        path: &str,
    ) -> anyhow::Result<Option<FileDiff>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(&node.plan).await?;
        self.store.revision_file(node, revision, path).await
    }

    /// Refuses a plan whose project isn't registered, or that its project doesn't list
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub(super) async fn listed(&self, plan: &PlanName) -> anyhow::Result<()> {
        // the project's plans, refused if it isn't registered
        let plans = self.plans(&plan.project).await?;
        if !plans.iter().any(|info| info.name == *plan) {
            return Err(NotListed(plan.clone()).into());
        }
        Ok(())
    }
}
