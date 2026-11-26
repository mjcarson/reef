//! The Timeline tab: a node's edits, starts, captures and comments merged newest first, and
//! drawn on the server as a line each

use dioxus::prelude::*;
use reef_api::{
    Author, CaptureShas, Comment, Edited, FileStatus, FileSummary, HistoryPage, Logged, Resolution,
    Revision, Target, Thread, ThreadMark, ThreadState, WorkRecord, WorkStart, Writer,
};
use reef_app::side::timeline::{Happened, TimelineList, merge};
use reef_model::{NodeName, Status};
use std::path::PathBuf;

/// The node every moment happened to
fn node() -> NodeName {
    "reef/central/C1".parse().expect("the name is good")
}

/// The author of a moment by a user, or by Claude for them
///
/// # Arguments
///
/// * `name` - The user's name
/// * `by` - Whether the user or Claude did it
fn by(name: &str, by: Writer) -> Author {
    Author {
        user: Some(name.to_string()),
        since: Some(1),
        by,
    }
}

/// An edit of the node's status, at a time
///
/// # Arguments
///
/// * `at` - When it was made
/// * `version` - The version it left
fn edit(at: u64, version: u64) -> Logged {
    Logged {
        at,
        author: by("ada", Writer::User),
        version: Some(version),
        whole_plan: false,
        edits: vec![Edited::Status {
            from: None,
            to: Some(Status::InProgress),
        }],
    }
}

/// A start of the node's work, as its history keeps it
///
/// # Arguments
///
/// * `at` - When it was kept, and when its record says it started
fn started(at: u64) -> Logged {
    Logged {
        at,
        author: by("bob", Writer::User),
        version: None,
        whole_plan: false,
        edits: vec![Edited::Started {
            started_at: at,
            branch: Some("reef/C1".to_string()),
            start: "a".repeat(40),
            again: false,
        }],
    }
}

/// The node's work record, started at a time
///
/// # Arguments
///
/// * `started_at` - When it says it started
fn work(started_at: u64) -> WorkRecord {
    WorkRecord {
        node: node(),
        version: 1,
        started_at,
        start: WorkStart {
            worktree: PathBuf::from("/work/reef"),
            branch: Some("main".to_string()),
            base_branch: None,
            start: "c".repeat(40),
            start_tree: "d".repeat(40),
            fork: None,
            pin: "refs/reef/start/central/C1".to_string(),
            pinned: "e".repeat(40),
        },
        started_by: by("carol", Writer::User),
        captures: Vec::new(),
    }
}

/// A revision of the node, captured at a time, of one file
///
/// # Arguments
///
/// * `revision` - Its number
/// * `at` - When it was kept
/// * `draft` - Whether it is the node's draft
fn revision(revision: u32, at: u64, draft: bool) -> Revision {
    Revision {
        node: node(),
        revision,
        at,
        shas: CaptureShas {
            head: "1".repeat(40),
            tree: "2".repeat(40),
            merge_base: None,
            base: "3".repeat(40),
        },
        conflicts: Vec::new(),
        files: vec![FileSummary {
            status: FileStatus::Added,
            old_path: None,
            new_path: Some("a.txt".to_string()),
            old_mode: None,
            new_mode: None,
            old_id: None,
            new_id: None,
            binary: false,
            too_large: false,
            added: 3,
            removed: 1,
        }],
        commits: Vec::new(),
        draft,
        author: by("bob", Writer::Claude),
    }
}

/// A comment, at a time, pending in a review or published
///
/// # Arguments
///
/// * `number` - Its number in the plan
/// * `at` - When it was written
/// * `body` - What it says
/// * `review` - The review it is pending in, if it is
fn comment(number: u32, at: u64, body: &str, review: Option<u32>) -> Comment {
    Comment {
        number,
        at,
        author: by("ada", Writer::User),
        review,
        body: body.to_string(),
        html: String::new(),
    }
}

