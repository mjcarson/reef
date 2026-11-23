//! The Changes tab: a node's captured revisions, one revision's files, and each file's diff once
//! it is opened
//!
//! The revisions are read again after each capture the plan's stream announces, so a node
//! worked on shows its newest revision without a reload. A revision lists its files without
//! their hunks, and a file's hunks are fetched only when it is opened, so a node with a large
//! diff opens at once. A node's latest revision may be its draft, which git's hooks rewrite
//! under its number while it is worked on: it says so, an open file's hunks are fetched again
//! each time it is rewritten, and its lines take no comment until it is sealed.

use super::Review;
use super::diff::FileView;
use crate::calls;
use crate::live::use_work_changes;
use dioxus::prelude::*;
use reef_api::{FileStatus, FileSummary, Revision};

/// A node's changes: its revisions to pick from, and the files of the one shown
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
#[component]
pub fn Changes(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    id: ReadSignal<String>,
) -> Element {
    let review = use_context::<Review>();
    // the node's revisions, read again after each capture announced
    let work = use_work_changes();
    let revisions = use_resource(move || async move {
        let _ = work();
        calls::revisions(project(), plan(), id()).await
    });
    let read = revisions.read();
    let listed = match &*read {
        None => {
            return rsx! {
                p { class: "reef-app-muted", "Loading…" }
            };
        }
        Some(Err(error)) => {
            return rsx! {
                p { class: "reef-app-error", "Reading the node's changes failed: {error}" }
            };
        }
        Some(Ok(listed)) => listed,
    };
    // a node never captured has nothing to show yet
    let Some(latest) = listed.last() else {
        return rsx! {
            p { class: "reef-app-muted reef-app-changes-none",
                "No changes captured yet. "
                code { "reef node capture" }
                " keeps them, from the node's worktree."
            }
        };
    };
    // the revision picked, else the latest
    let picked = *review.revision.read();
    let shown = picked
        .and_then(|number| listed.iter().find(|revision| revision.revision == number))
        .unwrap_or(latest)
        .clone();
    let options: Vec<(u32, String)> = listed
        .iter()
        .rev()
        .map(|revision| (revision.revision, option_label(revision, latest.revision)))
        .collect();
    let mut revision_signal = review.revision;
    rsx! {
        div { class: "reef-app-changes",
            label { class: "reef-app-revision-pick",
                "Revision "
                select {
                    class: "reef-app-revision",
                    onchange: move |event| revision_signal.set(event.value().parse().ok()),
                    for (number, label) in options {
                        option {
                            value: "{number}",
                            selected: number == shown.revision,
                            "{label}"
                        }
                    }
                }
            }
            RevisionView { project, plan, id, revision: shown }
        }
    }
}

/// One revision: when it was captured, what conflicted, and its files, each opened in place
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `revision` - The revision, its files without their hunks
#[component]
fn RevisionView(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    id: ReadSignal<String>,
    revision: Revision,
) -> Element {
    let review = use_context::<Review>();
    let (added, removed) = revision.counts();
    let files = revision.files.len();
    let number = revision.revision;
    let stamp = revision.at;
    let draft = revision.draft;
    let at = when(revision.at);
    rsx! {
        p { class: "reef-app-muted reef-app-revision-sum",
            if draft {
                span { class: "reef-app-tag draft", "draft" }
                " Captured {at}, sealed once the node is done · {files} "
            } else {
                "Captured {at} · {files} "
            }
            if files == 1 {
                "file"
            } else {
                "files"
            }
            ", "
            span { class: "reef-app-plus", "+{added}" }
            " "
            span { class: "reef-app-minus", "−{removed}" }
        }
        if !revision.conflicts.is_empty() {
            p { class: "reef-app-conflicts",
                "Conflicted carrying the start forward: "
                for (k , path) in revision.conflicts.iter().enumerate() {
                    if k > 0 {
                        ", "
                    }
                    code { "{path}" }
                }
            }
        }
        if revision.files.is_empty() {
            p { class: "reef-app-muted", "No files changed." }
        }
        ul { class: "reef-app-files",
            for file in revision.files.iter() {
                {
                    let path = file.path().to_string();
                    let open = review.open.read().contains(&path);
                    let conflicted = revision.conflicts.contains(&path);
                    rsx! {
                        FileRow {
                            project,
                            plan,
                            id,
                            revision: number,
                            stamp,
                            draft,
                            file: file.clone(),
                            open,
                            conflicted,
                        }
                    }
                }
            }
        }
    }
}

