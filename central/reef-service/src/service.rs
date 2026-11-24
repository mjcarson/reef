//! reef's service: what the front doors call, and the only code that touches shoal
//!
//! The JSON API (C53) and the app's server functions both call a [`Service`], so versions,
//! validation and atomic writes are written once. It reads and writes through a [`Store`], and
//! adds what the store leaves out: every write is checked against the whole plan with
//! `mdbook-reef check`'s rules, and a plan is built into the viewer's payload.
//!
//! - **A write is refused if it adds a problem.** The plan is built as it stands and as the
//!   write would leave it, and a problem found after that wasn't found before refuses the
//!   write with check's own message. A stored node's problems sit at its name, with no line,
//!   so they hold still across writes, and a fix gets through on a plan that already has
//!   other problems.
//! - **Puts and removals come in batches.** A group with no members yet is a piece of work
//!   that says neither why nor what, and a member alone names a group that isn't a node, so a
//!   group goes in with its members, and comes out with them, checked as one change.
//! - **Checked under the plan's write lock**, so two writes that are fine alone can't break the
//!   plan together.
//! - **Docs come from the plan's pages, over the project's working tree.** A stored node's links
//!   resolve from its repo's root, as `mdbook-reef check` resolves a book's from its `src`, and
//!   a link that leaves the repo is refused. A page the plan keeps hides the repo's file at its
//!   path. Docs are read off the async threads, and a plan's nodes and pages are read at one
//!   moment. A remote project's docs are its clone's at the commit last fetched ([`remotes`]),
//!   so every read of one moment sees one commit.
//! - **A page edit is checked as a node write is.** Taking out a page a node links to, or a
//!   heading a link names, adds a problem, and is refused.
//! - **Docs are served as pages.** A payload's links lead under [`paths::docs`], where
//!   [`Service::doc_page`] renders each doc as the page mdbook would write for it, so the
//!   viewer's *Read here* cuts a section out of it as out of a book's page.
//! - **Each applied write is a change.** A project registered, a plan listed, a batch written, a
//!   page edited, a whole plan written, a queue edited or a work record written is sent to every subscriber of [`Service::changes`],
//!   after the write's lock is let go, so two writers' changes to one plan can arrive in either
//!   order. A refused write sends nothing.
//! - **next follows the user's queue.** A project's queue spans its plans, so next reads every
//!   plan it is asked about, takes the queue's ready entries first, then each plan's other ready
//!   work in plan order. Asked about one plan, it takes that plan's entries alone.
//!
//! - **A node's work record comes from its checkout** ([`work`]). `reef start` and each capture
//!   send what they read there, and the service keeps it, reading no checkout itself.
//! - **Comments are threads on a node's work** ([`comments`]), each resolved by a mark, by hand
//!   or by the change a commit's trailer names.
//! - **A terminal loop is kept by its pane** ([`loops`]): the project it feeds, the node it
//!   holds, which no other loop may take, and whether it is paused.
//! - **Users sign in for tokens** ([`users`]), when reef's config requires it: the first user is
//!   added from the service's own host, and the rest by an admin.
//! - **A project registers by its git URL too** ([`remotes`]): the service clones one branch of
//!   it, and fetches it when asked and on the interval reef's config sets.
//!
//! A process keeps one service over shoal. `Service::shared` opens it once however often a
//! caller asks, since `dioxus::serve` runs its setup again after every hot-patch, and a second
//! pool would find the data directory locked.

