//! One file of a node's revision: fetched when it is opened, and drawn as a table of its hunks
//!
//! A revision lists its files without their hunks, so a node with many changes opens at once,
//! and a file's hunks are read only once the file is opened. Each hunk is a header and a row a
//! line, numbered on both sides as git numbers them. Each thread whose line is in the revision
//! is drawn under that line, where it was followed to, and a click on a line's numbers opens a
//! box under it to start one.

use super::Review;
use super::threads::{Compose, Composer, ThreadView, Threads};
use crate::calls;
use crate::canvas::dom;
use dioxus::prelude::*;
use reef_api::{Anchor, FileDiff, Line, LineKind, Side, Target, Thread};
use std::collections::HashMap;

/// A line of a file as threads are kept by: whether it is numbered in the base, and its number
pub(super) type Spot = (bool, u32);

/// An open file of a revision, fetched when it is opened and drawn once it comes, with the
/// threads on its lines
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `revision` - The revision's number
/// * `stamp` - When the revision was stored, which a draft's rewrite moves on
/// * `draft` - Whether the revision is the node's draft, whose lines take no comment
/// * `path` - The path the file is shown under
#[component]
pub fn FileView(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    id: ReadSignal<String>,
    revision: ReadSignal<u32>,
    stamp: ReadSignal<u64>,
    draft: ReadSignal<bool>,
    path: ReadSignal<String>,
) -> Element {
    // the file's hunks, read when it opens, again if it is opened in another revision, and
    // again each time its draft is rewritten
    let file = use_resource(move || async move {
        let _ = stamp();
        calls::revision_file(project(), plan(), id(), revision(), path()).await
    });
    // the threads whose line is in this file in this revision, if the plan's are read
    let threads = try_use_context::<Threads>();
    let on_file: Vec<Thread> = threads
        .map(|threads| threads.of(&id.read()))
        .unwrap_or_default()
        .into_iter()
        .filter(|thread| {
            thread
                .anchor_in(revision())
                .is_some_and(|anchor| anchor.path == *path.read())
        })
        .collect();
    // a line is commented on only where the plan's threads are kept, and never in a draft,
    // whose lines change under it
    let node = threads.filter(|_| !draft()).map(|_| id());
    let held = threads.is_some() && draft();
    match &*file.read() {
        None => rsx! {
            p { class: "reef-app-muted reef-app-diff-wait", "Loading…" }
        },
        Some(Err(error)) => rsx! {
            p { class: "reef-app-error", "Reading the file failed: {error}" }
        },
        Some(Ok(None)) => rsx! {
            p { class: "reef-app-muted", "This revision no longer lists the file." }
        },
        Some(Ok(Some(file))) => rsx! {
            if held {
                p { class: "reef-app-muted reef-app-diff-note",
                    "Comments open once this draft is sealed."
                }
            }
            Diff {
                revision: revision(),
                file: file.clone(),
                threads: on_file,
                node,
            }
        },
    }
}

