//! A file's diff draws as git shows it: a header a hunk, a row a line numbered on the sides it
//! is on, and a note where git says one
//!
//! The `Diff` component is rendered natively and parsed, so each check is of elements, classes
//! and text, not of how the renderer escapes or closes its tags.

use dioxus::prelude::*;
use reef_api::{
    Anchor, Comment, FileDiff, FileStatus, Hunk, Line, LineKind, Resolution, ResolvingCommit, Side,
    Target, Thread, ThreadState,
};
use reef_app::side::diff::Diff;
use reef_app::side::threads::{ThreadView, excerpt, place_label, resolution_label};

/// A line of a kind, numbered on the sides that kind is on
///
/// # Arguments
///
/// * `kind` - Its kind
/// * `old` - Its number in the base
/// * `new` - Its number in the worktree
/// * `text` - What it holds
fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> Line {
    Line {
        kind,
        old,
        new,
        text: text.to_string(),
        no_newline: false,
    }
}

/// A file renamed and edited: two hunks, a line removed and two added, the last with no newline
fn edited() -> FileDiff {
    // its last line ends the file with no newline
    let mut last = line(LineKind::Added, None, Some(21), "    x < y && \"done\"");
    last.no_newline = true;
    FileDiff {
        status: FileStatus::Renamed,
        old_path: Some("src/old.rs".to_string()),
        new_path: Some("src/new.rs".to_string()),
        old_mode: Some("100644".to_string()),
        new_mode: Some("100755".to_string()),
        old_id: Some("1".repeat(40)),
        new_id: Some("2".repeat(40)),
        binary: false,
        too_large: false,
        hunks: vec![
            Hunk {
                old_start: 1,
                old_lines: 2,
                new_start: 1,
                new_lines: 2,
                lines: vec![
                    line(LineKind::Context, Some(1), Some(1), "fn main() {"),
                    line(LineKind::Removed, Some(2), None, "    old();"),
                    line(LineKind::Added, None, Some(2), "    new();"),
                ],
            },
            Hunk {
                old_start: 20,
                old_lines: 1,
                new_start: 20,
                new_lines: 2,
                lines: vec![line(LineKind::Context, Some(20), Some(20), ""), last],
            },
        ],
    }
}

/// The Dioxus diff of a file, rendered natively
///
/// # Arguments
///
/// * `file` - The file to draw
fn native(file: FileDiff) -> String {
    dioxus_ssr::render_element(rsx! {
        Diff { revision: 3, file }
    })
}

/// An element's class, or nothing
///
/// # Arguments
///
/// * `node` - The element
fn class<'a>(node: roxmltree::Node<'a, 'a>) -> &'a str {
    node.attribute("class").unwrap_or_default()
}

/// The text an element holds, its children's included
///
/// # Arguments
///
/// * `node` - The element
fn text(node: roxmltree::Node) -> String {
    node.descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect()
}

/// Each hunk heads its lines, each line is numbered on its sides with its sign and its text,
/// and a file ending with no newline says so after its last line
#[test]
fn a_diff_draws_its_hunks_and_lines() {
    // the drawing, parsed
    let html = native(edited());
    let doc = roxmltree::Document::parse(&html).expect("the diff is well formed");
    let root = doc.root_element();
    assert_eq!(class(root), "reef-app-diff");
    assert_eq!(root.attribute("data-revision"), Some("3"));
    assert_eq!(root.attribute("data-path"), Some("src/new.rs"));
    assert_eq!(root.attribute("data-lines"), Some("5"));
    // the mode change is said above the lines
    let notes: Vec<String> = root
        .descendants()
        .filter(|n| class(*n).contains("reef-app-diff-note"))
        .map(text)
        .collect();
    assert_eq!(notes, ["Mode 100644 → 100755"]);
    // a header a hunk, as git writes it
    let heads: Vec<String> = root
        .descendants()
        .filter(|n| class(*n) == "reef-app-hunk-head")
        .map(text)
        .collect();
    assert_eq!(heads, ["@@ -1,2 +1,2 @@", "@@ -20,1 +20,2 @@"]);
    // a row a line: its class, its numbers on each side, its sign and its text, escaped as it
    // was written
    let rows: Vec<(String, Vec<String>)> = root
        .descendants()
        .filter(|n| n.has_tag_name("tr") && class(*n).starts_with("reef-app-line"))
        .map(|tr| {
            let cells = tr.children().filter(|n| n.is_element()).map(text).collect();
            (class(tr).to_string(), cells)
        })
        .collect();
    let row = |class: &str, old: &str, new: &str, code: &str| {
        (
            format!("reef-app-line {class}"),
            vec![old.to_string(), new.to_string(), code.to_string()],
        )
    };
    assert_eq!(
        rows,
        [
            row("context", "1", "1", " fn main() {"),
            row("removed", "2", "", "-    old();"),
            row("added", "", "2", "+    new();"),
            row("context", "20", "20", " "),
            row("added", "", "21", "+    x < y && \"done\""),
        ]
    );
    // and the last line is followed by git's note that the file ends without a newline
    let after_last = root
        .descendants()
        .filter(|n| n.has_tag_name("tr"))
        .last()
        .expect("the diff has rows");
    assert_eq!(class(after_last), "reef-app-eof");
    assert_eq!(text(after_last), "No newline at end of file");
}

