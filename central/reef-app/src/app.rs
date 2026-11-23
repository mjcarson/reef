//! The app's pages: the picker, and a plan's own page

use crate::calls::{self, ProjectPlans};
use crate::canvas::{Canvas, Keyed};
use crate::live::use_changes;
use crate::queue::{QueuePanel, use_queue};
use dioxus::prelude::*;

/// The theme reef.css reads with no fallback, which mdbook's themes define and a page outside
/// mdbook lacks, and the shell's own layout
const THEME: &str = r#"
:root { --fg: #333; --bg: #fff; --links: #2b79a2; --muted: #6b6b6b; --line: #ddd; --panel: #f6f6f6;
  --add-bg: #e6ffec; --add-fg: #1a7f37; --del-bg: #ffebe9; --del-fg: #cf222e; --hunk-bg: #ddf4ff; }
@media (prefers-color-scheme: dark) {
  :root { --fg: #ddd; --bg: #1f1f23; --links: #6fb3d9; --muted: #9a9a9a; --line: #3a3a40; --panel: #26262b;
    --add-bg: #12301d; --add-fg: #56d364; --del-bg: #3a1618; --del-fg: #f85149; --hunk-bg: #172a3d; }
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
.reef-app-stage { flex: 1; min-height: 0; display: flex; position: relative; }
.reef-app-stage > .reef.reef-page { margin: 0; border: 0; border-radius: 0; min-width: 0; }
.reef-app-over { position: absolute; top: 3rem; left: 0; right: 0; text-align: center; }
.reef-app-stage > .reef.reef-page { flex: 1; }
.reef-app-tabs { display: flex; gap: 0.2rem; margin: 0.6rem 0 0.2rem; border-bottom: 1px solid var(--line); }
.reef-app-tab { font: inherit; font-size: 0.85rem; color: var(--muted); background: none; border: 0; border-bottom: 2px solid transparent; padding: 0.25rem 0.6rem; margin-bottom: -1px; cursor: pointer; }
.reef-app-tab:hover { color: var(--fg); }
.reef-app-tab.on { color: var(--fg); border-bottom-color: var(--links); }
.reef-app-editor { margin-top: 0.6rem; }
.reef-app-html { display: contents; }
.reef-app-yaml { width: 100%; box-sizing: border-box; min-height: 24rem; resize: vertical; font-family: monospace; font-size: 0.85rem; color: var(--fg); background: var(--bg); border: 1px solid var(--line); border-radius: 6px; padding: 0.5rem; }
.reef-app-actions { display: flex; gap: 0.5rem; margin: 0.5rem 0; }
.reef-app-problems { margin: 0.3rem 0 0.6rem; padding-left: 1.2rem; color: #c0392b; font-size: 0.9rem; }
.reef-app-newer { white-space: pre-wrap; font-size: 0.8rem; background: var(--bg); border: 1px solid var(--line); border-radius: 6px; padding: 0.5rem; }
.reef-app-queue { flex: none; width: 15rem; overflow-y: auto; box-sizing: border-box; padding: 0.6rem 0.7rem; border-right: 1px solid var(--line); background: var(--panel); font-size: 0.85rem; }
.reef-app-queue-head { margin: 0 0 0.5rem; font-size: 0.95rem; }
.reef-app-queue-list { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.3rem; }
.reef-app-queue-list.dragging { user-select: none; cursor: grabbing; }
.reef-app-queue-entry { display: flex; gap: 0.35rem; align-items: flex-start; padding: 0.35rem 0.3rem; border: 1px solid var(--line); border-radius: 6px; background: var(--bg); }
.reef-app-queue-entry:focus { outline: 2px solid var(--links); outline-offset: 1px; }
.reef-app-queue-entry.dragged { opacity: 0.45; }
.reef-app-queue-entry.drop-before { box-shadow: 0 -3px 0 var(--links); }
.reef-app-queue-entry.drop-after { box-shadow: 0 3px 0 var(--links); }
.reef-app-queue-grip { cursor: grab; touch-action: none; color: var(--muted); padding: 0 0.1rem; }
.reef-app-queue-place { color: var(--muted); min-width: 1.4rem; text-align: right; }
.reef-app-queue-body { flex: 1; min-width: 0; }
.reef-app-queue-top { display: flex; gap: 0.4rem; align-items: baseline; flex-wrap: wrap; }
.reef-app-queue-id { overflow-wrap: anywhere; }
.reef-app-queue-state { font-size: 0.75rem; color: var(--muted); }
.reef-app-queue-state.s-ready { color: var(--reef-ready); }
.reef-app-queue-state.s-done { color: var(--reef-done); }
.reef-app-queue-state.s-blocked, .reef-app-queue-state.s-missing { color: var(--reef-blocked); }
.reef-app-queue-state.s-in-progress { color: var(--reef-progress); }
.reef-app-queue-title { margin-top: 0.1rem; overflow-wrap: anywhere; }
.reef-app-queue-needs { margin-top: 0.1rem; color: var(--muted); font-size: 0.78rem; }
.reef-app-queue-off { flex: none; font: inherit; line-height: 1; color: var(--muted); background: none; border: 0; cursor: pointer; padding: 0 0.15rem; }
.reef-app-queue-off:hover { color: var(--fg); }
.reef-app-queue-problem { margin: 0 0 0.5rem; }
.reef-app-changes { margin-top: 0.6rem; font-size: 0.85rem; }
.reef-app-revision-pick { display: flex; gap: 0.4rem; align-items: center; }
.reef-app-revision { flex: 1; min-width: 0; font: inherit; color: var(--fg); background: var(--bg); border: 1px solid var(--line); border-radius: 6px; padding: 0.2rem 0.3rem; }
.reef-app-revision-sum { margin: 0.4rem 0; }
.reef-app-plus { color: var(--add-fg); }
.reef-app-minus { color: var(--del-fg); }
.reef-app-conflicts { margin: 0.4rem 0; color: var(--reef-blocked); }
.reef-app-files { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.3rem; }
.reef-app-file { border: 1px solid var(--line); border-radius: 6px; background: var(--bg); min-width: 0; }
.reef-app-file-head { display: flex; gap: 0.45rem; align-items: baseline; width: 100%; box-sizing: border-box; font: inherit; color: inherit; text-align: left; background: var(--panel); border: 0; border-radius: 6px; padding: 0.3rem 0.45rem; cursor: pointer; }
.reef-app-file.open > .reef-app-file-head { border-bottom: 1px solid var(--line); border-radius: 6px 6px 0 0; }
.reef-app-file-head:hover .reef-app-file-path { text-decoration: underline; }
.reef-app-status { flex: none; font-family: monospace; font-weight: 600; width: 1ch; }
.reef-app-status.s-added { color: var(--add-fg); }
.reef-app-status.s-deleted { color: var(--del-fg); }
.reef-app-status.s-modified { color: var(--reef-progress); }
.reef-app-status.s-renamed { color: var(--links); }
.reef-app-file-path { flex: 1; min-width: 0; font-family: monospace; overflow-wrap: anywhere; }
.reef-app-file-counts { flex: none; font-family: monospace; font-size: 0.8rem; }
.reef-app-tag { flex: none; font-size: 0.72rem; color: var(--muted); border: 1px solid var(--line); border-radius: 4px; padding: 0 0.3rem; }
.reef-app-tag.conflicted { color: var(--reef-blocked); border-color: var(--reef-blocked); }
.reef-app-tag.draft { color: var(--reef-progress); border-color: var(--reef-progress); }
.reef-app-diff { overflow-x: auto; }
.reef-app-diff-note { margin: 0.4rem 0.5rem; }
.reef-app-diff-wait { margin: 0.4rem 0.5rem; }
.reef-app-hunks { border-collapse: collapse; width: 100%; font-family: monospace; font-size: 0.78rem; line-height: 1.45; }
.reef-app-hunk-head td { background: var(--hunk-bg); color: var(--muted); padding: 0.1rem 0.5rem; white-space: pre; }
.reef-app-ln { width: 1%; min-width: 2.5ch; padding: 0 0.4rem; text-align: right; color: var(--muted); user-select: none; vertical-align: top; }
.reef-app-code { padding: 0 0.5rem 0 0.2rem; white-space: pre; }
.reef-app-sign { display: inline-block; width: 1.4ch; user-select: none; }
.reef-app-line.added { background: var(--add-bg); }
.reef-app-line.removed { background: var(--del-bg); }
.reef-app-line.added .reef-app-sign { color: var(--add-fg); }
.reef-app-line.removed .reef-app-sign { color: var(--del-fg); }
.reef-app-eof td { color: var(--muted); font-style: italic; padding: 0 0.5rem; }
.reef-app-diff.commenting .reef-app-ln { cursor: pointer; position: relative; }
.reef-app-diff.commenting .reef-app-line:hover .reef-app-ln:first-child::before { content: "+"; position: absolute; left: 0.1rem; color: #fff; background: var(--links); border-radius: 3px; width: 1.1em; text-align: center; font-weight: 600; }
.reef-app-line.focus td { box-shadow: inset 0 1px 0 var(--links), inset 0 -1px 0 var(--links); }
.reef-app-thread-row > td, .reef-app-compose-row > td { padding: 0.35rem 0.5rem; background: var(--panel); border-top: 1px solid var(--line); border-bottom: 1px solid var(--line); font-family: system-ui, sans-serif; font-size: 0.85rem; white-space: normal; }
.reef-app-count { display: inline-block; min-width: 1.1em; margin-left: 0.3rem; padding: 0 0.3rem; font-size: 0.72rem; text-align: center; border-radius: 9px; color: var(--muted); border: 1px solid var(--line); }
.reef-app-count.some, .some > .reef-app-count { color: #fff; background: var(--links); border-color: var(--links); }
.reef-app-comments { margin-top: 0.6rem; font-size: 0.85rem; display: grid; gap: 0.5rem; }
.reef-app-comments-sum { margin: 0; }
.reef-app-comments-head { margin: 0.4rem 0 0; font-size: 0.9rem; }
.reef-app-thread { border: 1px solid var(--line); border-left: 3px solid var(--links); border-radius: 6px; padding: 0.4rem 0.55rem; background: var(--bg); }
.reef-app-thread.resolved { border-left-color: var(--reef-done); opacity: 0.8; }
.reef-app-thread.focus { box-shadow: 0 0 0 2px var(--links); }
.reef-app-thread.outdated { border-left-style: dashed; }
.reef-app-thread.pending { border-left-style: dotted; }
.reef-app-tag.reef-app-pending { color: var(--reef-progress); border-color: var(--reef-progress); }
.reef-app-review-bar { display: flex; flex-wrap: wrap; align-items: center; gap: 0.5rem; margin: 0 0 0.6rem; padding: 0.4rem 0.5rem; border: 1px solid var(--reef-progress); border-radius: 6px; }
.reef-app-review-sum { flex: 1; font-size: 0.85rem; }
.reef-app-comment + .reef-app-comment { margin-top: 0.4rem; padding-top: 0.4rem; border-top: 1px dashed var(--line); }
.reef-app-comment-head { color: var(--muted); font-size: 0.75rem; }
.reef-app-comment-number { font-family: monospace; }
.reef-app-comment-body { overflow-wrap: anywhere; }
.reef-app-comment-body > :first-child { margin-top: 0.15rem; }
.reef-app-comment-body > :last-child { margin-bottom: 0.15rem; }
.reef-app-comment-body pre { overflow-x: auto; background: var(--panel); padding: 0.3rem 0.4rem; border-radius: 4px; }
.reef-app-thread-foot { display: flex; gap: 0.4rem; align-items: center; margin-top: 0.35rem; }
.reef-app-thread-state { font-size: 0.75rem; margin-right: auto; }
.reef-app-thread-state.open { color: var(--links); }
.reef-app-thread-state.resolved { color: var(--reef-done); }
.reef-app-thread-resolution { margin: 0.25rem 0 0; font-size: 0.75rem; color: var(--muted); overflow-wrap: anywhere; }
.reef-app-composer { margin-top: 0.35rem; }
.reef-app-compose { width: 100%; box-sizing: border-box; resize: vertical; font: inherit; font-size: 0.85rem; color: var(--fg); background: var(--bg); border: 1px solid var(--line); border-radius: 6px; padding: 0.35rem 0.45rem; }
.reef-app-composer .reef-app-actions { margin: 0.3rem 0 0; }
.reef-app-line-thread { display: grid; gap: 0.25rem; }
.reef-app-line-thread-head { display: flex; gap: 0.5rem; align-items: center; justify-content: space-between; }
.reef-app-line-thread-head code { overflow-wrap: anywhere; }
.reef-app-open-box { position: relative; }
.reef-app-open-comments .reef-app-count { margin-left: 0; }
.reef-results.reef-app-open-list { left: auto; right: 0; width: min(380px, 80vw); }
.reef-app-open-where { font-size: 0.8rem; color: var(--muted); }
.reef-app-open-text { overflow-wrap: anywhere; }
.reef-app-open-none { padding: 4px 10px; }
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

/// A plan's page: a bar naming the plan, or saying why it can't be read, over its project's
/// queue and the plan's canvas
///
/// Its props are signals, so a route to another plan reads that plan again, and the canvas is
/// keyed by the plan, so another plan starts it afresh. The queue is the project's, so it sits
/// outside that key, and the canvas's sidebar reaches it through context.
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[component]
pub fn PlanPage(project: ReadSignal<String>, plan: ReadSignal<String>) -> Element {
    // the plan, read again when the route names another plan, so a plan that isn't there says so
    let info = use_server_future(move || calls::plan_info(project(), plan()))?;
    // the plan's changes, which read its payload again
    let changes = use_changes(project, plan);
    // its payload, read in the client alone, since only the client draws it, named for its plan,
    // and read again after each change
    let payload = use_resource(move || async move {
        let _ = changes();
        let (project, plan) = (project(), plan());
        let name = format!("{project}/{plan}");
        calls::payload(project, plan)
            .await
            .map(|payload| -> Keyed { (name, payload) })
    });
    // the project's queue, which the panel shows and the sidebar edits
    use_queue(project);
    let (project_now, plan_now) = (project(), plan());
    rsx! {
        div { class: "reef-app-plan",
            header { class: "reef-app-bar",
                Link { to: Route::Picker {}, "← Plans" }
                strong { class: "reef-app-plan-name", "{project}/{plan}" }
                if let Some(Err(error)) = &*info.read() {
                    span { class: "reef-app-error", "{error}" }
                }
            }
            div { class: "reef-app-stage",
                QueuePanel { project }
                // a key holds only on a block's first node, so the canvas is one of its own
                {
                    rsx! {
                        Canvas {
                            key: "{project_now}/{plan_now}",
                            project: project_now.clone(),
                            plan: plan_now.clone(),
                            payload,
                            changes,
                        }
                    }
                }
            }
        }
    }
}
