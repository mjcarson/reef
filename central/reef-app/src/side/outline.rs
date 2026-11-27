//! The semantic view of a revision's file: its changes grouped by the types and functions they
//! change, each tagged new, changed or removed
//!
//! A file the capture outlined lists its groups from the revision's listing at once: a struct,
//! enum, trait or inline module with its definition and its members, a loose function or item
//! alone, then the lines no item holds. A group opens to its lines, which fetches the file's
//! hunks once, and each of its items draws the runs of the hunks' lines that are its own, as
//! the classic view draws a line, so threads show and start on them there too. "Show in diff"
//! opens the group that holds its line.

use super::Review;
use super::changes::status_of;
use super::diff::{Lines, Spot, focused_spot, hunk_table, spot, threads_by_line};
use super::threads::Threads;
use crate::calls;
use dioxus::prelude::*;
use reef_api::outline::{
    Group, Item, OTHER_KEY, Outline, Segment, attribute, attribute_one, groups, segments,
};
use reef_api::{Anchor, FileDiff, FileSummary, Hunk, Line, LineKind, Side, Thread};
use std::collections::HashSet;

/// An outlined file of a revision: its head, then its groups, each opened in place, its hunks
/// fetched once one is
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `revision` - The revision's number
/// * `stamp` - When the revision was stored, which a draft's rewrite moves on
/// * `draft` - Whether the revision is the node's draft, whose lines take no comment
/// * `file` - The file as the revision lists it, its outline and all
/// * `conflicted` - Whether it conflicted carrying the start forward
#[allow(clippy::too_many_arguments)]
#[component]
pub fn OutlineFile(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    id: ReadSignal<String>,
    revision: ReadSignal<u32>,
    stamp: ReadSignal<u64>,
    draft: ReadSignal<bool>,
    file: ReadSignal<FileSummary>,
    conflicted: bool,
) -> Element {
    let review = use_context::<Review>();
    // whether any of its groups is open, and whether one ever was, which fetches its hunks
    let wanted = use_memo(move || {
        let path = file.read().path().to_string();
        review
            .groups
            .read()
            .get(&path)
            .is_some_and(|open| !open.is_empty())
    });
    let mut asked = use_signal(|| false);
    use_effect(move || {
        if wanted() && !*asked.peek() {
            asked.set(true);
        }
    });
    // the file's hunks, read once asked for, again in another revision, and again each time its
    // draft is rewritten
    let fetched = use_resource(move || async move {
        let _ = stamp();
        if !asked() {
            return None;
        }
        let path = file.read().path().to_string();
        Some(calls::revision_file(project(), plan(), id(), revision(), path).await)
    });
    // a line asked for by "Show in diff" opens its group: where the listing's spans put it
    // until the hunks come, then where its line is in them
    use_effect(move || {
        let Some(anchor) = review.focus.read().clone() else {
            return;
        };
        let summary = file.read();
        if anchor.revision != revision() || anchor.path != summary.path() {
            return;
        }
        if review.applied.read().as_ref() == Some(&anchor) {
            return;
        }
        let Some(outline) = summary.outline.as_ref() else {
            return;
        };
        let read = fetched.read();
        let hunks = match &*read {
            Some(Some(Ok(Some(diff)))) => Some(diff),
            _ => None,
        };
        let key = match hunks {
            Some(diff) => exact_key(diff, &anchor),
            None => listed_key(outline, summary.added, summary.removed, &anchor),
        };
        let mut open = review.groups;
        open.write()
            .entry(anchor.path.clone())
            .or_default()
            .insert(key);
        // once its line is found in the hunks, the line is opened for good
        if hunks.is_some() {
            let mut applied = review.applied;
            applied.set(Some(anchor));
        }
    });
    let summary = file.read();
    let path = summary.path().to_string();
    let (letter, status) = status_of(summary.status);
    // a rename shows where it came from
    let shown = match (&summary.old_path, &summary.new_path) {
        (Some(old), Some(new)) if old != new => format!("{old} → {new}"),
        _ => path.clone(),
    };
    let open: HashSet<String> = review.groups.read().get(&path).cloned().unwrap_or_default();
    // the threads whose line is in this file in this revision, if the plan's are read
    let threads = try_use_context::<Threads>();
    let on_file: Vec<Thread> = threads
        .map(|threads| threads.of(&id.read()))
        .unwrap_or_default()
        .into_iter()
        .filter(|thread| {
            thread
                .anchor_in(revision())
                .is_some_and(|anchor| anchor.path == path)
        })
        .collect();
    // a line is commented on only where the plan's threads are kept, and never in a draft
    let node = threads.filter(|_| !draft()).map(|_| id());
    let held = threads.is_some() && draft() && !open.is_empty();
    // the hunks as fetched, or why they aren't
    let (diff, failed) = match &*fetched.read() {
        Some(Some(Ok(Some(diff)))) => (Some(diff.clone()), None),
        Some(Some(Ok(None))) => (
            None,
            Some("This revision no longer lists the file.".to_string()),
        ),
        Some(Some(Err(error))) => (None, Some(format!("Reading the file failed: {error}"))),
        _ => (None, None),
    };
    let toggled = path.clone();
    let mut groups_open = review.groups;
    rsx! {
        li { class: "reef-app-file reef-app-ofile", "data-path": "{path}",
            div { class: "reef-app-ofile-head", title: "{status}",
                span { class: "reef-app-status s-{status}", "{letter}" }
                span { class: "reef-app-file-path", "{shown}" }
                span { class: "reef-app-file-counts",
                    span { class: "reef-app-plus", "+{summary.added}" }
                    " "
                    span { class: "reef-app-minus", "−{summary.removed}" }
                }
                if conflicted {
                    span { class: "reef-app-tag conflicted", "conflicted" }
                }
            }
            if held {
                p { class: "reef-app-muted reef-app-diff-note",
                    "Comments open once this draft is sealed."
                }
            }
            if let Some(outline) = summary.outline.clone() {
                OutlineView {
                    revision: revision(),
                    path: path.clone(),
                    outline,
                    added: summary.added,
                    removed: summary.removed,
                    open,
                    file: diff,
                    failed,
                    threads: on_file,
                    node,
                    on_toggle: move |key: String| {
                        // open the group, or close it if it is open
                        let mut all = groups_open.write();
                        let open = all.entry(toggled.clone()).or_default();
                        if !open.remove(&key) {
                            open.insert(key);
                        }
                    },
                }
            }
        }
    }
}

