//! Projects, the repos their plans describe, and the plans they hold
//!
//! A project names a repo, located at its main worktree's root, so a linked worktree is its main
//! repo. A project registered by its git URL instead is a remote one: the service keeps a clone
//! of one branch of it, and reads its docs at the commit it last fetched. A plan is listed in its
//! project with an empty current revision, so a put always has somewhere to go.

use reef_model::PlanName;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A project: a name, and the repo its plans describe
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// The project's name
    pub name: String,
    /// The root of its repo's main worktree, or the directory it names outside git, or for a
    /// remote project the service's clone of it, each as the service's host sees it
    pub repo: PathBuf,
    /// The git remote it was registered by, if it was, and the commit its docs are read at
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<Remote>,
}

impl Project {
    /// The repo as a person would name it: a remote project's URL, else its path on the
    /// service's host
    pub fn shown_repo(&self) -> String {
        match &self.remote {
            Some(remote) => remote.url.clone(),
            None => self.repo.display().to_string(),
        }
    }
}

/// The git remote a project was registered by, and what the service last fetched of it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    /// The URL the service fetches from
    pub url: String,
    /// The branch whose docs it reads
    pub branch: String,
    /// The commit the branch was at when it was last fetched, which its docs are read at
    pub commit: String,
    /// When it was last fetched, in milliseconds since the Unix epoch
    pub fetched_at: u64,
}

impl Remote {
    /// The fetched commit's first seven hex digits, as git abbreviates it
    pub fn short(&self) -> &str {
        // a commit is hex, so any byte is a char boundary
        &self.commit[..self.commit.len().min(7)]
    }
}

/// What came of registering a project
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AddedProject {
    /// It was registered, its repo located
    Added(Project),
    /// A project of that name is registered
    NameTaken,
    /// Another project is registered for that repo
    RepoTaken {
        /// The project registered for it
        project: String,
    },
}

/// One of a project's plans, as its listing names it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanInfo {
    /// The plan's full name
    pub name: PlanName,
    /// The revision readers load
    pub revision: u64,
}

/// A request to register a project, for a repo on the service's host or by its git URL
///
/// It names one of `repo` and `remote`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProject {
    /// The project's name
    pub name: String,
    /// A directory in the repo its plans describe, as the service's host sees it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<PathBuf>,
    /// The git remote its plans describe, which the service clones and fetches
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<NewRemote>,
}

impl NewProject {
    /// A request to register a project for the repo that holds a directory
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `repo` - A directory in its repo, as the service's host sees it
    pub fn dir<N: Into<String>, P: Into<PathBuf>>(name: N, repo: P) -> NewProject {
        NewProject {
            name: name.into(),
            repo: Some(repo.into()),
            remote: None,
        }
    }

    /// A request to register a project by its git URL
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `url` - The URL the service clones and fetches
    /// * `branch` - The branch whose docs it reads, or `None` for the remote's default
    pub fn remote<N: Into<String>, U: Into<String>>(
        name: N,
        url: U,
        branch: Option<String>,
    ) -> NewProject {
        NewProject {
            name: name.into(),
            repo: None,
            remote: Some(NewRemote {
                url: url.into(),
                branch,
            }),
        }
    }
}

