//! The front page, a plan explorer: every project's plans summed up, kept live (C121)
//!
//! Each project says how many plans it has and how many are active, then lists them, the active
//! ones first and the rest by their last update. Each plan shows what it follows, who listed it,
//! its state of work, its nodes done and those done in the last 24 hours, its last update, and
//! who is working on what.
//!
//! The server renders the page with every summary read, so it reads right before the page's
//! script runs. The client then follows one change stream for every project, and a change reads
//! only its own project's plans again ([`ExplorerChanges`]). The stream's first opening reads
//! every project once more, since the server read the page before the stream was followed, and a
//! change between them would be lost. Times are relative, counted from the server's clock as it
//! read the page and ticking on in the client, with the exact time on hover.

use dioxus::prelude::*;
use reef_api::{Author, PlanState, PlanSummary, Project};
use std::cell::Cell;
use std::rc::Rc;

use crate::app::{Route, follows};
use crate::calls::{self, Explored};
use crate::canvas::dom;
use crate::live::{ExplorerChanges, use_explorer_changes};
use crate::side::when;
use crate::sign_in::{CallFailed, SignedInAs};

/// How often the page's relative times are told again, in milliseconds
const TICK_MS: i32 = 30_000;

/// The front page: every project, how many of its plans are active, and each plan summed up, the
/// active ones first, kept live over one change stream
#[component]
pub fn Explorer() -> Element {
    // the changes every project and each one has had, on one stream
    let changes = use_explorer_changes();
    // every project's plans summed up, read on the server as the page renders
    let first = use_server_future(calls::explorer)?;
    // read again in the client once its stream opens, since the server read the page before it,
    // and after a gap, a reconnect or a project registered, the first read standing until then,
    // so the server's render and the browser's agree
    let all = changes.all;
    let again = use_resource(move || async move {
        match all() {
            0 => None,
            _ => Some(calls::explorer().await),
        }
    });
    // the server's clock as it read the page, ticking on in the client
    let start = match &*first.read() {
        Some(Ok(explored)) => explored.now,
        _ => 0,
    };
    let now = use_clock(start);
    // the newest read that answered, and why the last one didn't, if it didn't
    let (read, failed): (
        Option<Result<Explored, ServerFnError>>,
        Option<ServerFnError>,
    ) = match &*again.read() {
        Some(Some(Ok(explored))) => (Some(Ok(explored.clone())), None),
        Some(Some(Err(error))) => (first.read().clone(), Some(error.clone())),
        _ => (first.read().clone(), None),
    };
    rsx! {
        main { class: "reef-app-picker",
            header { class: "reef-app-picker-head",
                h1 { "reef" }
                SignedInAs {}
            }
            if let Some(error) = failed {
                p { CallFailed { error, doing: "Reading the projects again" } }
            }
            match read {
                Some(Ok(explored)) if explored.projects.is_empty() => rsx! {
                    p { class: "reef-app-muted",
                        "No projects yet. Register one with "
                        code { "reef project add <name>" }
                        " from its repo, or "
                        code { "reef project add <name> --url <git url>" }
                        " for one the service clones."
                    }
                },
                Some(Ok(explored)) => rsx! {
                    ul { class: "reef-app-projects",
                        for listing in explored.projects {
                            ProjectCard {
                                key: "{listing.project.name}",
                                project: listing.project,
                                plans: listing.plans,
                                read_at: explored.now,
                                now,
                                changes,
                            }
                        }
                    }
                },
                Some(Err(error)) => rsx! {
                    p { CallFailed { error, doing: "Reading the projects" } }
                },
                None => rsx! {
                    p { class: "reef-app-muted", "Loading…" }
                },
            }
        }
    }
}

/// A clock starting at the server's time as it read the page, which ticks on in the client
///
/// It counts on from that time by how long the page has been open, so relative times agree with
/// the server's rather than the browser's clock. The server's render never ticks it.
///
/// # Arguments
///
/// * `start` - The server's time as it read the page, in milliseconds since the epoch
fn use_clock(start: u64) -> ReadSignal<u64> {
    // the time now, and the timer that tells it again, kept outside any signal so the page's
    // end can read it after its signals are gone
    let now = use_signal(|| start);
    let timer = use_hook(|| Rc::new(Cell::new(None::<i32>)));
    // ticking from the browser's first render on, since effects run only there
    let ticking = timer.clone();
    use_effect(move || {
        let began = dom::now();
        tick(now, start, began, ticking.clone());
    });
    // and stopping once the page is left, so no tick writes a dropped signal
    use_drop(move || {
        if let Some(pending) = timer.take() {
            dom::cancel(pending);
        }
    });
    now.into()
}

