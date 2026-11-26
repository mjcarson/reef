//! The runners' sessions: what each runner the user started reports, kept in memory, and what
//! the app asks of it, queued as orders the runner long-polls for
//!
//! - **The service runs nothing.** A runner, `reef loop run`, spawns Claude on the user's own
//!   machine and reports its transcript here. The app's starts, pauses and stops change the
//!   runner's loop and wake it; its answers and replies are orders for the runner to carry out.
//! - **Kept in memory, not in shoal.** Claude Code keeps every transcript on disk, so a session
//!   lives as long as this process. A runner whose service restarted is told its session is
//!   unknown, and attaches again with the transcript it holds.
//! - **Each change is a [`Change::SessionChanged`]** to the runner's project, so the app reads
//!   the session again.
//! - **An answer is sent once.** A prompt answered leaves the session's pending prompts at once,
//!   and stays out of them however the runner reports it, until the runner no longer does.
//! - **Orders are read by number.** The runner asks for those after the last it read, which also
//!   lets go of every one it read, so a dropped connection misses none.
//! - **A terminal's session attaches too.** `reef loop run` typed in a Claude Code session
//!   attaches it on its terminal's pane under an id it names, and a follower reports its
//!   transcript and asks for orders to stay live. A pane shows one such session, so attaching
//!   another detaches the one before. The app's acts on it are refused as
//!   [`SessionActed::InTerminal`], since only its terminal answers it.
//! - **Who acted is kept, though the session isn't** (C89). A session says who attached it and
//!   each order who gave it, and its project's loops' history keeps who attached it, let go of it,
//!   answered a prompt of it or replied in it, which outlives this process. Reports keep nothing,
//!   since a runner sends one a turn.

use reef_api::{
    Attach, Author, Change, EditLoop, Entry, EntryKind, LoopAct, LoopEdit, LoopEdited, Order,
    Ordered, Pane, Prompt, PromptKind, Report, Session, SessionAct, SessionActed, SessionState,
    Transcript,
};
use reef_model::PlanName;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::instrument;

use super::Service;
use crate::backend::Backend;
use crate::store::{NotRegistered, Unkeepable};

/// The most entries a session keeps, the oldest let go past it
pub use reef_api::session::KEPT_ENTRIES;

/// The longest a runner's ask for orders is held open while none comes
pub const ORDERS_WAIT_MAX: Duration = Duration::from_secs(30);

/// The most characters a terminal's session's own id holds
pub const SESSION_ID_MAX: usize = 64;

/// A session nothing attached, or that this process has never heard of since it started
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoSession(pub String);

impl std::fmt::Display for NoSession {
    /// Says no session goes by the id, and how one attaches
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "no session `{}` is attached: `reef loop run` attaches a runner, or, typed in a \
             Claude Code session, that session",
            self.0
        )
    }
}

impl std::error::Error for NoSession {}

/// What the service keeps of one runner's session
#[derive(Debug)]
struct Kept {
    /// The session as it stands
    session: Session,
    /// Its transcript's latest entries, at most [`KEPT_ENTRIES`]
    entries: VecDeque<Entry>,
    /// How many of its first entries were let go
    dropped: u64,
    /// The orders the runner hasn't read yet
    orders: Vec<Ordered>,
    /// The number of the last order given
    last_order: u64,
    /// The prompts answered that the runner may still report as waiting
    answered: BTreeSet<String>,
    /// Woken when an order is given, for the runner's ask that waits on one
    notify: Arc<Notify>,
}

impl Kept {
    /// Adds entries to the transcript, letting go of the oldest past [`KEPT_ENTRIES`]
    ///
    /// # Arguments
    ///
    /// * `entries` - The entries, oldest first
    fn add(&mut self, entries: Vec<Entry>) {
        // each one counted, then the oldest let go
        self.session.entries += entries.len() as u64;
        self.entries.extend(entries);
        while self.entries.len() > KEPT_ENTRIES {
            self.entries.pop_front();
            self.dropped += 1;
        }
    }

    /// Gives the runner an order, and wakes its ask for one
    ///
    /// # Arguments
    ///
    /// * `order` - The order
    /// * `by` - Who gave it
    fn give(&mut self, order: Order, by: &Author) {
        // numbered after the last, then the waiting ask woken, or the next one let through
        self.last_order += 1;
        self.orders.push(Ordered {
            n: self.last_order,
            order,
            by: by.clone(),
        });
        self.notify.notify_one();
    }

