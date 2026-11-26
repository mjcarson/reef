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
//! - **Pending comments are served only where asked for.** A comment written in a review not yet
//!   submitted is left out of a read unless it asks for [`Pending::Include`], as the app does for
//!   its reviewer; everything else, Claude's tools included, never sees it.
//! - **A review is submitted or discarded at once,** over the version read, which is a change,
//!   [`Change::ReviewMarked`].
//! - **Each write names its author,** the user the caller's token signs in, and Claude if the
//!   caller says it writes as Claude, never a name a body gives.
//! - **Each comment kept is a change**, [`Change::CommentAdded`], and each mark or resolve one,
//!   [`Change::ThreadMarked`]. A comment, a mark or a resolve that wrote nothing sends none.

use reef_api::{Author, Change};
use reef_model::{NodeName, PlanName};
use tracing::instrument;

use super::Service;
use crate::backend::Backend;
use crate::store::{
    Commented, MarkReview, MarkThread, Marked, NewReply, NewThread, Pending, ResolveThread,
    Resolved, Review, Reviewed, Thread,
};

impl<B: Backend> Service<B> {
    /// Lists a plan's comment threads, by number, each comment rendered
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `pending` - Whether to serve what is pending in a review not yet submitted
    #[instrument(name = "Service::threads", skip_all, err(Debug))]
    pub async fn threads(&self, plan: &PlanName, pending: Pending) -> anyhow::Result<Vec<Thread>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        let threads = self.store.threads(plan, pending).await?;
        Ok(threads.into_iter().map(rendered).collect())
    }

    /// Lists one node's comment threads, by number, each comment rendered
    ///
    /// The node need not be in its plan any more, but its plan must be listed.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `pending` - Whether to serve what is pending in a review not yet submitted
    #[instrument(name = "Service::node_threads", skip_all, err(Debug))]
    pub async fn node_threads(
        &self,
        node: &NodeName,
        pending: Pending,
    ) -> anyhow::Result<Vec<Thread>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(&node.plan).await?;
        let threads = self.store.node_threads(node, pending).await?;
        Ok(threads.into_iter().map(rendered).collect())
    }

    /// Gets one comment thread of a plan, each comment rendered, if a thread starts at that
    /// number
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `number` - The thread's number
    /// * `pending` - Whether to serve it if it is pending in a review not yet submitted
    #[instrument(name = "Service::thread", skip_all, err(Debug))]
    pub async fn thread(
        &self,
        plan: &PlanName,
        number: u32,
        pending: Pending,
    ) -> anyhow::Result<Option<Thread>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        let thread = self.store.thread(plan, number, pending).await?;
        Ok(thread.map(rendered))
    }

    /// Lists a plan's open reviews, by number
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::open_reviews", skip_all, err(Debug))]
    pub async fn open_reviews(&self, plan: &PlanName) -> anyhow::Result<Vec<Review>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        self.store.open_reviews(plan).await
    }

    /// Gets a node's open review, if it has one
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Service::node_review", skip_all, err(Debug))]
    pub async fn node_review(&self, node: &NodeName) -> anyhow::Result<Option<Review>> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(&node.plan).await?;
        self.store.node_review(node).await
    }

    /// Submits a node's open review, publishing every comment in it at once, or discards it,
    /// over the version its writer read
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `mark` - Whether it is submitted or discarded, and the version it was marked against
    /// * `author` - Who marks it, from the caller's token
    #[instrument(name = "Service::mark_review", skip_all, err(Debug))]
    pub async fn mark_review(
        &self,
        node: &NodeName,
        mark: MarkReview,
        author: &Author,
    ) -> anyhow::Result<Reviewed> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(&node.plan).await?;
        // written under the project's comment lock, and told to the change stream once it is
        let reviewed = self.store.mark_review(node, mark, author).await?;
        if let Reviewed::Marked { review } = &reviewed {
            self.publish(Change::ReviewMarked {
                node: review.node.clone(),
                review: review.number,
                state: review.state,
                version: review.version,
            });
        }
        Ok(reviewed)
    }

    /// Starts a thread on a node, or on a line of one of its revisions
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `new` - What the thread is on, and its first comment's body
    /// * `author` - Who writes it, from the caller's token
    #[instrument(name = "Service::start_thread", skip_all, err(Debug))]
    pub async fn start_thread(
        &self,
        node: &NodeName,
        new: NewThread,
        author: &Author,
    ) -> anyhow::Result<Commented> {
        // a node its plan doesn't have can't be commented on, and a plan not listed is refused
        if self.store.get(&node.plan, &node.id).await?.is_none() {
            return Ok(Commented::NotInPlan { node: node.clone() });
        }
        // written under the project's comment lock, and told to the change stream once it is
        let commented = self.store.start_thread(node, new, author).await?;
        Ok(self.announced(commented))
    }

    /// Replies in a thread of a plan
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    /// * `reply` - The reply's body
    /// * `author` - Who writes it, from the caller's token
    #[instrument(name = "Service::reply", skip_all, err(Debug))]
    pub async fn reply(
        &self,
        plan: &PlanName,
        thread: u32,
        reply: NewReply,
        author: &Author,
    ) -> anyhow::Result<Commented> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        // written under the project's comment lock, and told to the change stream once it is
        let commented = self.store.reply(plan, thread, reply, author).await?;
        Ok(self.announced(commented))
    }

    /// Marks a thread open or resolved, over the version its writer read
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    /// * `mark` - The state to mark it, and the version it was marked against
    /// * `author` - Who marks it, from the caller's token
    #[instrument(name = "Service::mark_thread", skip_all, err(Debug))]
    pub async fn mark_thread(
        &self,
        plan: &PlanName,
        thread: u32,
        mark: MarkThread,
        author: &Author,
    ) -> anyhow::Result<Marked> {
        // refused if the project isn't registered, or its plan isn't listed
        self.listed(plan).await?;
        // written under the project's comment lock, and told to the change stream once it is
        let marked = self.store.mark_thread(plan, thread, mark, author).await?;
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
            other => other,
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
    /// * `author` - Who resolves it, from the caller's token
    #[instrument(name = "Service::resolve_thread", skip_all, err(Debug))]
    pub async fn resolve_thread(
        &self,
        plan: &PlanName,
        thread: u32,
        resolve: ResolveThread,
        author: &Author,
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
        let resolved = self
            .store
            .resolve_thread(plan, thread, resolve, author)
            .await?;
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
