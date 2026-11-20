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
//! - **Puts come in batches.** A group with no members yet is a piece of work that says
//!   neither why nor what, and a member alone names a group that isn't a node, so a group goes
//!   in with its members, checked as one change.
//! - **Checked under the plan's write lock**, so two writes that are fine alone can't break the
//!   plan together.
//! - **Docs come from the project's working tree.** A stored node's links resolve from its
//!   repo's root, as `mdbook-reef check` resolves a book's from its `src`. They are read off
//!   the async threads.
//!
//! A process keeps one service over shoal. `Service::shared` opens it once however often a
//! caller asks, since `dioxus::serve` runs its setup again after every hot-patch, and a second
//! pool would find the data directory locked.

use reef_model::{Diagnostic, Graph, Payload, PlanName, Record, nodes_from_records};
use reef_render::{DocSource, FsSource, Renderer, Sections};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::OnceCell;
use tracing::instrument;

use crate::backend::Backend;
use crate::client::{Mode, ShoalBackend};
use crate::conf::Conf;
use crate::memory::MemoryBackend;
use crate::store::{
    AddedProject, Checked, Listing, NodePut, NodeWrite, NotRegistered, PlanInfo, Project, Proposal,
    Rewrite, Store, StoredNode,
};

/// The process's one service over shoal, opened on first use
static SHARED: OnceCell<Service> = OnceCell::const_new();

