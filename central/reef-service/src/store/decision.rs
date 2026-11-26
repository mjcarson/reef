//! Each node's decisions: a row a decision in the node's partition, sorted by number
//!
//! - **Kept by the call that asked.** A decision is Claude Code's tool call, named by its id, so
//!   a transcript read again keeps nothing twice.
//! - **Numbered under its project's lock.** A new decision takes the number after its node's
//!   last, under the project's decisions' lock, so two hooks keeping one session's decisions at
//!   once can't take a number twice, nor keep one call twice.
//! - **An answer changes the row it answers,** over the version read. A decision read again
//!   with nothing new writes nothing, and a read that hasn't seen its answer yet never takes
//!   the outcome kept away. An approval stands against any later read.

use reef_api::{Decided, Decision, Outcome};
use reef_model::NodeName;
use std::sync::Arc;
use tracing::instrument;

use super::Store;
use crate::backend::{Backend, Expect, Written};
use crate::schema::DecisionRow;

/// A decision's number as its row's sort key, padded so the partition sorts by it
///
/// # Arguments
///
/// * `number` - The decision's number on its node
fn padded(number: u32) -> String {
    format!("{number:010}")
}

/// Reads a decision row back as the decision it keeps
///
/// # Arguments
///
/// * `row` - The row
fn decode(row: &DecisionRow) -> anyhow::Result<Decision> {
    // the number from its key, the rest from its record
    let number = row
        .number
        .parse()
        .map_err(|_| anyhow::anyhow!("decision row `{}` has no number", row.number))?;
    let decided: Decided = serde_json::from_str(&row.record)
        .map_err(|error| anyhow::anyhow!("decision {number} of `{}`: {error}", row.node))?;
    Ok(Decision {
        number,
        version: row.version,
        decided,
    })
}

/// A decision kept, with what a later read of its session found: its answer and when, if the
/// read saw one, else the answer kept
///
/// An approval stands. A plan approved with a clear is refused, with no words, in the planning
/// session's own transcript, and approved in the one the clear starts, so whichever is read
/// last, the approval is kept.
///
/// # Arguments
///
/// * `kept` - The decision as kept
/// * `read` - The decision as read again
fn merged(kept: &Decided, read: Decided) -> Decided {
    // an approval kept is never overturned, and any other answer read replaces what was kept
    let (outcome, answered_at) = match (&kept.outcome, read.outcome) {
        (Some(Outcome::Approved), _) => (kept.outcome.clone(), kept.answered_at),
        (_, Some(outcome)) => (Some(outcome), read.answered_at.or(kept.answered_at)),
        (_, None) => (kept.outcome.clone(), kept.answered_at),
    };
    // where and when it was asked stay as first kept
    Decided {
        call: kept.call.clone(),
        session: kept.session.clone(),
        via: kept.via,
        asked_at: kept.asked_at.min(read.asked_at),
        answered_at,
        kind: read.kind,
        outcome,
        // the first author known stands, as where and when it was asked does
        author: match kept.author.is_unknown() {
            true => read.author,
            false => kept.author.clone(),
        },
    }
}

impl<B: Backend> Store<B> {
    /// Finds a project's decisions' lock, making it on first use
    ///
    /// # Arguments
    ///
    /// * `project` - The project whose decisions' lock to find
    fn decision_lock(&self, project: &str) -> Arc<tokio::sync::Mutex<()>> {
        // the map is only held to find or add the lock, never across a wait
        self.decisions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(project.to_string())
            .or_default()
            .clone()
    }

    /// Lists a node's decisions, by number
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Store::decisions", skip_all, err(Debug))]
    pub async fn decisions(&self, node: &NodeName) -> anyhow::Result<Vec<Decision>> {
        // the node's partition, every row decoded
        let key = format!("{}/{}", node.plan.plan, node.id);
        let rows = self.backend.decisions(&node.plan.project, &key).await?;
        rows.iter().map(decode).collect()
    }

    /// Keeps a node's decisions, each by its call, returning how many were new and how many
    /// were kept already and changed
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `decided` - The decisions, in the order they were asked
    #[instrument(name = "Store::record_decisions", skip_all, err(Debug))]
    pub async fn record_decisions(
        &self,
        node: &NodeName,
        decided: Vec<Decided>,
    ) -> anyhow::Result<(u32, u32)> {
        // under the project's lock, against the node's decisions as they stand
        let lock = self.decision_lock(&node.plan.project);
        let _held = lock.lock().await;
        let mut kept = self.decisions(node).await?;
        let mut last = kept
            .iter()
            .map(|decision| decision.number)
            .max()
            .unwrap_or(0);
        let (mut added, mut updated) = (0, 0);
        for read in decided {
            let (number, version, decided, expect) = match kept
                .iter()
                .find(|decision| decision.decided.call == read.call)
            {
                // one kept already changes only if the read found something new
                Some(decision) => {
                    let decided = merged(&decision.decided, read);
                    if decided == decision.decided {
                        continue;
                    }
                    let expect = Expect::Version(decision.version);
                    (decision.number, decision.version + 1, decided, expect)
                }
                // a new one takes the next number
                None => {
                    last += 1;
                    (last, 1, read, Expect::Absent)
                }
            };
            let row = DecisionRow {
                project: node.plan.project.clone(),
                node: format!("{}/{}", node.plan.plan, node.id),
                number: padded(number),
                version,
                record: serde_json::to_string(&decided)?,
            };
            match self.backend.write_decision(row, expect).await? {
                Written::Applied => {}
                Written::Refused(refusal) => anyhow::bail!(
                    "decision {number} of `{node}` moved under its lock ({refusal:?}), so nothing \
                     more of it was kept"
                ),
            }
            // kept as written, so a later read in this batch finds it
            let decision = Decision {
                number,
                version,
                decided,
            };
            match kept.iter_mut().find(|kept| kept.number == number) {
                Some(slot) => {
                    *slot = decision;
                    updated += 1;
                }
                None => {
                    kept.push(decision);
                    added += 1;
                }
            }
        }
        Ok((added, updated))
    }
}
