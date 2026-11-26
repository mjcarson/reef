//! The terminal loops: a row a loop in one partition, sorted by the pane it feeds
//!
//! - **One loop a pane.** A loop is kept under its pane's key, so a pane holds one at most. A
//!   start where a loop of the same project and plan is sets it running again; one of another
//!   project or plan is [`LoopEdited::Busy`], and changes nothing.
//! - **One loop a node.** Every loop is read from the one partition under the loops' lock, so a
//!   node another loop holds is found, and refused as [`LoopEdited::Held`], before any write.
//!   Two loops taking one node at once are taken one at a time, and the second is refused.
//! - **A node's end is acted on once.** Recording the node whose end a loop acted on, when it
//!   acted on that node's end already, is [`LoopEdited::ActedAlready`], judged under the lock,
//!   so two Stops never both act on one node.
//! - **A session takes its pane's loop over.** A clear starts a new session in the same pane,
//!   and handing it the loop keeps everything else the loop holds ([`Loop::handed_to`]).
//! - **A group is held as one.** A hold names the loop's own node and the group's members, no
//!   more than its width, and takes them all or none: one another loop holds is
//!   [`LoopEdited::Held`]. A new hold replaces the members the last one took.
//! - **A pane joins a member.** A pane with no loop of its own joins the member of whichever loop
//!   holds its node ([`LoopEdit::Join`]), moving off any member it joined before, since a pane
//!   runs one session at a time. A member another pane joined is [`LoopEdited::Held`]. A pane
//!   working a member starts no loop of its own, and is [`LoopEdited::Busy`].
//! - **Written by version.** A start writes its row `if_absent`, every other change over the
//!   version read, and a stop deletes it at that version, so a write from another process
//!   between the read and the write is caught.
//! - **Every row checked against its full key on read**, so a hash collision is caught rather
//!   than served as a loop.
//! - **Each act names who made it** (C89). A loop keeps who started it and who changed it last,
//!   and every act that changed something keeps an entry in its project's loops' history, under
//!   the loops' lock: a start, a resume, a pause, a stop, a hand-off to another session, a take,
//!   a letting go and a join. A node's end acted on keeps none, since the next take says it, and
//!   nor does a hand-off to the session the loop had already, which every resume and clear sends.

use reef_api::loops::{holder, member_of};
use reef_api::{Author, Loop, LoopAct, LoopEdit, LoopEdited, LoopState, Member, Pane};
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
    /// How many nodes it works at once
    #[serde(default = "one", skip_serializing_if = "is_one")]
    width: u8,
    /// Whether reef opens a pane for each member
    #[serde(default = "yes", skip_serializing_if = "is_set")]
    opens: bool,
    /// The Claude Code process its session runs in, if one was named
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    /// The group's nodes past its own, and the panes that joined them
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    members: Vec<Member>,
    /// Who started it, nobody known for a loop started before reef kept it
    #[serde(default, skip_serializing_if = "Author::is_unknown")]
    started_by: Author,
    /// Who made its latest change, a node's end it acted on aside
    #[serde(default, skip_serializing_if = "Author::is_unknown")]
    changed_by: Author,
}

/// A loop's width in a row written before groups: one node at a time
fn one() -> u8 {
    1
}

/// Whether a loop works one node at a time, so its row needn't say
///
/// # Arguments
///
/// * `width` - The loop's width
fn is_one(width: &u8) -> bool {
    *width == 1
}

/// Whether reef opens the members' panes, in a row written before groups
fn yes() -> bool {
    true
}