/// reef's service: checked writes and built plans, over a store
pub struct Service<B = ShoalBackend> {
    /// The store every read and write goes through
    store: Store<B>,
    /// The doc sections a node's why, what and design choices are taken from
    sections: Sections,
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
        // reach shoal however the config says to
        let store = Store::open(conf).await?;
        Ok(Service::new(store))
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
    /// A service over a store, taking a node's brief form from the default doc sections
    ///
    /// # Arguments
    ///
    /// * `store` - The store every read and write goes through
    pub fn new(store: Store<B>) -> Service<B> {
        Service {
            store,
            sections: Sections::default(),
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
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `repo` - A directory in the repo its plans describe
    #[instrument(name = "Service::add_project", skip_all, err(Debug))]
    pub async fn add_project(&self, name: &str, repo: &Path) -> anyhow::Result<AddedProject> {
        self.store.add_project(name, repo).await
    }

    /// Lists every project, by name
    #[instrument(name = "Service::projects", skip_all, err(Debug))]
    pub async fn projects(&self) -> anyhow::Result<Vec<Project>> {
        self.store.projects().await
    }

    /// Finds the project a directory belongs to, a linked worktree's main repo's for one in it
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory, such as the working directory
    #[instrument(name = "Service::find_project", skip_all, err(Debug))]
    pub async fn find_project(&self, dir: &Path) -> anyhow::Result<Option<Project>> {
        self.store.find_project(dir).await
    }

    /// Lists a plan in its project, and returns whether it was added
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::add_plan", skip_all, err(Debug))]
    pub async fn add_plan(&self, plan: &PlanName) -> anyhow::Result<bool> {
        self.store.add_plan(plan).await
    }

    /// Lists a project's plans, by name
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Service::plans", skip_all, err(Debug))]
    pub async fn plans(&self, project: &str) -> anyhow::Result<Vec<PlanInfo>> {
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
    /// Each put names the version its writer read, or `None` for a new node. A stale put
    /// refuses the whole batch as [`Checked::Stale`], and a batch that would add a problem to
    /// the plan is refused as [`Checked::Refused`] with check's messages. Either writes
    /// nothing.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `puts` - The nodes to write, each with the version its writer read
    #[instrument(name = "Service::put", skip_all, err(Debug))]
    pub async fn put(&self, plan: &PlanName, puts: Vec<NodePut>) -> anyhow::Result<Checked> {
        // the plan's docs are its project's working tree
        let repo = self.repo(plan).await?;
        let sections = self.sections.clone();
        // judged under the plan's write lock, against the plan the puts land on
        self.store
            .put_checked(plan, puts, move |proposal| added(repo, sections, proposal))
            .await
    }

    /// Writes a whole plan as a new revision, unless it would add a problem
    ///
    /// An import into an empty plan is refused if it has any problem at all.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `nodes` - Every node of the plan, each a node of its own
    #[instrument(name = "Service::write_plan", skip_all, err(Debug))]
    pub async fn write_plan(
        &self,
        plan: &PlanName,
        nodes: Vec<NodeWrite>,
    ) -> anyhow::Result<Rewrite> {
        // the plan's docs are its project's working tree
        let repo = self.repo(plan).await?;
        let sections = self.sections.clone();
        // judged under the plan's write lock, against the plan it replaces
        self.store
            .write_plan_checked(plan, nodes, move |proposal| added(repo, sections, proposal))
            .await
    }

    /// Finds a plan's problems, as `mdbook-reef check` finds a book's
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::check", skip_all, err(Debug))]
    pub async fn check(&self, plan: &PlanName) -> anyhow::Result<Vec<Diagnostic>> {
        // the plan as it stands, and the working tree its docs are read from
        let (records, repo) = self.records(plan).await?;
        let sections = self.sections.clone();
        // build it off the async threads, since its docs are files
        let problems = tokio::task::spawn_blocking(move || {
            let renderer = Renderer::new(FsSource::new(repo), sections);
            build(&renderer, records).1
        })
        .await?;
        Ok(problems)
    }

    /// Builds a plan into the viewer's payload: every node, its body rendered, its links
    /// resolved
    ///
    /// A plan with problems still has a payload, built without the parts that broke. Its links
    /// start with `reef-root:/`, as a book's do before a page's way back to its root is filled
    /// in.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::payload", skip_all, err(Debug))]
    pub async fn payload(&self, plan: &PlanName) -> anyhow::Result<Payload> {
        // the plan as it stands, and the working tree its docs are read from
        let (records, repo) = self.records(plan).await?;
        let sections = self.sections.clone();
        // build and render it off the async threads, since its docs are files
        let payload = tokio::task::spawn_blocking(move || {
            let renderer = Renderer::new(FsSource::new(repo), sections);
            let (graph, _) = build(&renderer, records);
            // a stored node has no page of its own
            renderer.payload(&graph, |_| None)
        })
        .await?;
        Ok(payload)
    }

    /// Reads a plan's records as it stands, and the repo its docs are read from
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::records", skip_all, err(Debug))]
    async fn records(&self, plan: &PlanName) -> anyhow::Result<(Vec<Record>, PathBuf)> {
        // the repo first, so a plan of an unknown project says so
        let repo = self.repo(plan).await?;
        let records = self.store.list(plan).await?.records(plan)?;
        Ok((records, repo))
    }

    /// The repo a plan's docs are read from: its project's
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Service::repo", skip_all, err(Debug))]
    async fn repo(&self, plan: &PlanName) -> anyhow::Result<PathBuf> {
        // a plan belongs to a project that is registered
        let project = self
            .store
            .project(&plan.project)
            .await?
            .ok_or_else(|| NotRegistered(plan.project.clone()))?;
        Ok(project.repo)
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

/// The problems a write would add to a plan, judged off the async threads
///
/// Both sides are built with one renderer, so they see the docs as they were at one moment,
/// and each doc is read once.
///
/// # Arguments
///
/// * `repo` - The working tree the plan's docs are read from
/// * `sections` - The doc sections a node's brief form is taken from
/// * `proposal` - The plan as it stands and as the write would leave it
async fn added(
    repo: PathBuf,
    sections: Sections,
    proposal: Proposal,
) -> anyhow::Result<Vec<Diagnostic>> {
    // its docs are files, so build both sides off the async threads
    let added = tokio::task::spawn_blocking(move || {
        let renderer = Renderer::new(FsSource::new(repo), sections);
        let (_, before) = build(&renderer, proposal.before);
        let (_, after) = build(&renderer, proposal.after);
        new_problems(&before, after)
    })
    .await?;
    Ok(added)
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
    use super::{Service, new_problems};
    use crate::store::{Checked, NodePut, NodeWrite};
    use reef_model::{Description, Diagnostic, NodeSpec, PlanName};
    use std::path::Path;

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
            .add_project("reef", Path::new(env!("CARGO_MANIFEST_DIR")))
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
        send(service.write_plan(&plan, Vec::new()));
        send(service.check(&plan));
        send(service.payload(&plan));
    }
}