/// A thread on the node, with its comments, marks and resolutions
///
/// # Arguments
///
/// * `number` - Its number: its first comment's
/// * `comments` - Its comments, the first first
/// * `pending` - Whether its first comment is pending in a review
fn thread(number: u32, comments: Vec<Comment>, pending: bool) -> Thread {
    Thread {
        node: node(),
        number,
        version: 1,
        target: Target::Node,
        followed: Vec::new(),
        outdated: None,
        state: ThreadState::Open,
        resolutions: Vec::new(),
        marks: Vec::new(),
        pending,
        comments,
    }
}

/// A timeline whose captures and comments open something, as the tab's do, its handlers made
/// inside the runtime that draws it
///
/// # Arguments
///
/// * `moments` - What happened, newest first
#[component]
fn Opening(moments: Vec<reef_app::side::timeline::Moment>) -> Element {
    rsx! {
        TimelineList { moments, on_capture: move |_| {}, on_comment: move |_| {} }
    }
}

/// What each moment is and says, in the order given
///
/// # Arguments
///
/// * `moments` - The moments
fn told(moments: &[reef_app::side::timeline::Moment]) -> Vec<(u64, &'static str, String)> {
    moments
        .iter()
        .map(|moment| (moment.at, moment.kind(), moment.describe()))
        .collect()
}

/// The timeline merges a node's edits, starts, captures and comments newest first, leaves out
/// what is pending in a review, adds a start only its work record holds, and while older edits
/// are kept, cuts what is older than the oldest edit read
#[test]
fn the_timeline_merges_newest_first() {
    // an edit at 10, a start at 20, a revision at 30 and a draft at 50
    let page = HistoryPage {
        edits: vec![started(20), edit(10, 2)],
        older: None,
    };
    let revisions = vec![revision(2, 50, true), revision(1, 30, false)];
    // a thread with a comment at 40, a reply at 60, one still pending, a mark and a resolution,
    // and a thread pending whole
    let mut first = thread(
        1,
        vec![
            comment(1, 40, "\n  Why *this*?  \nMore below.", None),
            comment(3, 60, "Because.", None),
            comment(4, 65, "Not yet sent.", Some(1)),
        ],
        false,
    );
    first.marks = vec![ThreadMark {
        state: ThreadState::Resolved,
        at: 70,
        author: by("ada", Writer::User),
    }];
    first.resolutions = vec![Resolution {
        node: "reef/central/C2".parse().unwrap(),
        revision: 4,
        at: 80,
        commit: None,
        author: by("bob", Writer::User),
    }];
    let pending = thread(2, vec![comment(2, 45, "Held back", Some(1))], true);
    // the record names the start the history kept, so it isn't added again
    let timeline = merge(
        page.clone(),
        Some(work(20)),
        revisions.clone(),
        vec![first.clone(), pending.clone()],
    );
    assert!(!timeline.older);
    assert_eq!(
        told(&timeline.moments),
        vec![
            (
                80,
                "resolution",
                "resolved thread #1 by revision 4 of reef/central/C2".to_string()
            ),
            (70, "mark", "resolved thread #1".to_string()),
            (60, "comment", "replied in thread #1: Because.".to_string()),
            (
                50,
                "capture",
                "drafted revision 2: 1 file +3 −1".to_string()
            ),
            (
                40,
                "comment",
                "commented in thread #1: Why *this*?".to_string()
            ),
            (
                30,
                "capture",
                "captured revision 1: 1 file +3 −1".to_string()
            ),
            (20, "start", "started on reef/C1 at aaaaaaa".to_string()),
            (10, "edit", "status unset → in-progress".to_string()),
        ]
    );
    // a start only the record holds, from before starts were kept, is added from it
    let unkept = HistoryPage {
        edits: vec![edit(10, 2)],
        older: None,
    };
    let timeline = merge(unkept, Some(work(15)), Vec::new(), Vec::new());
    let start = &timeline.moments[0];
    assert_eq!(
        (start.at, start.kind(), start.describe()),
        (15, "start", "started on main at ccccccc".to_string())
    );
    assert_eq!(start.author.describe().as_deref(), Some("carol"));
    // while older edits are kept, nothing older than the oldest edit read is shown
    let paged = HistoryPage {
        edits: vec![edit(35, 3)],
        older: Some("cursor".to_string()),
    };
    let timeline = merge(paged, Some(work(20)), revisions, vec![first, pending]);
    assert!(timeline.older);
    let ats: Vec<u64> = timeline.moments.iter().map(|moment| moment.at).collect();
    assert_eq!(ats, vec![80, 70, 60, 50, 40, 35]);
}

