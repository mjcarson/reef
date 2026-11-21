//! The app's pages: the picker, and a plan's own page

use crate::calls::{self, ProjectPlans};
use dioxus::prelude::*;

/// The theme reef.css reads with no fallback, which mdbook's themes define and a page outside
/// mdbook lacks, and the shell's own layout
const THEME: &str = r#"
:root { --fg: #333; --bg: #fff; --links: #2b79a2; --muted: #6b6b6b; --line: #ddd; --panel: #f6f6f6; }
@media (prefers-color-scheme: dark) {
  :root { --fg: #ddd; --bg: #1f1f23; --links: #6fb3d9; --muted: #9a9a9a; --line: #3a3a40; --panel: #26262b; }
}
html, body { margin: 0; height: 100%; font-family: system-ui, sans-serif; color: var(--fg); background: var(--bg); }
a { color: var(--links); }
code { font-size: 0.92em; }
.reef-app-picker { max-width: 52rem; margin: 0 auto; padding: 2rem 1rem; }
.reef-app-picker h1 { margin: 0 0 1.5rem; }
.reef-app-projects { list-style: none; padding: 0; margin: 0; display: grid; gap: 1rem; }
.reef-app-project { border: 1px solid var(--line); border-radius: 8px; padding: 0.9rem 1.1rem; background: var(--panel); }
.reef-app-project h2 { margin: 0; font-size: 1.15rem; }
.reef-app-repo { margin: 0.2rem 0 0.6rem; color: var(--muted); font-family: monospace; font-size: 0.85rem; overflow-wrap: anywhere; }
.reef-app-plans { margin: 0; padding-left: 1.2rem; }
.reef-app-plans li { margin: 0.2rem 0; }
.reef-app-muted { color: var(--muted); }
.reef-app-error { color: #c0392b; }
.reef-app-plan { display: flex; flex-direction: column; height: 100vh; }
.reef-app-bar { display: flex; gap: 1rem; align-items: baseline; padding: 0.5rem 1rem; border-bottom: 1px solid var(--line); }
.reef-app-stage { flex: 1; min-height: 0; display: flex; }
"#;

/// The app's pages, by path
#[derive(Debug, Clone, PartialEq, Routable)]
#[rustfmt::skip]
pub enum Route {
    /// Every project and its plans
    #[route("/")]
    Picker {},
    /// One plan, full-window
    #[route("/plans/:project/:plan")]
    PlanPage { project: String, plan: String },
}

/// The app: its stylesheets, then whichever page the path names
#[component]
pub fn App() -> Element {
    rsx! {
        Head {}
        Router::<Route> {}
    }
}

/// The page's stylesheets: reef.css as mdbook-reef installs it, and the theme it expects
///
/// It takes no props, so it never renders again: `document::Style` warns when its props change,
/// and a plain `style` element gets a hydration marker in its text that breaks its first rule.
#[component]
fn Head() -> Element {
    rsx! {
        document::Stylesheet { href: "/reef.css" }
        document::Style { {THEME} }
    }
}

/// The front page: every project, each with its plans to open
#[component]
pub fn Picker() -> Element {
    // every project and its plans, read on the server as the page renders
    let projects = use_server_future(calls::projects)?;
    rsx! {
        main { class: "reef-app-picker",
            h1 { "reef" }
            match &*projects.read() {
                Some(Ok(listed)) if listed.is_empty() => rsx! {
                    p { class: "reef-app-muted",
                        "No projects yet. Register one with "
                        code { "reef project add <name> <repo>" }
                        "."
                    }
                },
                Some(Ok(listed)) => rsx! {
                    ul { class: "reef-app-projects",
                        for listing in listed.iter().cloned() {
                            ProjectEntry { key: "{listing.project.name}", listing }
                        }
                    }
                },
                Some(Err(error)) => rsx! {
                    p { class: "reef-app-error", "Reading the projects failed: {error}" }
                },
                None => rsx! {
                    p { class: "reef-app-muted", "Loading…" }
                },
            }
        }
    }
}

/// One project in the picker: its name, its repo, and a link to each of its plans
///
/// # Arguments
///
/// * `listing` - The project and its plans
#[component]
fn ProjectEntry(listing: ProjectPlans) -> Element {
    // the repo as a path to show, and each plan's route
    let repo = listing.project.repo.display().to_string();
    let plans: Vec<(String, Route)> = listing
        .plans
        .iter()
        .map(|info| {
            let route = Route::PlanPage {
                project: info.name.project.clone(),
                plan: info.name.plan.clone(),
            };
            (info.name.plan.clone(), route)
        })
        .collect();
    rsx! {
        li { class: "reef-app-project",
            h2 { "{listing.project.name}" }
            p { class: "reef-app-repo", "{repo}" }
            if plans.is_empty() {
                p { class: "reef-app-muted",
                    "No plans yet. Make one with "
                    code { "reef plan new <name>" }
                    "."
                }
            } else {
                ul { class: "reef-app-plans",
                    for (name, route) in plans {
                        li { key: "{name}",
                            Link { to: route, "{name}" }
                        }
                    }
                }
            }
        }
    }
}

/// A plan's page: a bar naming the plan, or saying why it can't be read, over the stage it is
/// drawn on
///
/// Its props are signals, so a route to another plan reads that plan again.
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[component]
pub fn PlanPage(project: ReadSignal<String>, plan: ReadSignal<String>) -> Element {
    // the plan, read again when the route names another plan, so a plan that isn't there says so
    let info = use_server_future(move || calls::plan_info(project(), plan()))?;
    rsx! {
        div { class: "reef-app-plan",
            header { class: "reef-app-bar",
                Link { to: Route::Picker {}, "← Plans" }
                strong { class: "reef-app-plan-name", "{project}/{plan}" }
                if let Some(Err(error)) = &*info.read() {
                    span { class: "reef-app-error", "{error}" }
                }
            }
            div { class: "reef-app-stage" }
        }
    }
}
