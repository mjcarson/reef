//! The git remotes projects were registered by: a row each, in one constant partition
//!
//! - **A remote project is two rows.** Its [`ProjectRow`] keeps its clone's path where a local
//!   project keeps its repo's, and its [`RemoteRow`] keeps the URL, the branch, the clone's path
//!   again and the commit last fetched, as JSON. So projects needed no new field, and no fresh
//!   data directory.
//! - **The project's row is written last.** A registration writes its remote's row, then its
//!   project's `if_absent`, so one killed between the two leaves a remote no project lists. A
//!   project's remote is joined to it only if both name the same clone, so such a row never
//!   attaches to a later project of the same name, local or remote, and a later registration of
//!   that name writes over it.
//! - **One repo, one project.** A remote is refused if another project was registered by a URL
//!   with the same [`key`](crate::remote::key), under the lock every registration holds.
//! - **A fetch writes over the version read**, so two writers' fetches can't both land.
//! - **Every row is checked against its full key on read**, so a hash collision is caught rather
//!   than read as another project's remote.

use reef_api::{AddedProject, Project, Remote};
use reef_model::PlanName;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::instrument;

use super::Store;
use crate::backend::{Backend, Expect, Written};
use crate::remote;
use crate::schema::{PROJECTS, ProjectRow, REMOTES, RemoteRow};

/// A remote's record, as its row keeps it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptRemote {
    /// The URL the service fetches from
    pub url: String,
    /// The branch whose docs it reads
    pub branch: String,
    /// The clone's path on the service's host, which its project's row names too
    pub clone: PathBuf,
    /// The commit the branch was at when it was last fetched
    pub commit: String,
    /// When it was last fetched, in milliseconds since the Unix epoch
    pub fetched_at: u64,
}

impl KeptRemote {
    /// The remote as the service shows it, with its project
    pub fn remote(&self) -> Remote {
        Remote {
            url: self.url.clone(),
            branch: self.branch.clone(),
            commit: self.commit.clone(),
            fetched_at: self.fetched_at,
        }
    }
}

/// A remote as the store keeps one: its project, its record and the row's version
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRemote {
    /// The name of the project it was registered for
    pub project: String,
    /// What it keeps
    pub kept: KeptRemote,
    /// The row's version, which a later write names
    pub version: u64,
}

/// A remote row read back, checked against the partition it was read from
///
/// # Arguments
///
/// * `row` - The row
fn read_remote(row: RemoteRow) -> anyhow::Result<StoredRemote> {
    // a row of another partition whose key hashed the same is a collision, not a remote
    anyhow::ensure!(
        row.list == REMOTES,
        "remote row `{}` of partition `{}` was stored where the remotes are: their partition \
         keys' hashes collide",
        row.project,
        row.list
    );
    // its project is its key, and the rest its record
    let kept: KeptRemote = serde_json::from_str(&row.record).map_err(|error| {
        anyhow::anyhow!(
            "project `{}`'s remote doesn't read as one: {error}",
            row.project
        )
    })?;
    Ok(StoredRemote {
        project: row.project,
        kept,
        version: row.version,
    })
}

/// The row that keeps a remote at a version
///
/// # Arguments
///
/// * `project` - The name of the project it is for
/// * `kept` - What it keeps
/// * `version` - The version it is written at
fn remote_row(project: &str, kept: &KeptRemote, version: u64) -> anyhow::Result<RemoteRow> {
    Ok(RemoteRow {
        list: REMOTES.to_string(),
        project: project.to_string(),
        version,
        record: serde_json::to_string(kept)?,
    })
}

impl<B: Backend> Store<B> {
    /// Reads every project's remote, by project, its project registered or not
    #[instrument(name = "Store::remotes", skip_all, err(Debug))]
    pub async fn remotes(&self) -> anyhow::Result<HashMap<String, StoredRemote>> {
        // every remote sits in the one listing partition
        let rows = self.backend.remotes().await?;
        rows.into_iter()
            .map(|row| read_remote(row).map(|stored| (stored.project.clone(), stored)))
            .collect()
    }

    /// Registers a project by its git remote, its clone already made and fetched
    ///
    /// Refuses a name another project has, or a repo another project was registered by, by its
    /// URL's key. The remote's row is written first and the project's last, so a registration
    /// killed between them lists no project.
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `kept` - The remote, its clone and the commit fetched
    #[instrument(name = "Store::add_remote_project", skip_all, err(Debug))]
    pub async fn add_remote_project(
        &self,
        name: &str,
        kept: KeptRemote,
    ) -> anyhow::Result<AddedProject> {
        // a project's name keeps the rule every plan's full name holds it to
        PlanName::new(name, "plan")?;
        let clone = kept
            .clone
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("{} is not a utf-8 path", kept.clone.display()))?
            .to_string();
        // one registration at a time, so the name or the repo can't be taken in between
        let _registering = self.projects.lock().await;
        let projects = self.projects().await?;
        if projects.iter().any(|project| project.name == name) {
            return Ok(AddedProject::NameTaken);
        }
        let wanted = remote::key(&kept.url);
        let taken = projects.into_iter().find(|project| {
            project
                .remote
                .as_ref()
                .is_some_and(|remote| wanted.is_some() && remote::key(&remote.url) == wanted)
        });
        if let Some(taken) = taken {
            return Ok(AddedProject::RepoTaken {
                project: taken.name,
            });
        }
        // the remote first, over one a registration killed partway left, if one did
        let orphan = self.remotes().await?.remove(name);
        let (version, expect) = match &orphan {
            Some(orphan) => (orphan.version + 1, Expect::Version(orphan.version)),
            None => (1, Expect::Absent),
        };
        let row = remote_row(name, &kept, version)?;
        if let Written::Refused(refusal) = self.backend.write_remote(row, expect).await? {
            anyhow::bail!(
                "project `{name}`'s remote was written by another process while it was \
                 registered here ({refusal:?})"
            );
        }
        // then the project, which lists it, only if no project of that name is
        let row = ProjectRow {
            list: PROJECTS.to_string(),
            name: name.to_string(),
            repo: clone,
        };
        Ok(match self.backend.insert_project(row).await? {
            Written::Applied => AddedProject::Added(Project {
                name: name.to_string(),
                repo: kept.clone.clone(),
                remote: Some(kept.remote()),
            }),
            Written::Refused(_) => AddedProject::NameTaken,
        })
    }

    /// Keeps what a fetch found, over the version its remote was read at
    ///
    /// # Arguments
    ///
    /// * `read` - The remote as it was read before the fetch
    /// * `commit` - The commit its branch is at now
    /// * `at` - When it was fetched, in milliseconds since the Unix epoch
    #[instrument(name = "Store::fetched", skip_all, err(Debug))]
    pub async fn fetched(
        &self,
        read: &StoredRemote,
        commit: &str,
        at: u64,
    ) -> anyhow::Result<StoredRemote> {
        // the same remote at its new commit, a version on
        let mut kept = read.kept.clone();
        kept.commit = commit.to_string();
        kept.fetched_at = at;
        let row = remote_row(&read.project, &kept, read.version + 1)?;
        // only over the version read, so a fetch elsewhere since isn't written over
        if let Written::Refused(refusal) = self
            .backend
            .write_remote(row, Expect::Version(read.version))
            .await?
        {
            anyhow::bail!(
                "project `{}`'s remote was written by another process while it was fetched \
                 ({refusal:?})",
                read.project
            );
        }
        Ok(StoredRemote {
            project: read.project.clone(),
            kept,
            version: read.version + 1,
        })
    }
}