/// A binary file, one too large to keep its lines, and one with no lines say so instead of
/// drawing hunks
#[test]
fn files_without_lines_say_why() {
    // a binary file, one too large, and an empty one added
    let mut binary = edited();
    binary.binary = true;
    binary.old_mode = binary.new_mode.clone();
    binary.hunks.clear();
    let mut large = edited();
    large.too_large = true;
    large.old_mode = large.new_mode.clone();
    large.hunks.clear();
    let mut empty = edited();
    empty.status = FileStatus::Added;
    empty.old_mode = empty.new_mode.clone();
    empty.hunks.clear();
    for (file, said) in [
        (binary, "Binary file, not shown."),
        (large, "Too large to keep its lines: +0 −0."),
        (empty, "No lines changed."),
    ] {
        let html = native(file);
        let doc = roxmltree::Document::parse(&html).expect("the diff is well formed");
        let root = doc.root_element();
        // no table of lines, and the note in its place
        assert!(
            !root.descendants().any(|n| n.has_tag_name("table")),
            "{html}"
        );
        assert_eq!(root.attribute("data-lines"), Some("0"));
        let notes: Vec<String> = root
            .descendants()
            .filter(|n| class(*n).contains("reef-app-diff-note"))
            .map(text)
            .collect();
        assert_eq!(notes, [said]);
    }
}

/// A thread on a line of the file's revision, with its first comment's body
///
/// # Arguments
///
/// * `number` - The thread's number
/// * `side` - The side its line is numbered on
/// * `line` - Its line's number there
/// * `body` - Its first comment's body
fn thread_on(number: u32, side: Side, line: u32, body: &str) -> Thread {
    Thread {
        node: "reef/central/C35".parse().unwrap(),
        number,
        version: 1,
        target: Target::Line(Anchor {
            revision: 3,
            path: "src/new.rs".to_string(),
            side,
            line,
        }),
        followed: Vec::new(),
        outdated: None,
        state: ThreadState::Open,
        resolutions: Vec::new(),
        comments: vec![Comment {
            number,
            at: 0,
            by: reef_api::Writer::User,
            body: body.to_string(),
            html: format!("<p>{body}</p>"),
        }],
    }
}

/// A thread started on revision 2 and followed to a line of revision 3, or one lost there
///
/// # Arguments
///
/// * `number` - The thread's number
/// * `to` - The line of revision 3 its line was followed to, or `None` if revision 3 lost it
fn followed_from_2(number: u32, to: Option<u32>) -> Thread {
    // started on line 7 of revision 2
    let mut thread = thread_on(number, Side::New, 7, "Moved?");
    if let Target::Line(anchor) = &mut thread.target {
        anchor.revision = 2;
    }
    match to {
        Some(line) => thread.followed.push(Anchor {
            revision: 3,
            path: "src/new.rs".to_string(),
            side: Side::New,
            line,
        }),
        None => thread.outdated = Some(3),
    }
    thread
}

/// A thread on a line draws under that line, numbered on its side, one followed from an
/// earlier revision draws where its line went, and a line names where a comment on it is
/// anchored
#[test]
fn threads_show_under_their_lines() {
    // one thread on the removed line, numbered in the base, one on an added line, one followed
    // to line 20, and one revision 3 lost
    let threads = vec![
        thread_on(4, Side::Old, 2, "Why gone?"),
        thread_on(5, Side::New, 21, "Done?"),
        followed_from_2(6, Some(20)),
        followed_from_2(7, None),
    ];
    let html = dioxus_ssr::render_element(rsx! {
        Diff { revision: 3, file: edited(), threads, node: Some("C35".to_string()) }
    });
    let doc = roxmltree::Document::parse(&html).expect("the diff is well formed");
    let root = doc.root_element();
    assert_eq!(class(root), "reef-app-diff commenting");
    // each row in order: a line by where it is anchored, or a thread by its number
    let rows: Vec<String> = root
        .descendants()
        .filter(|n| n.has_tag_name("tr") && !class(*n).starts_with("reef-app-hunk-head"))
        .map(
            |tr| match (tr.attribute("data-side"), tr.attribute("data-line")) {
                (Some(side), Some(line)) => format!("{side}:{line}"),
                _ => match tr.descendants().find_map(|n| n.attribute("data-thread")) {
                    Some(thread) => format!("thread {thread}"),
                    None => class(tr).to_string(),
                },
            },
        )
        .collect();
    assert_eq!(
        rows,
        [
            "new:1",
            "old:2",
            "thread 4",
            "new:2",
            "new:20",
            "thread 6",
            "new:21",
            "reef-app-eof",
            "thread 5",
        ]
    );
    // and a thread's comment is drawn as the service rendered it
    let body = root
        .descendants()
        .find(|n| class(*n) == "reef-app-comment-body")
        .map(text);
    assert_eq!(body.as_deref(), Some("Why gone?"));
}