/// An outlined file's groups, each with its kind, name, tag and counts, and the lines of each
/// one open
///
/// Its groups come from the revision's listing, so they show before the file's hunks are read.
/// An open group's lines come from the hunks as read, attributed by the outline read with them,
/// since a draft rewritten meanwhile may have outlined them anew.
///
/// # Arguments
///
/// * `revision` - The revision the file is in
/// * `path` - The path the file is shown under
/// * `outline` - Its outline, as the revision lists it
/// * `added` - How many lines it added
/// * `removed` - How many lines it removed
/// * `open` - The keys of its groups open
/// * `file` - The file, its hunks and all, once read
/// * `failed` - Why it couldn't be read, if it couldn't
/// * `threads` - The threads whose line is in it in this revision
/// * `node` - The id of the node it is a change of, if its lines can be commented on
/// * `on_toggle` - Opens or closes a group, by its key
#[allow(clippy::too_many_arguments)]
#[component]
pub fn OutlineView(
    revision: u32,
    path: String,
    outline: Outline,
    added: u32,
    removed: u32,
    open: HashSet<String>,
    #[props(default)] file: Option<FileDiff>,
    #[props(default)] failed: Option<String>,
    #[props(default)] threads: Vec<Thread>,
    #[props(default)] node: Option<String>,
    #[props(default)] on_toggle: EventHandler<String>,
) -> Element {
    // the line a box is open under, one for the whole file
    let composing = use_signal(|| None::<Spot>);
    let by_line = threads_by_line(revision, threads);
    let focused = focused_spot(revision, &path);
    let commenting = node.is_some();
    // the groups as listed, the lines no item holds among them if a thread is on one
    let mut listed = groups(&outline, added, removed);
    let other_thread = by_line
        .keys()
        .any(|&(old, line)| attribute_one(&outline, side_of(old), line).is_none());
    if other_thread && !listed.iter().any(Group::is_other) {
        listed.push(other_group(0, 0));
    }
    // the hunks as read, with the outline read with them, each line's item and its groups
    let read = file.as_ref().and_then(|diff| {
        let outline = diff.outline.as_ref()?;
        let (added, removed) = diff.counts();
        let mut groups = groups(outline, added, removed);
        groups.push(other_group(0, 0));
        Some(Read {
            hunks: &diff.hunks,
            outline,
            held: attribute(&outline.items, &diff.hunks),
            groups,
        })
    });
    let drawn_lines = Lines {
        revision,
        path: &path,
        threads: &by_line,
        composing,
        focused,
        node: node.as_deref(),
    };
    rsx! {
        div {
            class: if commenting { "reef-app-diff reef-app-outline commenting" } else { "reef-app-diff reef-app-outline" },
            "data-revision": "{revision}",
            "data-path": "{path}",
            ul { class: "reef-app-groups",
                for group in listed.iter() {
                    {
                        let is_open = open.contains(&group.key);
                        let key = group.key.clone();
                        let kind = group.kind.map(|kind| kind.label()).unwrap_or_default();
                        let change = group.change.label();
                        rsx! {
                            li {
                                key: "{group.key}",
                                class: if is_open { "reef-app-group open" } else { "reef-app-group" },
                                "data-key": "{group.key}",
                                "data-change": "{change}",
                                button {
                                    class: "reef-app-group-head",
                                    "aria-expanded": "{is_open}",
                                    onclick: move |_| on_toggle.call(key.clone()),
                                    if !kind.is_empty() {
                                        span { class: "reef-app-kind", "{kind}" }
                                    }
                                    span { class: "reef-app-group-name", "{group.name}" }
                                    if !group.is_other() {
                                        span { class: "reef-app-tag {change}", "{change}" }
                                    }
                                    span { class: "reef-app-file-counts",
                                        span { class: "reef-app-plus", "+{group.added}" }
                                        " "
                                        span { class: "reef-app-minus", "−{group.removed}" }
                                    }
                                }
                                if is_open {
                                    {group_body(&group.key, read.as_ref(), failed.as_deref(), &drawn_lines)}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A file's hunks as read, with what draws an open group's lines from them
struct Read<'a> {
    /// The hunks
    hunks: &'a [Hunk],
    /// The outline read with them
    outline: &'a Outline,
    /// Which item each line of each hunk goes to
    held: Vec<Vec<Option<usize>>>,
    /// The outline's groups, the lines no item holds among them
    groups: Vec<Group>,
}

/// The group of the lines no item holds, with its counts
///
/// # Arguments
///
/// * `added` - How many of them were added
/// * `removed` - How many were removed
fn other_group(added: u32, removed: u32) -> Group {
    Group {
        key: OTHER_KEY.to_string(),
        kind: None,
        name: "Other lines".to_string(),
        change: reef_api::ItemChange::Changed,
        head: None,
        members: Vec::new(),
        added,
        removed,
    }
}

/// The side a thread's spot is numbered on
///
/// # Arguments
///
/// * `old` - Whether it is numbered in the base
fn side_of(old: bool) -> Side {
    if old { Side::Old } else { Side::New }
}

/// Where a line's threads are kept, if it is numbered on its side
///
/// # Arguments
///
/// * `line` - The line
fn line_spot(line: &Line) -> Option<Spot> {
    match line.kind {
        LineKind::Removed => line.old.map(|n| spot(Side::Old, n)),
        LineKind::Added | LineKind::Context => line.new.map(|n| spot(Side::New, n)),
    }
}

/// The key of the group an item is in, the lines no item holds for none
///
/// # Arguments
///
/// * `groups` - The outline's groups
/// * `item` - The item's index, if a line has one
fn key_of(groups: &[Group], item: Option<usize>) -> String {
    item.and_then(|index| {
        groups
            .iter()
            .find(|group| group.head == Some(index) || group.members.contains(&index))
    })
    .map_or_else(|| OTHER_KEY.to_string(), |group| group.key.clone())
}

/// The key of the group a line falls in by the spans the revision lists, before its hunks are
/// read
///
/// # Arguments
///
/// * `outline` - The file's outline, as listed
/// * `added` - How many lines the file added
/// * `removed` - How many it removed
/// * `anchor` - Where the line is
fn listed_key(outline: &Outline, added: u32, removed: u32, anchor: &Anchor) -> String {
    let item = attribute_one(outline, anchor.side, anchor.line);
    key_of(&groups(outline, added, removed), item)
}

/// The key of the group a line is drawn in, found among the file's hunks as read
///
/// # Arguments
///
/// * `diff` - The file, its hunks and its outline
/// * `anchor` - Where the line is
fn exact_key(diff: &FileDiff, anchor: &Anchor) -> String {
    let Some(outline) = diff.outline.as_ref() else {
        return OTHER_KEY.to_string();
    };
    let held = attribute(&outline.items, &diff.hunks);
    let wanted = spot(anchor.side, anchor.line);
    // the item of the hunks' line at the anchor, if it is one of theirs
    let item = diff
        .hunks
        .iter()
        .zip(&held)
        .flat_map(|(hunk, held)| hunk.lines.iter().zip(held))
        .find(|(line, _)| line_spot(line) == Some(wanted))
        .and_then(|(_, item)| *item);
    let (added, removed) = diff.counts();
    key_of(&groups(outline, added, removed), item)
}

/// How a run of a hunk's lines is headed, as git heads a hunk: where it starts on each side and
/// how many lines it has there
///
/// # Arguments
///
/// * `hunk` - The hunk the run is in
/// * `segment` - The run
fn segment_head(hunk: &Hunk, segment: &Segment) -> String {
    let lines = &hunk.lines[segment.start..segment.end];
    // a side's start and count: its first number, or after the hunk's last line before the run
    let side = |number: fn(&Line) -> Option<u32>, start: u32| -> (u32, usize) {
        let count = lines.iter().filter_map(number).count();
        let first = lines.iter().find_map(number).unwrap_or_else(|| {
            hunk.lines[..segment.start]
                .iter()
                .rev()
                .find_map(number)
                .unwrap_or(start)
        });
        (first, count)
    };
    let (old_start, old_count) = side(|line| line.old, hunk.old_start);
    let (new_start, new_count) = side(|line| line.new, hunk.new_start);
    format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@")
}

/// How a group's member is named in its head: a type's own lines as its definition, anything
/// else as itself
///
/// # Arguments
///
/// * `item` - The member
fn member_label(item: &Item) -> String {
    if item.kind.owns() {
        "definition".to_string()
    } else {
        item.label()
    }
}

/// An open group's body: each of its items with the runs of lines that are its own, headed by
/// its name, tag and counts when the group has more than its one item
///
/// # Arguments
///
/// * `key` - The group's key
/// * `read` - The file's hunks as read, once they are
/// * `failed` - Why they couldn't be read, if they couldn't
/// * `drawn` - What every line of the file needs
fn group_body(key: &str, read: Option<&Read>, failed: Option<&str>, drawn: &Lines) -> Element {
    if let Some(failed) = failed {
        return rsx! {
            p { class: "reef-app-error reef-app-diff-note", "{failed}" }
        };
    }
    let Some(read) = read else {
        return rsx! {
            p { class: "reef-app-muted reef-app-diff-wait", "Loading…" }
        };
    };
    // the group as the hunks read have it, which a draft rewritten since may lack
    let Some(group) = read.groups.iter().find(|group| group.key == key) else {
        return rsx! {
            p { class: "reef-app-muted reef-app-diff-note",
                "No longer in this file as it was read again."
            }
        };
    };
    // a line keeps its run if a thread is on it
    let keep = |line: &Line| line_spot(line).is_some_and(|at| drawn.threads.contains_key(&at));
    // the group's head then its members, each headed when there is more than the one
    let headed = !group.members.is_empty();
    let items = reef_api::outline::drawn(group);
    rsx! {
        div { class: "reef-app-group-body",
            for item in items {
                {
                    let runs = segments(&read.held, read.hunks, item, keep);
                    let sections: Vec<(String, &[Line])> = runs
                        .iter()
                        .map(|run| {
                            let hunk = &read.hunks[run.hunk];
                            (segment_head(hunk, run), &hunk.lines[run.start..run.end])
                        })
                        .collect();
                    let member = item.map(|index| &read.outline.items[index]);
                    let label = member.map(member_label).unwrap_or_else(|| "Other lines".to_string());
                    rsx! {
                        div { class: "reef-app-member", "data-item": "{label}",
                            if headed && let Some(member) = member {
                                div { class: "reef-app-member-head",
                                    span { class: "reef-app-member-name", "{label}" }
                                    if member.kind != reef_api::ItemKind::Impl && let Some(via) = member.via_label() {
                                        span { class: "reef-app-muted reef-app-via", "impl {via}" }
                                    }
                                    span { class: "reef-app-tag {member.change.label()}", "{member.change.label()}" }
                                    span { class: "reef-app-file-counts",
                                        span { class: "reef-app-plus", "+{member.added}" }
                                        " "
                                        span { class: "reef-app-minus", "−{member.removed}" }
                                    }
                                }
                            }
                            if sections.is_empty() {
                                p { class: "reef-app-muted reef-app-diff-note", "No lines changed here." }
                            } else {
                                {hunk_table(&sections, drawn)}
                            }
                        }
                    }
                }
            }
        }
    }
}
