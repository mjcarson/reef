//! An outlined file draws as its groups: a struct with its definition and methods, a loose fn
//! removed, and the lines no item holds, each tagged and counted, each open group drawing only
//! its own lines, and each thread on its lines once
//!
//! The `OutlineView` component is rendered natively and parsed, so each check is of elements,
//! classes and text, not of how the renderer escapes or closes its tags.

use dioxus::prelude::*;
use reef_api::{
    Anchor, Author, Comment, FileDiff, FileStatus, Hunk, Item, ItemChange, ItemKind, Line,
    LineKind, Outline, Owner, Side, Span, Target, Thread, ThreadState,
};
use reef_app::side::outline::OutlineView;
use std::collections::HashSet;

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

/// An item of the outline below, owned by `Scaler` if a depth past the top
///
/// # Arguments
///
/// * `kind` - Its kind
/// * `name` - Its name
/// * `change` - What the revision did to it
/// * `depth` - How deep it is nested
/// * `old` - Its span in the base
/// * `new` - Its span in the worktree
/// * `counts` - How many lines it added and removed
fn item(
    kind: ItemKind,
    name: &str,
    change: ItemChange,
    depth: u32,
    old: Option<(u32, u32)>,
    new: Option<(u32, u32)>,
    counts: (u32, u32),
) -> Item {
    Item {
        kind,
        name: name.to_string(),
        owner: (depth > 0).then(|| Owner {
            name: "Scaler".to_string(),
            kind: Some(ItemKind::Struct),
        }),
        via: None,
        change,
        depth,
        old: old.map(|(start, end)| Span { start, end }),
        new: new.map(|(start, end)| Span { start, end }),
        added: counts.0,
        removed: counts.1,
    }
}

/// `src/scaler.rs` edited: an import added, a field added to `Scaler`, its `run` changed and
/// `stop` added, and the loose `gone` removed, all in one hunk, outlined as a capture would
///
/// ```text
/// + use std::io;
///   pub struct Scaler {
///       count: u32,
/// +     rate: u32,
///   }
///
///   impl Scaler {
///       fn run(&self) {
/// -         go_old();
/// +         go();
///       }
/// +
/// +     fn stop(&self) {}
///   }
/// -
/// - fn gone() {}
/// ```
fn scaler() -> FileDiff {
    use LineKind::{Added, Context, Removed};
    let lines = vec![
        line(Added, None, Some(1), "use std::io;"),
        line(Context, Some(1), Some(2), "pub struct Scaler {"),
        line(Context, Some(2), Some(3), "    count: u32,"),
        line(Added, None, Some(4), "    rate: u32,"),
        line(Context, Some(3), Some(5), "}"),
        line(Context, Some(4), Some(6), ""),
        line(Context, Some(5), Some(7), "impl Scaler {"),
        line(Context, Some(6), Some(8), "    fn run(&self) {"),
        line(Removed, Some(7), None, "        go_old();"),
        line(Added, None, Some(9), "        go();"),
        line(Context, Some(8), Some(10), "    }"),
        line(Added, None, Some(11), ""),
        line(Added, None, Some(12), "    fn stop(&self) {}"),
        line(Context, Some(9), Some(13), "}"),
        line(Removed, Some(10), None, ""),
        line(Removed, Some(11), None, "fn gone() {}"),
    ];
    let outline = Outline {
        language: "rust".to_string(),
        items: vec![
            item(
                ItemKind::Struct,
                "Scaler",
                ItemChange::Changed,
                0,
                Some((1, 3)),
                Some((2, 5)),
                (1, 0),
            ),
            item(
                ItemKind::Fn,
                "run",
                ItemChange::Changed,
                1,
                Some((6, 8)),
                Some((8, 10)),
                (1, 1),
            ),
            item(
                ItemKind::Fn,
                "stop",
                ItemChange::Added,
                1,
                None,
                Some((12, 12)),
                (2, 0),
            ),
            item(
                ItemKind::Fn,
                "gone",
                ItemChange::Removed,
                0,
                Some((11, 11)),
                None,
                (0, 2),
            ),
        ],
    };
    FileDiff {
        status: FileStatus::Modified,
        old_path: Some("src/scaler.rs".to_string()),
        new_path: Some("src/scaler.rs".to_string()),
        old_mode: Some("100644".to_string()),
        new_mode: Some("100644".to_string()),
        old_id: Some("1".repeat(40)),
        new_id: Some("2".repeat(40)),
        binary: false,
        too_large: false,
        hunks: vec![Hunk {
            old_start: 1,
            old_lines: 11,
            new_start: 1,
            new_lines: 13,
            lines,
        }],
        outline: Some(outline),
    }
}

