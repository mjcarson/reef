//! The terminal loops: a row a loop in one partition, sorted by the pane it feeds
//!
//! - **One loop a pane.** A loop is kept under its pane's key, so a pane holds one at most. A
//!   start where a loop of the same project and plan is sets it running again; one of another
//!   project or plan is [`LoopEdited::Busy`], and changes nothing.
//! - **One loop a node.** Every loop is read from the one partition under the loops' lock, so a
//!   node another loop holds is found, and refused as [`LoopEdited::Held`], before any write.
//!   Two loops taking one node at once are taken one at a time, and the second is refused.
//! - **A session takes its pane's loop over.** A clear starts a new session in the same pane,
//!   and handing it the loop keeps everything else the loop holds ([`Loop::handed_to`]).
//! - **Written by version.** A start writes its row `if_absent`, every other change over the
//!   version read, and a stop deletes it at that version, so a write from another process
//!   between the read and the write is caught.
//! - **Every row checked against its full key on read**, so a hash collision is caught rather
//!   than served as a loop.

use reef_api::loops::holder;
use reef_api::{Loop, LoopEdit, LoopEdited, LoopState, Pane};
use reef_model::NodeName;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::{Store, work::now_ms};
use crate::backend::{Backend, Expect, Written};
use crate::schema::{LOOPS, LoopRow};

/// What a loop row keeps as JSON: everything of a loop but its pane and version
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    /// The project it takes nodes from
    project: String,
    /// The one plan of the project it takes nodes from, if it names one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    /// The session feeding it now, if one has said
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<String>,
    /// The node it holds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    node: Option<NodeName>,
    /// The node whose end it acted on last
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acted: Option<NodeName>,
    /// Whether it takes the next node once this one is done
    state: LoopState,
    /// When it was started, in milliseconds since the epoch
    started_at: u64,
}

impl<B: Backend> Store<B> {
    /// Lists every loop, by pane
    #[instrument(name = "Store::loops", skip_all, err(Debug))]
    pub async fn loops(&self) -> anyhow::Result<Vec<Loop>> {
        // the one partition, each row checked against its full key
        let rows = self.backend.loops(None).await?;
        rows.into_iter().map(read_loop).collect()
    }

    /// Gets the loop a pane keeps, if it keeps one
    ///
    /// # Arguments
    ///
    /// * `pane` - The pane
    #[instrument(name = "Store::loop_in", skip_all, err(Debug))]
    pub async fn loop_in(&self, pane: &Pane) -> anyhow::Result<Option<Loop>> {
        // the one row under the pane's key
        let key = pane.to_string();
        let rows = self.backend.loops(Some(&key)).await?;
        for row in rows {
            let read = read_loop(row)?;
            // a row read under another key is not this pane's
            if read.pane == *pane {
                return Ok(Some(read));
            }
        }
        Ok(None)
    }

