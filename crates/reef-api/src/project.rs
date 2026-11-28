//! Projects, the repos their plans describe, and the plans they hold
//!
//! A project names a repo, located at its main worktree's root, so a linked worktree is its main
//! repo. A project registered by its git URL instead is a remote one: the service keeps a clone
//! of the branch it was registered by, and of each branch a plan of it follows, and reads each
//! plan's docs at the commit it last fetched of that plan's branch. A plan is listed in its
//! project with an empty current revision, so a put always has somewhere to go.

use reef_model::{Diagnostic, PlanName};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::user::Author;

/// When a project or a plan was registered, and by whom, as its history keeps it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registered {
    /// When, in milliseconds since the epoch
    pub at: u64,
    /// Who registered it
    #[serde(default, skip_serializing_if = "Author::is_unknown")]
    pub author: Author,
}

/// When a plan was last written, and by whom: its newest write to its nodes or pages, else its
/// listing, for a plan none of whose writes was kept (C119)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Updated {
    /// When, in milliseconds since the epoch
    pub at: u64,
    /// Who wrote it
    #[serde(default, skip_serializing_if = "Author::is_unknown")]
    pub author: Author,
}

impl From<Registered> for Updated {
    /// A plan's listing, as its last update until a write of it is kept
    ///
    /// # Arguments
    ///
    /// * `registered` - When the plan was listed, and by whom
    fn from(registered: Registered) -> Self {
        Updated {
            at: registered.at,
            author: registered.author,
        }
    }
}

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
    /// When it was registered and by whom, where its history says, and a listing reads it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered: Option<Registered>,
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
    /// The branch it was registered by, whose docs a plan that follows no branch of its own reads
    pub branch: String,
    /// The commit the branch was at when it was last fetched, which its docs are read at
    pub commit: String,
    /// When it was last fetched, in milliseconds since the Unix epoch
    pub fetched_at: u64,
    /// Each other branch a plan follows, where it was when it was last fetched
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub branches: BTreeMap<String, Tip>,
}

impl Remote {
    /// The fetched commit's first seven hex digits, as git abbreviates it
    pub fn short(&self) -> &str {
        short(&self.commit)
    }

    /// The commit a branch was at when it was last fetched, if it was fetched
    ///
    /// # Arguments
    ///
    /// * `branch` - The branch, or `None` for the one the project was registered by
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::{Remote, Tip};
    ///
    /// let mut remote = Remote {
    ///     url: "https://example.com/reef.git".to_string(),
    ///     branch: "main".to_string(),
    ///     commit: "a".repeat(40),
    ///     fetched_at: 5,
    ///     branches: Default::default(),
    /// };
    /// remote.branches.insert("dev".to_string(), Tip { commit: "b".repeat(40), fetched_at: 6 });
    /// assert_eq!(remote.commit_of(None), Some("a".repeat(40).as_str()));
    /// assert_eq!(remote.commit_of(Some("main")), Some("a".repeat(40).as_str()));
    /// assert_eq!(remote.commit_of(Some("dev")), Some("b".repeat(40).as_str()));
    /// assert_eq!(remote.commit_of(Some("gone")), None);
    /// ```
    pub fn commit_of(&self, branch: Option<&str>) -> Option<&str> {
        match branch {
            // the branch it was registered by is kept beside the URL
            None => Some(self.commit.as_str()),
            Some(branch) if branch == self.branch => Some(self.commit.as_str()),
            // and each other branch in the map
            Some(branch) => self.branches.get(branch).map(|tip| tip.commit.as_str()),
        }
    }
}

/// A commit's first seven hex digits, as git abbreviates it
///
/// # Arguments
///
/// * `commit` - The commit's id, in hex
pub fn short(commit: &str) -> &str {
    // a commit is hex, so any byte is a char boundary
    &commit[..commit.len().min(7)]
}