    /// Takes the prompts a runner reports as waiting, leaving out those already answered, and
    /// forgets an answer once the runner no longer reports its prompt
    ///
    /// # Arguments
    ///
    /// * `prompts` - The prompts the runner reports as waiting
    fn waiting(&mut self, prompts: Vec<Prompt>) {
        // an answer is kept only while its prompt is still reported
        self.answered
            .retain(|id| prompts.iter().any(|prompt| prompt.id == *id));
        let answered = &self.answered;
        self.session.prompts = prompts
            .into_iter()
            .filter(|prompt| !answered.contains(&prompt.id))
            .collect();
    }
}

/// The runners' sessions, by id
#[derive(Debug, Default)]
pub struct Sessions {
    /// Each session kept, by its id
    kept: Mutex<BTreeMap<String, Kept>>,
}

impl Sessions {
    /// Runs a closure on the sessions kept, under their lock
    ///
    /// # Arguments
    ///
    /// * `f` - What to do with them
    fn with<T>(&self, f: impl FnOnce(&mut BTreeMap<String, Kept>) -> T) -> T {
        // a lock poisoned by a panic still holds the sessions, which stay as they were
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut kept)
    }
}

/// The current time in milliseconds since the epoch
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

/// The id a session attaches under: a runner's pane's, else the one a terminal's session names,
/// which must be safe in a URL
///
/// # Arguments
///
/// * `attach` - What attaches
fn attached_id(attach: &Attach) -> Result<String, Unkeepable> {
    // a runner's pane names its session, whatever else the attach says
    if let Pane::Reef { id } = &attach.pane {
        return Ok(id.clone());
    }
    let Some(id) = &attach.id else {
        return Err(Unkeepable(format!(
            "`{}` is no runner's pane, so a session attached on it names its own id",
            attach.pane
        )));
    };
    // lowercase letters, digits and dashes alone, so the id is a path segment as it is
    let safe = id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if id.is_empty() || id.len() > SESSION_ID_MAX || !safe {
        return Err(Unkeepable(format!(
            "`{id}` can't name a session: use 1 to {SESSION_ID_MAX} lowercase letters, digits \
             and dashes"
        )));
    }
    Ok(id.clone())
}

/// Renders each plan a prompt holds as HTML, raw HTML in it escaped, as a comment's body is
///
/// # Arguments
///
/// * `prompt` - The prompt
fn rendered(mut prompt: Prompt) -> Prompt {
    // only a plan is shown as a page
    if let PromptKind::Plan { plan, html } = &mut prompt.kind {
        *html = reef_render::render_comment(plan);
    }
    prompt
}

/// Renders the plan of each prompt an entry raised
///
/// # Arguments
///
/// * `entry` - The entry
fn rendered_entry(mut entry: Entry) -> Entry {
    // an entry raising a plan shows it as the prompt does
    if let EntryKind::Asked { prompt } = entry.kind {
        entry.kind = EntryKind::Asked {
            prompt: rendered(prompt),
        };
    }
    entry
}