/// Tells the time again after a while, then again after that, keeping each timer to cancel
///
/// # Arguments
///
/// * `now` - The time to tell
/// * `start` - The server's time as it read the page
/// * `began` - When the clock began, in milliseconds since the page started
/// * `timer` - The timer pending, to cancel once the page is left
fn tick(mut now: Signal<u64>, start: u64, began: f64, timer: Rc<Cell<Option<i32>>>) {
    let pending = timer.clone();
    let next = dom::later(TICK_MS, move || {
        // the server's time, and how long the page has been open since
        let since = (dom::now() - began).max(0.0) as u64;
        now.set(start.saturating_add(since));
        tick(now, start, began, pending);
    });
    timer.set(next);
}

/// One project on the front page: its name, how many plans it has and how many are active, its
/// repo, who registered it, and each of its plans summed up, read again after each of its changes
///
/// # Arguments
///
/// It shows whichever read of its plans began last: the page's, or its own after one of its
/// changes, so a page read again after a gap isn't hidden behind an older read of its own.
///
/// # Arguments
///
/// * `project` - The project
/// * `plans` - Its plans summed up, as the page's read found them
/// * `read_at` - When the page's read began, on the server's clock
/// * `now` - The time, which relative times count back from
/// * `changes` - The changes the page's stream counted, every project's and each one's
#[component]
fn ProjectCard(
    project: Project,
    plans: Vec<PlanSummary>,
    read_at: u64,
    now: ReadSignal<u64>,
    changes: ExplorerChanges,
) -> Element {
    // how many of its own changes mean reading its plans again, which moves with nothing else
    let name = project.name.clone();
    let seen = use_memo(move || changes.of(&name));
    // its plans read again after each, the page's read standing until the first
    let named = project.name.clone();
    let fresh = use_resource(move || {
        let named = named.clone();
        async move {
            match seen() {
                0 => None,
                _ => Some(calls::plan_summaries(named).await),
            }
        }
    });
    // the read that began last, and why its own last read didn't answer, if it didn't
    let (mut plans, failed) = match &*fresh.read() {
        Some(Some(Ok(summed))) if summed.at >= read_at => (summed.plans.clone(), None),
        Some(Some(Err(error))) => (plans.clone(), Some(error.clone())),
        _ => (plans.clone(), None),
    };
    // the active plans first, and how many there are of each
    ordered(&mut plans);
    let active = plans.iter().filter(|summary| summary.active).count();
    let count = format!(
        "{} · {active} active",
        counted(plans.len(), "plan", "plans")
    );
    // the repo to show: a remote's URL, branch and commit, else its path
    let repo = match &project.remote {
        Some(remote) => format!("{} · {} @ {}", remote.url, remote.branch, remote.short()),
        None => project.repo.display().to_string(),
    };
    // who registered it, if anyone known did
    let registered = project
        .registered
        .as_ref()
        .and_then(|registered| registered.author.describe());
    rsx! {
        li { class: "reef-app-project", "data-project": "{project.name}",
            div { class: "reef-app-project-head",
                h2 { "{project.name}" }
                span { class: "reef-app-plan-count", "{count}" }
            }
            p { class: "reef-app-repo", "{repo}" }
            if let Some(who) = registered {
                p { class: "reef-app-muted reef-app-registered",
                    "Registered by "
                    span { class: "reef-app-author", "{who}" }
                }
            }
            if plans.is_empty() {
                p { class: "reef-app-muted",
                    "No plans yet. Make one with "
                    code { "reef plan new <name>" }
                    "."
                }
            } else {
                ul { class: "reef-app-plans",
                    for summary in plans {
                        PlanRow {
                            key: "{summary.plan.name}",
                            project: project.clone(),
                            summary,
                            now,
                        }
                    }
                }
            }
            if let Some(error) = failed {
                p { CallFailed { error, doing: "Reading its plans again" } }
            }
        }
    }
}