/// The git remote a new project is registered by
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewRemote {
    /// The URL the service clones and fetches, which carries no password
    pub url: String,
    /// The branch whose docs it reads, or `None` for the remote's default
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// What came of fetching a project's remote
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fetched {
    /// Its branch moved, and its docs are now read at the new commit
    Moved {
        /// The commit its docs were read at before
        from: String,
        /// The commit they are read at now
        to: String,
    },
    /// Its branch is where it was
    Unchanged {
        /// The commit its docs are read at
        commit: String,
    },
    /// It is a local project, whose docs are its working tree as it stands, so nothing is fetched
    Local,
}

/// A request to list a plan in a project
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPlan {
    /// The plan's name within its project
    pub name: String,
}

/// What came of listing a plan
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedPlan {
    /// Whether it was added; a plan already listed is left as it is
    pub added: bool,
}

#[cfg(test)]
mod tests {
    use super::{AddedProject, Fetched, NewProject, PlanInfo, Project, Remote};
    use reef_model::PlanName;
    use serde_json::json;

    /// Each answer to a registration is pinned, since clients match on it
    #[test]
    fn added_project_is_externally_tagged_in_kebab_case() {
        // a project registered names where its repo was found
        let added = AddedProject::Added(Project {
            name: "reef".to_string(),
            repo: "/src/reef".into(),
            remote: None,
        });
        assert_eq!(
            serde_json::to_value(&added).unwrap(),
            json!({"added": {"name": "reef", "repo": "/src/reef"}})
        );
        // a taken name is a bare tag, and a taken repo names its project
        assert_eq!(
            serde_json::to_value(AddedProject::NameTaken).unwrap(),
            json!("name-taken")
        );
        let taken = AddedProject::RepoTaken {
            project: "other".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&taken).unwrap(),
            json!({"repo-taken": {"project": "other"}})
        );
    }

    /// A plan's full name is one string, as everywhere else reef names a plan
    #[test]
    fn plan_info_names_its_plan_in_full() {
        // a plan and the revision readers load
        let info = PlanInfo {
            name: PlanName::new("reef", "central").unwrap(),
            revision: 3,
        };
        let value = serde_json::to_value(&info).unwrap();
        assert_eq!(value, json!({"name": "reef/central", "revision": 3}));
        // and it reads back as itself
        assert_eq!(serde_json::from_value::<PlanInfo>(value).unwrap(), info);
    }

    /// A remote project names its URL, branch and commit, and a local one's JSON is as it was
    #[test]
    fn a_remote_project_names_its_url_branch_and_commit() {
        // a project registered by its URL, and the commit its docs are read at
        let project = Project {
            name: "reef".to_string(),
            repo: "/data/clones/reef.1.git".into(),
            remote: Some(Remote {
                url: "https://example.com/reef.git".to_string(),
                branch: "main".to_string(),
                commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
                fetched_at: 5,
            }),
        };
        let value = serde_json::to_value(&project).unwrap();
        assert_eq!(value["remote"]["url"], "https://example.com/reef.git");
        assert_eq!(value["remote"]["branch"], "main");
        assert_eq!(serde_json::from_value::<Project>(value).unwrap(), project);
        assert_eq!(project.remote.as_ref().unwrap().short(), "0123456");
        assert_eq!(project.shown_repo(), "https://example.com/reef.git");
        // a local project leaves the key out, and one written before remotes reads back as local
        let local = json!({"name": "reef", "repo": "/src/reef"});
        let read: Project = serde_json::from_value(local.clone()).unwrap();
        assert_eq!(read.remote, None);
        assert_eq!(read.shown_repo(), "/src/reef");
        assert_eq!(serde_json::to_value(&read).unwrap(), local);
        // a request names one of a directory and a remote, leaving the other out
        let dir = serde_json::to_value(NewProject::dir("reef", "/src/reef")).unwrap();
        assert_eq!(dir, json!({"name": "reef", "repo": "/src/reef"}));
        let remote = NewProject::remote("reef", "https://example.com/reef.git", None);
        assert_eq!(
            serde_json::to_value(&remote).unwrap(),
            json!({"name": "reef", "remote": {"url": "https://example.com/reef.git"}})
        );
    }

    /// Each answer to a fetch is pinned, since clients match on it
    #[test]
    fn fetched_is_externally_tagged_in_kebab_case() {
        // a branch that moved names both commits
        let moved = Fetched::Moved {
            from: "a".to_string(),
            to: "b".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&moved).unwrap(),
            json!({"moved": {"from": "a", "to": "b"}})
        );
        // one that didn't names where it is, and a local project is a bare tag
        let unchanged = Fetched::Unchanged {
            commit: "a".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&unchanged).unwrap(),
            json!({"unchanged": {"commit": "a"}})
        );
        assert_eq!(
            serde_json::to_value(Fetched::Local).unwrap(),
            json!("local")
        );
    }
}