/// Whether a flag is set, so a row needn't say
///
/// # Arguments
///
/// * `flag` - The flag
fn is_set(flag: &bool) -> bool {
    *flag
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
    /// * `author` - Who makes it
    #[instrument(name = "Store::edit_loop", skip_all, err(Debug))]
    pub async fn edit_loop(
        &self,
        pane: &Pane,
        edit: LoopEdit,
        author: &Author,
    ) -> anyhow::Result<LoopEdited> {
        // one change to the loops at a time, so no two take one node between a read and a write
        let _held = self.loop_lock.lock().await;
        let loops = self.loops().await?;
        let current = loops.iter().find(|read| read.pane == *pane).cloned();
        // what was asked, to say in the history once it lands
        let asked = edit.clone();
        let edited = self
            .edit_held(pane, current.clone(), &loops, edit, author)
            .await?;
        // kept in the history of the project whose loop it is, under the same lock
        if let Some((project, act)) = loop_act(&asked, current.as_ref(), &loops, &edited) {
            self.keep_loop_act(&project, author, pane, act).await;
        }
        Ok(edited)
    }

    /// Changes a pane's loop, the loops' lock held and every loop read
    ///
    /// # Arguments
    ///
    /// * `pane` - The pane whose loop it changes
    /// * `current` - The pane's own loop, if it has one
    /// * `loops` - Every loop as read
    /// * `edit` - The change
    /// * `author` - Who makes it
    async fn edit_held(
        &self,
        pane: &Pane,
        current: Option<Loop>,
        loops: &[Loop],
        edit: LoopEdit,
        author: &Author,
    ) -> anyhow::Result<LoopEdited> {
        // a join is taken by a pane with no loop of its own, to another loop's member
        if let LoopEdit::Join { node } = edit {
            return self.join(pane, current, loops, node, author).await;
        }
        // a start is the one change a pane without a loop can take
        let LoopEdit::Start {
            project,
            plan,
            session,
            width,
            opens,
            pid,
        } = edit
        else {
            let Some(current) = current else {
                return Ok(LoopEdited::NoLoop { pane: pane.clone() });
            };
            return self.change(current, loops, edit, author).await;
        };
        // a pane working another loop's member starts no loop of its own
        if current.is_none()
            && let Some((by, _)) = member_of(loops, pane)
        {
            return Ok(LoopEdited::Busy { r#loop: by.clone() });
        }
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
                    width: width.unwrap_or(1).max(1),
                    opens: opens.unwrap_or(true),
                    pid,
                    members: Vec::new(),
                    started_by: author.clone(),
                    changed_by: author.clone(),
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
                    // a width or a choice of panes named changes it from its next group on
                    width: width.map_or(current.width, |width| width.max(1)),
                    opens: opens.unwrap_or(current.opens),
                    pid: pid.or(current.pid),
                    changed_by: author.clone(),
                    ..current
                };
                self.keep_loop(&resumed, Expect::Version(read)).await?;
                Ok(LoopEdited::Resumed { r#loop: resumed })
            }
            // and one feeding another project or plan is left as it is
            Some(current) => Ok(LoopEdited::Busy { r#loop: current }),
        }
    }

    /// Joins a pane to the member of whichever loop holds a node, under the loops' lock
    ///
    /// # Arguments
    ///
    /// * `pane` - The pane joining, which must run no loop of its own
    /// * `current` - The pane's own loop, if it has one
    /// * `loops` - Every loop as read
    /// * `node` - The member's node
    /// * `author` - Who joins it
    async fn join(
        &self,
        pane: &Pane,
        current: Option<Loop>,
        loops: &[Loop],
        node: reef_model::NodeName,
        author: &Author,
    ) -> anyhow::Result<LoopEdited> {
        // a pane running a loop of its own works that loop's node, not another's member
        if let Some(current) = current {
            return Ok(LoopEdited::Busy { r#loop: current });
        }
        // the loop holding the node as a member, if one does
        let Some(by) = loops.iter().find(|feeding| feeding.member(&node).is_some()) else {
            return Ok(LoopEdited::NotMember { node });
        };
        match by.member(&node).and_then(|member| member.pane.as_ref()) {
            // joined already from this pane, which changes nothing
            Some(joined) if joined == pane => {
                return Ok(LoopEdited::Joined { r#loop: by.clone() });
            }
            // a session in another pane works it
            Some(_) => {
                return Ok(LoopEdited::Held {
                    node,
                    by: by.clone(),
                });
            }
            None => {}
        }
        // a pane runs one session at a time, so it leaves any member it joined before
        if let Some((before, _)) = member_of(loops, pane)
            && before.pane != by.pane
        {
            let left = Loop {
                members: unjoined(&before.members, pane),
                version: before.version + 1,
                changed_by: author.clone(),
                ..before.clone()
            };
            self.keep_loop(&left, Expect::Version(before.version))
                .await?;
        }
        // the member joined, any other it joined in this loop left
        let mut members = unjoined(&by.members, pane);
        for member in &mut members {
            if member.node == node {
                member.pane = Some(pane.clone());
            }
        }
        let joined = Loop {
            members,
            version: by.version + 1,
            changed_by: author.clone(),
            ..by.clone()
        };
        self.keep_loop(&joined, Expect::Version(by.version)).await?;
        Ok(LoopEdited::Joined { r#loop: joined })
    }

    /// Changes a pane's loop that is there by any edit but a start
    ///
    /// # Arguments
    ///
    /// * `current` - The pane's loop as read
    /// * `loops` - Every loop as read, the pane's included
    /// * `edit` - The change, which is not a start
    /// * `author` - Who makes it
    async fn change(
        &self,
        current: Loop,
        loops: &[Loop],
        edit: LoopEdit,
        author: &Author,
    ) -> anyhow::Result<LoopEdited> {
        let read = current.version;
        // a node's end acted on is the loop's own doing, so it leaves who changed it last
        let changed_by = match edit {
            LoopEdit::Acted { .. } => current.changed_by.clone(),
            _ => author.clone(),
        };
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
            // a join is taken by the caller
            LoopEdit::Join { .. } => anyhow::bail!("a join was handed to a loop's change"),
            // handed to the session now in its pane, keeping everything else
            LoopEdit::Session { session, pid } => {
                let pid = pid.or(current.pid);
                Loop {
                    pid,
                    ..current.handed_to(&session)
                }
            }
            // a group it takes is no wider than it is
            LoopEdit::Hold {
                node: Some(_),
                ref members,
            } if members.len() + 1 > usize::from(current.width) => {
                return Ok(LoopEdited::TooWide {
                    named: members.len() + 1,
                    r#loop: current,
                });
            }
            // each node it takes must be its own project's or plan's, and held by no other loop
            LoopEdit::Hold {
                node: Some(node),
                members,
            } => {
                let others: Vec<Loop> = loops
                    .iter()
                    .filter(|other| other.pane != current.pane)
                    .cloned()
                    .collect();
                for taken in std::iter::once(&node).chain(&members) {
                    if !current.covers(taken) {
                        return Ok(LoopEdited::NotInLoop {
                            node: taken.clone(),
                            r#loop: current,
                        });
                    }
                    if let Some(by) = holder(&others, taken) {
                        return Ok(LoopEdited::Held {
                            node: taken.clone(),
                            by: by.clone(),
                        });
                    }
                }
                // the new group's members, none joined yet, in place of the last group's
                let members = members
                    .into_iter()
                    .map(|node| Member { node, pane: None })
                    .collect();
                // a node whose end it acted on, taken again for a round of its comments, has
                // that round's end to act on as well
                let acted = current.acted.clone().filter(|acted| *acted != node);
                Loop {
                    node: Some(node),
                    members,
                    acted,
                    ..current
                }
            }
            // or it lets go of everything it holds
            LoopEdit::Hold { node: None, .. } => Loop {
                node: None,
                members: Vec::new(),
                ..current
            },
            // a node's end it acted on already is never acted on again, judged under the lock
            LoopEdit::Acted { node } if current.acted.as_ref() == Some(&node) => {
                return Ok(LoopEdited::ActedAlready {
                    node,
                    r#loop: current,
                });
            }
            // else the node whose end it acted on, so it acts on it no more
            LoopEdit::Acted { node } => Loop {
                acted: Some(node),
                ..current
            },
        };
        // written over the version read, saying who changed it
        let changed = Loop {
            version: read + 1,
            changed_by,
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
        width: kept.width,
        opens: kept.opens,
        pid: kept.pid,
        members: kept.members.clone(),
        started_by: kept.started_by.clone(),
        changed_by: kept.changed_by.clone(),
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
        width: kept.width,
        opens: kept.opens,
        pid: kept.pid,
        members: kept.members,
        started_by: kept.started_by,
        changed_by: kept.changed_by,
    })
}

/// What an edit of a pane's loop did, as its project's loops' history keeps it, and the
/// project it was done in, or `None` for one that changed nothing worth keeping
///
/// A node's end acted on keeps none, since the next take says it, and nor does a hand-off to
/// the session the loop had already, which every resume and clear sends, nor a join from the
/// pane that joined already. Every outcome that wrote nothing keeps none.
///
/// # Arguments
///
/// * `asked` - The edit asked for
/// * `current` - The pane's own loop as it was read, if it had one
/// * `loops` - Every loop as it was read
/// * `edited` - What came of it
fn loop_act(
    asked: &LoopEdit,
    current: Option<&Loop>,
    loops: &[Loop],
    edited: &LoopEdited,
) -> Option<(String, LoopAct)> {
    // the nodes a loop held, its own first
    let held = |held: &Loop| -> Vec<NodeName> { held.group().cloned().collect() };
    let act = match (asked, edited) {
        // a new loop, and a paused one set running again
        (_, LoopEdited::Started { r#loop }) => LoopAct::Started {
            plan: r#loop.plan.clone(),
            width: r#loop.width,
        },
        (_, LoopEdited::Resumed { .. }) => LoopAct::Resumed,
        // a stop, saying what it let go of, since its row is gone
        (_, LoopEdited::Stopped { r#loop }) => LoopAct::Stopped { held: held(r#loop) },
        // a join from a pane not joined already
        (LoopEdit::Join { node }, LoopEdited::Joined { r#loop }) => {
            let read = loops.iter().find(|read| read.pane == r#loop.pane)?;
            if read.version == r#loop.version {
                return None;
            }
            LoopAct::Joined {
                node: node.clone(),
                of: r#loop.pane.clone(),
            }
        }
        // and each change a pane's own loop took
        (LoopEdit::Pause, LoopEdited::Edited { .. }) => LoopAct::Paused,
        (LoopEdit::Session { session, .. }, LoopEdited::Edited { .. }) => {
            if current?.session.as_ref() == Some(session) {
                return None;
            }
            LoopAct::HandedOff {
                session: session.clone(),
            }
        }
        (
            LoopEdit::Hold {
                node: Some(node),
                members,
            },
            LoopEdited::Edited { .. },
        ) => LoopAct::Took {
            node: node.clone(),
            members: members.clone(),
        },
        (LoopEdit::Hold { node: None, .. }, LoopEdited::Edited { .. }) => {
            let held = held(current?);
            if held.is_empty() {
                return None;
            }
            LoopAct::LetGo { held }
        }
        // a node's end acted on, and every outcome that wrote nothing
        _ => return None,
    };
    // the project of the loop it was done to: a join's is the loop it joined
    let project = match edited {
        LoopEdited::Started { r#loop }
        | LoopEdited::Resumed { r#loop }
        | LoopEdited::Stopped { r#loop }
        | LoopEdited::Joined { r#loop }
        | LoopEdited::Edited { r#loop } => r#loop.project.clone(),
        _ => return None,
    };
    Some((project, act))
}

/// A group's members with a pane's join taken off whichever it was on
///
/// # Arguments
///
/// * `members` - The members
/// * `pane` - The pane leaving
fn unjoined(members: &[Member], pane: &Pane) -> Vec<Member> {
    members
        .iter()
        .map(|member| match member.pane.as_ref() == Some(pane) {
            true => Member {
                node: member.node.clone(),
                pane: None,
            },
            false => member.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Kept, loop_row, read_loop};
    use crate::schema::LoopRow;
    use reef_api::{Loop, LoopState, Member, Pane};

    /// A loop reads back from its row as it was written
    #[test]
    fn loops_read_back_from_their_rows() -> anyhow::Result<()> {
        // a paused loop in a tmux pane, holding a node and having acted on another
        let kept = Loop {
            changed_by: reef_api::Author::default(),
            started_by: reef_api::Author::default(),
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
            width: 2,
            opens: false,
            pid: Some(4242),
            members: vec![Member {
                node: "reef/central/C40".parse()?,
                pane: Some(Pane::Claude { pid: 7 }),
            }],
        };
        let row = loop_row(&kept)?;
        assert_eq!(row.pane, "tmux:/tmp/tmux-1000/default:%3");
        assert_eq!(read_loop(row)?, kept);
        Ok(())
    }

    /// A row written before groups reads as a loop one node wide, opening panes, with no members
    #[test]
    fn rows_from_before_groups_read_one_wide() -> anyhow::Result<()> {
        // the record as C61 wrote it
        let row = LoopRow {
            list: super::LOOPS.to_string(),
            pane: "claude:7".to_string(),
            version: 3,
            record:
                r#"{"project":"reef","node":"reef/central/C39","state":"running","started_at":1}"#
                    .to_string(),
        };
        let read = read_loop(row)?;
        assert_eq!((read.width, read.opens, read.pid), (1, true, None));
        assert!(read.members.is_empty());
        // and written back one wide, it says nothing groups added
        let written = loop_row(&read)?;
        assert!(!written.record.contains("width"), "{}", written.record);
        assert!(!written.record.contains("members"), "{}", written.record);
        Ok(())
    }

    /// A row of another partition, or one not named by a pane, is an error, never served
    #[test]
    fn stray_loop_rows_are_errors() -> anyhow::Result<()> {
        // a row whose partition key hashed as the loops' does
        let record = serde_json::to_string(&Kept {
            changed_by: reef_api::Author::default(),
            started_by: reef_api::Author::default(),
            project: "reef".to_string(),
            plan: None,
            session: None,
            node: None,
            acted: None,
            state: LoopState::Running,
            started_at: 1,
            width: 1,
            opens: true,
            pid: None,
            members: Vec::new(),
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