/// Where a branch was when the service last fetched it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tip {
    /// The commit it was at
    pub commit: String,
    /// When it was fetched there, in milliseconds since the Unix epoch
    pub fetched_at: u64,
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
    /// When it was listed and by whom, where its history says, and a listing reads it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered: Option<Registered>,
    /// The branch it follows, whose docs it reads, or `None` for its project's own: the branch
    /// a remote project was registered by, or a local project's working tree
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The branches it followed before, the latest last
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub was: Vec<String>,
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
    /// The branch it was registered by moved, and its docs are now read at the new commit
    Moved {
        /// The commit its docs were read at before
        from: String,
        /// The commit they are read at now
        to: String,
        /// What came of fetching each other branch a plan follows
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        branches: Vec<BranchFetched>,
    },
    /// The branch it was registered by is where it was
    Unchanged {
        /// The commit its docs are read at
        commit: String,
        /// What came of fetching each other branch a plan follows
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        branches: Vec<BranchFetched>,
    },
    /// It is a local project, whose docs are its working tree as it stands, so nothing is fetched
    Local,
}

impl Fetched {
    /// What came of fetching each branch past the one the project was registered by
    pub fn branches(&self) -> &[BranchFetched] {
        match self {
            Fetched::Moved { branches, .. } | Fetched::Unchanged { branches, .. } => branches,
            Fetched::Local => &[],
        }
    }
}

/// What came of fetching one branch a plan follows, past the one its project was registered by
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fetched", rename_all = "kebab-case")]
pub enum BranchFetched {
    /// It moved, or was fetched for the first time, and its plans' docs are read at its new commit
    Moved {
        /// The branch
        branch: String,
        /// The commit it was at before, or `None` for a branch fetched for the first time
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        /// The commit it is at now
        to: String,
    },
    /// It is where it was
    Unchanged {
        /// The branch
        branch: String,
        /// The commit it is at
        commit: String,
    },
    /// It couldn't be fetched, such as one deleted from the remote, so its plans read their docs
    /// where it was last fetched
    Failed {
        /// The branch
        branch: String,
        /// Why
        error: String,
    },
}

/// A request to list a plan in a project
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPlan {
    /// The plan's name within its project
    pub name: String,
}

impl NewPlan {
    /// A request to list a plan
    ///
    /// # Arguments
    ///
    /// * `name` - The plan's name within its project
    pub fn new<N: Into<String>>(name: N) -> NewPlan {
        NewPlan { name: name.into() }
    }
}

