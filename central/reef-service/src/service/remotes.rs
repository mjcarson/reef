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
//!   other. Each is written to the remote's row, whether its branches moved or not, so the row
//!   says when it was last fetched.
//! - **A fetch takes every branch a plan follows**, beside the one the project was registered
//!   by, one branch at a time, so a branch deleted from the remote holds up no other: it keeps
//!   where it was last fetched, and says it failed. The registered branch failing fails the fetch.
//! - **A branch that moved is a change**, [`Change::ProjectFetched`], naming the plans that read
//!   their docs there, so only those plans' followers hear of it.
//! - **Every remote project is fetched on an interval**, when the service was given one, by
//!   [`Service::follow_remotes`], which a server runs beside its routes. A fetch that fails is
//!   logged and the rest go on.

use reef_api::{AddedProject, BranchFetched, Change, Fetched, PlanInfo, Project, Tip};
use reef_model::PlanName;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::instrument;

use super::{Asker, Service};
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

    /// Clones remote projects from these hosts alone, or from any if there are none
    ///
    /// # Arguments
    ///
    /// * `hosts` - The hosts a remote may name, in any case, or none for any host
    pub fn cloning_from(mut self, hosts: Vec<String>) -> Self {
        self.hosts = hosts.iter().map(|host| host.to_ascii_lowercase()).collect();
        self
    }

    /// Checks a remote's host is one this service clones from
    ///
    /// A URL with no host, a path on this host, is judged by who asks instead.
    ///
    /// # Arguments
    ///
    /// * `url` - The remote's URL, checked already
    fn judge_host(&self, url: &gix::Url) -> Result<(), BadRemote> {
        // any host, if none is named, and a path on this host has none
        let Some(host) = url.host() else {
            return Ok(());
        };
        if self.hosts.is_empty() || self.hosts.contains(&host.to_ascii_lowercase()) {
            return Ok(());
        }
        Err(BadRemote(format!(
            "this service clones remote projects from {} alone, and `{host}` is not one: ask its \
             admin to add it to `remotes.hosts` in its config",
            self.hosts.join(", ")
        )))
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
    /// * `asker` - Who asks, since a path on this host is an admin's to name once callers sign in
    #[instrument(name = "Service::add_remote_project", skip_all, err(Debug))]
    pub async fn add_remote_project(
        &self,
        name: &str,
        url: &str,
        branch: Option<&str>,
        asker: Asker<'_>,
    ) -> anyhow::Result<AddedProject> {
        // a name and a URL every project keeps to, and somewhere to clone it
        PlanName::new(name, "plan")?;
        let checked = remote::checked(url)?;
        // a path on this host is an admin's to name, and another host one this service clones from
        match checked.scheme {
            gix::url::Scheme::File => {
                self.judge_on_host(asker, "register a project by a path on its host")
                    .await?;
            }
            _ => self.judge_host(&checked)?,
        }
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
            branches: BTreeMap::new(),
        };
        let added = match self
            .store
            .add_remote_project(name, kept, &asker.author())
            .await
        {
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

    /// Fetches a remote project's branches, so each plan's docs are read at its branch's tip
    ///
    /// The branch the project was registered by is fetched first, and its failure fails the
    /// fetch. Each other branch a plan follows is fetched after it, one at a time, and one that
    /// fails keeps where it was last fetched. A local project's docs are its working tree as it
    /// stands, so it is [`Fetched::Local`], and nothing is fetched.
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
        let read = self.kept_remote(&project).await?;
        // the registered branch's tip, fetched into the clone
        let kept = &read.kept;
        remote::fetch(&kept.clone, &kept.url, &kept.branch).await?;
        let commit = remote::tip(&kept.clone, &kept.branch).await?;
        // then each other branch a plan follows, one at a time, each tip kept where one was
        let plans = self.store.plans(name).await?;
        let followed: BTreeSet<String> = plans
            .iter()
            .filter_map(|info| info.branch.clone())
            .filter(|branch| *branch != kept.branch)
            .collect();
        let at = now_ms();
        let mut tips = kept.branches.clone();
        let mut outcomes = Vec::with_capacity(followed.len());
        for branch in followed {
            let before = kept.branches.get(&branch).map(|tip| tip.commit.clone());
            let fetched = async {
                remote::fetch(&kept.clone, &kept.url, &branch).await?;
                remote::tip(&kept.clone, &branch).await
            };
            match fetched.await {
                Ok(to) => {
                    // where it is now, and whether that moved it
                    tips.insert(
                        branch.clone(),
                        Tip {
                            commit: to.clone(),
                            fetched_at: at,
                        },
                    );
                    outcomes.push(match before {
                        Some(from) if from == to => BranchFetched::Unchanged { branch, commit: to },
                        from => BranchFetched::Moved { branch, from, to },
                    });
                }
                Err(error) => {
                    // one that failed is left where it was, and says why
                    tracing::warn!(
                        project = name,
                        %branch,
                        error = %format!("{error:#}"),
                        "fetching a branch a plan follows failed"
                    );
                    outcomes.push(BranchFetched::Failed {
                        branch,
                        error: format!("{error:#}"),
                    });
                }
            }
        }
        // kept whether anything moved or not, so the row says when it was fetched
        let from = kept.commit.clone();
        let registered = kept.branch.clone();
        self.store.fetched(&read, &commit, tips, at).await?;
        // each branch that moved moves the docs of the plans that follow it
        if commit != from {
            self.publish(Change::ProjectFetched {
                project: name.to_string(),
                commit: commit.clone(),
                branch: Some(registered.clone()),
                plans: Some(following(&plans, &registered, None)),
            });
        }
        for outcome in &outcomes {
            if let BranchFetched::Moved { branch, to, .. } = outcome {
                self.publish(Change::ProjectFetched {
                    project: name.to_string(),
                    commit: to.clone(),
                    branch: Some(branch.clone()),
                    plans: Some(following(&plans, &registered, Some(branch))),
                });
            }
        }
        Ok(match commit == from {
            true => Fetched::Unchanged {
                commit,
                branches: outcomes,
            },
            false => Fetched::Moved {
                from,
                to: commit,
                branches: outcomes,
            },
        })
    }

    /// Fetches one branch of a remote project into its clone, for a plan about to follow it
    ///
    /// It holds the project's fetch lock, so no fetch running meanwhile writes over it.
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `branch` - The branch, its name checked already
    #[instrument(name = "Service::fetch_branch", skip_all, err(Debug))]
    pub(crate) async fn fetch_branch(&self, name: &str, branch: &str) -> anyhow::Result<()> {
        // one fetch of a project at a time
        let lock = self.fetch_lock(name);
        let _fetching = lock.lock().await;
        let project = self
            .store
            .project(name)
            .await?
            .ok_or_else(|| NotRegistered(name.to_string()))?;
        let read = self.kept_remote(&project).await?;
        // the branch's tip, fetched into the clone, beside every tip kept already
        let kept = &read.kept;
        remote::fetch(&kept.clone, &kept.url, branch).await?;
        let commit = remote::tip(&kept.clone, branch).await?;
        let at = now_ms();
        let mut tips = kept.branches.clone();
        tips.insert(
            branch.to_string(),
            Tip {
                commit,
                fetched_at: at,
            },
        );
        // the registered branch where it was, and the row's time where it was too
        self.store
            .fetched(&read, &kept.commit, tips, kept.fetched_at)
            .await?;
        Ok(())
    }

    /// The remote a project was registered by, as its row keeps it, refused if it is gone
    ///
    /// # Arguments
    ///
    /// * `project` - The project, as the store lists it
    async fn kept_remote(&self, project: &Project) -> anyhow::Result<crate::store::StoredRemote> {
        // the row whose clone the project's row names
        self.store
            .remotes()
            .await?
            .remove(&project.name)
            .filter(|read| read.kept.clone == project.repo)
            .ok_or_else(|| anyhow::anyhow!("project `{}`'s remote is no longer kept", project.name))
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

/// The plans of a remote project that read their docs at a branch: those that follow it, and
/// for the branch the project was registered by, those that follow no branch of their own too
///
/// # Arguments
///
/// * `plans` - The project's plans
/// * `registered` - The branch the project was registered by
/// * `branch` - The branch, or `None` for the registered one
fn following(plans: &[PlanInfo], registered: &str, branch: Option<&str>) -> Vec<String> {
    // the registered branch is the one a plan with none follows
    let wanted = branch.unwrap_or(registered);
    plans
        .iter()
        .filter(|info| info.branch.as_deref().unwrap_or(registered) == wanted)
        .map(|info| info.name.plan.clone())
        .collect()
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