/// A thread on a line of `src/scaler.rs` in revision 2
///
/// # Arguments
///
/// * `number` - The thread's number
/// * `side` - The side its line is numbered on
/// * `line` - Its line's number there
fn thread_on(number: u32, side: Side, line: u32) -> Thread {
    Thread {
        node: "reef/central/C101".parse().unwrap(),
        number,
        version: 1,
        target: Target::Line(Anchor {
            revision: 2,
            path: "src/scaler.rs".to_string(),
            side,
            line,
        }),
        followed: Vec::new(),
        outdated: None,
        state: ThreadState::Open,
        resolutions: Vec::new(),
        marks: Vec::new(),
        comments: vec![Comment {
            number,
            at: 0,
            author: Author::default(),
            body: "Why?".to_string(),
            html: "<p>Why?</p>".to_string(),
            review: None,
        }],
        pending: false,
    }
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

/// The elements under one whose class holds a name
///
/// # Arguments
///
/// * `node` - Where to look
/// * `name` - The class to look for
fn having<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Vec<roxmltree::Node<'a, 'a>> {
    node.descendants()
        .filter(|n| class(*n).split(' ').any(|c| c == name))
        .collect()
}

/// What the page below draws the outline with
#[derive(Props, Clone, PartialEq)]
struct Drawn {
    /// The keys of the groups open
    open: HashSet<String>,
    /// Whether the file's hunks have been read
    read: bool,
    /// The threads on its lines
    threads: Vec<Thread>,
}

/// The outline as a page draws it, inside the runtime its toggle's handler needs
///
/// # Arguments
///
/// * `props` - What to draw it with
#[allow(non_snake_case)]
fn Page(props: Drawn) -> Element {
    let file = scaler();
    let outline = file.outline.clone().unwrap();
    let file = props.read.then_some(file);
    rsx! {
        OutlineView {
            revision: 2,
            path: "src/scaler.rs".to_string(),
            outline,
            added: 5,
            removed: 3,
            open: props.open,
            file,
            threads: props.threads,
            node: Some("C101".to_string()),
        }
    }
}

/// The outline drawn with the groups given open, the file's hunks read if given, rendered
/// natively
///
/// # Arguments
///
/// * `open` - The keys of the groups open
/// * `read` - Whether the file's hunks have been read
/// * `threads` - The threads on its lines
fn native(open: &[&str], read: bool, threads: Vec<Thread>) -> String {
    let open: HashSet<String> = open.iter().map(|key| key.to_string()).collect();
    let mut dom = VirtualDom::new_with_props(
        Page,
        Drawn {
            open,
            read,
            threads,
        },
    );
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

/// Each group shows its kind, name, tag and counts, in the order the diff reaches it, with the
/// lines no item holds last, and a closed group draws no lines
#[test]
fn groups_are_listed_with_their_tags() {
    let html = native(&[], false, Vec::new());
    let doc = roxmltree::Document::parse(&html).expect("the outline is well formed");
    let root = doc.root_element();
    assert_eq!(class(root), "reef-app-diff reef-app-outline commenting");
    assert_eq!(root.attribute("data-path"), Some("src/scaler.rs"));
    let groups: Vec<(String, String, String)> = having(root, "reef-app-group")
        .into_iter()
        .map(|li| {
            let head = having(li, "reef-app-group-head")[0];
            (
                li.attribute("data-key").unwrap_or_default().to_string(),
                li.attribute("data-change").unwrap_or_default().to_string(),
                text(head),
            )
        })
        .collect();
    assert_eq!(
        groups,
        [
            (
                "owner:Scaler".to_string(),
                "changed".to_string(),
                "structScalerchanged+4 −1".to_string()
            ),
            (
                "item:fn:gone:0".to_string(),
                "removed".to_string(),
                "fngoneremoved+0 −2".to_string()
            ),
            (
                "other".to_string(),
                "changed".to_string(),
                "Other lines+1 −0".to_string()
            ),
        ]
    );
    // the tag is classed by its change, for its colour
    let tags: Vec<&str> = having(root, "reef-app-tag")
        .into_iter()
        .map(class)
        .collect();
    assert_eq!(tags, ["reef-app-tag changed", "reef-app-tag removed"]);
    // closed, nothing draws a line
    assert!(having(root, "reef-app-group-body").is_empty());
    assert!(root.descendants().all(|n| !n.has_tag_name("table")));
}

/// An open group draws each of its members with only its own lines, a loose fn its lines alone,
/// and the lines no item holds their own
#[test]
fn an_open_group_draws_its_members_lines() {
    let html = native(
        &["owner:Scaler", "item:fn:gone:0", "other"],
        true,
        Vec::new(),
    );
    let doc = roxmltree::Document::parse(&html).expect("the outline is well formed");
    let root = doc.root_element();
    // each drawn item by its label, with the lines it draws, by sign and text
    let members: Vec<(String, Option<String>, Vec<String>)> = having(root, "reef-app-member")
        .into_iter()
        .map(|member| {
            let head = having(member, "reef-app-member-head")
                .first()
                .map(|h| text(*h));
            let lines = having(member, "reef-app-line")
                .into_iter()
                .map(|tr| text(having(tr, "reef-app-code")[0]))
                .collect();
            (
                member
                    .attribute("data-item")
                    .unwrap_or_default()
                    .to_string(),
                head,
                lines,
            )
        })
        .collect();
    assert_eq!(
        members,
        [
            (
                "definition".to_string(),
                Some("definitionchanged+1 −0".to_string()),
                vec![
                    " pub struct Scaler {".to_string(),
                    "     count: u32,".to_string(),
                    "+    rate: u32,".to_string(),
                    " }".to_string(),
                ]
            ),
            (
                "fn run".to_string(),
                Some("fn runchanged+1 −1".to_string()),
                vec![
                    "     fn run(&self) {".to_string(),
                    "-        go_old();".to_string(),
                    "+        go();".to_string(),
                    "     }".to_string(),
                ]
            ),
            (
                "fn stop".to_string(),
                Some("fn stopnew+2 −0".to_string()),
                vec!["+".to_string(), "+    fn stop(&self) {}".to_string()]
            ),
            // a loose fn alone has no head of its own: its group's is its head
            (
                "fn gone".to_string(),
                None,
                vec!["-".to_string(), "-fn gone() {}".to_string()]
            ),
            (
                "Other lines".to_string(),
                None,
                vec!["+use std::io;".to_string()]
            ),
        ]
    );
    // each run is headed as git heads a hunk, where it starts on each side
    let heads: Vec<String> = having(root, "reef-app-hunk-head")
        .into_iter()
        .map(text)
        .collect();
    assert_eq!(
        heads,
        [
            "@@ -1,3 +2,4 @@",
            "@@ -6,3 +8,3 @@",
            "@@ -8,0 +11,2 @@",
            "@@ -10,2 +13,0 @@",
            "@@ -1,0 +1,1 @@",
        ]
    );
}

/// A thread on a line draws once, under its line in the item that holds it, and one on a line
/// no item holds keeps the other lines' run though nothing changed in it
#[test]
fn each_thread_draws_once_under_its_line() {
    // on `stop`'s line, and on the context line `impl Scaler {`, which no changed item holds
    let threads = vec![thread_on(7, Side::New, 12), thread_on(8, Side::New, 7)];
    let html = native(&["owner:Scaler", "item:fn:gone:0", "other"], true, threads);
    let doc = roxmltree::Document::parse(&html).expect("the outline is well formed");
    let root = doc.root_element();
    // two threads, each drawn once
    assert_eq!(having(root, "reef-app-thread-row").len(), 2);
    // the first under `stop`'s line
    let stop = having(root, "reef-app-member")
        .into_iter()
        .find(|member| member.attribute("data-item") == Some("fn stop"))
        .unwrap();
    assert_eq!(having(stop, "reef-app-thread-row").len(), 1);
    // the second in the other lines, beside the import
    let other = having(root, "reef-app-member")
        .into_iter()
        .find(|member| member.attribute("data-item") == Some("Other lines"))
        .unwrap();
    let rows: Vec<String> = having(other, "reef-app-line")
        .into_iter()
        .map(|tr| text(having(tr, "reef-app-code")[0]))
        .collect();
    // the run of lines no item holds that the thread is in, its blank line above included
    assert_eq!(rows, ["+use std::io;", " ", " impl Scaler {"]);
    assert_eq!(having(other, "reef-app-thread-row").len(), 1);
}

/// An open group whose file hasn't been read yet says so
#[test]
fn an_open_group_waits_for_its_lines() {
    let html = native(&["owner:Scaler"], false, Vec::new());
    let doc = roxmltree::Document::parse(&html).expect("the outline is well formed");
    let waiting: Vec<String> = having(doc.root_element(), "reef-app-diff-wait")
        .into_iter()
        .map(text)
        .collect();
    assert_eq!(waiting, ["Loading…"]);
}