impl<B: Backend> Service<B> {
    /// Attaches a runner's session, or a terminal's Claude Code session, or attaches either again
    /// with the transcript it holds after this service restarted
    ///
    /// A terminal's session attached on a pane detaches any other live one on it, so a pane
    /// shows one.
    ///
    /// # Arguments
    ///
    /// * `attach` - The pane, project, host and checkout, and what it holds already
    /// * `author` - Who attaches it, from the caller's token
    #[instrument(name = "Service::attach_session", skip_all, err(Debug))]
    pub async fn attach_session(&self, attach: Attach, author: &Author) -> anyhow::Result<Session> {
        // a runner's pane's id, else the id a terminal's session names
        let id = attached_id(&attach)?;
        // a registered project, and a plan it lists if one is named
        match &attach.plan {
            Some(plan) => self.listed(&PlanName::new(&attach.project, plan)?).await?,
            None if self.store.project(&attach.project).await?.is_none() => {
                return Err(NotRegistered(attach.project.clone()).into());
            }
            None => {}
        }
        let now = now_ms();
        let session = Session {
            id: id.clone(),
            pane: attach.pane.clone(),
            project: attach.project.clone(),
            plan: attach.plan.clone(),
            host: attach.host,
            cwd: attach.cwd,
            attached_at: now,
            seen_at: now,
            state: attach.state,
            claude: attach.claude,
            entries: 0,
            prompts: attach.prompts.into_iter().map(rendered).collect(),
            attached_by: author.clone(),
        };
        // what is kept of it
        let mut kept = Kept {
            session,
            entries: VecDeque::new(),
            dropped: 0,
            orders: Vec::new(),
            last_order: 0,
            answered: BTreeSet::new(),
            notify: Arc::new(Notify::new()),
        };
        kept.add(attach.entries.into_iter().map(rendered_entry).collect());
        let session = kept.session.clone();
        let replaced = self.sessions.with(|sessions| {
            // any other live session a terminal's pane showed lets go, so it shows this one
            let mut replaced = Vec::new();
            if !session.pane.runs() {
                let others = sessions.values_mut().filter(|other| {
                    other.session.id != id
                        && other.session.pane == session.pane
                        && other.session.clone().at(now).live()
                });
                for other in others {
                    other.session.state = SessionState::Detached;
                    other.session.prompts.clear();
                    other.orders.clear();
                    replaced.push((other.session.project.clone(), other.session.id.clone()));
                }
            }
            // kept afresh, whatever was kept under its id before
            sessions.insert(id.clone(), kept);
            replaced
        });
        // kept in its project's loops' history, before the change stream hears of it
        let attached = LoopAct::Attached {
            session: session.id.clone(),
        };
        self.store
            .keep_loop_act(&session.project, author, &session.pane, attached)
            .await;
        // each session that changed told to the change stream, the one replaced first
        for (project, session) in replaced {
            self.publish(Change::SessionChanged { project, session });
        }
        self.publish(Change::SessionChanged {
            project: session.project.clone(),
            session: session.id.clone(),
        });
        Ok(session)
    }

    /// Takes a runner's report: the entries since its last, and where it stands now
    ///
    /// A session let go of stays let go, so a terminal's session another replaced on its pane
    /// isn't brought back by its follower's last report, which reads it detached.
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `report` - What the runner reports
    #[instrument(name = "Service::report_session", skip_all, err(Debug))]
    pub async fn report_session(&self, id: &str, report: Report) -> anyhow::Result<Session> {
        // the entries kept, and the rest of the session as the runner now says
        let session = self.sessions.with(|sessions| {
            let kept = sessions
                .get_mut(id)
                .ok_or_else(|| NoSession(id.to_string()))?;
            kept.add(report.entries.into_iter().map(rendered_entry).collect());
            kept.waiting(report.prompts.into_iter().map(rendered).collect());
            if kept.session.state != SessionState::Detached {
                kept.session.state = report.state;
            }
            kept.session.claude = report.claude;
            kept.session.seen_at = now_ms();
            Ok::<_, NoSession>(kept.session.clone())
        })?;
        self.publish(Change::SessionChanged {
            project: session.project.clone(),
            session: session.id.clone(),
        });
        Ok(session)
    }

    /// Lets go of a runner's session, as its runner does when it stops: kept to read, but given
    /// no more orders
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `author` - Who lets go of it, from the caller's token
    #[instrument(name = "Service::detach_session", skip_all, err(Debug))]
    pub async fn detach_session(&self, id: &str, author: &Author) -> anyhow::Result<Session> {
        // no prompt waits on a runner that let go
        let (session, was) = self.sessions.with(|sessions| {
            let kept = sessions
                .get_mut(id)
                .ok_or_else(|| NoSession(id.to_string()))?;
            let was = kept.session.state;
            kept.session.state = SessionState::Detached;
            kept.session.prompts.clear();
            kept.orders.clear();
            Ok::<_, NoSession>((kept.session.clone(), was))
        })?;
        // kept in its project's loops' history, unless it had let go already
        if was != SessionState::Detached {
            let detached = LoopAct::Detached {
                session: session.id.clone(),
            };
            self.store
                .keep_loop_act(&session.project, author, &session.pane, detached)
                .await;
        }
        self.publish(Change::SessionChanged {
            project: session.project.clone(),
            session: session.id.clone(),
        });
        Ok(session)
    }

