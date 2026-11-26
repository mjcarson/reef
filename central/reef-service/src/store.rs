//! reef's store: plans, their nodes and their pages, read and written by version
//!
//! A store keeps the logic, over a [`Backend`] that reads and writes rows the way shoal does:
//! [`ShoalBackend`] in service, [`MemoryBackend`] in tests and on stable.
//!
//! - **Versions by conditional write.** Each node row carries a version, and a put names the one
//!   it read. A stale put is refused as [`Put::Stale`], an answer and not a failure.
//! - **Writes to a plan serialized.** reef-service is shoal's only writer, so a lock per plan in
//!   this process keeps a write out of a revision another write is about to retire. Reads take
//!   the same lock shared, so a read never sees a revision part-pruned.
//! - **Every row checked against its full key.** Shoal hashes partition keys to a `u64`, and a
//!   collision is silent, so a row from another partition is caught rather than served.
//! - **Whole-plan writes and batches as revisions.** Shoal has no transactions, so a bulk write,
//!   or a batch of more than one write, goes to a new revision's partition, named `pending` on
//!   the plan's row first, and is made current by one conditional write to that row. A write
//!   killed midway leaves the old revision current and whole, and the next write prunes what it
//!   left. A batch's revision carries every node it doesn't touch at the version it was at.
//! - **A plan's pages in revisions of their own.** A whole-plan write that carries pages writes
//!   them under its new revision too, and its flip makes both current at once. A plan's row
//!   names its pages' revision apart from its nodes', so a batch of node writes leaves the pages
//!   where they are rather than copying them. A page edit is one row's conditional write in the
//!   pages' current revision, checked as a node write is.
//! - **Checks under the plan's lock.** The store keeps rows; it doesn't judge what they make.
//!   `batch_checked` and `write_plan_checked` hand a caller's check the plan as it stands and as
//!   the write would leave it, under the plan's write lock, and write only if it finds no
//!   problem. A batch puts and removes nodes as one change. [`Service`](crate::Service) checks every write this way; `put` and `write_plan`
//!   are the unchecked layer under it.
//! - **A project's queue sorted by rank.** Each entry is one row in its project's partition,
//!   sorted by a [`rank`](crate::rank) string, so adding, moving or removing an entry writes one
//!   row. A move writes the entry's new row, then deletes its old one only if it is still the row
//!   read. Of two rows for one node, the one written later wins, so a move killed between the two
//!   reads as made, and the next edit prunes the other. A rank grown long spreads the queue anew,
//!   one entry at a time, last first, so the queue reads in order after every step. Queue writes
//!   take a lock per project; reads take none.
//! - **A node's work record a row** ([`work`]): where its work happens, what it started from, and
//!   what each capture since read, as JSON in its project's partition. A start is written where
//!   no record is, or over the version of the one it replaces.
//! - **A node's revisions in its own partition** ([`revision`]): each capture of its changes a
//!   row listing its files, each file a row of its revision's with its hunks, the files written
//!   first so a revision that reads has every file it lists.
//! - **A plan's comments in its own partition** ([`comment`]): each comment a row numbered in
//!   its plan, a thread's first holding the thread's target and state, which a mark writes again
//!   by version, and where its line went in later revisions, which a capture's follow writes
//!   ([`follow`]).
//! - **Every terminal loop in one partition** ([`loops`]): each a row under the pane it feeds,
//!   so a pane keeps one loop, and every change to one is judged against them all under one
//!   lock, so two loops never hold one node.
//! - **Every user and every token in a partition of its own** ([`users`]): a user under their
//!   name with their password's hash, added under one lock and judged against every user, and a
//!   token under its hash alone, so no token is ever stored.

mod branches;
mod comment;
mod decision;
mod follow;
mod history;
mod loops;
mod remotes;
mod revision;
mod users;
mod work;

pub use branches::{BranchProposal, BranchSet, KeptBranch, StoredBranch};
pub use comment::{Followed, Pending};
pub use history::BadCursor;
pub use remotes::{KeptRemote, StoredRemote};
pub use users::{StoredToken, StoredUser, UserDropped, UserKept};

pub use revision::FILE_ROW_MAX;

// the store's data types are the JSON API's, so the service and its clients share them
pub use reef_api::{
    AddCapture, CaptureRead, CaptureShas, Captured, FileDiff, FileSummary, Revision, StartWork,
    Started, WorkRecord, WorkStart,
};
pub use reef_api::{
    AddedProject, Checked, Listing, NodePut, NodeRemove, NodeWrite, PageEdit, PageEdited,
    PageListing, PageWrite, Place, PlanInfo, PlanWrite, Project, Puts, QueueEdit, Queued, Ranked,
    Rewrite, Rewritten, StalePut, StoredNode, StoredPage, records,
};
use reef_api::{Author, Edited, PLAN_PAGES_MAX_BYTES, check_page_path};
pub use reef_api::{
    Comment, Commented, MarkReview, MarkThread, Marked, NewReply, NewThread, Resolution,
    ResolveThread, Resolved, Resolves, Review, ReviewState, Reviewed, Thread, TrailerRead,
};
use reef_model::{Diagnostic, NodeName, PlanName, Record};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tracing::instrument;

use crate::backend::{Backend, Expect, Pointers, Refusal, RevisionKey, Written};
use crate::client::{Mode, ShoalBackend};
use crate::conf::Conf;
use crate::memory::MemoryBackend;
use crate::rank;
use crate::remote;
use crate::repo::{self, Located};
use crate::schema::{NodeRow, PROJECT_SUBJECT, PROJECTS, PageRow, PlanRow, ProjectRow, QueueRow};

/// How many rows a bulk write sends in one bundle, at most
const BUNDLE_ROWS: usize = 256;

/// How many bytes of specs, bodies and pages a bulk write sends in one bundle, at most
///
/// A bundle must fit in one of shoal's frames, 64 MiB by default. A row bigger than this goes
/// in a bundle of its own.
const BUNDLE_BYTES: usize = 16 << 20;

/// A plan as it stands and as a write would leave it, for a check to judge
#[derive(Debug, Clone)]
pub struct Proposal {
    /// The plan's nodes as they stand, in written order
    pub before: Vec<Record>,
    /// The plan's nodes as the write would leave them, in written order
    pub after: Vec<Record>,
    /// The plan's pages as they stand, by path
    pub pages: Vec<PageWrite>,
    /// The plan's pages as the write would leave them, or `None` for a write that leaves them
    /// as they stand
    pub pages_after: Option<Vec<PageWrite>>,
    /// The branch the plan follows, read under its write lock, whose docs the check reads, or
    /// `None` for its project's own
    pub branch: Option<String>,
}

/// What came of a put
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Put {
    /// The put was written, and the node is now at this version
    Applied {
        /// The node's new version
        version: u64,
    },
    /// The node was not as the put expected, so nothing was written
    Stale(Refusal),
}

/// A project the store was asked about that isn't registered
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotRegistered(pub String);

impl std::fmt::Display for NotRegistered {
    /// Names the project
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "project `{}` is not registered", self.0)
    }
}

impl std::error::Error for NotRegistered {}

/// A plan the store was asked about that isn't listed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotListed(pub PlanName);

impl std::fmt::Display for NotListed {
    /// Names the plan
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "plan `{}` is not listed", self.0)
    }
}

impl std::error::Error for NotListed {}

/// A write the store can't keep as it was given, such as a node with members inline
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unkeepable(pub String);

impl std::fmt::Display for Unkeepable {
    /// Says what the store can't keep, and how to write it instead
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unkeepable {}

/// reef's store, over shoal or its in-memory twin
pub struct Store<B = ShoalBackend> {
    /// The rows under the store
    backend: B,
    /// Each plan's lock: shared by reads, held alone by writes
    locks: Mutex<HashMap<PlanName, Arc<RwLock<()>>>>,
    /// Held to register a project, so two can't take one repo between a check and a write
    projects: tokio::sync::Mutex<()>,
    /// Each project's queue lock, held alone by its writes
    queues: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Each project's work records' lock, held alone by their writes
    works: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Each project's comments' lock, held alone by their writes
    comments: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The loops' lock, held alone by their writes, since every loop shares one partition
    loop_lock: tokio::sync::Mutex<()>,
    /// Each project's decisions' lock, held alone by their writes
    decisions: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The users' lock, held to add one, so two can't both be the first
    user_lock: tokio::sync::Mutex<()>,
    /// The last stamp an edit kept in a history took, so the next is always past it
    edit_clock: Mutex<u64>,
    /// A random number this store's edit keys end with, so a store opened again after a restart
    /// never takes a key the last one wrote within the same millisecond
    edit_nonce: u64,
}

impl Store<ShoalBackend> {
    /// Opens a store: the external node reef's config names, else a pool in this process
    ///
    /// Only one process can open a data directory, so a process opens one store; most callers
    /// want [`Service::shared`](crate::Service::shared).
    ///
    /// # Arguments
    ///
    /// * `conf` - reef's config
    #[instrument(name = "Store::open", skip_all, err(Debug))]
    pub async fn open(conf: &Conf) -> anyhow::Result<Store> {
        // reach shoal however the config says to
        let backend = ShoalBackend::open(conf).await?;
        Ok(Store::new(backend))
    }

    /// How this store reaches shoal
    pub fn mode(&self) -> Mode {
        self.backend.mode()
    }

    /// Closes the store, stopping its in-process pool if it started one
    #[instrument(name = "Store::close", skip_all, err(Debug))]
    pub async fn close(self) -> anyhow::Result<()> {
        // the backend holds the client and the pool
        self.backend.close().await
    }
}

impl Store<MemoryBackend> {
    /// A store over an empty in-memory twin
    pub fn memory() -> Store<MemoryBackend> {
        Store::new(MemoryBackend::new())
    }
}

impl<B: Backend> Store<B> {
    /// A store over a backend
    ///
    /// # Arguments
    ///
    /// * `backend` - The rows under the store
    pub fn new(backend: B) -> Store<B> {
        Store {
            backend,
            locks: Mutex::new(HashMap::new()),
            projects: tokio::sync::Mutex::new(()),
            queues: Mutex::new(HashMap::new()),
            works: Mutex::new(HashMap::new()),
            comments: Mutex::new(HashMap::new()),
            loop_lock: tokio::sync::Mutex::new(()),
            decisions: Mutex::new(HashMap::new()),
            user_lock: tokio::sync::Mutex::new(()),
            edit_clock: Mutex::new(0),
            edit_nonce: fresh_revision(),
        }
    }

    /// The rows under this store
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Gives up the store, keeping the rows under it
    pub fn into_backend(self) -> B {
        self.backend
    }

    /// Finds a plan's lock, making it on first use
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan whose lock to find
    fn plan_lock(&self, plan: &PlanName) -> Arc<RwLock<()>> {
        // the map is only held to find or add the lock, never across a wait
        self.locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(plan.clone())
            .or_default()
            .clone()
    }

    /// Waits to read a plan: alongside other reads, never during a write
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan to read
    async fn reading(&self, plan: &PlanName) -> OwnedRwLockReadGuard<()> {
        self.plan_lock(plan).read_owned().await
    }