/// One plan on the front page: its link, what it follows, its state of work and who listed it;
/// its nodes done, those done in the last 24 hours and its last update; and who is working on
/// what, while anyone is
///
/// It holds no list of its own, so a selector over the project's plans meets only the plans.
///
/// # Arguments
///
/// * `project` - The plan's project
/// * `summary` - The plan summed up
/// * `now` - The time, which its last update counts back from
#[component]
fn PlanRow(project: Project, summary: PlanSummary, now: ReadSignal<u64>) -> Element {
    // the plan's page, what it follows, and who listed it
    let info = &summary.plan;
    let route = Route::PlanPage {
        project: info.name.project.clone(),
        plan: info.name.plan.clone(),
        node: String::new(),
    };
    let follows = follows(&project, info);
    let listed = info
        .registered
        .as_ref()
        .and_then(|registered| registered.author.describe());
    // its state of work, and whether work goes on in it now
    let state = state_class(summary.state);
    let said = summary.state.describe();
    let class = match summary.active {
        true => "reef-app-plan-row active",
        false => "reef-app-plan-row",
    };
    // how far it has come, and how far today, naming the nodes on hover
    let done = format!("{} of {} done", summary.nodes.done, summary.nodes.total());
    let today = format!("{} done in the last 24 hours", summary.done_today.len());
    let today_class = match summary.done_today.is_empty() {
        true => "reef-app-done-today",
        false => "reef-app-done-today some",
    };
    let today_ids = summary.done_today.join(", ");
    // when it was last written, relative and exact, and by whom
    let updated = summary.updated.as_ref().map(|updated| {
        let by = updated
            .author
            .describe()
            .map(|who| format!(" by {who}"))
            .unwrap_or_default();
        (ago(now(), updated.at), when(updated.at), by)
    });
    // each node in progress and who started it, each live session and its node, and each run
    let working: Vec<(String, String, String)> = summary
        .in_progress
        .iter()
        .map(|working| {
            let started = working
                .started_at
                .map(|at| format!("started {}", when(at)))
                .unwrap_or_default();
            (
                working.node.clone(),
                who(&working.started_by, "nobody"),
                started,
            )
        })
        .collect();
    let sessions: Vec<(String, String, Option<String>, &'static str)> = summary
        .sessions
        .iter()
        .map(|session| {
            (
                session.id.clone(),
                who(&session.attached_by, "someone"),
                session.node.clone(),
                session.state.describe(),
            )
        })
        .collect();
    let runs: Vec<(String, String, Option<String>, &'static str)> = summary
        .runs
        .iter()
        .map(|run| {
            (
                run.id.clone(),
                who(&run.started_by, "someone"),
                run.node.clone(),
                run.state.describe(),
            )
        })
        .collect();
    rsx! {
        li { class, "data-plan": "{info.name}", "data-state": state,
            div { class: "reef-app-plan-head",
                Link { to: route, "{info.name.plan}" }
                span { class: "reef-app-branch", "{follows}" }
                span { class: "reef-app-plan-state s-{state}", "{said}" }
                if let Some(who) = listed {
                    span { class: "reef-app-listed", "listed by {who}" }
                }
            }
            div { class: "reef-app-plan-facts",
                span { class: "reef-app-plan-done", "{done}" }
                span { class: today_class, title: "{today_ids}", "{today}" }
                if let Some((relative, exact, by)) = updated {
                    span { class: "reef-app-updated", title: "{exact}", "updated {relative}{by}" }
                }
            }
            if !working.is_empty() || !sessions.is_empty() || !runs.is_empty() {
                div { class: "reef-app-plan-work",
                    if !working.is_empty() {
                        span { class: "reef-app-work-kind", "In progress" }
                        for (node, who, started) in working {
                            span { key: "{node}", class: "reef-app-worker", title: "{started}",
                                code { "{node}" }
                                " {who}"
                            }
                        }
                    }
                    if !sessions.is_empty() {
                        span { class: "reef-app-work-kind", "Sessions" }
                        for (id, who, node, state) in sessions {
                            span { key: "{id}", class: "reef-app-live-session", title: "{state}",
                                "{who}"
                                if let Some(node) = node {
                                    " on "
                                    code { "{node}" }
                                }
                            }
                        }
                    }
                    if !runs.is_empty() {
                        span { class: "reef-app-work-kind", "Testing" }
                        for (id, who, node, state) in runs {
                            span { key: "{id}", class: "reef-app-live-run",
                                "{who}"
                                if let Some(node) = node {
                                    " on "
                                    code { "{node}" }
                                }
                                ", {state}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Orders a project's plans as the front page lists them: the active ones first, each part by
/// its last update, the newest first and a plan with none last, then by name
///
/// # Arguments
///
/// * `plans` - The plans, ordered in place
pub fn ordered(plans: &mut [PlanSummary]) {
    plans.sort_by(|a, b| {
        // active before idle, then the newest update first, then by name
        let at = |summary: &PlanSummary| summary.updated.as_ref().map(|updated| updated.at);
        b.active
            .cmp(&a.active)
            .then_with(|| at(b).cmp(&at(a)))
            .then_with(|| a.plan.name.plan.cmp(&b.plan.name.plan))
    });
}

/// How long before now a time was, as the front page says it
///
/// # Arguments
///
/// * `now` - The time now, in milliseconds since the epoch
/// * `at` - The time, which a time ahead of now counts as now
///
/// # Examples
///
/// ```
/// use reef_app::explorer::ago;
///
/// let now = 1_795_381_165_854;
/// assert_eq!(ago(now, now - 59_000), "just now");
/// assert_eq!(ago(now, now + 5_000), "just now");
/// assert_eq!(ago(now, now - 5 * 60_000), "5 min ago");
/// assert_eq!(ago(now, now - 3 * 3_600_000), "3 h ago");
/// assert_eq!(ago(now, now - 86_400_000), "1 day ago");
/// assert_eq!(ago(now, now - 9 * 86_400_000), "9 days ago");
/// ```
pub fn ago(now: u64, at: u64) -> String {
    // how many seconds before now, a time ahead of the clock being now
    let secs = now.saturating_sub(at) / 1000;
    match secs {
        0..60 => "just now".to_string(),
        60..3_600 => format!("{} min ago", secs / 60),
        3_600..86_400 => format!("{} h ago", secs / 3_600),
        _ => match secs / 86_400 {
            1 => "1 day ago".to_string(),
            days => format!("{days} days ago"),
        },
    }
}

/// A count of things, its noun singular for one
///
/// # Arguments
///
/// * `count` - How many
/// * `one` - The noun for one
/// * `many` - The noun for any other count
fn counted(count: usize, one: &str, many: &str) -> String {
    match count {
        1 => format!("1 {one}"),
        _ => format!("{count} {many}"),
    }
}

/// An author's name as the front page shows it, or a stand-in for nobody known
///
/// # Arguments
///
/// * `author` - The author
/// * `unknown` - What to say if nobody is known
fn who(author: &Author, unknown: &str) -> String {
    author.describe().unwrap_or_else(|| unknown.to_string())
}

/// The class a plan's state of work is drawn with, as its JSON names it
///
/// # Arguments
///
/// * `state` - The plan's state of work
fn state_class(state: PlanState) -> &'static str {
    match state {
        PlanState::Testing => "testing",
        PlanState::Sessions => "sessions",
        PlanState::NoSessions => "no-sessions",
    }
}

#[cfg(test)]
mod tests {
    use super::{counted, ordered};
    use reef_api::{NodeCounts, PlanInfo, PlanSummary, Updated, Working};
    use reef_model::PlanName;

    /// A plan summed up, active if a node of it is in progress, last updated when given
    ///
    /// # Arguments
    ///
    /// * `name` - The plan's name within its project
    /// * `active` - Whether a node of it is in progress
    /// * `at` - When it was last updated, if it was
    fn summary(name: &str, active: bool, at: Option<u64>) -> PlanSummary {
        // the plan as its listing names it
        let plan = PlanInfo {
            name: PlanName::new("reef", name).unwrap(),
            revision: 1,
            registered: None,
            branch: None,
            was: Vec::new(),
            docs: None,
        };
        // a node in progress makes it active
        let working = match active {
            true => vec![Working {
                node: "A1".to_string(),
                started_by: Default::default(),
                started_at: None,
            }],
            false => Vec::new(),
        };
        let updated = at.map(|at| Updated {
            at,
            author: Default::default(),
        });
        PlanSummary::new(
            plan,
            None,
            NodeCounts::default(),
            updated,
            Vec::new(),
            working,
            Vec::new(),
            Vec::new(),
        )
    }

    /// The active plans come first, each part newest update first, a plan with none last, and
    /// ties by name
    #[test]
    fn active_plans_come_first_then_the_newest() {
        // two active plans and four idle ones, one never updated and two updated together
        let mut plans = vec![
            summary("idle-old", false, Some(10)),
            summary("never", false, None),
            summary("busy-old", true, Some(5)),
            summary("idle-new", false, Some(30)),
            summary("busy-new", true, Some(20)),
            summary("idle-b", false, Some(15)),
            summary("idle-a", false, Some(15)),
        ];
        ordered(&mut plans);
        let names: Vec<&str> = plans
            .iter()
            .map(|summary| summary.plan.name.plan.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "busy-new", "busy-old", "idle-new", "idle-a", "idle-b", "idle-old", "never"
            ]
        );
    }

    /// One thing is counted singular, and any other count plural
    #[test]
    fn counts_say_one_or_many() {
        assert_eq!(counted(1, "plan", "plans"), "1 plan");
        assert_eq!(counted(0, "plan", "plans"), "0 plans");
        assert_eq!(counted(3, "plan", "plans"), "3 plans");
    }
}