/// A file's diff: a header row a hunk, then a row a line, each numbered on the sides it is on,
/// with the threads on each line under it
///
/// A binary file and one too large to keep its hunks say so instead. Given its node, a click on
/// a line's numbers opens a box under the line to start a thread on it.
///
/// # Arguments
///
/// * `revision` - The revision the file is in
/// * `file` - The file, its hunks and all
/// * `threads` - The threads whose line is in it in this revision
/// * `node` - The id of the node it is a change of, if its lines can be commented on
#[component]
pub fn Diff(
    revision: u32,
    file: FileDiff,
    #[props(default)] threads: Vec<Thread>,
    #[props(default)] node: Option<String>,
) -> Element {
    // the line a box is open under, and the line to bring into view, if any is this file's
    let composing = use_signal(|| None::<Spot>);
    // how many lines it draws, which a check waits for
    let lines: usize = file.hunks.iter().map(|hunk| hunk.lines.len()).sum();
    let path = file.path().to_string();
    let (added, removed) = file.counts();
    // a mode change is said above its lines
    let mode = match (&file.old_mode, &file.new_mode) {
        (Some(old), Some(new)) if old != new => Some(format!("Mode {old} → {new}")),
        _ => None,
    };
    // each line's threads, by where their line is in this revision
    let by_line = threads_by_line(revision, threads);
    let focused = focused_spot(revision, &path);
    let commenting = node.is_some();
    let drawn = Lines {
        revision,
        path: &path,
        threads: &by_line,
        composing,
        focused,
        node: node.as_deref(),
    };
    // a section a hunk, headed as git heads it
    let sections: Vec<(String, &[Line])> = file
        .hunks
        .iter()
        .map(|hunk| {
            let head = format!(
                "@@ -{},{} +{},{} @@",
                hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
            );
            (head, hunk.lines.as_slice())
        })
        .collect();
    rsx! {
        div {
            class: if commenting { "reef-app-diff commenting" } else { "reef-app-diff" },
            "data-revision": "{revision}",
            "data-path": "{path}",
            "data-lines": "{lines}",
            if let Some(mode) = mode {
                p { class: "reef-app-muted reef-app-diff-note", "{mode}" }
            }
            if file.binary {
                p { class: "reef-app-muted reef-app-diff-note", "Binary file, not shown." }
            } else if file.too_large {
                p { class: "reef-app-muted reef-app-diff-note",
                    "Too large to keep its lines: +{added} −{removed}."
                }
            } else if file.hunks.is_empty() {
                p { class: "reef-app-muted reef-app-diff-note", "No lines changed." }
            } else {
                {hunk_table(&sections, &drawn)}
            }
        }
    }
}

/// Each line's threads in a revision, by where their line is in it
///
/// # Arguments
///
/// * `revision` - The revision drawn
/// * `threads` - The threads whose line is in its file
pub(super) fn threads_by_line(revision: u32, threads: Vec<Thread>) -> HashMap<Spot, Vec<Thread>> {
    let mut by_line: HashMap<Spot, Vec<Thread>> = HashMap::new();
    for thread in threads {
        // a thread is drawn where its line is in this revision, if it is in it
        if let Some(anchor) = thread.anchor_in(revision) {
            by_line
                .entry(spot(anchor.side, anchor.line))
                .or_default()
                .push(thread);
        }
    }
    by_line
}

/// The line of a file to bring into view, if the Changes tab was asked for one of its lines
///
/// # Arguments
///
/// * `revision` - The revision drawn
/// * `path` - The path its file is shown under
pub(super) fn focused_spot(revision: u32, path: &str) -> Option<Spot> {
    try_use_context::<Review>()
        .and_then(|review| review.focus.read().clone())
        .filter(|anchor| anchor.revision == revision && anchor.path == path)
        .map(|anchor| spot(anchor.side, anchor.line))
}

/// A table of a file's lines, a section a run of them: a head row with the section's head,
/// then each line's rows
///
/// One handler serves every line: given the file's node, a click on a line's numbers opens a
/// box under it.
///
/// # Arguments
///
/// * `sections` - Each section's head and its lines
/// * `drawn` - What every line of the file needs
pub(super) fn hunk_table(sections: &[(String, &[Line])], drawn: &Lines) -> Element {
    let commenting = drawn.node.is_some();
    let mut composing = drawn.composing;
    rsx! {
        table {
            class: "reef-app-hunks",
            // one handler for every line: a click on a line's numbers opens its box
            onclick: move |event| {
                if !commenting {
                    return;
                }
                let Some(row) = dom::target(&*event.data())
                    .and_then(|target| dom::closest(&target, "td.reef-app-ln"))
                    .and_then(|cell| dom::closest(&cell, "tr.reef-app-line"))
                else {
                    return;
                };
                let old = dom::attr(&row, "data-side").as_deref() == Some("old");
                if let Some(line) = dom::attr(&row, "data-line").and_then(|n| n.parse().ok()) {
                    composing.set(Some((old, line)));
                }
            },
            for (head, lines) in sections.iter() {
                tbody { class: "reef-app-hunk",
                    tr { class: "reef-app-hunk-head",
                        td { colspan: "3", "{head}" }
                    }
                    for line in lines.iter() {
                        {line_rows(line, drawn)}
                    }
                }
            }
        }
    }
}