    /// Waits to write a plan: alone
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan to write
    async fn writing(&self, plan: &PlanName) -> OwnedRwLockWriteGuard<()> {
        self.plan_lock(plan).write_owned().await
    }

    /// Finds a project's queue lock, making it on first use
    ///
    /// # Arguments
    ///
    /// * `project` - The project whose queue's lock to find
    fn queue_lock(&self, project: &str) -> Arc<tokio::sync::Mutex<()>> {
        // the map is only held to find or add the lock, never across a wait
        self.queues
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(project.to_string())
            .or_default()
            .clone()
    }

    /// Gets a plan's row, checked against its full key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::plan_row", skip_all, err(Debug))]
    async fn plan_row(&self, plan: &PlanName) -> anyhow::Result<PlanRow> {
        // a plan with no row is not listed, which a caller can tell from a failure
        let row = self
            .backend
            .plan(plan)
            .await?
            .ok_or_else(|| NotListed(plan.clone()))?;
        // a row of another project whose key hashed the same is a collision, not this plan
        anyhow::ensure!(
            row.project == plan.project && row.name == plan.plan,
            "plan row `{}/{}` was stored where `{plan}` is: their partition keys' hashes collide",
            row.project,
            row.name
        );
        Ok(row)
    }

    /// Registers a project for a repo, refusing a name or a repo another project has
    ///
    /// The repo is located first: a directory in a git repo, a linked worktree's too, names
    /// its main worktree's root, so one repo is one path however it was reached. Shoal has no
    /// secondary index, so the repo is checked by listing every project, under a lock this
    /// process holds, since it is shoal's only writer.
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `repo` - A directory in the repo its plans describe
    /// * `author` - Who registers it, which its history keeps once it is
    #[instrument(name = "Store::add_project", skip_all, err(Debug))]
    pub async fn add_project(
        &self,
        name: &str,
        repo: &Path,
        author: &Author,
    ) -> anyhow::Result<AddedProject> {
        // a project's name keeps the rule every plan's full name holds it to
        PlanName::new(name, "plan")?;
        // find the repo's root, which asks git, off the async threads
        let dir = repo.to_path_buf();
        let repo = tokio::task::spawn_blocking(move || repo::locate(&dir))
            .await??
            .into_path();
        let repo_text = repo
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("{} is not a utf-8 path", repo.display()))?
            .to_string();
        // one registration at a time, so the repo can't be taken between the check and the write
        let _registering = self.projects.lock().await;
        if let Some(taken) = self.projects().await?.into_iter().find(|p| p.repo == repo) {
            return Ok(AddedProject::RepoTaken {
                project: taken.name,
            });
        }
        // written only if no project of that name is
        let row = ProjectRow {
            list: PROJECTS.to_string(),
            name: name.to_string(),
            repo: repo_text,
        };
        Ok(match self.backend.insert_project(row).await? {
            Written::Applied => {
                // who registered it, kept in its history now that it is
                let registered = self
                    .keep_registered(name, PROJECT_SUBJECT.to_string(), author)
                    .await;
                AddedProject::Added(Project {
                    name: name.to_string(),
                    repo,
                    remote: None,
                    registered,
                })
            }
            Written::Refused(_) => AddedProject::NameTaken,
        })
    }

    /// Lists every project, by name, each remote one with its remote
    ///
    /// A project's remote is joined to it only if both name the same clone, so a remote a
    /// registration killed partway left never attaches to a later project of that name.
    #[instrument(name = "Store::projects", skip_all, err(Debug))]
    pub async fn projects(&self) -> anyhow::Result<Vec<Project>> {
        // every project sits in the one listing partition, and every remote in another
        let rows = self.backend.projects().await?;
        let mut remotes = self.remotes().await?;
        rows.into_iter()
            .map(|row| {
                // a row of another partition that hashed the same is a collision, not a project
                anyhow::ensure!(
                    row.list == PROJECTS,
                    "project row `{}` from `{}` was stored where the projects are listed: their \
                     partition keys' hashes collide",
                    row.name,
                    row.list
                );
                // its remote, if one names the clone its row does
                let repo = PathBuf::from(row.repo);
                let remote = remotes
                    .remove(&row.name)
                    .filter(|stored| stored.kept.clone == repo)
                    .map(|stored| stored.kept.remote());
                Ok(Project {
                    name: row.name,
                    repo,
                    remote,
                    registered: None,
                })
            })
            .collect()
    }

    /// Finds a registered project by name
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    #[instrument(name = "Store::project", skip_all, err(Debug))]
    pub async fn project(&self, name: &str) -> anyhow::Result<Option<Project>> {
        // every project sits in the one listing partition, which is small
        let projects = self.projects().await?;
        Ok(projects.into_iter().find(|project| project.name == name))
    }

    /// Lists a project's plans, by name
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Store::plans", skip_all, err(Debug))]
    pub async fn plans(&self, project: &str) -> anyhow::Result<Vec<PlanInfo>> {
        // a project's plans sit in that project's partition, and their branches in another
        let rows = self.backend.plans(project).await?;
        let mut branches = self.plan_branches(project).await?;
        rows.into_iter()
            .map(|row| {
                // a row of another project whose key hashed the same is a collision
                anyhow::ensure!(
                    row.project == project,
                    "plan row `{}/{}` was stored where `{project}`'s plans are listed: their \
                     partition keys' hashes collide",
                    row.project,
                    row.name
                );
                // the branch it follows, if it has one of its own
                let branch = branches
                    .remove(&row.name)
                    .and_then(|stored| stored.kept.branch);
                Ok(PlanInfo {
                    name: PlanName::new(row.project, row.name)?,
                    revision: row.current,
                    registered: None,
                    branch,
                })
            })
            .collect()
    }

    /// Finds the project a directory belongs to, or one of its checkout's remotes
    ///
    /// A directory on this host is matched first, against the local projects. One in a git repo
    /// belongs to the project registered for that repo's main worktree, so a linked worktree
    /// finds its main repo's project, and one outside git belongs to the project registered for
    /// its nearest registered ancestor. Failing that, or for a directory on another host, the
    /// first of its checkout's remotes that a remote project was registered by finds it, however
    /// either URL is spelled.
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory, such as the working directory
    /// * `remotes` - The URLs its checkout fetches from, in the order to try them
    #[instrument(name = "Store::find_project", skip_all, err(Debug))]
    pub async fn find_project(
        &self,
        dir: &Path,
        remotes: &[String],
    ) -> anyhow::Result<Option<Project>> {
        let projects = self.projects().await?;
        // a directory this host has, among the projects whose repos are on it
        let on_host = tokio::fs::metadata(dir)
            .await
            .is_ok_and(|meta| meta.is_dir());
        if on_host {
            // locate the directory, which asks git, off the async threads
            let dir = dir.to_path_buf();
            let located = tokio::task::spawn_blocking(move || repo::locate(&dir)).await??;
            let mut local = projects.iter().filter(|project| project.remote.is_none());
            let found = match located {
                // git draws a repo's bounds, so only the project for its root
                Located::Repo(root) => local.find(|project| project.repo == root),
                // outside git, the deepest project that holds the directory
                Located::Dir(dir) => local
                    .filter(|project| dir.starts_with(&project.repo))
                    .max_by_key(|project| project.repo.components().count()),
            };
            if let Some(found) = found {
                return Ok(Some(found.clone()));
            }
        }
        // else the first remote of its checkout's that a remote project was registered by
        for url in remotes {
            let Some(wanted) = remote::key(url) else {
                continue;
            };
            let found = projects.iter().find(|project| {
                project
                    .remote
                    .as_ref()
                    .is_some_and(|remote| remote::key(&remote.url).as_ref() == Some(&wanted))
            });
            if let Some(found) = found {
                return Ok(Some(found.clone()));
            }
        }
        Ok(None)
    }

    /// Lists a plan in its project, with an empty revision to put nodes into
    ///
    /// Returns whether it was added; a plan already listed is left as it is. Its project must
    /// be registered.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `author` - Who lists it, which its history keeps once it is
    #[instrument(name = "Store::add_plan", skip_all, err(Debug))]
    pub async fn add_plan(&self, plan: &PlanName, author: &Author) -> anyhow::Result<bool> {
        // a plan belongs to a project that is registered
        let projects = self.projects().await?;
        if !projects.iter().any(|project| project.name == plan.project) {
            return Err(NotRegistered(plan.project.clone()).into());
        }
        // its first revision is current and empty, so a put has somewhere to go, and its pages'
        // revision is the same one, empty too
        let revision = fresh_revision();
        let row = PlanRow {
            project: plan.project.clone(),
            name: plan.plan.clone(),
            current: revision,
            pending: 0,
            retired: 0,
            pages_current: revision,
            pages_retired: 0,
            version: 1,
        };
        // written only if its project lists no plan of that name
        let _write = self.writing(plan).await;
        let added = self.backend.insert_plan(row).await? == Written::Applied;
        // who listed it, kept in its history under the same lock, if it was added
        if added {
            self.keep_registered(&plan.project, history::plan_subject(plan), author)
                .await;
        }
        Ok(added)
    }

    /// Gets one node of a plan's current revision
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `id` - The node's id
    #[instrument(name = "Store::get", skip_all, err(Debug))]
    pub async fn get(&self, plan: &PlanName, id: &str) -> anyhow::Result<Option<StoredNode>> {
        // never while a write is retiring the revision this reads
        let _read = self.reading(plan).await;
        let row = self.plan_row(plan).await?;
        // the one row of the current revision
        let nodes = self.revision(plan, row.current, Some(id)).await?;
        Ok(nodes.into_iter().next())
    }

    /// Lists a plan's current revision, in the order the plan was written in
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::list", skip_all, err(Debug))]
    pub async fn list(&self, plan: &PlanName) -> anyhow::Result<Listing> {
        // never while a write is retiring the revision this reads
        let _read = self.reading(plan).await;
        let row = self.plan_row(plan).await?;
        // every row of the current revision, put in written order
        let mut nodes = self.revision(plan, row.current, None).await?;
        listed_order(&mut nodes);
        Ok(Listing {
            revision: row.current,
            nodes,
        })
    }

    /// Lists a plan's pages as they stand, by path
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::pages", skip_all, err(Debug))]
    pub async fn pages(&self, plan: &PlanName) -> anyhow::Result<PageListing> {
        // never while a write is retiring the revision this reads
        let _read = self.reading(plan).await;
        let row = self.plan_row(plan).await?;
        // every page of the pages' current revision
        let pages = self.page_revision(plan, row.pages_current, None).await?;
        Ok(PageListing {
            revision: row.pages_current,
            pages,
        })
    }

    /// Gets one of a plan's pages, or `None` if it has none at that path
    ///
    /// A path no page can have names none, so asking for one is no failure.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `path` - The page's path
    #[instrument(name = "Store::page", skip_all, err(Debug))]
    pub async fn page(&self, plan: &PlanName, path: &str) -> anyhow::Result<Option<StoredPage>> {
        // never while a write is retiring the revision this reads
        let _read = self.reading(plan).await;
        let row = self.plan_row(plan).await?;
        // a path no page can have is no page, though the plan must be listed
        if check_page_path(path).is_err() {
            return Ok(None);
        }
        // the one row of the pages' current revision
        let pages = self
            .page_revision(plan, row.pages_current, Some(path))
            .await?;
        Ok(pages.into_iter().next())
    }

