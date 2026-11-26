//! The service's histories: who made each edit of a node, a page or a project's queue, what it
//! changed and when
//!
//! - **Read where the plan is listed.** A node's or a page's history is read from a listed plan
//!   of a registered project, whether or not the node or page is still in it, so a removed one
//!   still says who removed it. A queue's history is read from a registered project.
//! - **Newest first, a page at a time.** A page holds at most
//!   [`HISTORY_MAX`](reef_api::HISTORY_MAX) entries, and names the cursor onto the older ones
//!   when any are kept. An entry is kept once its write lands, so a write whose history failed
//!   to keep reads as a gap.

use reef_api::{HistoryPage, HistoryQuery};
use reef_model::{NodeName, PlanName};
use tracing::instrument;

use super::Service;
use crate::backend::Backend;
use crate::store::NotRegistered;

impl<B: Backend> Service<B> {
    /// Reads a page of a node's history, newest first, a node removed since included
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `query` - Which page to read
    #[instrument(name = "Service::node_history", skip_all, err(Debug))]
    pub async fn node_history(
        &self,
        node: &NodeName,
        query: &HistoryQuery,
    ) -> anyhow::Result<HistoryPage> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(&node.plan).await?;
        self.store.node_history(node, query).await
    }

    /// Reads a page of one of a plan's own pages' history, newest first, a page taken out since
    /// included
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `path` - The page's path
    /// * `query` - Which page of its history to read
    #[instrument(name = "Service::page_history", skip_all, err(Debug))]
    pub async fn page_history(
        &self,
        plan: &PlanName,
        path: &str,
        query: &HistoryQuery,
    ) -> anyhow::Result<HistoryPage> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        self.store.page_history(plan, path, query).await
    }

    /// Reads a page of a project's queue's history, newest first
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `query` - Which page to read
    #[instrument(name = "Service::queue_history", skip_all, err(Debug))]
    pub async fn queue_history(
        &self,
        project: &str,
        query: &HistoryQuery,
    ) -> anyhow::Result<HistoryPage> {
        // a project that isn't registered has no queue
        if self.store.project(project).await?.is_none() {
            return Err(NotRegistered(project.to_string()).into());
        }
        self.store.queue_history(project, query).await
    }
}