    /// The orders for a runner after the last it read, waiting up to a while for one if there
    /// are none yet
    ///
    /// Each ask says the runner is alive, and lets go of every order up to the one it names.
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `after` - The number of the last order the runner read, or 0
    /// * `wait` - How long to wait for an order if none is there, at most [`ORDERS_WAIT_MAX`]
    #[instrument(name = "Service::session_orders", skip_all, err(Debug))]
    pub async fn session_orders(
        &self,
        id: &str,
        after: u64,
        wait: Duration,
    ) -> anyhow::Result<Vec<Ordered>> {
        let deadline = tokio::time::Instant::now() + wait.min(ORDERS_WAIT_MAX);
        loop {
            // the orders not read yet, else what to wait on for one
            let ready = self.sessions.with(|sessions| {
                let kept = sessions
                    .get_mut(id)
                    .ok_or_else(|| NoSession(id.to_string()))?;
                kept.session.seen_at = now_ms();
                kept.orders.retain(|ordered| ordered.n > after);
                Ok::<_, NoSession>(match kept.orders.is_empty() {
                    true => Err(kept.notify.clone()),
                    false => Ok(kept.orders.clone()),
                })
            })?;
            let notify = match ready {
                Ok(orders) => return Ok(orders),
                Err(notify) => notify,
            };
            // an order given since is a permit the wait takes at once
            if tokio::time::timeout_at(deadline, notify.notified())
                .await
                .is_err()
            {
                return Ok(Vec::new());
            }
        }
    }

    /// Lists the sessions of runners taking nodes from a project, each gone if its runner went
    /// silent
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Service::project_sessions", skip_all, err(Debug))]
    pub async fn project_sessions(&self, project: &str) -> anyhow::Result<Vec<Session>> {
        // a project that isn't registered has no sessions
        if self.store.project(project).await?.is_none() {
            return Err(NotRegistered(project.to_string()).into());
        }
        let now = now_ms();
        Ok(self.sessions.with(|sessions| {
            sessions
                .values()
                .filter(|kept| kept.session.project == project)
                .map(|kept| kept.session.clone().at(now))
                .collect()
        }))
    }

    /// A session's transcript after an entry, as far back as it keeps
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `after` - How many of its entries were read already
    #[instrument(name = "Service::session_transcript", skip_all, err(Debug))]
    pub async fn session_transcript(&self, id: &str, after: u64) -> anyhow::Result<Transcript> {
        let now = now_ms();
        let transcript = self.sessions.with(|sessions| {
            let kept = sessions.get(id).ok_or_else(|| NoSession(id.to_string()))?;
            // from the entry after those read, or the oldest kept if those are let go
            let from = after.max(kept.dropped);
            let skip = usize::try_from(from - kept.dropped).unwrap_or(usize::MAX);
            Ok::<_, NoSession>(Transcript {
                session: kept.session.clone().at(now),
                from,
                entries: kept.entries.iter().skip(skip).cloned().collect(),
            })
        })?;
        Ok(transcript)
    }

