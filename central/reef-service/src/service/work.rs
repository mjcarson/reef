//! The service's work records: a node started where its work happens, and each capture's SHAs
//!
//! - **A node started is in its plan.** A start of a node its plan doesn't have is
//!   [`Started::NotInPlan`], and writes nothing. A record outlives its node, so a node removed
//!   since it started can still be read, and captured.
//! - **The service reads no checkout.** What a start or a capture read comes from the client
//!   that read it, in the node's own checkout, so the service is the same once it is shared.
//! - **Each record written is a change**, [`Change::WorkRecorded`], naming its node and the
//!   version it is now at. A start or a capture that wrote nothing sends none.

use reef_api::Change;
use reef_model::NodeName;
use tracing::instrument;

use super::Service;
use crate::backend::Backend;
use crate::store::{
    AddCapture, Captured, NotListed, NotRegistered, StartWork, Started, WorkRecord,
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
        self.listed(node).await?;
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

    /// Adds the SHAs a capture read to its node's record, if it was taken from the node's start
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `add` - The start the capture was taken from, and what it read
    #[instrument(name = "Service::add_capture", skip_all, err(Debug))]
    pub async fn add_capture(&self, node: &NodeName, add: AddCapture) -> anyhow::Result<Captured> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(node).await?;
        // written under the project's work lock, and told to the change stream once it is
        let captured = self.store.add_capture(node, add).await?;
        if let Captured::Added { record } = &captured {
            self.publish(Change::WorkRecorded {
                node: node.clone(),
                version: record.version,
            });
        }
        Ok(captured)
    }

    /// Refuses a node whose project isn't registered, or whose plan isn't listed
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    async fn listed(&self, node: &NodeName) -> anyhow::Result<()> {
        // the project's plans, refused if it isn't registered
        let plans = self.plans(&node.plan.project).await?;
        if !plans.iter().any(|info| info.name == node.plan) {
            return Err(NotListed(node.plan.clone()).into());
        }
        Ok(())
    }
}