use reef_api::{Change, Listed, PutNode, PutPage, QueueListing, check_page_path, paths};
use reef_model::{
    Diagnostic, Graph, Next, NodeName, Payload, PlanGraph, PlanName, Record, nodes_from_records,
};
use reef_render::docs::page_node_problems;
use reef_render::{
    Cached, DocSource, Layered, MemSource, ROOT, Renderer, Sections, doc_body, render_page,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{OnceCell, broadcast};
use tracing::instrument;

use crate::backend::Backend;
use crate::client::{Mode, ShoalBackend};
use crate::conf::Conf;
use crate::memory::MemoryBackend;
use crate::store::{
    AddedProject, Checked, Listing, NodePut, NotListed, NotRegistered, PageEdit, PageEdited,
    PageListing, PageWrite, PlanInfo, PlanWrite, Project, Proposal, Puts, QueueEdit, Queued,
    Rewrite, Store, StoredNode, StoredPage,
};
use crate::tree::Docs;

mod comments;
mod decisions;
mod loops;
mod remotes;
mod sessions;
mod users;
mod work;

pub use sessions::{KEPT_ENTRIES, NoSession, ORDERS_WAIT_MAX};
pub use users::{Asker, BROWSER_TOKEN_LIFE, NotAllowed, NotSignedIn, token_hash};

/// The process's one service over shoal, opened on first use
static SHARED: OnceCell<Service> = OnceCell::const_new();

/// How many changes a subscriber can fall behind by before it misses some
const CHANGE_BACKLOG: usize = 256;

/// reef's service: checked writes and built plans, over a store
pub struct Service<B = ShoalBackend> {
    /// The store every read and write goes through
    store: Store<B>,
    /// The doc sections a node's why, what and design choices are taken from
    sections: Sections,
    /// Where each applied write is sent, for every subscriber to the change stream, until the
    /// stream is ended
    changes: Mutex<Option<broadcast::Sender<Change>>>,
    /// The sessions of runners the user started, kept in memory
    sessions: sessions::Sessions,
    /// Whether every caller must sign in
    requires_sign_in: bool,
    /// The tokens looked up lately, trusted for a minute
    tokens: users::Tokens,
    /// Held to hash or check a password, so only a few run at once
    hashing: tokio::sync::Semaphore,
    /// The directory remote projects are cloned into, or `None` if this service keeps no clones
    clones: Option<PathBuf>,
    /// The hosts a remote may be cloned from, lowercased, or empty for any
    hosts: Vec<String>,
    /// How often every remote project is fetched, or `None` if only when asked
    fetch_every: Option<Duration>,
    /// Each remote project's fetch lock, held while its clone is fetched and its row written
    fetches: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Service<ShoalBackend> {
    /// Opens a service over shoal: the external node reef's config names, else a pool in this
    /// process
    ///
    /// Only one process can open a data directory, so a process opens one service; most
    /// callers want `shared`.
    ///
    /// # Arguments
    ///
    /// * `conf` - reef's config
    #[instrument(name = "Service::open", skip_all, err(Debug))]
    pub async fn open(conf: &Conf) -> anyhow::Result<Service> {
        // reach shoal however the config says to, asking callers to sign in if it says to
        let store = Store::open(conf).await?;
        // clone remote projects where it says, unless it names nowhere and no home has a default
        let clones = match conf.clones_dir() {
            Ok(dir) => Some(dir),
            Err(_) if conf.remotes.clones.is_none() => None,
            Err(error) => return Err(error),
        };
        Ok(Service::new(store)
            .requiring_sign_in(conf.auth.required)
            .cloning_into(clones)
            .cloning_from(conf.remotes.hosts.clone())
            .fetching_every(conf.fetch_every()?))
    }

    /// Gets the process's service, opening it on first use
    ///
    /// Every later caller gets the service the first opened, whatever config it passes.
    ///
    /// # Arguments
    ///
    /// * `conf` - reef's config, used only by the first call
    #[instrument(name = "Service::shared", skip_all, err(Debug))]
    pub async fn shared(conf: &Conf) -> anyhow::Result<&'static Service> {
        // open it once, and hand every later caller the same one
        SHARED.get_or_try_init(|| Service::open(conf)).await
    }

    /// How this service reaches shoal
    pub fn mode(&self) -> Mode {
        self.store.mode()
    }

    /// Closes the service, stopping its in-process pool if it started one
    #[instrument(name = "Service::close", skip_all, err(Debug))]
    pub async fn close(self) -> anyhow::Result<()> {
        // the store holds the client and the pool
        self.store.close().await
    }
}

impl Service<MemoryBackend> {
    /// A service over an empty in-memory twin
    pub fn memory() -> Service<MemoryBackend> {
        Service::new(Store::memory())
    }
}

impl<B: Backend> Service<B> {
    /// A service over a store, taking a node's brief form from the default doc sections, asking
    /// no caller to sign in
    ///
    /// # Arguments
    ///
    /// * `store` - The store every read and write goes through
    pub fn new(store: Store<B>) -> Service<B> {
        // nobody subscribes until a client follows the change stream
        let (changes, _) = broadcast::channel(CHANGE_BACKLOG);
        Service {
            store,
            sections: Sections::default(),
            changes: Mutex::new(Some(changes)),
            sessions: sessions::Sessions::default(),
            requires_sign_in: false,
            tokens: users::Tokens::default(),
            hashing: tokio::sync::Semaphore::new(users::HASHING_AT_ONCE),
            clones: None,
            hosts: Vec::new(),
            fetch_every: None,
            fetches: Mutex::new(HashMap::new()),
        }
    }

    /// Subscribes to every change this service applies from now on
    ///
    /// A subscriber more than 256 changes behind misses the oldest, and its next receive says
    /// how many it missed. Once the stream is ended, a new subscriber's receiver is closed.
    pub fn changes(&self) -> broadcast::Receiver<Change> {
        // a lock poisoned by a panic still holds a sender, which is all this reads
        let changes = self.changes.lock().unwrap_or_else(PoisonError::into_inner);
        match changes.as_ref() {
            Some(sender) => sender.subscribe(),
            // the stream has ended, so this subscriber's receiver is closed from the start
            None => broadcast::channel(1).1,
        }
    }

    /// Ends the change stream: every subscriber's receiver closes once it has read what was
    /// sent, and changes made after this are sent nowhere
    ///
    /// A server ends it before it stops, since a follower's stream never ends of its own accord
    /// and a graceful stop waits for every open request.
    pub fn end_changes(&self) {
        // dropping the only sender closes every receiver
        self.changes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    /// Sends a change to every subscriber, if any is listening
    ///
    /// # Arguments
    ///
    /// * `change` - The change a write applied
    fn publish(&self, change: Change) {
        // a change with no subscriber, or after the stream has ended, is no failure
        let changes = self.changes.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = changes.as_ref() {
            let _ = sender.send(change);
        }
    }

    /// The store under this service
    ///
    /// Its writes are unchecked, so the front doors never write through it.
    pub fn store(&self) -> &Store<B> {
        &self.store
    }

    /// Gives up the service, keeping the store under it
    pub fn into_store(self) -> Store<B> {
        self.store
    }

    /// Registers a project for a repo, refusing a name or a repo another project has
    ///
    /// With sign-in required, only an admin registers a directory on the service's host.
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `repo` - A directory in the repo its plans describe
    /// * `asker` - Who asks
    #[instrument(name = "Service::add_project", skip_all, err(Debug))]
    pub async fn add_project(
        &self,
        name: &str,
        repo: &Path,
        asker: Asker<'_>,
    ) -> anyhow::Result<AddedProject> {
        // a directory on this host is an admin's to name, once callers sign in
        self.judge_on_host(asker, "register a project by a directory on its host")
            .await?;
        // register it, and tell the change stream if it was
        let added = self.store.add_project(name, repo).await?;
        if let AddedProject::Added(project) = &added {
            self.publish(Change::ProjectAdded {
                project: project.clone(),
            });
        }
        Ok(added)
    }

    /// Lists every project, by name
    #[instrument(name = "Service::projects", skip_all, err(Debug))]
    pub async fn projects(&self) -> anyhow::Result<Vec<Project>> {
        self.store.projects().await
    }

    /// Finds the project a directory belongs to, a linked worktree's main repo's for one in it,
    /// or the remote project one of its checkout's remotes was registered by
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory, such as the working directory
    /// * `remotes` - The URLs its checkout fetches from, in the order to try them
    #[instrument(name = "Service::find_project", skip_all, err(Debug))]
    pub async fn find_project(
        &self,
        dir: &Path,
        remotes: &[String],
    ) -> anyhow::Result<Option<Project>> {
        self.store.find_project(dir, remotes).await
    }

    /// Lists a plan in its project, and returns whether it was added
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::add_plan", skip_all, err(Debug))]
    pub async fn add_plan(&self, plan: &PlanName) -> anyhow::Result<bool> {
        // list it, and tell the change stream if it wasn't listed already
        let added = self.store.add_plan(plan).await?;
        if added {
            self.publish(Change::PlanAdded { plan: plan.clone() });
        }
        Ok(added)
    }

    /// Lists a project's plans, by name, refusing a project that isn't registered
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Service::plans", skip_all, err(Debug))]
    pub async fn plans(&self, project: &str) -> anyhow::Result<Vec<PlanInfo>> {
        // a project that isn't registered has no plans to list, and says so
        if self.store.project(project).await?.is_none() {
            return Err(NotRegistered(project.to_string()).into());
        }
        self.store.plans(project).await
    }

    /// Gets one node of a plan, at the version a put of it names
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `id` - The node's id
    #[instrument(name = "Service::get", skip_all, err(Debug))]
    pub async fn get(&self, plan: &PlanName, id: &str) -> anyhow::Result<Option<StoredNode>> {
        self.store.get(plan, id).await
    }

    /// Lists a plan's nodes, in the order the plan was written in
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::list", skip_all, err(Debug))]
    pub async fn list(&self, plan: &PlanName) -> anyhow::Result<Listing> {
        self.store.list(plan).await
    }

    /// Puts a batch of nodes into a plan, unless one is stale or they would add a problem
    ///
    /// This is [`Service::batch`] with no removals.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `puts` - The nodes to write, each with the version its writer read
    #[instrument(name = "Service::put", skip_all, err(Debug))]
    pub async fn put(&self, plan: &PlanName, puts: Vec<NodePut>) -> anyhow::Result<Checked> {
        // a batch of puts alone
        self.batch(plan, Puts::new(puts)).await
    }

    /// Puts and removes a batch of nodes in a plan, unless one is stale or they would add a
    /// problem
    ///
    /// Each put names the version its writer read, or `None` for a new node, and each removal
    /// the version its writer read. A stale node refuses the whole batch as [`Checked::Stale`],
    /// and a batch that would add a problem to the plan is refused as [`Checked::Refused`] with
    /// check's messages. Either writes nothing. A node another needs can't be removed alone, nor
    /// a group without its members.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `batch` - The nodes to write and to remove, each with the version its writer read
    #[instrument(name = "Service::batch", skip_all, err(Debug))]
    pub async fn batch(&self, plan: &PlanName, batch: Puts) -> anyhow::Result<Checked> {
        // the plan's docs are its project's working tree, or its clone at the commit fetched
        let docs = self.docs(plan).await?;
        let sections = self.sections.clone();
        // the ids put and removed, for the change the batch makes if it is written
        let ids: Vec<String> = batch
            .puts
            .iter()
            .map(|put| put.node.spec.id.clone())
            .collect();
        let removed: Vec<String> = batch
            .removes
            .iter()
            .map(|remove| remove.id.clone())
            .collect();
        // judged under the plan's write lock, against the plan the batch lands on
        let checked = self
            .store
            .batch_checked(plan, batch, move |proposal| added(docs, sections, proposal))
            .await?;
        // tell the change stream of each node written, at its new version, and each removed
        if let Checked::Applied { versions } = &checked {
            let nodes = ids
                .into_iter()
                .zip(versions.iter().copied())
                .map(|(id, version)| PutNode { id, version })
                .collect();
            self.publish(Change::NodesPut {
                plan: plan.clone(),
                nodes,
                removed,
            });
        }
        Ok(checked)
    }

    /// Writes a whole plan as a new revision, unless it would add a problem
    ///
    /// An import into an empty plan is refused if it has any problem at all. A write that
    /// carries pages replaces the plan's pages with them; one without leaves them as they are.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `write` - Every node of the plan, each a node of its own, and its pages if they change
    #[instrument(name = "Service::write_plan", skip_all, err(Debug))]
    pub async fn write_plan(&self, plan: &PlanName, write: PlanWrite) -> anyhow::Result<Rewrite> {
        // the plan's docs are its pages over its project's repo
        let docs = self.docs(plan).await?;
        let sections = self.sections.clone();
        // judged under the plan's write lock, against the plan it replaces
        let rewrite = self
            .store
            .write_plan_checked(plan, write, move |proposal| added(docs, sections, proposal))
            .await?;
        // tell the change stream of the revision readers now load
        if let Rewrite::Applied(rewritten) = &rewrite {
            self.publish(Change::PlanWritten {
                plan: plan.clone(),
                revision: rewritten.revision,
            });
        }
        Ok(rewrite)
    }

    /// Lists a plan's own pages, by path
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::pages", skip_all, err(Debug))]
    pub async fn pages(&self, plan: &PlanName) -> anyhow::Result<PageListing> {
        // a plan of a project that isn't registered says so
        self.docs(plan).await?;
        self.store.pages(plan).await
    }

    /// Gets one of a plan's own pages, or `None` if it keeps none at that path
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `path` - The page's path
    #[instrument(name = "Service::page", skip_all, err(Debug))]
    pub async fn page(&self, plan: &PlanName, path: &str) -> anyhow::Result<Option<StoredPage>> {
        // a plan of a project that isn't registered says so
        self.docs(plan).await?;
        self.store.page(plan, path).await
    }

    /// Writes or takes out one of a plan's own pages, unless it is stale or would add a problem
    ///
    /// A page put names the version its writer read, or `None` for a new page, and a removal
    /// the version its writer read. A stale page is refused as [`PageEdited::Stale`], and an
    /// edit that would add a problem to the plan, such as taking out a page a node links to, is
    /// refused as [`PageEdited::Refused`] with check's messages. Either writes nothing.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `edit` - The page to write or take out, with the version its writer read
    #[instrument(name = "Service::edit_page", skip_all, err(Debug))]
    pub async fn edit_page(&self, plan: &PlanName, edit: PageEdit) -> anyhow::Result<PageEdited> {
        // the plan's docs are its pages over its project's repo
        let docs = self.docs(plan).await?;
        let sections = self.sections.clone();
        let path = edit.path().to_string();
        // judged under the plan's write lock, against the plan the edit lands on
        let edited = self
            .store
            .edit_page_checked(plan, edit, move |proposal| added(docs, sections, proposal))
            .await?;
        // tell the change stream of the page written, at its new version, or taken out
        if let PageEdited::Applied { version } = &edited {
            let (pages, removed) = match version {
                Some(version) => (
                    vec![PutPage {
                        path,
                        version: *version,
                    }],
                    Vec::new(),
                ),
                None => (Vec::new(), vec![path]),
            };
            self.publish(Change::PagesPut {
                plan: plan.clone(),
                pages,
                removed,
            });
        }
        Ok(edited)
    }

    /// Finds a plan's problems, as `mdbook-reef check` finds a book's
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::check", skip_all, err(Debug))]
    pub async fn check(&self, plan: &PlanName) -> anyhow::Result<Vec<Diagnostic>> {
        // the plan and its pages as they stand, and the repo under them
        let (records, pages, docs) = self.snapshot(plan).await?;
        let sections = self.sections.clone();
        // build it off the async threads, since its docs are files or git's objects
        let problems = tokio::task::spawn_blocking(move || {
            let renderer = Renderer::new(Layered::new(&pages, docs.source()?), sections);
            anyhow::Ok(checked(&renderer, records, &pages).1)
        })
        .await??;
        Ok(problems)
    }

    /// Says what work in a plan is ready to start, its entries of its project's queue first,
    /// then the rest in plan order, and what the rest waits on, as `mdbook-reef next` says it of
    /// a book
    ///
    /// The plan is built as check builds it, so the answer counts its problems too; a plan with
    /// problems still has an order. The queue's entries of other plans are left out.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::next", skip_all, err(Debug))]
    pub async fn next(&self, plan: &PlanName) -> anyhow::Result<Next> {
        // the plan alone, narrowed from its project
        self.next_over(&plan.project, vec![plan.clone()]).await
    }

    /// Says what work across a project's plans is ready to start, the user's queue first, then
    /// each plan's other ready work in plan order, plan by plan, and what the rest waits on
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Service::project_next", skip_all, err(Debug))]
    pub async fn project_next(&self, project: &str) -> anyhow::Result<Next> {
        // every plan the project lists, by name, refused if it isn't registered
        let plans = self.plans(project).await?;
        let plans = plans.into_iter().map(|info| info.name).collect();
        self.next_over(project, plans).await
    }

    /// Lists a project's queue: each entry with its rank, and whether next takes it, and if not
    /// why
    ///
    /// Each plan an entry names is built without its docs, which no entry's state depends on.
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Service::queue", skip_all, err(Debug))]
    pub async fn queue(&self, project: &str) -> anyhow::Result<QueueListing> {
        // every plan the project lists, refused if it isn't registered
        let plans: Vec<PlanName> = self
            .plans(project)
            .await?
            .into_iter()
            .map(|info| info.name)
            .collect();
        let ranked = self.store.queue(project).await?;
        // each listed plan an entry names, built; an entry of a plan gone is missing
        let mut graphs = Vec::new();
        for plan in plans
            .iter()
            .filter(|p| ranked.iter().any(|r| r.node.plan == **p))
        {
            let records = self.store.list(plan).await?.records(plan)?;
            let (graph, problems) = Graph::build(nodes_from_records(records));
            graphs.push(PlanGraph {
                name: plan.clone(),
                graph,
                problems: problems.len(),
            });
        }
        // what each entry is now, beside its rank
        let queue: Vec<NodeName> = ranked.iter().map(|r| r.node.clone()).collect();
        let entries = reef_model::queue::entries(&graphs, &queue)
            .into_iter()
            .zip(ranked)
            .map(|(entry, ranked)| Listed {
                rank: ranked.rank,
                entry,
            })
            .collect();
        Ok(QueueListing { plans, entries })
    }

    /// Edits a project's queue: puts a node in it, moves one, or takes one off
    ///
    /// A node added must be in its plan, or the edit is [`Queued::NotInPlan`], and its plan must
    /// be listed. A node moved or taken off need not be, so an entry whose node is gone can
    /// still be taken off.
    ///
    /// # Arguments
    ///
    /// * `project` - The project whose queue to edit
    /// * `edit` - The edit
    #[instrument(name = "Service::edit_queue", skip_all, err(Debug))]
    pub async fn edit_queue(&self, project: &str, edit: QueueEdit) -> anyhow::Result<Queued> {
        // a project that isn't registered has no queue
        if self.store.project(project).await?.is_none() {
            return Err(NotRegistered(project.to_string()).into());
        }
        // a node added is one of this project's plans has, which the store checks is its own
        if let QueueEdit::Add { node, .. } = &edit
            && node.plan.project == project
            && self.store.get(&node.plan, &node.id).await?.is_none()
        {
            return Ok(Queued::NotInPlan { node: node.clone() });
        }
        // edited under the queue's lock, and told to the change stream once it is
        let queued = self.store.edit_queue(project, edit).await?;
        if let Queued::Applied { .. } = &queued {
            self.publish(Change::QueueChanged {
                project: project.to_string(),
            });
        }
        Ok(queued)
    }

    /// Says what work in some of a project's plans is ready to start, the queue's entries of
    /// those plans first
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `plans` - The plans to read, in the order their work is taken
    #[instrument(name = "Service::next_over", skip_all, err(Debug))]
    async fn next_over(&self, project: &str, plans: Vec<PlanName>) -> anyhow::Result<Next> {
        // the repo the plans' docs are read from, refused if it isn't registered
        let project_read = self
            .store
            .project(project)
            .await?
            .ok_or_else(|| NotRegistered(project.to_string()))?;
        let docs = Docs::of(&project_read);
        // each plan and its pages as they stand
        let mut read = Vec::with_capacity(plans.len());
        for plan in &plans {
            let (records, pages, _) = self.snapshot(plan).await?;
            read.push((plan.clone(), records, pages));
        }
        // and the queue's entries of those plans, in its order
        let queue: Vec<NodeName> = self
            .store
            .queue(project)
            .await?
            .into_iter()
            .map(|ranked| ranked.node)
            .filter(|node| plans.contains(&node.plan))
            .collect();
        let sections = self.sections.clone();
        // build them off the async threads, since their docs are files or git's objects, each
        // over its own pages and every one over the same read of the repo
        let next = tokio::task::spawn_blocking(move || {
            let repo = Cached::new(docs.source()?);
            let graphs: Vec<PlanGraph> = read
                .into_iter()
                .map(|(name, records, pages)| {
                    let renderer = Renderer::new(Layered::new(&pages, &repo), sections.clone());
                    let (graph, problems) = checked(&renderer, records, &pages);
                    PlanGraph {
                        name,
                        graph,
                        problems: problems.len(),
                    }
                })
                .collect();
            anyhow::Ok(Next::queued(&graphs, &queue))
        })
        .await??;
        Ok(next)
    }

    /// Builds a plan into the viewer's payload: every node, its body rendered, its links
    /// resolved
    ///
    /// A plan with problems still has a payload, built without the parts that broke. Its links
    /// to its docs are absolute paths under [`paths::docs`], where [`Service::doc_page`] serves
    /// them, since the viewer resolves a link against the window's URL.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::payload", skip_all, err(Debug))]
    pub async fn payload(&self, plan: &PlanName) -> anyhow::Result<Payload> {
        // the plan and its pages as they stand, and the repo under them
        let (records, pages, docs) = self.snapshot(plan).await?;
        let sections = self.sections.clone();
        // build and render it off the async threads, since its docs are files or git's objects
        let payload = tokio::task::spawn_blocking(move || {
            let renderer = Renderer::new(Layered::new(pages, docs.source()?), sections);
            let (graph, _) = build(&renderer, records);
            // a stored node has no page of its own
            anyhow::Ok(renderer.payload(&graph, |_| None))
        })
        .await??;
        // its links lead to the pages this service renders its docs as
        rooted(payload, &paths::docs(plan))
    }

    /// Renders one of a plan's docs as the page its links lead to, or `None` if no doc of the
    /// plan's is written to that page
    ///
    /// The page is the one mdbook would write for the doc: `docs/design.html` for the plan's
    /// page or the repo's file `docs/design.md`, its `<main>` holding the doc with each heading
    /// under the id mdbook gives it. A page the plan keeps hides the repo's file at its path. Its
    /// links lead under [`paths::docs`], as the payload's do. A page whose path leaves the repo,
    /// or whose doc a symlink leads out of it to, is not found.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `page` - The page's path from the root of the plan's docs
    #[instrument(name = "Service::doc_page", skip_all, err(Debug))]
    pub async fn doc_page(&self, plan: &PlanName, page: &str) -> anyhow::Result<Option<String>> {
        // the docs of a plan its project lists, read from that project's repo
        let repo = self.docs(plan).await?;
        let plans = self.store.plans(&plan.project).await?;
        if !plans.iter().any(|info| info.name == *plan) {
            return Err(NotListed(plan.clone()).into());
        }
        let docs = paths::docs(plan);
        // the plan's pages the page could be written from, read alone rather than with the rest
        let page = reef_render::paths::join("", page);
        let mut pages = MemSource::new();
        for md in reef_render::paths::md_of(&page) {
            // a path no page can have, such as one that climbs out, is left to the repo to refuse
            if check_page_path(&md).is_err() {
                continue;
            }
            if let Some(stored) = self.store.page(plan, &md).await? {
                pages.insert(stored.path, stored.text);
            }
        }
        // read and render it off the async threads, since its doc may be a file or a blob
        let html = tokio::task::spawn_blocking(move || {
            let source = Layered::new(pages, repo.source()?);
            anyhow::Ok(page_of(&source, &page).map(|html| html.replace(ROOT, &docs)))
        })
        .await??;
        Ok(html)
    }

    /// Reads a plan's records and its pages as they stand at one moment, and where its other
    /// docs are read from
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::snapshot", skip_all, err(Debug))]
    async fn snapshot(&self, plan: &PlanName) -> anyhow::Result<(Vec<Record>, MemSource, Docs)> {
        // the repo first, so a plan of an unknown project says so
        let docs = self.docs(plan).await?;
        let (listing, pages) = self.store.snapshot(plan).await?;
        let records = listing.records(plan)?;
        let pages = page_source(pages.pages.into_iter().map(PageWrite::from).collect());
        Ok((records, pages, docs))
    }

    /// Where a plan's docs are read from: its project's working tree, or for a remote project
    /// its clone at the commit last fetched
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::docs", skip_all, err(Debug))]
    async fn docs(&self, plan: &PlanName) -> anyhow::Result<Docs> {
        // a plan belongs to a project that is registered
        let project = self
            .store
            .project(&plan.project)
            .await?
            .ok_or_else(|| NotRegistered(plan.project.clone()))?;
        Ok(Docs::of(&project))
    }
}

/// Builds a plan as `mdbook-reef check` does: the checked graph, then each node briefed from
/// its docs
///
/// Returns the graph and its problems, the graph's own first.
///
/// # Arguments
///
/// * `renderer` - Reads the docs the nodes link to
/// * `records` - The plan's records, in written order
fn build<S: DocSource>(renderer: &Renderer<S>, records: Vec<Record>) -> (Graph, Vec<Diagnostic>) {
    // check the nodes as one graph
    let (mut graph, mut problems) = Graph::build(nodes_from_records(records));
    // check their doc links and fill in their brief forms from their docs
    problems.extend(renderer.prepare(&mut graph));
    (graph, problems)
}

/// Builds a plan as check does, its pages with it: the checked graph, then each node briefed
/// from its docs, then each link of a page that names a node the plan doesn't have
///
/// Returns the graph and its problems, the nodes' first, then the pages'.
///
/// # Arguments
///
/// * `renderer` - Reads the docs the nodes link to, the plan's pages over its repo
/// * `records` - The plan's records, in written order
/// * `pages` - The plan's pages, whose links to nodes are checked
fn checked<S: DocSource>(
    renderer: &Renderer<S>,
    records: Vec<Record>,
    pages: &MemSource,
) -> (Graph, Vec<Diagnostic>) {
    // the nodes as check builds them
    let (graph, mut problems) = build(renderer, records);
    // and each page's links to nodes, against the nodes built
    problems.extend(page_node_problems(&graph, pages.iter()));
    (graph, problems)
}

/// The problems a write would add to a plan, judged off the async threads
///
/// Both sides read the repo through one cache, so they see its docs as they were at one
/// moment, and each is read once. A write that leaves the pages as they stand builds both sides
/// with one renderer; one that changes them builds each over its own pages.
///
/// # Arguments
///
/// * `docs` - The repo under the plan's pages: its working tree, or its clone at a commit
/// * `sections` - The doc sections a node's brief form is taken from
/// * `proposal` - The plan as it stands and as the write would leave it
async fn added(
    docs: Docs,
    sections: Sections,
    proposal: Proposal,
) -> anyhow::Result<Vec<Diagnostic>> {
    let Proposal {
        before,
        after,
        pages,
        pages_after,
    } = proposal;
    // its docs are files or git's objects, so build both sides off the async threads
    let added = tokio::task::spawn_blocking(move || {
        let repo = Cached::new(docs.source()?);
        let pages = page_source(pages);
        let (before, after) = match pages_after {
            // the same docs on both sides, each read once
            None => {
                let renderer = Renderer::new(Layered::new(&pages, &repo), sections);
                (
                    checked(&renderer, before, &pages).1,
                    checked(&renderer, after, &pages).1,
                )
            }
            // each side over its own pages, the repo read once for both
            Some(pages_after) => {
                let pages_after = page_source(pages_after);
                let stood = Renderer::new(Layered::new(&pages, &repo), sections.clone());
                let left = Renderer::new(Layered::new(&pages_after, &repo), sections);
                (
                    checked(&stood, before, &pages).1,
                    checked(&left, after, &pages_after).1,
                )
            }
        };
        anyhow::Ok(new_problems(&before, after))
    })
    .await??;
    Ok(added)
}

/// A plan's pages as a source of docs, each by its path
///
/// # Arguments
///
/// * `pages` - The plan's pages
fn page_source(pages: Vec<PageWrite>) -> MemSource {
    pages
        .into_iter()
        .map(|page| (page.path, page.text))
        .collect()
}

/// A payload with its links put under the path its plan's docs are served from
///
/// Every link a payload holds starts with `ROOT`, wherever in it the link sits, so the root is
/// swapped in its JSON, as mdbook-reef fills in each page's way back to a book's root.
///
/// # Arguments
///
/// * `payload` - The payload, its links starting with `ROOT`
/// * `docs` - The path its plan's docs are served from, ending in `/`
fn rooted(payload: Payload, docs: &str) -> anyhow::Result<Payload> {
    // the docs' path is percent-encoded, so it needs no escaping in JSON
    let json = serde_json::to_string(&payload)?.replace(ROOT, docs);
    Ok(serde_json::from_str(&json)?)
}

/// A doc rendered as the page at a path, its links starting with `ROOT`, or `None` if no doc
/// of the source's is written to that page
///
/// # Arguments
///
/// * `source` - Where the plan's docs are read from
/// * `page` - The page's path from the root of the docs
fn page_of<S: DocSource>(source: &S, page: &str) -> Option<String> {
    // a page is named as a link to it would be, with `.` and `..` worked out
    let page = reef_render::paths::join("", page);
    // the first markdown file the page could be written from that the source has
    reef_render::paths::md_of(&page).into_iter().find_map(|md| {
        let text = source.read(&md)?;
        Some(render_page(doc_body(&text), &md))
    })
}

/// The problems found after a write that weren't found before it, in the order found after
///
/// Problems are compared as check prints them, and counted, so a second copy of a problem the
/// plan already had is a new one.
///
/// # Arguments
///
/// * `before` - The plan's problems as it stands
/// * `after` - Its problems as the write would leave it
fn new_problems(before: &[Diagnostic], after: Vec<Diagnostic>) -> Vec<Diagnostic> {
    // how many times each problem is found before the write
    let mut seen: HashMap<String, usize> = HashMap::new();
    for problem in before {
        *seen.entry(problem.to_string()).or_default() += 1;
    }
    // each problem after it that isn't one of those
    after
        .into_iter()
        .filter(|problem| match seen.get_mut(&problem.to_string()) {
            Some(count) if *count > 0 => {
                *count -= 1;
                false
            }
            _ => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Asker, Service, new_problems};
    use crate::store::{Checked, NodePut, NodeWrite};
    use reef_api::{Change, PutNode};
    use reef_model::{Description, Diagnostic, NodeSpec, PlanName};
    use std::path::Path;
    use tokio::sync::broadcast::error::TryRecvError;

    /// A problem at a node's name, as a stored node's are
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id in `reef/central`
    /// * `message` - What the problem is
    fn problem(id: &str, message: &str) -> Diagnostic {
        Diagnostic {
            source: format!("reef/central/{id}"),
            line: None,
            message: message.to_string(),
        }
    }

    /// A piece of work that says why and what, needing what it names
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    /// * `needs` - The ids it needs
    fn work(id: &str, needs: &[&str]) -> NodeWrite {
        NodeWrite {
            spec: NodeSpec {
                id: id.to_string(),
                needs: needs.iter().map(|need| need.to_string()).collect(),
                description: Some(Description {
                    why: Some("Something is missing.".to_string()),
                    what: Some("It is there.".to_string()),
                    details: None,
                }),
                ..NodeSpec::default()
            },
            body: String::new(),
            order: 0,
        }
    }

    /// A put that breaks the plan is refused with the message check gives the plan it would make
    #[tokio::test]
    async fn breaking_put_is_refused_with_checks_message() -> anyhow::Result<()> {
        // a plan of a project for the repo this crate is in
        let service = Service::memory();
        service
            .add_project(
                "reef",
                Path::new(env!("CARGO_MANIFEST_DIR")),
                Asker::default(),
            )
            .await?;
        let plan = PlanName::new("reef", "central")?;
        service.add_plan(&plan).await?;
        // a node needing one that isn't there is refused, with check's message, and not written
        let put = NodePut::new(work("C11", &["X"]), None);
        let refused = service.put(&plan, vec![put]).await?;
        let expected = vec![problem("C11", "needs `X`, which is not a node")];
        assert_eq!(refused, Checked::Refused(expected.clone()));
        assert!(service.list(&plan).await?.nodes.is_empty());
        // the same node written unchecked is the problem check finds in the plan it makes
        service
            .store()
            .put(&plan, work("C11", &["X"]), None)
            .await?;
        assert_eq!(service.check(&plan).await?, expected);
        Ok(())
    }

    /// An applied put tells the change stream of each node at its new version, and a refused
    /// one tells it nothing
    #[tokio::test]
    async fn applied_put_is_a_change_and_refused_put_is_none() -> anyhow::Result<()> {
        // a plan of a project for the repo this crate is in, followed from before it is listed
        let service = Service::memory();
        let mut changes = service.changes();
        service
            .add_project(
                "reef",
                Path::new(env!("CARGO_MANIFEST_DIR")),
                Asker::default(),
            )
            .await?;
        let plan = PlanName::new("reef", "central")?;
        service.add_plan(&plan).await?;
        // registering the project and listing the plan are changes of their own
        assert!(
            matches!(changes.try_recv()?, Change::ProjectAdded { project } if project.name == "reef")
        );
        assert_eq!(
            changes.try_recv()?,
            Change::PlanAdded { plan: plan.clone() }
        );
        // a put that breaks the plan is refused, and nothing is sent
        let refused = service
            .put(&plan, vec![NodePut::new(work("C11", &["X"]), None)])
            .await?;
        assert!(matches!(refused, Checked::Refused(_)));
        assert!(changes.try_recv().is_err());
        // one that is written names each node at its new version
        service
            .put(&plan, vec![NodePut::new(work("C11", &[]), None)])
            .await?;
        let expected = Change::NodesPut {
            plan: plan.clone(),
            nodes: vec![PutNode {
                id: "C11".to_string(),
                version: 1,
            }],
            removed: Vec::new(),
        };
        assert_eq!(changes.try_recv()?, expected);
        // once the stream is ended, every receiver closes, and a new one is closed from the start
        service.end_changes();
        assert_eq!(changes.try_recv(), Err(TryRecvError::Closed));
        assert_eq!(service.changes().try_recv(), Err(TryRecvError::Closed));
        // and a write after it still goes in, sent nowhere
        let after = service
            .put(&plan, vec![NodePut::new(work("C53", &["C11"]), None)])
            .await?;
        assert!(matches!(after, Checked::Applied { .. }));
        Ok(())
    }

    /// A problem the plan had before a write isn't new after it, but a second copy of it is
    #[test]
    fn new_problems_counts_repeats() {
        // one copy before, two after
        let twice = problem("a", "needs `x`, which is not a node");
        let before = vec![twice.clone()];
        let after = vec![twice.clone(), problem("b", "new"), twice.clone()];
        // the second copy is new, and so is the problem never seen
        assert_eq!(
            new_problems(&before, after),
            vec![problem("b", "new"), twice]
        );
        // and a write that fixes a problem adds none
        assert!(new_problems(&before, Vec::new()).is_empty());
    }

    /// New problems come out in the order check found them after the write
    #[test]
    fn new_problems_keep_after_order() {
        // three new problems, found in an order that isn't sorted
        let after = vec![problem("c", "3"), problem("a", "1"), problem("b", "2")];
        assert_eq!(new_problems(&[], after.clone()), after);
    }

    /// Every checked write and every build is a future a multi-threaded server can run
    #[test]
    fn futures_are_send() {
        /// Accepts only what can be sent across threads
        ///
        /// # Arguments
        ///
        /// * `_future` - The future to accept
        fn send<T: Send>(_future: T) {}
        // never run: this test passes once it compiles
        let service = Service::memory();
        let plan = PlanName::new("reef", "central").unwrap();
        send(service.put(&plan, Vec::new()));
        send(service.write_plan(&plan, Vec::new().into()));
        send(service.check(&plan));
        send(service.payload(&plan));
        send(service.doc_page(&plan, "design.html"));
        send(service.pages(&plan));
        send(service.page(&plan, "design.md"));
        let page = crate::store::PageEdit::Remove {
            path: "design.md".to_string(),
            read: 1,
        };
        send(service.edit_page(&plan, page));
        send(service.next(&plan));
        send(service.project_next("reef"));
        send(service.queue("reef"));
        let edit = crate::store::QueueEdit::Remove {
            node: plan.node("C1").unwrap(),
        };
        send(service.edit_queue("reef", edit));
    }
}