    /// Changes a pane's loop, under the loops' lock, and answers what came of it
    ///
    /// The caller has found a project a start names registered, its plan listed, and a node a
    /// hold names in its plan. Everything else is judged here, against every loop as it stands.
    ///
    /// # Arguments
    ///
    /// * `pane` - The pane whose loop it changes
    /// * `edit` - The change
    #[instrument(name = "Store::edit_loop", skip_all, err(Debug))]
    pub async fn edit_loop(&self, pane: &Pane, edit: LoopEdit) -> anyhow::Result<LoopEdited> {
        // one change to the loops at a time, so no two take one node between a read and a write
        let _held = self.loop_lock.lock().await;
        let loops = self.loops().await?;
        let current = loops.iter().find(|read| read.pane == *pane).cloned();
        // a start is the one change a pane without a loop can take
        let LoopEdit::Start {
            project,
            plan,
            session,
        } = edit
        else {
            let Some(current) = current else {
                return Ok(LoopEdited::NoLoop { pane: pane.clone() });
            };
            return self.change(current, &loops, edit).await;
        };
        match current {
            // a pane with no loop starts one, running and holding nothing
            None => {
                let started = Loop {
                    pane: pane.clone(),
                    project,
                    plan,
                    session,
                    node: None,
                    acted: None,
                    state: LoopState::Running,
                    version: 1,
                    started_at: now_ms(),
                };
                self.keep_loop(&started, Expect::Absent).await?;
                Ok(LoopEdited::Started { r#loop: started })
            }
            // one of the same project and plan runs again, fed by the session that said so
            Some(current) if current.project == project && current.plan == plan => {
                let read = current.version;
                let resumed = Loop {
                    session: session.or(current.session),
                    state: LoopState::Running,
                    version: read + 1,
                    ..current
                };
                self.keep_loop(&resumed, Expect::Version(read)).await?;
                Ok(LoopEdited::Resumed { r#loop: resumed })
            }
            // and one feeding another project or plan is left as it is
            Some(current) => Ok(LoopEdited::Busy { r#loop: current }),
        }
    }

    /// Changes a pane's loop that is there by any edit but a start
    ///
    /// # Arguments
    ///
    /// * `current` - The pane's loop as read
    /// * `loops` - Every loop as read, the pane's included
    /// * `edit` - The change, which is not a start
    async fn change(
        &self,
        current: Loop,
        loops: &[Loop],
        edit: LoopEdit,
    ) -> anyhow::Result<LoopEdited> {
        let read = current.version;
        let changed = match edit {
            // a start is taken by the caller
            LoopEdit::Start { .. } => anyhow::bail!("a start was handed to a loop's change"),
            // paused, it stops once its node is done
            LoopEdit::Pause => Loop {
                state: LoopState::Paused,
                ..current
            },
            // stopped, its row goes, and the node it held with it
            LoopEdit::Stop => {
                let row = loop_row(&current)?;
                return match self.backend.delete_loop(&row).await? {
                    Written::Applied => Ok(LoopEdited::Stopped { r#loop: current }),
                    // under the lock, only a writer in another process could have moved it
                    Written::Refused(refusal) => anyhow::bail!(
                        "the loop in {} wasn't stopped, since another writer wrote it \
                         meanwhile: {}",
                        current.pane,
                        refusal.describe()
                    ),
                };
            }
            // handed to the session now in its pane, keeping everything else
            LoopEdit::Session { session } => current.handed_to(&session),
            // a node it takes must be its own project's or plan's, and held by no other loop
            LoopEdit::Hold { node: Some(node) } => {
                if !current.covers(&node) {
                    return Ok(LoopEdited::NotInLoop {
                        node,
                        r#loop: current,
                    });
                }
                let others: Vec<Loop> = loops
                    .iter()
                    .filter(|other| other.pane != current.pane)
                    .cloned()
                    .collect();
                if let Some(by) = holder(&others, &node) {
                    return Ok(LoopEdited::Held {
                        node,
                        by: by.clone(),
                    });
                }
                Loop {
                    node: Some(node),
                    ..current
                }
            }
            // or it lets go of the one it holds
            LoopEdit::Hold { node: None } => Loop {
                node: None,
                ..current
            },
            // the node whose end it acted on, so it acts on it no more
            LoopEdit::Acted { node } => Loop {
                acted: Some(node),
                ..current
            },
        };
        // written over the version read
        let changed = Loop {
            version: read + 1,
            ..changed
        };
        self.keep_loop(&changed, Expect::Version(read)).await?;
        Ok(LoopEdited::Edited { r#loop: changed })
    }

    /// Writes a loop's row at its version, under the condition given, which must hold
    ///
    /// # Arguments
    ///
    /// * `kept` - The loop, at the version to write
    /// * `expect` - What must be stored under its pane's key
    async fn keep_loop(&self, kept: &Loop, expect: Expect) -> anyhow::Result<()> {
        let row = loop_row(kept)?;
        match self.backend.write_loop(row, expect).await? {
            Written::Applied => Ok(()),
            // under the lock, only a writer in another process could have written it
            Written::Refused(refusal) => anyhow::bail!(
                "the loop in {} wasn't written, since another writer wrote it meanwhile: {}",
                kept.pane,
                refusal.describe()
            ),
        }
    }
}

/// A loop's row, at its version
///
/// # Arguments
///
/// * `kept` - The loop
fn loop_row(kept: &Loop) -> anyhow::Result<LoopRow> {
    // everything but its pane and version as JSON, those two beside it
    let record = Kept {
        project: kept.project.clone(),
        plan: kept.plan.clone(),
        session: kept.session.clone(),
        node: kept.node.clone(),
        acted: kept.acted.clone(),
        state: kept.state,
        started_at: kept.started_at,
    };
    Ok(LoopRow {
        list: LOOPS.to_string(),
        pane: kept.pane.to_string(),
        version: kept.version,
        record: serde_json::to_string(&record)?,
    })
}

/// A loop row read back, checked against the partition it was read from
///
/// # Arguments
///
/// * `row` - The row
fn read_loop(row: LoopRow) -> anyhow::Result<Loop> {
    // a row of another partition whose key hashed the same is a collision, not a loop
    anyhow::ensure!(
        row.list == LOOPS,
        "loop row `{}` of partition `{}` was stored where the loops are: their partition keys' \
         hashes collide",
        row.pane,
        row.list
    );
    // its pane is its key, and the rest its record
    let pane: Pane = row.pane.parse().map_err(|error| {
        anyhow::anyhow!("loop row `{}` isn't named by a pane: {error}", row.pane)
    })?;
    let kept: Kept = serde_json::from_str(&row.record)
        .map_err(|error| anyhow::anyhow!("the loop in {pane} doesn't read as one: {error}"))?;
    Ok(Loop {
        pane,
        project: kept.project,
        plan: kept.plan,
        session: kept.session,
        node: kept.node,
        acted: kept.acted,
        state: kept.state,
        version: row.version,
        started_at: kept.started_at,
    })
}

#[cfg(test)]
mod tests {
    use super::{Kept, loop_row, read_loop};
    use crate::schema::LoopRow;
    use reef_api::{Loop, LoopState, Pane};

    /// A loop reads back from its row as it was written
    #[test]
    fn loops_read_back_from_their_rows() -> anyhow::Result<()> {
        // a paused loop in a tmux pane, holding a node and having acted on another
        let kept = Loop {
            pane: Pane::Tmux {
                server: "/tmp/tmux-1000/default".to_string(),
                pane: "%3".to_string(),
            },
            project: "reef".to_string(),
            plan: Some("central".to_string()),
            session: Some("s1".to_string()),
            node: Some("reef/central/C39".parse()?),
            acted: Some("reef/central/C37".parse()?),
            state: LoopState::Paused,
            version: 4,
            started_at: 9,
        };
        let row = loop_row(&kept)?;
        assert_eq!(row.pane, "tmux:/tmp/tmux-1000/default:%3");
        assert_eq!(read_loop(row)?, kept);
        Ok(())
    }

    /// A row of another partition, or one not named by a pane, is an error, never served
    #[test]
    fn stray_loop_rows_are_errors() -> anyhow::Result<()> {
        // a row whose partition key hashed as the loops' does
        let record = serde_json::to_string(&Kept {
            project: "reef".to_string(),
            plan: None,
            session: None,
            node: None,
            acted: None,
            state: LoopState::Running,
            started_at: 1,
        })?;
        let stray = LoopRow {
            list: "other".to_string(),
            pane: "claude:7".to_string(),
            version: 1,
            record: record.clone(),
        };
        let error = read_loop(stray).expect_err("a collided row was served");
        assert!(format!("{error:#}").contains("collide"), "{error:#}");
        // and one whose key names no pane
        let unnamed = LoopRow {
            list: super::LOOPS.to_string(),
            pane: "screen:1".to_string(),
            version: 1,
            record,
        };
        let error = read_loop(unnamed).expect_err("a row named by no pane was served");
        assert!(
            format!("{error:#}").contains("isn't named by a pane"),
            "{error:#}"
        );
        Ok(())
    }
}