/// A long comment's first line is cut short, at a character
#[test]
fn a_long_comment_is_cut_short() {
    let long = "é".repeat(200);
    let page = HistoryPage::default();
    let timeline = merge(
        page,
        None,
        Vec::new(),
        vec![thread(1, vec![comment(1, 1, &long, None)], false)],
    );
    let Happened::Commented { excerpt, .. } = &timeline.moments[0].happened else {
        panic!("not a comment: {:?}", timeline.moments[0]);
    };
    assert_eq!(excerpt.chars().count(), 121);
    assert!(excerpt.ends_with('…'));
}

/// Each moment is a line saying when, who, its kind and what happened, a capture's and a
/// comment's a button that opens its tab, and an empty timeline says so
#[test]
fn the_timeline_tab_draws_a_line_a_moment() {
    let page = HistoryPage {
        edits: vec![started(20), edit(10, 2)],
        older: None,
    };
    let moments = merge(
        page,
        None,
        vec![revision(1, 30, false)],
        vec![thread(1, vec![comment(1, 40, "Why?", None)], false)],
    )
    .moments;
    let html = dioxus_ssr::render_element(rsx! {
        Opening { moments }
    });
    let doc = roxmltree::Document::parse(&html).expect("the list is well-formed");
    let entries: Vec<(String, String, String)> = doc
        .descendants()
        .filter(|n| n.attribute("data-kind").is_some())
        .map(|n| {
            let kind = n.attribute("data-kind").unwrap_or_default().to_string();
            // who, then what it says
            let who = n
                .descendants()
                .find(|d| d.attribute("class") == Some("reef-app-author"))
                .and_then(|d| d.text())
                .unwrap_or_default()
                .to_string();
            let what = n
                .descendants()
                .find(|d| {
                    d.attribute("class")
                        .is_some_and(|class| class.contains("reef-app-timeline-what"))
                })
                .map(|d| {
                    d.descendants()
                        .filter(|t| t.is_text())
                        .filter_map(|t| t.text())
                        .collect()
                })
                .unwrap_or_default();
            (kind, who, what)
        })
        .collect();
    assert_eq!(
        entries,
        vec![
            (
                "comment".to_string(),
                "ada".to_string(),
                "commented in thread #1: Why?".to_string()
            ),
            (
                "capture".to_string(),
                "Claude, for bob".to_string(),
                "captured revision 1: 1 file +3 −1".to_string()
            ),
            (
                "start".to_string(),
                "bob".to_string(),
                "started on reef/C1 at aaaaaaa".to_string()
            ),
            (
                "edit".to_string(),
                "ada".to_string(),
                "status unset → in-progress".to_string()
            ),
        ]
    );
    // a capture and a comment open their tabs, and an edit opens nothing
    let buttons: Vec<&str> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "button")
        .filter_map(|n| n.ancestors().find_map(|a| a.attribute("data-kind")))
        .collect();
    assert_eq!(buttons, ["comment", "capture"]);
    // and nothing kept says so
    let none = dioxus_ssr::render_element(rsx! {
        TimelineList { moments: Vec::new() }
    });
    assert!(none.contains("reef-app-timeline-none"), "{none}");
}
