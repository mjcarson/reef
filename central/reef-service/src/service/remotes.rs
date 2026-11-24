//! The service's remote projects: registered by a git URL, cloned, and fetched when asked and
//! on an interval
//!
//! - **A remote project is cloned before it is registered.** The URL is checked, the branch is
//!   the one given or the remote's default, and the service makes a bare clone of its own and
//!   fetches the branch's tip into it, so a remote it can't reach is refused before anything is
//!   kept. A failure removes the clone it was making.
//! - **Each clone has a directory of its own**, named by its project and the moment it was made,
//!   so a registration retried after a failure never meets the clone that one left.
//! - **Docs are read at the commit the store keeps** ([`crate::tree`]), so a fetch lands for
//!   readers only once its commit is written to the project's remote.
//! - **A fetch is one project's at a time**, under its own lock, so one slow remote holds up no
//!   other. Each is written to the remote's row, whether its branch moved or not, so the row says
//!   when it was last fetched. A branch that moved is a change, [`Change::ProjectFetched`], which
//!   reaches every follower of any of its project's plans.
//! - **Every remote project is fetched on an interval**, when the service was given one, by
//!   [`Service::follow_remotes`], which a server runs beside its routes. A fetch that fails is
//!   logged and the rest go on.

use reef_api::{AddedProject, Change, Fetched, Project};
use reef_model::PlanName;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::instrument;

use super::Service;
use crate::backend::Backend;
use crate::remote::{self, BadRemote};
use crate::store::{KeptRemote, NotRegistered};

impl<B: Backend> Service<B> {
    /// Clones remote projects into a directory, or keeps no clones
    ///
    /// # Arguments
    ///
    /// * `clones` - The directory every clone is made in, or `None` to refuse remote projects
    pub fn cloning_into(mut self, clones: Option<PathBuf>) -> Self {
        self.clones = clones;
        self
    }

    /// Fetches every remote project on an interval, or only when asked
    ///
    /// # Arguments
    ///
    /// * `every` - How long between fetches, or `None` for only when asked
    pub fn fetching_every(mut self, every: Option<Duration>) -> Self {
        self.fetch_every = every;
        self
    }

    /// Registers a project by its git remote, cloning the branch whose docs it reads
    ///
    /// Refuses a name another project has, or a repo another project was registered by,
    /// however its URL is spelled.
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `url` - The URL the service clones and fetches
    /// * `branch` - The branch whose docs it reads, or `None` for the remote's default
    #[instrument(name = "Service::add_remote_project", skip_all, err(Debug))]
    pub async fn add_remote_project(
        &self,
        name: &str,
        url: &str,
        branch: Option<&str>,
    ) -> anyhow::Result<AddedProject> {
        // a name and a URL every project keeps to, and somewhere to clone it
        PlanName::new(name, "plan")?;
        remote::checked(url)?;
        let Some(clones) = &self.clones else {
            return Err(BadRemote(
                "this service keeps no clones of remote projects: set `remotes.clones` in its \
                 config"
                    .to_string(),
            )
            .into());
        };
        // a taken name or repo is said before anything is cloned, and checked again once it is
        if let Some(taken) = taken(&self.store.projects().await?, name, url) {
            return Ok(taken);
        }
        // the branch named, or the one the remote's HEAD names
        let branch = match branch {
            Some(branch) => {
                remote::check_branch(branch).await?;
                branch.to_string()
            }
            None => remote::default_branch(url).await?,
        };
        // a clone of its own, at one path however the directory was named
        tokio::fs::create_dir_all(clones).await?;
        let clones = tokio::fs::canonicalize(clones).await?;
        let clone = clones.join(format!("{name}.{}.git", now_ms()));
        // made here alone, so no other registration can be making a clone in it
        tokio::fs::create_dir(&clone)
            .await
            .map_err(|error| anyhow::anyhow!("making a clone at {}: {error}", clone.display()))?;
        let commit = match cloned(&clone, url, &branch).await {
            Ok(commit) => commit,
            Err(error) => {
                remove(&clone).await;
                return Err(error);
            }
        };
        // kept, its project last, or the clone taken away again
        let kept = KeptRemote {
            url: url.to_string(),
            branch,
            clone: clone.clone(),
            commit,
            fetched_at: now_ms(),
        };
        let added = match self.store.add_remote_project(name, kept).await {
            Ok(added) => added,
            Err(error) => {
                remove(&clone).await;
                return Err(error);
            }
        };
        // tell the change stream if it was registered, and keep no clone if it wasn't
        match &added {
            AddedProject::Added(project) => self.publish(Change::ProjectAdded {
                project: project.clone(),
            }),
            AddedProject::NameTaken | AddedProject::RepoTaken { .. } => remove(&clone).await,
        }
        Ok(added)
    }

