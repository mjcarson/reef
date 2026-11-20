//! Projects, the repos their plans describe, and the plans they hold
//!
//! A project names a repo, located at its main worktree's root, so a linked worktree is its main
//! repo. A plan is listed in its project with an empty current revision, so a put always has
//! somewhere to go.

use reef_model::PlanName;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A project: a name, and the repo its plans describe
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// The project's name
    pub name: String,
    /// The root of its repo's main worktree, or the directory it names outside git
    pub repo: PathBuf,
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

/// A request to register a project for a repo
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProject {
    /// The project's name
    pub name: String,
    /// A directory in the repo its plans describe, as the service's host sees it
    pub repo: PathBuf,
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
    use super::{AddedProject, PlanInfo, Project};
    use reef_model::PlanName;
    use serde_json::json;

    /// Each answer to a registration is pinned, since clients match on it
    #[test]
    fn added_project_is_externally_tagged_in_kebab_case() {
        // a project registered names where its repo was found
        let added = AddedProject::Added(Project {
            name: "reef".to_string(),
            repo: "/src/reef".into(),
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
}
