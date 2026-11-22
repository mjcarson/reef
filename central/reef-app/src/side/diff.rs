//! One file of a node's revision: fetched when it is opened, and drawn as a table of its hunks
//!
//! A revision lists its files without their hunks, so a node with many changes opens at once,
//! and a file's hunks are read only once the file is opened. Each hunk is a header and a row a
//! line, numbered on both sides as git numbers them.

use crate::calls;
use dioxus::prelude::*;
use reef_api::{FileDiff, Line, LineKind};

/// An open file of a revision, fetched when it is opened and drawn once it comes
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `revision` - The revision's number
/// * `path` - The path the file is shown under
#[component]
pub fn FileView(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    id: ReadSignal<String>,
    revision: ReadSignal<u32>,
    path: ReadSignal<String>,
) -> Element {
    // the file's hunks, read when it opens, and again if it is opened in another revision
    let file = use_resource(move || async move {
        calls::revision_file(project(), plan(), id(), revision(), path()).await
    });
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
            Diff { revision: revision(), file: file.clone() }
        },
    }
}

/// A file's diff: a header row a hunk, then a row a line, each numbered on the sides it is on
///
/// A binary file and one too large to keep its hunks say so instead.
///
/// # Arguments
///
/// * `revision` - The revision the file is in
/// * `file` - The file, its hunks and all
#[component]
pub fn Diff(revision: u32, file: FileDiff) -> Element {
    // how many lines it draws, which a check waits for
    let lines: usize = file.hunks.iter().map(|hunk| hunk.lines.len()).sum();
    let path = file.path().to_string();
    let (added, removed) = file.counts();
    // a mode change is said above its lines
    let mode = match (&file.old_mode, &file.new_mode) {
        (Some(old), Some(new)) if old != new => Some(format!("Mode {old} → {new}")),
        _ => None,
    };
    rsx! {
        div {
            class: "reef-app-diff",
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
                table { class: "reef-app-hunks",
                    for hunk in file.hunks.iter() {
                        tbody { class: "reef-app-hunk",
                            tr { class: "reef-app-hunk-head",
                                td { colspan: "3",
                                    "@@ -{hunk.old_start},{hunk.old_lines} +{hunk.new_start},{hunk.new_lines} @@"
                                }
                            }
                            for line in hunk.lines.iter() {
                                {line_rows(line)}
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The rows of one line of a hunk: its number on each side it is on, its sign and its text,
/// and a note after it if the file ends with it and no newline
///
/// A plain function rather than a component, so a file of thousands of lines draws no scope a
/// line.
///
/// # Arguments
///
/// * `line` - The line
fn line_rows(line: &Line) -> Element {
    // the side it is on, and how it is marked there
    let (class, sign) = shown(line.kind);
    let old = line.old.map(|n| n.to_string()).unwrap_or_default();
    let new = line.new.map(|n| n.to_string()).unwrap_or_default();
    rsx! {
        tr { class: "reef-app-line {class}",
            td { class: "reef-app-ln", "{old}" }
            td { class: "reef-app-ln", "{new}" }
            td { class: "reef-app-code",
                span { class: "reef-app-sign", "{sign}" }
                "{line.text}"
            }
        }
        if line.no_newline {
            tr { class: "reef-app-eof",
                td { colspan: "3", "No newline at end of file" }
            }
        }
    }
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