    /// Fetches a remote project's branch, so its docs are read at its tip
    ///
    /// A local project's docs are its working tree as it stands, so it is [`Fetched::Local`],
    /// and nothing is fetched.
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    #[instrument(name = "Service::fetch", skip_all, err(Debug))]
    pub async fn fetch(&self, name: &str) -> anyhow::Result<Fetched> {
        // one fetch of a project at a time
        let lock = self.fetch_lock(name);
        let _fetching = lock.lock().await;
        // a registered project, and the remote it was registered by, if it was
        let project = self
            .store
            .project(name)
            .await?
            .ok_or_else(|| NotRegistered(name.to_string()))?;
        if project.remote.is_none() {
            return Ok(Fetched::Local);
        }
        let read = self
            .store
            .remotes()
            .await?
            .remove(name)
            .filter(|read| read.kept.clone == project.repo)
            .ok_or_else(|| anyhow::anyhow!("project `{name}`'s remote is no longer kept"))?;
        // the branch's tip, fetched into the clone
        let kept = &read.kept;
        remote::fetch(&kept.clone, &kept.url, &kept.branch).await?;
        let commit = remote::tip(&kept.clone, &kept.branch).await?;
        // kept whether it moved or not, so the row says when it was fetched
        let from = kept.commit.clone();
        self.store.fetched(&read, &commit, now_ms()).await?;
        if commit == from {
            return Ok(Fetched::Unchanged { commit });
        }
        // a branch that moved moves every plan of the project's docs
        self.publish(Change::ProjectFetched {
            project: name.to_string(),
            commit: commit.clone(),
        });
        Ok(Fetched::Moved { from, to: commit })
    }

    /// Fetches every remote project, one at a time, logging each fetch that fails
    ///
    /// Returns each remote project's name and what came of its fetch.
    #[instrument(name = "Service::fetch_remotes", skip_all, err(Debug))]
    pub async fn fetch_remotes(&self) -> anyhow::Result<Vec<(String, anyhow::Result<Fetched>)>> {
        // every project registered by its remote
        let projects = self.store.projects().await?;
        let mut fetched = Vec::new();
        for project in projects
            .into_iter()
            .filter(|project| project.remote.is_some())
        {
            // one remote that can't be reached holds up none of the rest
            let result = self.fetch(&project.name).await;
            if let Err(error) = &result {
                tracing::warn!(
                    project = %project.name,
                    error = %format!("{error:#}"),
                    "fetching a remote project failed"
                );
            }
            fetched.push((project.name, result));
        }
        Ok(fetched)
    }

    /// Fetches every remote project on the interval this service was given, until dropped
    ///
    /// The first fetch is at once, so a server that restarts catches up with every push it
    /// missed. With no interval it waits forever, fetching nothing. A server runs it beside its
    /// routes and drops it when it stops, which kills any git still running.
    pub async fn follow_remotes(&self) {
        // nothing to follow without an interval
        let Some(every) = self.fetch_every else {
            return std::future::pending().await;
        };
        // a slow round delays the next rather than bunching them up
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            if let Err(error) = self.fetch_remotes().await {
                tracing::warn!(
                    error = %format!("{error:#}"),
                    "listing the remote projects to fetch failed"
                );
            }
        }
    }

    /// The lock a project's fetches hold, made the first time it is asked for
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    fn fetch_lock(&self, project: &str) -> Arc<tokio::sync::Mutex<()>> {
        // the map is only held to find or add the lock, never across a wait
        self.fetches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(project.to_string())
            .or_default()
            .clone()
    }
}

/// What a registration would be refused as, by the projects already registered, if anything
///
/// # Arguments
///
/// * `projects` - Every project registered
/// * `name` - The new project's name
/// * `url` - Its remote's URL
fn taken(projects: &[Project], name: &str, url: &str) -> Option<AddedProject> {
    // a name is one project's
    if projects.iter().any(|project| project.name == name) {
        return Some(AddedProject::NameTaken);
    }
    // and so is a repo, however its URL is spelled
    let wanted = remote::key(url)?;
    projects
        .iter()
        .find(|project| {
            project
                .remote
                .as_ref()
                .is_some_and(|remote| remote::key(&remote.url).as_ref() == Some(&wanted))
        })
        .map(|project| AddedProject::RepoTaken {
            project: project.name.clone(),
        })
}

/// Makes a clone of a remote's branch, and returns the commit it fetched
///
/// # Arguments
///
/// * `clone` - Where the clone goes, an empty directory
/// * `url` - The remote's URL, already checked
/// * `branch` - The branch to fetch
#[instrument(name = "remotes::cloned", skip_all, err(Debug))]
async fn cloned(clone: &Path, url: &str, branch: &str) -> anyhow::Result<String> {
    // an empty bare repo, filled as every later fetch fills it
    remote::make(clone, branch).await?;
    remote::fetch(clone, url, branch).await?;
    remote::tip(clone, branch).await
}

/// Takes away a clone a registration made and won't keep, saying so if it can't
///
/// # Arguments
///
/// * `clone` - The clone's directory
async fn remove(clone: &Path) {
    // a clone that was never made is nothing to remove
    match tokio::fs::remove_dir_all(clone).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(
            clone = %clone.display(),
            %error,
            "a clone a registration won't keep couldn't be removed"
        ),
    }
}

/// The current time in milliseconds since the epoch
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}
