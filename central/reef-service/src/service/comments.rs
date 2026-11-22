//! The service's comments: threads on a node's lines or on the node as a whole, their replies,
//! and their marks
//!
//! - **A thread is started on a node in its plan.** A new thread on a node its plan doesn't have
//!   is [`Commented::NotInPlan`], and one on a line no revision of the node holds is
//!   [`Commented::NoLine`]; neither writes anything. A thread outlives its node, so one on a node
//!   removed since can still be read, replied to and marked.
//! - **Read rendered.** Each comment's body is rendered as it is read, raw HTML escaped, so a
//!   client shows its HTML as it is.
//! - **Resolved by hand, a revision linked.** A thread resolved by hand names a revision of a node
//!   in its project that answered it, which must be stored, or it is [`Resolved::NoRevision`].
//! - **Each comment kept is a change**, [`Change::CommentAdded`], and each mark or resolve one,
//!   [`Change::ThreadMarked`]. A comment, a mark or a resolve that wrote nothing sends none.

use reef_api::Change;
use reef_model::{NodeName, PlanName};
use tracing::instrument;

use super::Service;
use crate::backend::Backend;
use crate::store::{
    Commented, MarkThread, Marked, NewReply, NewThread, ResolveThread, Resolved, Thread,
};

impl<B: Backend> Service<B> {
    /// Lists a plan's comment threads, by number, each comment rendered
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::threads", skip_all, err(Debug))]
    pub async fn threads(&self, plan: &PlanName) -> anyhow::Result<Vec<Thread>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        let threads = self.store.threads(plan).await?;
        Ok(threads.into_iter().map(rendered).collect())
    }

    /// Lists one node's comment threads, by number, each comment rendered
    ///
    /// The node need not be in its plan any more, but its plan must be listed.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Service::node_threads", skip_all, err(Debug))]
    pub async fn node_threads(&self, node: &NodeName) -> anyhow::Result<Vec<Thread>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(&node.plan).await?;
        let threads = self.store.node_threads(node).await?;
        Ok(threads.into_iter().map(rendered).collect())
    }

    /// Gets one comment thread of a plan, each comment rendered, if a thread starts at that
    /// number
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `number` - The thread's number
    #[instrument(name = "Service::thread", skip_all, err(Debug))]
    pub async fn thread(&self, plan: &PlanName, number: u32) -> anyhow::Result<Option<Thread>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        let thread = self.store.thread(plan, number).await?;
        Ok(thread.map(rendered))
    }

    /// Starts a thread on a node, or on a line of one of its revisions
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `new` - What the thread is on, and its first comment's body
    #[instrument(name = "Service::start_thread", skip_all, err(Debug))]
    pub async fn start_thread(&self, node: &NodeName, new: NewThread) -> anyhow::Result<Commented> {
        // a node its plan doesn't have can't be commented on, and a plan not listed is refused
        if self.store.get(&node.plan, &node.id).await?.is_none() {
            return Ok(Commented::NotInPlan { node: node.clone() });
        }
        // written under the project's comment lock, and told to the change stream once it is
        let commented = self.store.start_thread(node, new).await?;
        Ok(self.announced(commented))
    }

    /// Replies in a thread of a plan
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    /// * `reply` - The reply's body
    #[instrument(name = "Service::reply", skip_all, err(Debug))]
    pub async fn reply(
        &self,
        plan: &PlanName,
        thread: u32,
        reply: NewReply,
    ) -> anyhow::Result<Commented> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        // written under the project's comment lock, and told to the change stream once it is
        let commented = self.store.reply(plan, thread, reply).await?;
        Ok(self.announced(commented))
    }

    /// Marks a thread open or resolved, over the version its writer read
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    /// * `mark` - The state to mark it, and the version it was marked against
    #[instrument(name = "Service::mark_thread", skip_all, err(Debug))]
    pub async fn mark_thread(
        &self,
        plan: &PlanName,
        thread: u32,
        mark: MarkThread,
    ) -> anyhow::Result<Marked> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        // written under the project's comment lock, and told to the change stream once it is
        let marked = self.store.mark_thread(plan, thread, mark).await?;
        Ok(match marked {
            Marked::Marked { thread } => {
                self.publish(Change::ThreadMarked {
                    node: thread.node.clone(),
                    thread: thread.number,
                    state: thread.state,
                    version: thread.version,
                });
                Marked::Marked {
                    thread: rendered(thread),
                }
            }
            Marked::Stale { thread } => Marked::Stale {
                thread: rendered(thread),
            },
            Marked::NoThread { thread } => Marked::NoThread { thread },
        })
    }

    /// Resolves a thread by hand, over the version its writer read, linking a revision of a node
    /// in its project to it as the one that answered it
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    /// * `resolve` - The revision that answered it, and the version it was resolved against
    #[instrument(name = "Service::resolve_thread", skip_all, err(Debug))]
    pub async fn resolve_thread(
        &self,
        plan: &PlanName,
        thread: u32,
        resolve: ResolveThread,
    ) -> anyhow::Result<Resolved> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        // the revision must be one of a node of this project, in a plan it lists
        let no_revision = || Resolved::NoRevision {
            node: resolve.node.clone(),
            revision: resolve.revision,
        };
        if resolve.node.plan.project != plan.project {
            return Ok(no_revision());
        }
        self.listed(&resolve.node.plan).await?;
        if self
            .store
            .node_revision(&resolve.node, resolve.revision)
            .await?
            .is_none()
        {
            return Ok(no_revision());
        }
        // written under the project's comment lock, and told to the change stream once it is
        let resolved = self.store.resolve_thread(plan, thread, resolve).await?;
        Ok(match resolved {
            Resolved::Resolved { thread } => {
                self.publish(Change::ThreadMarked {
                    node: thread.node.clone(),
                    thread: thread.number,
                    state: thread.state,
                    version: thread.version,
                });
                Resolved::Resolved {
                    thread: rendered(thread),
                }
            }
            Resolved::Stale { thread } => Resolved::Stale {
                thread: rendered(thread),
            },
            other => other,
        })
    }

    /// Tells the change stream of a comment kept, and renders its thread
    ///
    /// # Arguments
    ///
    /// * `commented` - What came of the comment
    fn announced(&self, commented: Commented) -> Commented {
        match commented {
            Commented::Added { thread, comment } => {
                self.publish(Change::CommentAdded {
                    node: thread.node.clone(),
                    thread: thread.number,
                    comment,
                });
                Commented::Added {
                    thread: rendered(thread),
                    comment,
                }
            }
            // nothing was written, so nothing is told
            other => other,
        }
    }
}

/// A thread with each of its comments' bodies rendered, raw HTML escaped
///
/// # Arguments
///
/// * `thread` - The thread as stored
fn rendered(mut thread: Thread) -> Thread {
    for comment in &mut thread.comments {
        comment.html = reef_render::render_comment(&comment.body);
    }
    thread
}
