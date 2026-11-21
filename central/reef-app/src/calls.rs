//! The server functions the app's pages call, each a call on the service
//!
//! They sit under `/app/`, apart from the JSON API's `/api/v1`, whose wire format the CLI and
//! the hooks depend on. Their answers are the service's own types, from `reef-api`.

use dioxus::prelude::*;
use reef_api::{PlanInfo, Project};
#[cfg(feature = "server")]
use reef_model::PlanName;
use serde::{Deserialize, Serialize};

/// A project and its plans, as the picker lists them
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectPlans {
    /// The project: its name and repo
    pub project: Project,
    /// Its plans, by name
    pub plans: Vec<PlanInfo>,
}

/// Every registered project, by name, with its plans
#[get("/app/projects")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::projects", skip_all, err(Debug))
)]
pub async fn projects() -> Result<Vec<ProjectPlans>, ServerFnError> {
    // each project, then the plans it lists
    let service = crate::server::service();
    let projects = service.projects().await.map_err(failed)?;
    let mut listed = Vec::with_capacity(projects.len());
    for project in projects {
        let plans = service.plans(&project.name).await.map_err(failed)?;
        listed.push(ProjectPlans { project, plans });
    }
    Ok(listed)
}

/// One plan's name and the revision its readers load
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[get("/app/plan?project&plan")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::plan_info", skip_all, err(Debug))
)]
pub async fn plan_info(project: String, plan: String) -> Result<PlanInfo, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let name = plan_name(project, plan)?;
    // the plan among its project's, which names an unregistered project
    let plans = crate::server::service()
        .plans(&name.project)
        .await
        .map_err(failed)?;
    plans
        .into_iter()
        .find(|info| info.name == name)
        .ok_or_else(|| ServerFnError::new(format!("`{name}` is not a plan of its project")))
}

/// A plan's full name from the route's two parts
///
/// # Arguments
///
/// * `project` - The project's name
/// * `plan` - The plan's name within it
#[cfg(feature = "server")]
fn plan_name(project: String, plan: String) -> Result<PlanName, ServerFnError> {
    PlanName::new(project, plan).map_err(|error| ServerFnError::new(error.to_string()))
}

/// A service's error as a server function's, with its causes
///
/// # Arguments
///
/// * `error` - What went wrong
#[cfg(feature = "server")]
fn failed(error: anyhow::Error) -> ServerFnError {
    ServerFnError::new(format!("{error:#}"))
}