/// What each line of a drawn file needs: where it is, the threads by line, and the line a box
/// is open under or to bring into view
pub(super) struct Lines<'a> {
    /// The revision the file is in
    pub revision: u32,
    /// The path the file is shown under
    pub path: &'a str,
    /// Each line's threads
    pub threads: &'a HashMap<Spot, Vec<Thread>>,
    /// The line a box is open under, if any
    pub composing: Signal<Option<Spot>>,
    /// The line to bring into view, if it is this file's
    pub focused: Option<Spot>,
    /// The node the file is a change of, if its lines can be commented on
    pub node: Option<&'a str>,
}

/// The rows of one line of a hunk: its number on each side it is on, its sign and its text, a
/// note after it if the file ends with it and no newline, then its threads and any box open
/// under it
///
/// A plain function rather than a component, so a file of thousands of lines draws no scope a
/// line.
///
/// # Arguments
///
/// * `line` - The line
/// * `drawn` - What every line of the file needs
fn line_rows(line: &Line, drawn: &Lines) -> Element {
    // the side it is on, and how it is marked there
    let (class, sign) = shown(line.kind);
    let old = line.old.map(|n| n.to_string()).unwrap_or_default();
    let new = line.new.map(|n| n.to_string()).unwrap_or_default();
    // where a comment on it is anchored
    let anchor = Anchor::of(drawn.revision, drawn.path, line);
    let at = anchor.as_ref().map(|anchor| spot(anchor.side, anchor.line));
    let (side, number) = match &anchor {
        Some(anchor) if anchor.side == Side::Old => ("old", anchor.line),
        Some(anchor) => ("new", anchor.line),
        None => ("", 0),
    };
    let threads = at.and_then(|at| drawn.threads.get(&at));
    let focused = at.is_some() && at == drawn.focused;
    let composing = at.is_some() && at == *drawn.composing.read();
    let mut box_open = drawn.composing;
    let node = drawn.node.map(str::to_string);
    let cells = rsx! {
        td { class: "reef-app-ln", "{old}" }
        td { class: "reef-app-ln", "{new}" }
        td { class: "reef-app-code",
            span { class: "reef-app-sign", "{sign}" }
            "{line.text}"
        }
    };
    rsx! {
        // only the line asked for listens for its mounting, to be brought into view
        if focused {
            tr {
                class: "reef-app-line {class} focus",
                "data-side": "{side}",
                "data-line": "{number}",
                onmounted: move |event| {
                    if let Some(el) = dom::mounted(&event.data()) {
                        dom::scroll_into_view(&el);
                    }
                },
                {cells}
            }
        } else {
            tr {
                class: "reef-app-line {class}",
                "data-side": "{side}",
                "data-line": "{number}",
                {cells}
            }
        }
        if line.no_newline {
            tr { class: "reef-app-eof",
                td { colspan: "3", "No newline at end of file" }
            }
        }
        if let Some(threads) = threads {
            for thread in threads.iter() {
                tr { key: "{thread.number}", class: "reef-app-thread-row",
                    td { colspan: "3",
                        ThreadView { thread: thread.clone() }
                    }
                }
            }
        }
        if composing && let (Some(id), Some(anchor)) = (node, anchor) {
            tr { class: "reef-app-compose-row",
                td { colspan: "3",
                    Composer {
                        compose: Compose::Thread {
                            id,
                            target: Target::Line(anchor),
                        },
                        label: "Comment",
                        on_close: move |_| box_open.set(None),
                    }
                }
            }
        }
    }
}

/// Where a line is, as threads are kept by: whether it is numbered in the base, and its number
///
/// # Arguments
///
/// * `side` - The side its number counts on
/// * `line` - Its number there
pub(super) fn spot(side: Side, line: u32) -> Spot {
    (side == Side::Old, line)
}

/// The class a line of a kind is drawn with, and the sign git marks it with
///
/// # Arguments
///
/// * `kind` - The line's kind
pub fn shown(kind: LineKind) -> (&'static str, &'static str) {
    match kind {
        LineKind::Added => ("added", "+"),
        LineKind::Removed => ("removed", "-"),
        LineKind::Context => ("context", " "),
    }
}
