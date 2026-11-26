//! The service's decisions: each plan a node's sessions asked approval for and its verdict, and
//! each question and the answer picked, as reef's hooks read them from the sessions' transcripts
//!
//! - **Kept on a node of a listed plan.** A node its plan doesn't have is
//!   [`DecisionsRecorded::NotInPlan`], and keeps nothing.
//! - **Plans rendered when read,** as a comment's body is, raw HTML escaped, so a change to the
//!   renderer reaches every plan kept.
//! - **Each keep that changed something is a change**, [`Change::DecisionsRecorded`], to the
//!   node's plan, so its Decisions tab reads them again. A transcript read again with nothing new
//!   sends none.

use reef_api::{
    Author, Change, Decided, Decision, DecisionKind, DecisionsRecorded, RecordDecisions,
};
use reef_model::NodeName;
use tracing::instrument;

use super::Service;
use crate::backend::Backend;

impl<B: Backend> Service<B> {
    /// Lists a node's decisions, by number, each plan rendered
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Service::node_decisions", skip_all, err(Debug))]
    pub async fn node_decisions(&self, node: &NodeName) -> anyhow::Result<Vec<Decision>> {
        // a plan that isn't listed has no nodes to decide on
        self.listed(&node.plan).await?;
        let mut decisions = self.store.decisions(node).await?;
        // each plan as HTML, raw HTML in it escaped
        for decision in &mut decisions {
            if let DecisionKind::Plan { plan, html } = &mut decision.decided.kind {
                *html = reef_render::render_comment(plan);
            }
        }
        Ok(decisions)
    }

    /// Keeps a node's decisions, each by the call that asked, and tells the change stream if
    /// anything was new
    ///
    /// Each decision names the user whose hook sent it, from the caller's token, whatever its
    /// body says.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `record` - The decisions a hook read from a session's transcript
    /// * `author` - Who sent them, from the caller's token
    #[instrument(name = "Service::record_decisions", skip_all, err(Debug))]
    pub async fn record_decisions(
        &self,
        node: &NodeName,
        record: RecordDecisions,
        author: &Author,
    ) -> anyhow::Result<DecisionsRecorded> {
        // a node of a listed plan, which its plan has
        self.listed(&node.plan).await?;
        if self.store.get(&node.plan, &node.id).await?.is_none() {
            return Ok(DecisionsRecorded::NotInPlan { node: node.clone() });
        }
        // each named by the caller, never by what its body says
        let decisions = record
            .decisions
            .into_iter()
            .map(|decided| Decided {
                author: author.clone(),
                ..decided
            })
            .collect();
        let (added, updated) = self.store.record_decisions(node, decisions).await?;
        if added + updated > 0 {
            self.publish(Change::DecisionsRecorded { node: node.clone() });
        }
        Ok(DecisionsRecorded::Recorded {
            node: node.clone(),
            added,
            updated,
        })
    }
}