/// A list names a thread by where its line is now, and by its first line cut short
#[test]
fn threads_are_named_by_where_and_what() {
    // on a removed line, on an added one, and on the node as a whole
    let old = thread_on(4, Side::Old, 2, "Why gone?");
    assert_eq!(place_label(&old), "r3 · src/new.rs:2 (removed)");
    assert_eq!(
        place_label(&thread_on(5, Side::New, 21, "x")),
        "r3 · src/new.rs:21"
    );
    let general = Thread {
        target: Target::Node,
        ..old.clone()
    };
    assert_eq!(place_label(&general), "general");
    // one followed is named where it went, and one lost where it was last, and when it was lost
    assert_eq!(
        place_label(&followed_from_2(6, Some(20))),
        "r3 · src/new.rs:20"
    );
    assert_eq!(
        place_label(&followed_from_2(7, None)),
        "r2 · src/new.rs:7 · outdated in r3"
    );
    // the first line with anything on it, cut at 80 characters
    let long = thread_on(6, Side::New, 1, &format!("\n  {}\nmore", "é".repeat(90)));
    assert_eq!(excerpt(&long), format!("{}…", "é".repeat(80)));
    assert_eq!(excerpt(&old), "Why gone?");
}

/// An outdated thread says so before its state, and one still on its line doesn't
#[test]
fn outdated_threads_say_so() {
    // a thread revision 3 lost, and one it kept
    for (thread, outdated) in [
        (followed_from_2(7, None), true),
        (followed_from_2(6, Some(20)), false),
    ] {
        let html = dioxus_ssr::render_element(rsx! {
            ThreadView { thread }
        });
        let doc = roxmltree::Document::parse(&html).expect("the thread is well formed");
        let root = doc.root_element();
        assert_eq!(class(root).split(' ').any(|c| c == "outdated"), outdated);
        let tag: Vec<String> = root
            .descendants()
            .filter(|n| class(*n).contains("reef-app-outdated"))
            .map(text)
            .collect();
        assert_eq!(tag.len(), usize::from(outdated), "{html}");
        if outdated {
            assert_eq!(tag, ["outdated"]);
        }
    }
}

/// A thread resolved by a change names it under its foot: its revision, another node's by that
/// node's id, and the commit whose trailer named it; one reopened since says what it was
/// reopened after, and one no change answered says nothing
#[test]
fn resolved_threads_name_the_change_that_answered_them() {
    // resolved by its own node's revision 4, through a commit's trailer
    let commit = ResolvingCommit {
        commit: "0123456789abcdef".repeat(2) + "01234567",
        summary: "Take the lock first".to_string(),
        author: "dev@example.invalid".to_string(),
        authored: 100,
    };
    let mut thread = thread_on(8, Side::New, 3, "Lock?");
    thread.state = ThreadState::Resolved;
    thread.resolutions.push(Resolution {
        node: thread.node.clone(),
        revision: 4,
        at: 1,
        commit: Some(commit.clone()),
    });
    assert_eq!(
        resolution_label(&thread).as_deref(),
        Some("answered by r4 · 01234567 Take the lock first")
    );
    // drawn under the foot, its full commit on hover
    let html = dioxus_ssr::render_element(rsx! {
        ThreadView { thread: thread.clone() }
    });
    let doc = roxmltree::Document::parse(&html).expect("the thread is well formed");
    let line: Vec<_> = doc
        .root_element()
        .descendants()
        .filter(|n| class(*n).contains("reef-app-thread-resolution"))
        .collect();
    assert_eq!(line.len(), 1, "{html}");
    assert_eq!(
        text(line[0]),
        "answered by r4 · 01234567 Take the lock first"
    );
    assert_eq!(line[0].attribute("title"), Some(commit.commit.as_str()));
    // another node's revision linked by hand names that node, and a reopen says so
    thread.resolutions.push(Resolution {
        node: "reef/central/C38".parse().unwrap(),
        revision: 1,
        at: 2,
        commit: None,
    });
    assert_eq!(
        resolution_label(&thread).as_deref(),
        Some("answered by C38 r1")
    );
    thread.state = ThreadState::Open;
    assert_eq!(
        resolution_label(&thread).as_deref(),
        Some("reopened after C38 r1")
    );
    // and a thread no change answered shows no line
    let plain = thread_on(9, Side::New, 3, "Lock?");
    assert_eq!(resolution_label(&plain), None);
    let html = dioxus_ssr::render_element(rsx! {
        ThreadView { thread: plain }
    });
    assert!(!html.contains("reef-app-thread-resolution"), "{html}");
}