    /// Reads a plan's nodes and its pages as they stand at one moment
    ///
    /// Both are read under one hold of the plan's read lock, from one read of its row, so no
    /// write lands between them.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::snapshot", skip_all, err(Debug))]
    pub async fn snapshot(&self, plan: &PlanName) -> anyhow::Result<(Listing, PageListing)> {
        // never while a write is retiring the revisions this reads
        let _read = self.reading(plan).await;
        let row = self.plan_row(plan).await?;
        // every node of the current revision, put in written order
        let mut nodes = self.revision(plan, row.current, None).await?;
        listed_order(&mut nodes);
        // and every page of the pages' current revision
        let pages = self.page_revision(plan, row.pages_current, None).await?;
        Ok((
            Listing {
                revision: row.current,
                nodes,
            },
            PageListing {
                revision: row.pages_current,
                pages,
            },
        ))
    }

    /// Reads a revision's pages, or one of them, checking each against its full key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision of the plan's pages to read
    /// * `path` - The one page to read, or `None` for all of them
    #[instrument(name = "Store::page_revision", skip_all, err(Debug))]
    async fn page_revision(
        &self,
        plan: &PlanName,
        revision: u64,
        path: Option<&str>,
    ) -> anyhow::Result<Vec<StoredPage>> {
        // the revision's partition, whole or one row of it
        let key = RevisionKey::new(plan, revision);
        let rows = self.backend.pages(&key, path).await?;
        let mut pages = rows
            .into_iter()
            .map(|row| {
                // a row of another partition that hashed the same is a collision, not a page
                anyhow::ensure!(
                    key.holds_page(&row) && path.is_none_or(|path| row.path == path),
                    "page row `{}/{}/{}` at revision {} was stored where `{plan}`'s pages at \
                     revision {revision} are: their partition keys' hashes collide",
                    row.project,
                    row.plan,
                    row.path,
                    row.revision
                );
                Ok(stored_page(row))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        // by path, as a listing reads
        pages.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(pages)
    }

    /// Reads a revision's rows, or one of them, checking each against its full key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision to read
    /// * `id` - The one node to read, or `None` for all of them
    #[instrument(name = "Store::revision", skip_all, err(Debug))]
    async fn revision(
        &self,
        plan: &PlanName,
        revision: u64,
        id: Option<&str>,
    ) -> anyhow::Result<Vec<StoredNode>> {
        // the revision's partition, whole or one row of it
        let key = RevisionKey::new(plan, revision);
        let rows = self.backend.nodes(&key, id).await?;
        rows.into_iter()
            .map(|row| {
                // a row of another partition that hashed the same is a collision, not a node
                anyhow::ensure!(
                    key.holds(&row) && id.is_none_or(|id| row.id == id),
                    "node row `{}/{}/{}` at revision {} was stored where `{plan}` at revision \
                     {revision} is: their partition keys' hashes collide",
                    row.project,
                    row.plan,
                    row.id,
                    row.revision
                );
                stored(row)
            })
            .collect()
    }

    /// Writes one node into a plan's current revision, if it is as the writer read it
    ///
    /// A put names the version it read the node at, or `None` for a node that must not exist
    /// yet. A node that has moved on, or isn't as expected, comes back as [`Put::Stale`].
    ///
    /// This doesn't check what the node does to its plan; `put_checked` does.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `node` - The node as it should be written
    /// * `read` - The version it was read at, or `None` for a new node
    #[instrument(name = "Store::put", skip_all, err(Debug))]
    pub async fn put(
        &self,
        plan: &PlanName,
        node: NodeWrite,
        read: Option<u64>,
    ) -> anyhow::Result<Put> {
        // a node with members inline would hide them from every read by id
        check_node(plan, &node)?;
        // one write to this plan at a time, so this never lands in a revision being retired
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // prune whatever a killed write left, so it never outlives the next write
        let (row, _) = self.recover(plan, row).await?;
        // the row as it should be, one version on from the one read
        let (version, expect) = match read {
            Some(read) => (read + 1, Expect::Version(read)),
            None => (1, Expect::Absent),
        };
        let new = node_row(plan, row.current, node, version)?;
        // written only over the version read, or only where there is no row
        Ok(match self.backend.write_node(new, expect).await? {
            Written::Applied => Put::Applied { version },
            Written::Refused(refusal) => Put::Stale(refusal),
        })
    }

    /// Writes a whole plan as a new revision, and makes it current in one write
    ///
    /// The steps, under the plan's lock:
    ///
    /// 1. Prune any revision a killed write left pending or retired.
    /// 2. Read the current revision's versions, so each node's moves on, and the pages' if the
    ///    write carries pages.
    /// 3. Reserve the new revision: name it `pending` on the plan's row.
    /// 4. Write its rows, nodes then pages, each bundle's answers checked.
    /// 5. Flip: make it current and the old one retired, in one conditional write, and its
    ///    pages current too if it carries pages.
    /// 6. Prune the old revision, and its pages if they were replaced, and clear `retired`.
    ///
    /// A write killed or failed before step 5 leaves the old revision current and whole, with
    /// the new one named pending for the next write to prune. One that fails pruning after step
    /// 5 has still made its revision current, and says so with `left_retired`. A write without
    /// pages leaves the plan's pages as they are.
    ///
    /// This doesn't check what the nodes make of the plan; `write_plan_checked` does.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `write` - Every node of the plan, each a node of its own, and its pages if they change
    #[instrument(name = "Store::write_plan", skip_all, err(Debug))]
    pub async fn write_plan(&self, plan: &PlanName, write: PlanWrite) -> anyhow::Result<Rewritten> {
        let PlanWrite { nodes, pages } = write;
        // every node and page must be one the store can keep, under a name no other has
        check_plan_nodes(plan, &nodes)?;
        if let Some(pages) = &pages {
            check_pages(pages)?;
        }
        // one write to this plan at a time
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // 1. prune whatever a killed write left behind
        let (row, recovered) = self.recover(plan, row).await?;
        // 2. carry each node's version on, so a put read before this write is refused after it
        let old: HashMap<String, u64> = self.keys(plan, row.current).await?.into_iter().collect();
        // and each page's, if the pages change
        let pages = match pages {
            Some(pages) => {
                let old: HashMap<String, u64> = self
                    .page_keys(plan, row.pages_current)
                    .await?
                    .into_iter()
                    .collect();
                Some(pages_moved_on(pages, &old))
            }
            None => None,
        };
        // 3 to 6. write the new revision, make it current and prune the old one
        self.rewrite(plan, row, recovered, moved_on(nodes, &old), pages)
            .await
    }

    /// Writes a whole plan as a new revision, if a check finds no problem it would add
    ///
    /// Under the plan's write lock, this prunes what a killed write left, reads the current
    /// revision and pages, and hands `check` the plan as it stands and as this write would make
    /// it. A check that finds problems refuses the write before a revision is reserved, so
    /// nothing is written. Otherwise the plan is written as `write_plan` writes it.
    ///
    /// The check runs under the plan's write lock, so it must not read or write this plan
    /// through the store, or it waits forever.
    ///
    /// Once written, each node and page it added, changed or removed keeps an entry in its
    /// history, marked as a whole plan's write.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `write` - Every node of the plan, each a node of its own, and its pages if they change
    /// * `author` - Who writes it
    /// * `check` - Finds the problems the write would add, or none
    #[instrument(name = "Store::write_plan_checked", skip_all, err(Debug))]
    pub async fn write_plan_checked<F, Fut>(
        &self,
        plan: &PlanName,
        write: PlanWrite,
        author: &Author,
        check: F,
    ) -> anyhow::Result<Rewrite>
    where
        F: FnOnce(Proposal) -> Fut + Send,
        Fut: Future<Output = anyhow::Result<Vec<Diagnostic>>> + Send,
    {
        let PlanWrite { mut nodes, pages } = write;
        // every node and page must be one the store can keep, under a name no other has
        check_plan_nodes(plan, &nodes)?;
        if let Some(pages) = &pages {
            check_pages(pages)?;
        }
        // one write to this plan at a time, so the check judges the plan this lands on
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // prune whatever a killed write left behind
        let (row, recovered) = self.recover(plan, row).await?;
        // the revision as it stands, whose versions each node carries on
        let mut current = self.revision(plan, row.current, None).await?;
        listed_order(&mut current);
        let old: HashMap<String, u64> = current
            .iter()
            .map(|node| (node.spec.id.clone(), node.version))
            .collect();
        // and the pages as they stand, whose versions each page carries on
        let stood = self.page_revision(plan, row.pages_current, None).await?;
        let old_pages: HashMap<String, u64> = stood
            .iter()
            .map(|page| (page.path.clone(), page.version))
            .collect();
        // judge the plan as this write would make it, and write nothing if it adds a problem
        in_written_order(&mut nodes);
        let stood_nodes: Vec<NodeWrite> = current.into_iter().map(NodeWrite::from).collect();
        let stood_pages: Vec<PageWrite> = stood.into_iter().map(PageWrite::from).collect();
        let proposal = Proposal {
            before: records(plan, stood_nodes.clone())?,
            after: records(plan, nodes.clone())?,
            pages: stood_pages.clone(),
            pages_after: pages.clone(),
            // the branch it follows, read under the same lock, so it is judged where it lands
            branch: self.plan_branch(plan).await?,
        };
        let problems = check(proposal).await?;
        if !problems.is_empty() {
            return Ok(Rewrite::Refused(problems));
        }
        // what it changes of each node and page, told once it is written
        let nodes = moved_on(nodes, &old);
        let pages = pages.map(|pages| pages_moved_on(pages, &old_pages));
        let before: HashMap<String, NodeWrite> = stood_nodes
            .into_iter()
            .map(|node| (node.spec.id.clone(), node))
            .collect();
        let pages_before: HashMap<String, PageWrite> = stood_pages
            .into_iter()
            .map(|page| (page.path.clone(), page))
            .collect();
        let edits = history::plan_edits(plan, &before, &nodes, &pages_before, pages.as_deref())?;
        // write the new revision, make it current and prune the old one
        let rewritten = self.rewrite(plan, row, recovered, nodes, pages).await?;
        self.keep_edits(&plan.project, author, true, edits).await;
        Ok(Rewrite::Applied(rewritten))
    }

    /// Writes a plan's nodes as a new revision, makes it current, and prunes the old one
    ///
    /// These are steps 3 to 6 of `write_plan`, and how a batch of several writes lands. The
    /// caller holds the plan's write lock, has pruned what a killed write left, and has decided
    /// each node's version, and each page's if the pages change.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `row` - The plan's row as it now stands
    /// * `recovered` - How many rows a killed write had left, which the caller pruned
    /// * `nodes` - Every node of the plan, each a node of its own, with the version to write it at
    /// * `pages` - Every page of the plan with the version to write it at, or `None` to leave
    ///   the pages as they are
    #[instrument(name = "Store::rewrite", skip_all, err(Debug))]
    async fn rewrite(
        &self,
        plan: &PlanName,
        row: PlanRow,
        recovered: usize,
        nodes: Vec<(NodeWrite, u64)>,
        pages: Option<Vec<(PageWrite, u64)>>,
    ) -> anyhow::Result<Rewritten> {
        // 3. reserve the new revision, so a write killed from here on can be found
        let revision = fresh_revision();
        let reserve = Pointers {
            pending: Some(revision),
            ..Pointers::default()
        };
        let version = self.update_plan(plan, row.version, reserve).await?;
        // 4. write its rows, a bundle at a time
        let mut rows = Vec::with_capacity(nodes.len());
        for (node, version) in nodes {
            rows.push(node_row(plan, revision, node, version)?);
        }
        for bundle in bundles(rows, |row| row.spec.len() + row.body.len()) {
            self.backend.write_nodes(bundle).await?;
        }
        // and its pages under the same revision, if they change
        let replaces_pages = pages.is_some();
        if let Some(pages) = pages {
            let rows: Vec<PageRow> = pages
                .into_iter()
                .map(|(page, version)| page_row(plan, revision, page, version))
                .collect();
            for bundle in bundles(rows, |row| row.path.len() + row.text.len()) {
                self.backend.write_pages(bundle).await?;
            }
        }
        // 5. flip: the new revision current, the old one retired, in one conditional write, and
        // its pages current over the old ones if it carries them
        let mut flip = Pointers {
            current: Some(revision),
            pending: Some(0),
            retired: Some(row.current),
            ..Pointers::default()
        };
        if replaces_pages {
            flip.pages_current = Some(revision);
            flip.pages_retired = Some(row.pages_current);
        }
        let version = self.update_plan(plan, version, flip).await?;
        // 6. prune the old revision, and its pages if they were replaced, then say there is
        // nothing left to prune; the new revision is current either way, so a failure here is
        // left for the next write
        let pruned = match self.prune_nodes(plan, row.current).await {
            Ok(_) if replaces_pages => self.prune_pages(plan, row.pages_current).await,
            pruned => pruned,
        };
        let left_retired = match pruned {
            Ok(_) => {
                let cleared = Pointers {
                    retired: Some(0),
                    pages_retired: replaces_pages.then_some(0),
                    ..Pointers::default()
                };
                self.update_plan(plan, version, cleared).await.is_err()
            }
            Err(error) => {
                tracing::warn!(%plan, error = %format!("{error:#}"), "pruning a retired revision failed");
                true
            }
        };
        Ok(Rewritten {
            revision,
            recovered,
            left_retired,
        })
    }

    /// Writes a batch of nodes to a plan as one change, if each is as its writer read it and a
    /// check finds no problem they would add
    ///
    /// This is [`Store::batch_checked`] with no removals.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `puts` - The nodes to write, each with the version its writer read
    /// * `author` - Who writes them
    /// * `check` - Finds the problems the puts would add, or none
    #[instrument(name = "Store::put_checked", skip_all, err(Debug))]
    pub async fn put_checked<F, Fut>(
        &self,
        plan: &PlanName,
        puts: Vec<NodePut>,
        author: &Author,
        check: F,
    ) -> anyhow::Result<Checked>
    where
        F: FnOnce(Proposal) -> Fut + Send,
        Fut: Future<Output = anyhow::Result<Vec<Diagnostic>>> + Send,
    {
        // a batch of puts alone
        self.batch_checked(plan, Puts::new(puts), author, check)
            .await
    }

    /// Writes a batch of puts and removals to a plan as one change, if each node is as its writer
    /// read it and a check finds no problem the batch would add
    ///
    /// The steps, under the plan's write lock:
    ///
    /// 1. Prune whatever a killed write left.
    /// 2. Read the current revision whole.
    /// 3. Refuse the batch as [`Checked::Stale`] if any put or removal finds its node other than
    ///    its writer read it, naming every such node.
    /// 4. Hand `check` the plan as it stands and as the batch would leave it, each removed node
    ///    gone and each put replacing its node in place or joining the plan, and refuse the
    ///    batch as [`Checked::Refused`] if it finds problems.
    /// 5. Write it so it lands whole or not at all. One put or one removal is a single row's
    ///    write, which shoal makes atomic: the put only over the version its writer read. A
    ///    batch of more is written as a revision of its own, as `write_plan` writes one, with
    ///    every node it doesn't touch at the version it was at, so their writers' edits stay
    ///    good, each put one version on from the one read, and each removed node left out.
    ///
    /// A refused batch writes nothing, and so does one killed or failed before its revision is
    /// made current: the plan stays as it was, and the next write prunes what it left. A
    /// single write refused in step 5 means another process wrote the plan, since reef-service
    /// is shoal's only writer, and is an error.
    ///
    /// Once written, each node a put changed, and each one removed, keeps an entry in its
    /// history. A put that left its node as it was keeps none.
    ///
    /// The check runs under the plan's write lock, so it must not read or write this plan
    /// through the store, or it waits forever.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `batch` - The nodes to write and to remove, each with the version its writer read
    /// * `author` - Who writes it
    /// * `check` - Finds the problems the batch would add, or none
    #[instrument(name = "Store::batch_checked", skip_all, err(Debug))]
    pub async fn batch_checked<F, Fut>(
        &self,
        plan: &PlanName,
        batch: Puts,
        author: &Author,
        check: F,
    ) -> anyhow::Result<Checked>
    where
        F: FnOnce(Proposal) -> Fut + Send,
        Fut: Future<Output = anyhow::Result<Vec<Diagnostic>>> + Send,
    {
        let Puts { puts, removes } = batch;
        // every node must be one the store can keep, each put or removed once
        check_batch(plan, &puts, &removes)?;
        // one write to this plan at a time, so the check judges the plan the batch lands on
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // 1. prune whatever a killed write left
        let (row, recovered) = self.recover(plan, row).await?;
        // 2. the revision as it stands, in written order
        let mut current = self.revision(plan, row.current, None).await?;
        listed_order(&mut current);
        // 3. every node must be as its writer read it, before anything is judged
        let stale = stale_batch(&current, &puts, &removes);
        if !stale.is_empty() {
            return Ok(Checked::Stale(stale));
        }
        // an empty batch changes nothing
        if puts.is_empty() && removes.is_empty() {
            return Ok(Checked::Applied {
                versions: Vec::new(),
            });
        }
        // each node the batch touches as it stood, for its history to tell what changed
        let touched: HashSet<&str> = puts
            .iter()
            .map(|put| put.node.spec.id.as_str())
            .chain(removes.iter().map(|remove| remove.id.as_str()))
            .collect();
        let stood: HashMap<String, NodeWrite> = current
            .iter()
            .filter(|node| touched.contains(node.spec.id.as_str()))
            .map(|node| (node.spec.id.clone(), NodeWrite::from(node.clone())))
            .collect();
        // 4. the plan as the batch would leave it, each node at the version it would be at: each
        // removal gone, each node it doesn't touch as it is, and each put replacing its node in
        // place or joining it, one version on from the one read
        let before = records(plan, current.iter().cloned().map(NodeWrite::from).collect())?;
        let removed: HashSet<&str> = removes.iter().map(|remove| remove.id.as_str()).collect();
        let mut after: Vec<(NodeWrite, u64)> = current
            .into_iter()
            .filter(|node| !removed.contains(node.spec.id.as_str()))
            .map(|node| {
                let version = node.version;
                (NodeWrite::from(node), version)
            })
            .collect();
        let at: HashMap<String, usize> = after
            .iter()
            .enumerate()
            .map(|(i, (node, _))| (node.spec.id.clone(), i))
            .collect();
        let mut versions = Vec::with_capacity(puts.len());
        for put in &puts {
            let version = put.read.map_or(1, |read| read + 1);
            versions.push(version);
            match at.get(&put.node.spec.id) {
                Some(&i) => after[i] = (put.node.clone(), version),
                None => after.push((put.node.clone(), version)),
            }
        }
        // judge it over the pages as they stand, which a batch leaves where they are, and write
        // nothing if it adds a problem
        let mut judged: Vec<NodeWrite> = after.iter().map(|(node, _)| node.clone()).collect();
        in_written_order(&mut judged);
        let pages = self.page_revision(plan, row.pages_current, None).await?;
        let proposal = Proposal {
            before,
            after: records(plan, judged)?,
            pages: pages.into_iter().map(PageWrite::from).collect(),
            pages_after: None,
            // the branch it follows, read under the same lock, so it is judged where it lands
            branch: self.plan_branch(plan).await?,
        };
        let problems = check(proposal).await?;
        if !problems.is_empty() {
            return Ok(Checked::Refused(problems));
        }
        // what each put and removal changes, told once it is written
        let edits = history::batch_edits(plan, &stood, &puts, &versions, &removes)?;
        // 5. one write lands whole as it is, and several as a revision of their own
        if puts.len() + removes.len() == 1 {
            self.write_one(plan, &row, puts, removes).await?;
        } else {
            self.rewrite(plan, row, recovered, after, None).await?;
        }
        self.keep_edits(&plan.project, author, false, edits).await;
        Ok(Checked::Applied { versions })
    }

    /// Writes a batch's one put, or deletes its one removal, in the plan's current revision
    ///
    /// The put is written only over the version its writer read, or only where there is no row
    /// for a new node; the removal's version was checked under the plan's write lock. The caller
    /// holds that lock and has judged the batch.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `row` - The plan's row as it now stands
    /// * `puts` - The batch's puts: one, or none if it removes
    /// * `removes` - The batch's removals: one, or none if it puts
    #[instrument(name = "Store::write_one", skip_all, err(Debug))]
    async fn write_one(
        &self,
        plan: &PlanName,
        row: &PlanRow,
        puts: Vec<NodePut>,
        removes: Vec<NodeRemove>,
    ) -> anyhow::Result<()> {
        // a put, one version on from the one read
        for put in puts {
            let id = put.node.spec.id.clone();
            let (version, expect) = match put.read {
                Some(read) => (read + 1, Expect::Version(read)),
                None => (1, Expect::Absent),
            };
            let new = node_row(plan, row.current, put.node, version)?;
            let written = self
                .backend
                .write_node(new, expect)
                .await
                .map_err(|error| error.context(format!("writing node `{id}` of plan `{plan}`")))?;
            // this process holds the plan's lock and saw the node as read, so another wrote it
            if let Written::Refused(refusal) = written {
                anyhow::bail!(
                    "node `{id}` of plan `{plan}` changed under this batch ({refusal}): another \
                     process wrote the plan"
                );
            }
        }
        // or a removal, deleted outright
        let key = RevisionKey::new(plan, row.current);
        for remove in removes {
            let id = remove.id;
            self.backend
                .delete_nodes(&key, vec![id.clone()])
                .await
                .map_err(|error| error.context(format!("removing node `{id}` of plan `{plan}`")))?;
        }
        Ok(())
    }

    /// Writes or takes out one of a plan's pages, if it is as its writer read it and a check
    /// finds no problem the edit would add
    ///
    /// The steps, under the plan's write lock:
    ///
    /// 1. Prune whatever a killed write left.
    /// 2. Read the current revision and the pages as they stand.
    /// 3. Refuse the edit as [`PageEdited::Stale`] if the page is other than its writer read it.
    /// 4. Hand `check` the plan as it stands, over its pages as they stand and as the edit would
    ///    leave them, and refuse the edit as [`PageEdited::Refused`] if it finds problems.
    /// 5. Write the page into the pages' current revision, only over the version read or only
    ///    where there is no row, or delete it.
    ///
    /// A page write refused in step 5 means another process wrote the plan, since reef-service
    /// is shoal's only writer, and is an error.
    ///
    /// The check runs under the plan's write lock, so it must not read or write this plan
    /// through the store, or it waits forever.
    ///
    /// Once written, a page added, edited or taken out keeps an entry in its history; one written
    /// again as it was keeps none.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `edit` - The page to write or take out, with the version its writer read
    /// * `author` - Who edits it
    /// * `check` - Finds the problems the edit would add, or none
    #[instrument(name = "Store::edit_page_checked", skip_all, err(Debug))]
    pub async fn edit_page_checked<F, Fut>(
        &self,
        plan: &PlanName,
        edit: PageEdit,
        author: &Author,
        check: F,
    ) -> anyhow::Result<PageEdited>
    where
        F: FnOnce(Proposal) -> Fut + Send,
        Fut: Future<Output = anyhow::Result<Vec<Diagnostic>>> + Send,
    {
        // the page must be one the store can keep
        match &edit {
            PageEdit::Put { page, .. } => check_page(page)?,
            PageEdit::Remove { path, .. } => check_page_path(path).map_err(Unkeepable)?,
        }
        // one write to this plan at a time, so the check judges the plan the edit lands on
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // 1. prune whatever a killed write left
        let (row, _) = self.recover(plan, row).await?;
        // 2. the plan as it stands, its nodes in written order and its pages by path
        let mut current = self.revision(plan, row.current, None).await?;
        listed_order(&mut current);
        let pages = self.page_revision(plan, row.pages_current, None).await?;
        // 3. the page must be as its writer read it, before anything is judged
        let path = edit.path().to_string();
        let stored = pages
            .iter()
            .find(|page| page.path == path)
            .map(|page| page.version);
        // what the edit changes of the page, told once it is written
        let edited = history::page_edit(
            pages
                .iter()
                .find(|page| page.path == path)
                .map(|page| page.text.as_str()),
            history::page_text(&edit),
        );
        let read = match &edit {
            PageEdit::Put { read, .. } => *read,
            PageEdit::Remove { read, .. } => Some(*read),
        };
        let refusal = match (stored, read) {
            (None, None) => None,
            (Some(stored), Some(read)) if stored == read => None,
            (Some(_), None) => Some(Refusal::Exists),
            (None, Some(_)) => Some(Refusal::Missing),
            (Some(_), Some(_)) => Some(Refusal::Moved),
        };
        if let Some(refusal) = refusal {
            return Ok(PageEdited::Stale { path, refusal });
        }
        // 4. the pages as the edit would leave them, the page put in its place or taken out
        let before: Vec<PageWrite> = pages.into_iter().map(PageWrite::from).collect();
        let mut after: Vec<PageWrite> = before
            .iter()
            .filter(|page| page.path != path)
            .cloned()
            .collect();
        if let PageEdit::Put { page, .. } = &edit {
            after.push(page.clone());
            after.sort_by(|a, b| a.path.cmp(&b.path));
        }
        check_pages_size(&after)?;
        // judged over the plan's nodes as they stand, and nothing written if it adds a problem
        let nodes = records(plan, current.into_iter().map(NodeWrite::from).collect())?;
        let proposal = Proposal {
            before: nodes.clone(),
            after: nodes,
            pages: before,
            pages_after: Some(after),
            // the branch it follows, read under the same lock, so it is judged where it lands
            branch: self.plan_branch(plan).await?,
        };
        let problems = check(proposal).await?;
        if !problems.is_empty() {
            return Ok(PageEdited::Refused(problems));
        }
        // 5. the page written in place, one version on from the one read, or deleted outright
        match edit {
            PageEdit::Put { page, read } => {
                let (version, expect) = match read {
                    Some(read) => (read + 1, Expect::Version(read)),
                    None => (1, Expect::Absent),
                };
                let new = page_row(plan, row.pages_current, page, version);
                let written = self
                    .backend
                    .write_page(new, expect)
                    .await
                    .map_err(|error| {
                        error.context(format!("writing page `{path}` of plan `{plan}`"))
                    })?;
                // this process holds the plan's lock and saw the page as read, so another wrote it
                if let Written::Refused(refusal) = written {
                    anyhow::bail!(
                        "page `{path}` of plan `{plan}` changed under this edit ({refusal}): \
                         another process wrote the plan"
                    );
                }
                self.keep_page_edit(plan, &path, Some(version), edited, author)
                    .await;
                Ok(PageEdited::Applied {
                    version: Some(version),
                })
            }
            PageEdit::Remove { .. } => {
                let key = RevisionKey::new(plan, row.pages_current);
                self.backend
                    .delete_pages(&key, vec![path.clone()])
                    .await
                    .map_err(|error| {
                        error.context(format!("removing page `{path}` of plan `{plan}`"))
                    })?;
                self.keep_page_edit(plan, &path, None, edited, author).await;
                Ok(PageEdited::Applied { version: None })
            }
        }
    }

    /// Keeps a page's edit in its history, if it changed the page
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan the page is one of
    /// * `path` - The page's path
    /// * `version` - The version the edit left it at, or `None` for one that took it out
    /// * `edited` - What the edit changed of it, or `None` for nothing
    /// * `author` - Who edited it
    async fn keep_page_edit(
        &self,
        plan: &PlanName,
        path: &str,
        version: Option<u64>,
        edited: Option<Edited>,
        author: &Author,
    ) {
        // a page written again as it was keeps nothing
        let Some(edited) = edited else {
            return;
        };
        let pending = history::Pending {
            subject: history::page_subject(plan, path),
            version,
            edits: vec![edited],
        };
        self.keep_edits(&plan.project, author, false, vec![pending])
            .await;
    }

    /// Prunes any revision a killed or failed write left named on a plan's row
    ///
    /// Each table is pruned of the revisions it may hold that aren't its current one: the
    /// nodes of the pending and retired revisions, and the pages of the pending revision and
    /// the pages' retired one. A revision can be current in one table and retired in the other,
    /// since a batch of node writes leaves the pages where they are, so neither table is pruned
    /// by the other's pointers.
    ///
    /// Returns the plan's row as it now stands, and how many rows were pruned. The caller holds
    /// the plan's write lock.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `row` - The plan's row as it was read
    #[instrument(name = "Store::recover", skip_all, err(Debug))]
    async fn recover(&self, plan: &PlanName, row: PlanRow) -> anyhow::Result<(PlanRow, usize)> {
        // nothing named is nothing left over
        if row.pending == 0 && row.retired == 0 && row.pages_retired == 0 {
            return Ok((row, 0));
        }
        // only a revision that is named and isn't the table's current one is left over
        let nodes_left = [row.pending, row.retired]
            .into_iter()
            .filter(|revision| *revision != 0 && *revision != row.current);
        let pages_left = [row.pending, row.pages_retired]
            .into_iter()
            .filter(|revision| *revision != 0 && *revision != row.pages_current);
        // prune each, then say there is nothing left to prune
        let mut pruned = 0;
        for revision in nodes_left {
            pruned += self.prune_nodes(plan, revision).await?;
        }
        for revision in pages_left {
            pruned += self.prune_pages(plan, revision).await?;
        }
        let cleared = Pointers {
            pending: Some(0),
            retired: Some(0),
            pages_retired: Some(0),
            ..Pointers::default()
        };
        let version = self.update_plan(plan, row.version, cleared).await?;
        tracing::info!(%plan, pruned, "pruned what a killed write left");
        Ok((
            PlanRow {
                pending: 0,
                retired: 0,
                pages_retired: 0,
                version,
                ..row
            },
            pruned,
        ))
    }

    /// Changes a plan's pointers, if its row is at the version read, and returns its new version
    ///
    /// The caller holds the plan's write lock, so a refusal means another process wrote it.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `read` - The version the row was read at
    /// * `pointers` - The revisions that change
    #[instrument(name = "Store::update_plan", skip_all, err(Debug))]
    async fn update_plan(
        &self,
        plan: &PlanName,
        read: u64,
        pointers: Pointers,
    ) -> anyhow::Result<u64> {
        // written only over the version read
        match self.backend.update_plan(plan, read, pointers).await? {
            Written::Applied => Ok(read + 1),
            Written::Refused(refusal) => anyhow::bail!(
                "plan `{plan}` changed under this write, from version {read}: {refusal}"
            ),
        }
    }

    /// Lists a revision's node ids and versions, checking each against its full key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision to list
    #[instrument(name = "Store::keys", skip_all, err(Debug))]
    async fn keys(&self, plan: &PlanName, revision: u64) -> anyhow::Result<Vec<(String, u64)>> {
        // the whole partition, projected to its keys
        let key = RevisionKey::new(plan, revision);
        let keys = self.backend.node_keys(&key).await?;
        keys.into_iter()
            .map(|node| {
                // a key of another partition that hashed the same is never this revision's
                anyhow::ensure!(
                    key.holds_key(&node),
                    "node key `{}/{}/{}` at revision {} was stored where `{plan}` at revision \
                     {revision} is: their partition keys' hashes collide",
                    node.project,
                    node.plan,
                    node.id,
                    node.revision
                );
                Ok((node.id, node.version))
            })
            .collect()
    }

    /// Deletes every node row of a revision, and returns how many there were
    ///
    /// Shoal can't delete a partition, so this lists the revision's keys and deletes each row,
    /// a bundle at a time. A key of another partition is refused before anything is deleted,
    /// since deleting by this revision's key would delete the other's row.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision to delete
    #[instrument(name = "Store::prune_nodes", skip_all, err(Debug))]
    async fn prune_nodes(&self, plan: &PlanName, revision: u64) -> anyhow::Result<usize> {
        // find the rows to delete
        let key = RevisionKey::new(plan, revision);
        let mut ids: Vec<String> = self
            .keys(plan, revision)
            .await?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let count = ids.len();
        // and delete them, each answer checked
        while !ids.is_empty() {
            let rest = ids.split_off(ids.len().min(BUNDLE_ROWS));
            self.backend
                .delete_nodes(&key, std::mem::replace(&mut ids, rest))
                .await?;
        }
        Ok(count)
    }

    /// Lists a revision's page paths and versions, checking each against its full key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision of the plan's pages to list
    #[instrument(name = "Store::page_keys", skip_all, err(Debug))]
    async fn page_keys(
        &self,
        plan: &PlanName,
        revision: u64,
    ) -> anyhow::Result<Vec<(String, u64)>> {
        // the whole partition, projected to its keys
        let key = RevisionKey::new(plan, revision);
        let keys = self.backend.page_keys(&key).await?;
        keys.into_iter()
            .map(|page| {
                // a key of another partition that hashed the same is never this revision's
                anyhow::ensure!(
                    key.holds_page_key(&page),
                    "page key `{}/{}/{}` at revision {} was stored where `{plan}`'s pages at \
                     revision {revision} are: their partition keys' hashes collide",
                    page.project,
                    page.plan,
                    page.path,
                    page.revision
                );
                Ok((page.path, page.version))
            })
            .collect()
    }

    /// Deletes every page row of a revision, and returns how many there were
    ///
    /// As with nodes, a key of another partition is refused before anything is deleted.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision of the plan's pages to delete
    #[instrument(name = "Store::prune_pages", skip_all, err(Debug))]
    async fn prune_pages(&self, plan: &PlanName, revision: u64) -> anyhow::Result<usize> {
        // find the rows to delete
        let key = RevisionKey::new(plan, revision);
        let mut paths: Vec<String> = self
            .page_keys(plan, revision)
            .await?
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        let count = paths.len();
        // and delete them, each answer checked
        while !paths.is_empty() {
            let rest = paths.split_off(paths.len().min(BUNDLE_ROWS));
            self.backend
                .delete_pages(&key, std::mem::replace(&mut paths, rest))
                .await?;
        }
        Ok(count)
    }

    /// Lists a project's queue, in its order
    ///
    /// No lock is taken: every step of a queue write leaves the queue reading in order. A node
    /// in two rows, as a move killed midway leaves it, reads as the row written later.
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Store::queue", skip_all, err(Debug))]
    pub async fn queue(&self, project: &str) -> anyhow::Result<Vec<Ranked>> {
        // each node's winning row, in rank order
        let (rows, _) = settle(self.queue_rows(project).await?);
        ranked_all(&rows)
    }

    /// Edits a project's queue: puts a node in it, moves one, or takes one off
    ///
    /// Each edit writes the node's one row, and a move deletes its old one after. Adding a node
    /// already queued moves it. Moving or taking off a node that isn't queued, or placing one
    /// beside a node that isn't, is [`Queued::NotQueued`], and changes nothing. A placement that
    /// would make a rank longer than [`rank::LONG`] spreads the queue's ranks anew first.
    ///
    /// The store keeps rows; it doesn't check the nodes named are in their plans. The service
    /// does.
    ///
    /// An edit that changed the queue keeps an entry in its history, under the queue's lock.
    ///
    /// # Arguments
    ///
    /// * `project` - The project whose queue to edit
    /// * `edit` - The edit
    /// * `author` - Who edits it
    #[instrument(name = "Store::edit_queue", skip_all, err(Debug))]
    pub async fn edit_queue(
        &self,
        project: &str,
        edit: QueueEdit,
        author: &Author,
    ) -> anyhow::Result<Queued> {
        // every node the edit names is of this project's
        let named = std::iter::once(edit.node()).chain(edit.place().and_then(Place::anchor));
        for node in named {
            if node.plan.project != project {
                let message =
                    format!("node `{node}` is not of project `{project}`, whose queue this is");
                return Err(Unkeepable(message).into());
            }
        }
        // one write to a project's queue at a time, its history kept under the same lock
        let lock = self.queue_lock(project);
        let _write = lock.lock().await;
        let (queued, edited) = self.edit_queue_held(project, &edit).await?;
        if let Some(edited) = edited {
            let pending = history::Pending {
                subject: crate::schema::QUEUE_SUBJECT.to_string(),
                version: None,
                edits: vec![edited],
            };
            self.keep_edits(project, author, false, vec![pending]).await;
        }
        Ok(queued)
    }

    /// Edits a project's queue under its lock, which the caller holds, and answers what came of
    /// it and what it changed, `None` for an edit that changed nothing
    ///
    /// # Arguments
    ///
    /// * `project` - The project whose queue to edit
    /// * `edit` - The edit
    async fn edit_queue_held(
        &self,
        project: &str,
        edit: &QueueEdit,
    ) -> anyhow::Result<(Queued, Option<Edited>)> {
        // the queue as it stands, after pruning what a killed write left
        let read = self.queue_rows(project).await?;
        let mut seq = read.iter().map(|row| row.seq).max().unwrap_or(0);
        let (mut rows, leftovers) = settle(read);
        self.prune_queue(leftovers).await?;
        let node = edit.node();
        let queued = rows.iter().any(|row| is_entry_of(row, node));
        let at = match edit {
            // taking a node off deletes its one row
            QueueEdit::Remove { .. } => {
                let Some(i) = rows.iter().position(|row| is_entry_of(row, node)) else {
                    return Ok((Queued::NotQueued { node: node.clone() }, None));
                };
                let row = rows.remove(i);
                self.delete_row(&row).await?;
                let queue = ranked_all(&rows)?;
                let edited = Edited::Unqueued { node: node.clone() };
                return Ok((Queued::Applied { queue }, Some(edited)));
            }
            // a move is of a node already queued
            QueueEdit::Move { .. } if !queued => {
                return Ok((Queued::NotQueued { node: node.clone() }, None));
            }
            QueueEdit::Add { at, .. } | QueueEdit::Move { at, .. } => at,
        };
        // a queued node placed beside itself stays where it is
        if queued && at.anchor() == Some(node) {
            let queue = ranked_all(&rows)?;
            return Ok((Queued::Applied { queue }, None));
        }
        // at most twice: once as the queue stands, and once spread anew if that was too long
        let mut spread = false;
        loop {
            // where the node goes among the other entries
            let old = rows.iter().position(|row| is_entry_of(row, node));
            let others: Vec<&QueueRow> =
                rows.iter().filter(|row| !is_entry_of(row, node)).collect();
            let index = match place_index(&others, at) {
                Ok(index) => index,
                Err(anchor) => return Ok((Queued::NotQueued { node: anchor }, None)),
            };
            // a node placed where it already is changes nothing
            if old == Some(index) {
                let queue = ranked_all(&rows)?;
                return Ok((Queued::Applied { queue }, None));
            }
            // a rank between its new neighbours, unless that is too long to keep
            let lo = index.checked_sub(1).map(|i| others[i].rank.as_str());
            let hi = others.get(index).map(|row| row.rank.as_str());
            let made = rank::between(lo, hi)?;
            if made.len() > rank::LONG && !spread {
                rows = self.rebalance(rows, &mut seq).await?;
                spread = true;
                continue;
            }
            // the new row, written after every row read, then the old one deleted
            seq += 1;
            let row = QueueRow {
                project: project.to_string(),
                rank: made,
                plan: node.plan.plan.clone(),
                id: node.id.clone(),
                seq,
            };
            let replaced = old.map(|i| rows[i].clone());
            self.move_row(replaced.as_ref(), row.clone()).await?;
            // and the queue as the edit leaves it, a node queued already told as moved
            let mut after: Vec<QueueRow> = others.into_iter().cloned().collect();
            after.insert(index, row);
            let queue = ranked_all(&after)?;
            let edited = match queued {
                true => Edited::Moved {
                    node: node.clone(),
                    at: at.clone(),
                },
                false => Edited::Queued {
                    node: node.clone(),
                    at: at.clone(),
                },
            };
            return Ok((Queued::Applied { queue }, Some(edited)));
        }
    }

    /// Reads a project's queue rows, checking each against its full key and its rank
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Store::queue_rows", skip_all, err(Debug))]
    async fn queue_rows(&self, project: &str) -> anyhow::Result<Vec<QueueRow>> {
        let rows = self.backend.queue(project).await?;
        for row in &rows {
            // a row of another project whose key hashed the same is a collision, not an entry
            anyhow::ensure!(
                row.project == project,
                "queue row `{}` of `{}` was stored where `{project}`'s queue is: their partition \
                 keys' hashes collide",
                row.rank,
                row.project
            );
            // and a row sorts by a rank, which placing beside it splits
            anyhow::ensure!(
                rank::is_valid(&row.rank),
                "`{project}`'s queue row for `{}/{}` is at `{}`, which is no rank",
                row.plan,
                row.id,
                row.rank
            );
        }
        Ok(rows)
    }

    /// Spreads a queue's ranks anew: short ranks after its last, one entry at a time, last first
    ///
    /// The entries moved sort after those not moved yet, in their order, so the queue reads in
    /// order after every step. One killed midway leaves at most one entry in two rows, which a
    /// read settles and the next edit prunes.
    ///
    /// # Arguments
    ///
    /// * `rows` - The queue's rows, one for each node, in rank order
    /// * `seq` - The queue's write count, moved on by each row written
    #[instrument(name = "Store::rebalance", skip_all, err(Debug))]
    async fn rebalance(&self, rows: Vec<QueueRow>, seq: &mut u64) -> anyhow::Result<Vec<QueueRow>> {
        // consecutive integers after the last rank, which are short
        let Some(last) = rows.last() else {
            return Ok(rows);
        };
        let ranks = rank::spread(Some(&last.rank), None, rows.len())?;
        // the last entry moved first, so what has moved stays after what hasn't
        let mut moved = Vec::with_capacity(rows.len());
        for (row, rank) in rows.iter().zip(ranks).rev() {
            *seq += 1;
            let new = QueueRow {
                rank,
                seq: *seq,
                ..row.clone()
            };
            self.move_row(Some(row), new.clone()).await?;
            moved.push(new);
        }
        moved.reverse();
        Ok(moved)
    }

    /// Writes an entry's new row, then deletes its old one, if it had one
    ///
    /// Once the new row is in, the move is made: it was written after every row read, so it wins
    /// over the old row for as long as that is left.
    ///
    /// # Arguments
    ///
    /// * `old` - The entry's row as it was read, or `None` for an entry new to the queue
    /// * `new` - Its new row
    #[instrument(name = "Store::move_row", skip_all, err(Debug))]
    async fn move_row(&self, old: Option<&QueueRow>, new: QueueRow) -> anyhow::Result<()> {
        // the new row first, under a rank no row read holds
        let (project, rank) = (new.project.clone(), new.rank.clone());
        if let Written::Refused(refusal) = self.backend.insert_queue(new).await? {
            anyhow::bail!(
                "another process wrote `{project}`'s queue: its rank `{rank}` was refused as \
                 {refusal:?}"
            );
        }
        // then the old one, only if it is still the row read
        if let Some(old) = old {
            self.delete_row(old).await?;
        }
        Ok(())
    }

    /// Deletes a queue row, if it is still the row read
    ///
    /// A row already gone is no failure, so a prune that is tried again is harmless.
    ///
    /// # Arguments
    ///
    /// * `row` - The row as it was read
    #[instrument(name = "Store::delete_row", skip_all, err(Debug))]
    async fn delete_row(&self, row: &QueueRow) -> anyhow::Result<()> {
        match self.backend.delete_queue(row).await? {
            Written::Applied | Written::Refused(Refusal::Missing) => Ok(()),
            // another row under its rank is another writer's, which this process can't be
            Written::Refused(_) => anyhow::bail!(
                "another process wrote `{}`'s queue: the row at rank `{}` is no longer the one read",
                row.project,
                row.rank
            ),
        }
    }

    /// Deletes the rows a killed queue write left, each only if it is still the row read
    ///
    /// # Arguments
    ///
    /// * `leftovers` - The rows each node's later row won over
    #[instrument(name = "Store::prune_queue", skip_all, err(Debug))]
    async fn prune_queue(&self, leftovers: Vec<QueueRow>) -> anyhow::Result<usize> {
        // each answer checked, one at a time
        for row in &leftovers {
            self.delete_row(row).await?;
        }
        Ok(leftovers.len())
    }
}

/// Settles a queue's rows: each node's row written last, in rank order, and the rows it won over
///
/// A move writes its new row before deleting the old one, so the later row is the move. Two of
/// a node's rows written at once can't happen; if they did, the one ranked first would win.
///
/// # Arguments
///
/// * `rows` - A queue's rows, as read
fn settle(mut rows: Vec<QueueRow>) -> (Vec<QueueRow>, Vec<QueueRow>) {
    // in rank order, as shoal compares sort keys, whatever order they were read in
    rows.sort_by(|a, b| a.rank.cmp(&b.rank));
    // the winning row of each node: the highest seq, the first ranked of equals
    let mut best: HashMap<(&str, &str), usize> = HashMap::new();
    for (i, row) in rows.iter().enumerate() {
        let winner = best
            .entry((row.plan.as_str(), row.id.as_str()))
            .or_insert(i);
        if row.seq > rows[*winner].seq {
            *winner = i;
        }
    }
    let winners: HashSet<usize> = best.into_values().collect();
    // split, keeping rank order on both sides
    let (mut kept, mut left) = (Vec::new(), Vec::new());
    for (i, row) in rows.into_iter().enumerate() {
        if winners.contains(&i) {
            kept.push(row);
        } else {
            left.push(row);
        }
    }
    (kept, left)
}

/// Whether a queue row is the entry of a node
///
/// # Arguments
///
/// * `row` - The row, of the node's project's queue
/// * `node` - The node
fn is_entry_of(row: &QueueRow, node: &NodeName) -> bool {
    row.plan == node.plan.plan && row.id == node.id
}

/// Where a place puts a node among a queue's other entries, or the node it is beside if that
/// isn't queued
///
/// # Arguments
///
/// * `others` - The queue's other entries, in order
/// * `at` - The place
fn place_index(others: &[&QueueRow], at: &Place) -> Result<usize, NodeName> {
    match at {
        Place::Top => Ok(0),
        Place::Bottom => Ok(others.len()),
        Place::Before(anchor) | Place::After(anchor) => {
            // beside a node that is queued
            let i = others
                .iter()
                .position(|row| is_entry_of(row, anchor))
                .ok_or_else(|| anchor.clone())?;
            Ok(if matches!(at, Place::After(_)) {
                i + 1
            } else {
                i
            })
        }
    }
}

/// A queue's rows as its entries, each naming its node in full
///
/// # Arguments
///
/// * `rows` - The rows, in order
fn ranked_all(rows: &[QueueRow]) -> anyhow::Result<Vec<Ranked>> {
    rows.iter()
        .map(|row| {
            // a row names its node by its project, its plan and its id
            let node =
                PlanName::new(row.project.clone(), row.plan.clone())?.node(row.id.clone())?;
            Ok(Ranked {
                node,
                rank: row.rank.clone(),
            })
        })
        .collect()
}

/// Refuses a whole plan the store can't keep: a node it can't keep as one row, or an id twice
///
/// # Arguments
///
/// * `plan` - The plan it is written to
/// * `nodes` - Every node of the plan
fn check_plan_nodes(plan: &PlanName, nodes: &[NodeWrite]) -> anyhow::Result<()> {
    // every node must be one the store can keep, under an id no other node has
    let mut ids = HashSet::with_capacity(nodes.len());
    for node in nodes {
        check_node(plan, node)?;
        if !ids.insert(node.spec.id.as_str()) {
            let message = format!("node `{}` is written twice", node.spec.id);
            return Err(Unkeepable(message).into());
        }
    }
    Ok(())
}

/// Refuses a batch the store can't keep: a node it can't keep as one row, an id put or removed
/// twice, or an id both put and removed
///
/// # Arguments
///
/// * `plan` - The plan it is written to
/// * `puts` - The nodes to write
/// * `removes` - The nodes to remove
fn check_batch(plan: &PlanName, puts: &[NodePut], removes: &[NodeRemove]) -> anyhow::Result<()> {
    // every node put must be one the store can keep, each put once
    let mut ids = HashSet::with_capacity(puts.len());
    for put in puts {
        check_node(plan, &put.node)?;
        anyhow::ensure!(
            ids.insert(put.node.spec.id.as_str()),
            "node `{}` is put twice in one batch",
            put.node.spec.id
        );
    }
    // and every node removed must be named in its plan, removed once, and not put as well
    let mut gone = HashSet::with_capacity(removes.len());
    for remove in removes {
        plan.node(remove.id.as_str())?;
        if ids.contains(remove.id.as_str()) {
            let message = format!("node `{}` is put and removed in one batch", remove.id);
            return Err(Unkeepable(message).into());
        }
        if !gone.insert(remove.id.as_str()) {
            let message = format!("node `{}` is removed twice in one batch", remove.id);
            return Err(Unkeepable(message).into());
        }
    }
    Ok(())
}

/// The puts and removals of a batch that find their nodes other than their writers read them
///
/// # Arguments
///
/// * `current` - The plan's current revision, as read under its write lock
/// * `puts` - The batch's puts
/// * `removes` - The batch's removals
fn stale_batch(current: &[StoredNode], puts: &[NodePut], removes: &[NodeRemove]) -> Vec<StalePut> {
    // each node's version as it stands, by id
    let versions: HashMap<&str, u64> = current
        .iter()
        .map(|node| (node.spec.id.as_str(), node.version))
        .collect();
    // each put, as a conditional write would judge it
    let reads = puts
        .iter()
        .map(|put| (put.node.spec.id.as_str(), put.read))
        // and each removal, which must find its node at the version read
        .chain(
            removes
                .iter()
                .map(|remove| (remove.id.as_str(), Some(remove.read))),
        );
    reads
        .filter_map(|(id, read)| {
            let refusal = match (versions.get(id), read) {
                (None, None) => return None,
                (Some(stored), Some(read)) if *stored == read => return None,
                (Some(_), None) => Refusal::Exists,
                (None, Some(_)) => Refusal::Missing,
                (Some(_), Some(_)) => Refusal::Moved,
            };
            Some(StalePut {
                id: id.to_string(),
                refusal,
            })
        })
        .collect()
}

/// Puts stored nodes in the order a listing reads in: by written order, then by id
///
/// # Arguments
///
/// * `nodes` - The nodes to sort
fn listed_order(nodes: &mut [StoredNode]) {
    nodes.sort_by(|a, b| (a.order, &a.spec.id).cmp(&(b.order, &b.spec.id)));
}

/// Puts nodes to be written in the order a listing of them will read in: by written order,
/// then by id
///
/// # Arguments
///
/// * `nodes` - The nodes to sort
fn in_written_order(nodes: &mut [NodeWrite]) {
    nodes.sort_by(|a, b| (a.order, &a.spec.id).cmp(&(b.order, &b.spec.id)));
}

/// Each node of a whole-plan write at the version it is written at: one on from the version
/// the current revision has it at, else 1 for a node new to the plan
///
/// # Arguments
///
/// * `nodes` - Every node of the plan as it should be written
/// * `old` - The version of each node of the current revision, by id
fn moved_on(nodes: Vec<NodeWrite>, old: &HashMap<String, u64>) -> Vec<(NodeWrite, u64)> {
    nodes
        .into_iter()
        .map(|node| {
            // carried on, so a put read before this write is refused after it
            let version = old.get(&node.spec.id).map_or(1, |version| version + 1);
            (node, version)
        })
        .collect()
}

/// Each page of a whole-plan write at the version it is written at: one on from the version
/// the pages' current revision has it at, else 1 for a page new to the plan
///
/// # Arguments
///
/// * `pages` - Every page of the plan as it should be written
/// * `old` - The version of each page as it stands, by path
fn pages_moved_on(pages: Vec<PageWrite>, old: &HashMap<String, u64>) -> Vec<(PageWrite, u64)> {
    pages
        .into_iter()
        .map(|page| {
            // carried on, so an edit read before this write is refused after it
            let version = old.get(&page.path).map_or(1, |version| version + 1);
            (page, version)
        })
        .collect()
}

/// Refuses a page the store can't keep: one at a path no page can have, or one a book would
/// read as a node, since its frontmatter has an `id`
///
/// # Arguments
///
/// * `page` - The page
fn check_page(page: &PageWrite) -> anyhow::Result<()> {
    // its path must be one a book's walk keeps and a link can name
    check_page_path(&page.path).map_err(Unkeepable)?;
    // and a page exported into a book must import back as a page, not a node
    if !matches!(reef_model::spec::parse_node_file(&page.text), Ok(None)) {
        let message = format!(
            "page `{}` opens with a node's frontmatter, an `id`, so a book would read it as a \
             node: put it as a node, or take its `id` out",
            page.path
        );
        return Err(Unkeepable(message).into());
    }
    Ok(())
}

/// Refuses pages that hold more text together than a plan may keep
///
/// # Arguments
///
/// * `pages` - Every page of the plan
fn check_pages_size(pages: &[PageWrite]) -> anyhow::Result<()> {
    // a plan's pages are read as one partition, which must fit in one of shoal's frames
    let bytes: usize = pages
        .iter()
        .map(|page| page.path.len() + page.text.len())
        .sum();
    if bytes > PLAN_PAGES_MAX_BYTES {
        let message = format!(
            "the plan's pages hold {bytes} bytes, more than the {PLAN_PAGES_MAX_BYTES} a plan \
             keeps"
        );
        return Err(Unkeepable(message).into());
    }
    Ok(())
}

/// Refuses a whole plan's pages the store can't keep: a page it can't keep, a path twice, or
/// more text than a plan may keep
///
/// # Arguments
///
/// * `pages` - Every page of the plan
fn check_pages(pages: &[PageWrite]) -> anyhow::Result<()> {
    // every page must be one the store can keep, at a path no other page has
    let mut paths = HashSet::with_capacity(pages.len());
    for page in pages {
        check_page(page)?;
        if !paths.insert(page.path.as_str()) {
            let message = format!("page `{}` is written twice", page.path);
            return Err(Unkeepable(message).into());
        }
    }
    // and all of them must fit in one read
    check_pages_size(pages)
}

/// Cuts rows into bundles of at most [`BUNDLE_ROWS`] rows and [`BUNDLE_BYTES`] bytes, in order
///
/// A row bigger than a bundle's bytes goes in a bundle of its own.
///
/// # Arguments
///
/// * `rows` - The rows to send
/// * `size` - How many bytes a row sends
fn bundles<T>(rows: Vec<T>, size: impl Fn(&T) -> usize) -> Vec<Vec<T>> {
    let mut out = Vec::new();
    let mut bundle = Vec::new();
    let mut bytes = 0;
    for row in rows {
        // a full bundle is sent before the row that would overfill it
        let row_bytes = size(&row);
        if !bundle.is_empty() && (bundle.len() == BUNDLE_ROWS || bytes + row_bytes > BUNDLE_BYTES) {
            out.push(std::mem::take(&mut bundle));
            bytes = 0;
        }
        bytes += row_bytes;
        bundle.push(row);
    }
    // and whatever is left is the last
    if !bundle.is_empty() {
        out.push(bundle);
    }
    out
}

/// Builds a page's row in a revision of the plan's pages
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `revision` - The revision of the plan's pages the row is part of
/// * `page` - The page
/// * `version` - The version the row is written at
fn page_row(plan: &PlanName, revision: u64, page: PageWrite, version: u64) -> PageRow {
    PageRow {
        project: plan.project.clone(),
        plan: plan.plan.clone(),
        revision,
        path: page.path,
        version,
        text: page.text,
    }
}

/// Reads a page out of its row
///
/// # Arguments
///
/// * `row` - The row
fn stored_page(row: PageRow) -> StoredPage {
    StoredPage {
        path: row.path,
        text: row.text,
        version: row.version,
    }
}

/// Refuses a node the store can't keep as one row/// Refuses a node the store can't keep as one row
///
/// # Arguments
///
/// * `plan` - The plan it is written to
/// * `node` - The node
fn check_node(plan: &PlanName, node: &NodeWrite) -> anyhow::Result<()> {
    // its id must name it within its plan
    plan.node(node.spec.id.as_str())?;
    // and each member is a node of its own, its `group` naming this one
    if !node.spec.nodes.is_empty() {
        let message = format!(
            "node `{}` lists {} members inline: put each as a node of its own, its `group` \
             naming `{}`",
            node.spec.id,
            node.spec.nodes.len(),
            node.spec.id
        );
        return Err(Unkeepable(message).into());
    }
    Ok(())
}

/// Builds a node's row in a revision
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `revision` - The revision the row is part of
/// * `node` - The node
/// * `version` - The version the row is written at
fn node_row(
    plan: &PlanName,
    revision: u64,
    node: NodeWrite,
    version: u64,
) -> anyhow::Result<NodeRow> {
    Ok(NodeRow {
        project: plan.project.clone(),
        plan: plan.plan.clone(),
        revision,
        id: node.spec.id.clone(),
        version,
        // the fields reef filters on, beside the spec they come from
        status: node
            .spec
            .status
            .map(|s| s.as_str().to_string())
            .unwrap_or_default(),
        group: node.spec.group.clone().unwrap_or_default(),
        order: node.order,
        spec: serde_json::to_string(&node.spec)?,
        body: node.body,
    })
}

/// Reads a node out of its row
///
/// # Arguments
///
/// * `row` - The row
fn stored(row: NodeRow) -> anyhow::Result<StoredNode> {
    // the spec is JSON, which reads back equal
    let spec = serde_json::from_str(&row.spec)
        .map_err(|error| anyhow::anyhow!("node `{}`'s spec: {error}", row.id))?;
    Ok(StoredNode {
        spec,
        body: row.body,
        order: row.order,
        version: row.version,
    })
}

/// A random revision id, never 0, which means none
fn fresh_revision() -> u64 {
    // std's hasher state is randomly seeded per process, and the clock separates calls
    loop {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default(),
        );
        let revision = hasher.finish();
        if revision != 0 {
            return revision;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AddedProject, BUNDLE_BYTES, BUNDLE_ROWS, NodeWrite, Store, bundles, settle};
    use crate::backend::RevisionKey;
    use crate::schema::{NodeRow, PageRow, QueueRow};
    use reef_model::PlanName;
    use std::path::Path;

    /// A second project for a repo, however its path is spelled, or under a taken name, is refused
    #[tokio::test]
    async fn duplicate_name_or_repo_refused() -> anyhow::Result<()> {
        // a project for the repo this crate is in, located at its root
        let store = Store::memory();
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let AddedProject::Added(reef) = store.add_project("reef", here, &reef_api::NOBODY).await?
        else {
            panic!("the first project for a repo was refused");
        };
        assert!(
            reef.repo.join(".git").exists(),
            "{} is no repo's root",
            reef.repo.display()
        );
        // another for the same repo, spelled through a subdirectory and `..`, is refused
        let respelled = here.join("src").join("..");
        let taken = AddedProject::RepoTaken {
            project: "reef".to_string(),
        };
        assert_eq!(
            store
                .add_project("again", &respelled, &reef_api::NOBODY)
                .await?,
            taken
        );
        // and so is the name, for another directory
        assert_eq!(
            store
                .add_project("reef", Path::new("/"), &reef_api::NOBODY)
                .await?,
            AddedProject::NameTaken
        );
        // a name that breaks the rule names keep is an error
        assert!(
            store
                .add_project("no spaces", Path::new("/"), &reef_api::NOBODY)
                .await
                .is_err()
        );
        // and only the first is registered, whose registration its listing leaves to the service
        assert!(reef.registered.is_some());
        let unjoined = reef_api::Project {
            registered: None,
            ..reef
        };
        assert_eq!(store.projects().await?, vec![unjoined]);
        Ok(())
    }

    /// A row stored in a plan's partition under another plan's key is an error, never a node
    #[tokio::test]
    async fn collided_row_is_an_error() -> anyhow::Result<()> {
        // a plan with one node of its own
        let store = Store::memory();
        store
            .add_project(
                "reef",
                Path::new(env!("CARGO_MANIFEST_DIR")),
                &reef_api::NOBODY,
            )
            .await?;
        let plan = PlanName::new("reef", "central")?;
        store.add_plan(&plan, &reef_api::NOBODY).await?;
        store.put(&plan, NodeWrite::new("C7", 0), None).await?;
        // and a row of another project's, landing in its partition as a hash collision would
        let current = store.plan_row(&plan).await?.current;
        let stray = NodeRow {
            project: "other".to_string(),
            plan: "central".to_string(),
            revision: current,
            id: "C8".to_string(),
            version: 1,
            status: String::new(),
            group: String::new(),
            order: 1,
            spec: "{\"id\":\"C8\"}".to_string(),
            body: String::new(),
        };
        store
            .backend()
            .plant(&RevisionKey::new(&plan, current), stray);
        // a get of it is caught, and so is a list of the whole revision
        let got = store
            .get(&plan, "C8")
            .await
            .expect_err("a collided row was served");
        assert!(format!("{got:#}").contains("collide"), "{got:#}");
        let listed = store
            .list(&plan)
            .await
            .expect_err("a collided row was listed");
        assert!(format!("{listed:#}").contains("collide"), "{listed:#}");
        // while the plan's own node still reads back
        assert!(store.get(&plan, "C7").await?.is_some());
        Ok(())
    }

    /// A queue row of project `reef`'s plan `central`
    ///
    /// # Arguments
    ///
    /// * `id` - The node queued
    /// * `rank` - Its rank
    /// * `seq` - The write count it was written at
    fn queued(id: &str, rank: &str, seq: u64) -> QueueRow {
        QueueRow {
            project: "reef".to_string(),
            rank: rank.to_string(),
            plan: "central".to_string(),
            id: id.to_string(),
            seq,
        }
    }

    /// A row stored in a plan's pages under another plan's key is an error, never a page
    #[tokio::test]
    async fn collided_page_row_is_an_error() -> anyhow::Result<()> {
        // a plan with one page of its own
        let store = Store::memory();
        store
            .add_project(
                "reef",
                Path::new(env!("CARGO_MANIFEST_DIR")),
                &reef_api::NOBODY,
            )
            .await?;
        let plan = PlanName::new("reef", "central")?;
        store.add_plan(&plan, &reef_api::NOBODY).await?;
        let pages = store.plan_row(&plan).await?.pages_current;
        let own = PageRow {
            project: "reef".to_string(),
            plan: "central".to_string(),
            revision: pages,
            path: "a.md".to_string(),
            version: 1,
            text: "# A\n".to_string(),
        };
        store
            .backend()
            .plant_page(&RevisionKey::new(&plan, pages), own);
        assert!(store.page(&plan, "a.md").await?.is_some());
        // and a row of another project's, landing in its partition as a hash collision would
        let stray = PageRow {
            project: "other".to_string(),
            plan: "central".to_string(),
            revision: pages,
            path: "b.md".to_string(),
            version: 1,
            text: "# B\n".to_string(),
        };
        store
            .backend()
            .plant_page(&RevisionKey::new(&plan, pages), stray);
        // a get of it is caught, and so are a listing and a snapshot of the whole revision
        let got = store
            .page(&plan, "b.md")
            .await
            .expect_err("a collided row was served");
        assert!(format!("{got:#}").contains("collide"), "{got:#}");
        let listed = store
            .pages(&plan)
            .await
            .expect_err("a collided row was listed");
        assert!(format!("{listed:#}").contains("collide"), "{listed:#}");
        assert!(store.snapshot(&plan).await.is_err());
        // while the plan's own page still reads back
        assert!(store.page(&plan, "a.md").await?.is_some());
        Ok(())
    }

    /// Bundles stop at either budget, a row too big for one goes alone, and none is empty
    #[test]
    fn bundles_cut_by_rows_and_bytes() {
        // small rows, cut by count
        let rows: Vec<usize> = vec![1; BUNDLE_ROWS * 2 + 1];
        let cut = bundles(rows, |row| *row);
        let lens: Vec<usize> = cut.iter().map(Vec::len).collect();
        assert_eq!(lens, [BUNDLE_ROWS, BUNDLE_ROWS, 1]);
        // big rows, cut by bytes, one too big for a bundle alone in its own
        let half = BUNDLE_BYTES / 2;
        let rows = vec![half, half, 1, BUNDLE_BYTES * 2, 1];
        let cut = bundles(rows, |row| *row);
        assert_eq!(
            cut,
            [vec![half, half], vec![1], vec![BUNDLE_BYTES * 2], vec![1]]
        );
        // and nothing makes no bundles
        assert!(bundles(Vec::<usize>::new(), |row| *row).is_empty());
    }

    /// A row stored in a project's queue under another project's key is an error, never an entry
    #[tokio::test]
    async fn collided_queue_row_is_an_error() -> anyhow::Result<()> {
        // a row of another project's, landing in reef's partition as a hash collision would
        let store = Store::memory();
        let stray = QueueRow {
            project: "other".to_string(),
            ..queued("C1", "a0", 1)
        };
        store.backend().plant_queue("reef", stray);
        // a read of the queue is caught
        let error = store
            .queue("reef")
            .await
            .expect_err("a collided row was served");
        assert!(format!("{error:#}").contains("collide"), "{error:#}");
        Ok(())
    }

    /// Of a node's rows the one written later wins, wherever it is ranked, and the rest are
    /// left over, each side in rank order
    #[test]
    fn settle_keeps_the_newest_row_of_each_node() {
        // A moved from the bottom to the top, its old row left; B once
        let rows = vec![
            queued("A", "a2", 1),
            queued("B", "a1", 2),
            queued("A", "Zz", 3),
        ];
        let (kept, left) = settle(rows);
        assert_eq!(kept, [queued("A", "Zz", 3), queued("B", "a1", 2)]);
        assert_eq!(left, [queued("A", "a2", 1)]);
        // and of two written at once, the first ranked
        let (kept, left) = settle(vec![queued("A", "a1", 4), queued("A", "a0", 4)]);
        assert_eq!(
            (kept, left),
            (vec![queued("A", "a0", 4)], vec![queued("A", "a1", 4)])
        );
    }
}