/// One file of a revision: its status, path and counts, which open its diff below them
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `revision` - The revision's number
/// * `stamp` - When the revision was stored, which a draft's rewrite moves on
/// * `draft` - Whether the revision is the node's draft
/// * `file` - The file, without its hunks
/// * `open` - Whether its diff is open
/// * `conflicted` - Whether it conflicted carrying the start forward
#[allow(clippy::too_many_arguments)]
#[component]
fn FileRow(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    id: ReadSignal<String>,
    revision: u32,
    stamp: u64,
    draft: bool,
    file: FileSummary,
    open: bool,
    conflicted: bool,
) -> Element {
    let review = use_context::<Review>();
    let path = file.path().to_string();
    let (letter, status) = status_of(file.status);
    // a rename shows where it came from
    let shown = match (&file.old_path, &file.new_path) {
        (Some(old), Some(new)) if old != new => format!("{old} → {new}"),
        _ => path.clone(),
    };
    let toggled = path.clone();
    let mut opened = review.open;
    rsx! {
        li { class: if open { "reef-app-file open" } else { "reef-app-file" },
            button {
                class: "reef-app-file-head",
                "data-path": "{path}",
                "aria-expanded": "{open}",
                title: "{status}",
                onclick: move |_| {
                    // open it, or close it if it is open
                    let mut files = opened.write();
                    if !files.remove(&toggled) {
                        files.insert(toggled.clone());
                    }
                },
                span { class: "reef-app-status s-{status}", "{letter}" }
                span { class: "reef-app-file-path", "{shown}" }
                span { class: "reef-app-file-counts",
                    span { class: "reef-app-plus", "+{file.added}" }
                    " "
                    span { class: "reef-app-minus", "−{file.removed}" }
                }
                if file.binary {
                    span { class: "reef-app-tag", "binary" }
                }
                if file.too_large {
                    span { class: "reef-app-tag", "too large" }
                }
                if conflicted {
                    span { class: "reef-app-tag conflicted", "conflicted" }
                }
            }
            if open {
                FileView {
                    project,
                    plan,
                    id,
                    revision,
                    stamp,
                    draft,
                    path: path.clone(),
                }
            }
        }
    }
}

/// A revision as the picker names it: its number, its counts, when it was captured, and
/// whether it is a draft and the latest
///
/// # Arguments
///
/// * `revision` - The revision
/// * `latest` - The latest revision's number
fn option_label(revision: &Revision, latest: u32) -> String {
    let (added, removed) = revision.counts();
    let mut label = format!(
        "{} · +{added} −{removed} · {}",
        revision.revision,
        when(revision.at)
    );
    if revision.draft {
        label.push_str(" · draft");
    }
    if revision.revision == latest {
        label.push_str(" · latest");
    }
    label
}

/// The letter a file's status is shown by, and its name as a class
///
/// # Arguments
///
/// * `status` - The file's status
fn status_of(status: FileStatus) -> (&'static str, &'static str) {
    match status {
        FileStatus::Added => ("A", "added"),
        FileStatus::Deleted => ("D", "deleted"),
        FileStatus::Modified => ("M", "modified"),
        FileStatus::Renamed => ("R", "renamed"),
    }
}

/// A time in milliseconds since the epoch, as a date and a time of day in UTC
///
/// # Arguments
///
/// * `ms` - The time
///
/// # Examples
///
/// ```
/// use reef_app::side::when;
///
/// assert_eq!(when(1_795_381_165_854), "2026-11-22 20:59 UTC");
/// assert_eq!(when(0), "1970-01-01 00:00 UTC");
/// ```
pub fn when(ms: u64) -> String {
    // the day since the epoch, and the minute within it
    let secs = ms / 1000;
    let (days, minute) = (secs / 86_400, (secs % 86_400) / 60);
    // the civil date of a day count, as Howard Hinnant's days_from_civil inverts it
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        minute / 60,
        minute % 60
    )
}