/// A request to set the branch a plan follows, or to drop it so the plan follows its project's
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanBranch {
    /// The branch to follow, without `refs/heads/`, or `None` for the project's own
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// What came of setting the branch a plan follows
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Branched {
    /// The plan follows the branch now, its docs read there
    Applied {
        /// The plan as its listing names it now
        plan: PlanInfo,
        /// Whether that changed what it followed; a plan set to the branch it follows already
        /// is left as it is
        changed: bool,
    },
    /// The repo has no such branch, or the remote couldn't be fetched for it, so nothing changed
    NoBranch {
        /// The branch asked for
        branch: String,
        /// Why it can't be followed
        reason: String,
    },
    /// The plan's docs at the branch would add problems, so nothing changed
    Refused(Vec<Diagnostic>),
}

/// What came of listing a plan
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedPlan {
    /// Whether it was added; a plan already listed is left as it is
    pub added: bool,
}

#[cfg(test)]
mod tests {
    use super::{
        AddedProject, BranchFetched, Branched, Fetched, NewProject, PlanBranch, PlanInfo, Project,
        Remote, Tip,
    };
    use reef_model::PlanName;
    use serde_json::json;

    /// Each answer to a registration is pinned, since clients match on it
    #[test]
    fn added_project_is_externally_tagged_in_kebab_case() {
        // a project registered names where its repo was found
        let added = AddedProject::Added(Project {
            registered: None,
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
            registered: None,
            name: PlanName::new("reef", "central").unwrap(),
            revision: 3,
            branch: None,
            was: Vec::new(),
        };
        let value = serde_json::to_value(&info).unwrap();
        assert_eq!(value, json!({"name": "reef/central", "revision": 3}));
        // and it reads back as itself
        assert_eq!(serde_json::from_value::<PlanInfo>(value).unwrap(), info);
        // a plan that follows a branch names it
        let follows = PlanInfo {
            branch: Some("dev".to_string()),
            ..info
        };
        let value = serde_json::to_value(&follows).unwrap();
        assert_eq!(
            value,
            json!({"name": "reef/central", "revision": 3, "branch": "dev"})
        );
        assert_eq!(serde_json::from_value::<PlanInfo>(value).unwrap(), follows);
    }

    /// A request to set a plan's branch names it, or leaves it out to follow the project's, and
    /// each answer is pinned, since clients match on it
    #[test]
    fn plan_branch_names_its_branch_or_none() {
        // a branch named, and none
        let set = PlanBranch {
            branch: Some("dev".to_string()),
        };
        assert_eq!(
            serde_json::to_value(&set).unwrap(),
            json!({"branch": "dev"})
        );
        let clear = PlanBranch { branch: None };
        assert_eq!(serde_json::to_value(&clear).unwrap(), json!({}));
        assert_eq!(
            serde_json::from_value::<PlanBranch>(json!({})).unwrap(),
            clear
        );
        // a branch the repo hasn't names it and why
        let missing = Branched::NoBranch {
            branch: "gone".to_string(),
            reason: "no such branch".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&missing).unwrap(),
            json!({"no-branch": {"branch": "gone", "reason": "no such branch"}})
        );
    }

    /// A remote project names its URL, branch and commit, and a local one's JSON is as it was
    #[test]
    fn a_remote_project_names_its_url_branch_and_commit() {
        // a project registered by its URL, and the commit its docs are read at
        let project = Project {
            registered: None,
            name: "reef".to_string(),
            repo: "/data/clones/reef.1.git".into(),
            remote: Some(Remote {
                url: "https://example.com/reef.git".to_string(),
                branch: "main".to_string(),
                commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
                fetched_at: 5,
                branches: Default::default(),
            }),
        };
        let value = serde_json::to_value(&project).unwrap();
        assert_eq!(value["remote"]["url"], "https://example.com/reef.git");
        assert_eq!(value["remote"]["branch"], "main");
        // a remote with no other branch leaves the key out, so it reads as it was written before
        assert!(value["remote"].get("branches").is_none());
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
            branches: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&moved).unwrap(),
            json!({"moved": {"from": "a", "to": "b"}})
        );
        // one that didn't names where it is, and a local project is a bare tag
        let unchanged = Fetched::Unchanged {
            commit: "a".to_string(),
            branches: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&unchanged).unwrap(),
            json!({"unchanged": {"commit": "a"}})
        );
        // each other branch a plan follows says what came of its own fetch
        let others = Fetched::Unchanged {
            commit: "a".to_string(),
            branches: vec![
                BranchFetched::Moved {
                    branch: "dev".to_string(),
                    from: None,
                    to: "c".to_string(),
                },
                BranchFetched::Failed {
                    branch: "gone".to_string(),
                    error: "no such ref".to_string(),
                },
            ],
        };
        let value = serde_json::to_value(&others).unwrap();
        assert_eq!(
            value,
            json!({"unchanged": {"commit": "a", "branches": [
                {"fetched": "moved", "branch": "dev", "to": "c"},
                {"fetched": "failed", "branch": "gone", "error": "no such ref"},
            ]}})
        );
        assert_eq!(serde_json::from_value::<Fetched>(value).unwrap(), others);
        assert_eq!(others.branches().len(), 2);
        // and a tip is a commit and when it was fetched
        let tip = Tip {
            commit: "c".to_string(),
            fetched_at: 9,
        };
        assert_eq!(
            serde_json::to_value(&tip).unwrap(),
            json!({"commit": "c", "fetched_at": 9})
        );
        assert_eq!(
            serde_json::to_value(Fetched::Local).unwrap(),
            json!("local")
        );
    }
}