    /// Does what the app asks of a runner's session: changes its loop and wakes it, or orders it
    /// to answer a prompt or to say something to Claude
    ///
    /// A terminal's session takes none of it, since only its terminal answers it.
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `act` - What the app asks
    /// * `author` - Who asks it, from the caller's token or cookie
    #[instrument(name = "Service::act_on_session", skip_all, err(Debug))]
    pub async fn act_on_session(
        &self,
        id: &str,
        act: SessionAct,
        author: &Author,
    ) -> anyhow::Result<SessionActed> {
        // the session as it stands, gone if its runner went silent
        let now = now_ms();
        let session = self.sessions.with(|sessions| {
            sessions
                .get(id)
                .map(|kept| kept.session.clone().at(now))
                .ok_or_else(|| NoSession(id.to_string()))
        })?;
        // a terminal's session is answered in its terminal alone
        if !session.pane.runs() {
            return Ok(SessionActed::InTerminal { session });
        }
        let acted = match act {
            // a pause is read at the node's end, and needs no runner to be there
            SessionAct::Pause => {
                let edited = self.session_loop(&session, LoopEdit::Pause, author).await?;
                SessionActed::Looped {
                    session,
                    edited: Box::new(edited),
                }
            }
            // a stop lets go of the node whether or not the runner is there to end its Claude
            SessionAct::Stop => {
                let edited = self.session_loop(&session, LoopEdit::Stop, author).await?;
                let session = self
                    .order(id, Order::Wake, author, |_| true)?
                    .unwrap_or(session);
                SessionActed::Looped {
                    session,
                    edited: Box::new(edited),
                }
            }
            // the rest need a runner to carry them out
            _ if !session.live() => SessionActed::Gone { session },
            // a start, or a paused loop set running again, one node at a time
            SessionAct::Start => {
                let start = LoopEdit::Start {
                    project: session.project.clone(),
                    plan: session.plan.clone(),
                    session: None,
                    width: None,
                    opens: Some(false),
                    pid: None,
                };
                let edited = self.session_loop(&session, start, author).await?;
                let session = match &edited {
                    LoopEdited::Started { .. } | LoopEdited::Resumed { .. } => self
                        .order(id, Order::Wake, author, |_| true)?
                        .unwrap_or(session),
                    _ => session,
                };
                SessionActed::Looped {
                    session,
                    edited: Box::new(edited),
                }
            }
            // an answer to a prompt that still waits for one, sent once
            SessionAct::Answer { prompt, answer } => {
                let waiting = |kept: &mut Kept| {
                    let found = kept.session.prompts.iter().any(|p| p.id == prompt);
                    if found {
                        kept.answered.insert(prompt.clone());
                        kept.session.prompts.retain(|p| p.id != prompt);
                    }
                    found
                };
                let order = Order::Answer {
                    prompt: prompt.clone(),
                    answer,
                };
                match self.order(id, order, author, waiting)? {
                    Some(session) => {
                        // kept in its project's loops' history once it is given
                        let answered = LoopAct::Answered {
                            session: session.id.clone(),
                            prompt,
                        };
                        self.store
                            .keep_loop_act(&session.project, author, &session.pane, answered)
                            .await;
                        SessionActed::Ordered { session }
                    }
                    None => SessionActed::NoPrompt { prompt, session },
                }
            }
            // a reply to Claude, whose turn ended mid-node
            SessionAct::Say { text } => {
                let waiting = |kept: &mut Kept| {
                    let waits = kept.session.state == SessionState::Waiting;
                    if waits {
                        kept.session.state = SessionState::Working;
                    }
                    waits
                };
                match self.order(id, Order::Say { text }, author, waiting)? {
                    Some(session) => {
                        // kept in its project's loops' history once it is given, its words left
                        // to the transcript
                        let said = LoopAct::Said {
                            session: session.id.clone(),
                        };
                        self.store
                            .keep_loop_act(&session.project, author, &session.pane, said)
                            .await;
                        SessionActed::Ordered { session }
                    }
                    None => SessionActed::NotWaiting { session },
                }
            }
        };
        Ok(acted)
    }

    /// Changes a session's loop, as its pane's
    ///
    /// # Arguments
    ///
    /// * `session` - The session
    /// * `edit` - The change to its loop
    /// * `author` - Who asks it
    async fn session_loop(
        &self,
        session: &Session,
        edit: LoopEdit,
        author: &Author,
    ) -> anyhow::Result<LoopEdited> {
        // the loop is the runner's pane's, edited and told as any pane's is
        let edit = EditLoop {
            pane: session.pane.clone(),
            edit,
        };
        self.edit_loop(edit, author).await
    }

    /// Gives a session's runner an order if the session takes it, and tells the change stream
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `order` - The order
    /// * `by` - Who gives it
    /// * `takes` - Whether the session takes it, changing what it keeps if it does
    fn order(
        &self,
        id: &str,
        order: Order,
        by: &Author,
        takes: impl FnOnce(&mut Kept) -> bool,
    ) -> anyhow::Result<Option<Session>> {
        // given under the lock, so a take and its order can't be split
        let session = self.sessions.with(|sessions| {
            let kept = sessions
                .get_mut(id)
                .ok_or_else(|| NoSession(id.to_string()))?;
            if !takes(kept) {
                return Ok::<_, NoSession>(None);
            }
            kept.give(order, by);
            Ok(Some(kept.session.clone()))
        })?;
        if let Some(session) = &session {
            self.publish(Change::SessionChanged {
                project: session.project.clone(),
                session: session.id.clone(),
            });
        }
        Ok(session)
    }
}
