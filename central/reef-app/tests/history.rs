//! The History tab, drawn on the server: each edit of a node, newest first, saying when, who,
//! the version it left and what it changed

use dioxus::prelude::*;
use reef_api::{Author, Edited, Logged, Writer};
use reef_app::side::history::HistoryList;
use reef_model::{Met, Status};

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

/// Each edit is a line: when, who if anyone known, the version it left, what it changed, and
/// whether it came of a whole plan's write
#[test]
fn the_history_tab_says_who_changed_what() {
    // ada set it done with Claude, then nobody known imported the plan over it
    let entries = vec![
        Logged {
            at: 1_795_381_165_854,
            author: Author {
                user: Some("ada".to_string()),
                since: Some(1),
                by: Writer::Claude,
            },
            version: Some(4),
            whole_plan: false,
            edits: vec![
                Edited::Status {
                    from: Some(Status::InProgress),
                    to: Some(Status::Done),
                },
                Edited::Criterion {
                    number: 1,
                    text: "Kept".to_string(),
                    from: None,
                    to: Some(Met::Met),
                    test: None,
                },
            ],
        },
        Logged {
            at: 0,
            author: Author::default(),
            version: Some(3),
            whole_plan: true,
            edits: vec![Edited::Keys {
                keys: vec!["needs".to_string()],
            }],
        },
    ];
    let html = dioxus_ssr::render_element(rsx! {
        HistoryList { entries }
    });
    let doc = roxmltree::Document::parse(&html).expect("the history is well formed");
    let lines: Vec<_> = doc
        .root_element()
        .descendants()
        .filter(|n| class(*n) == "reef-app-history-entry")
        .collect();
    assert_eq!(lines.len(), 2, "{html}");
    // the first names its author, its version and what it changed
    let pick = |line: roxmltree::Node, name: &str| {
        line.descendants()
            .find(|n| class(*n).split(' ').any(|class| class == name))
            .map(text)
    };
    assert_eq!(
        pick(lines[0], "reef-app-author").as_deref(),
        Some("Claude, for ada")
    );
    assert_eq!(
        pick(lines[0], "reef-app-history-version").as_deref(),
        Some("v4")
    );
    assert_eq!(
        pick(lines[0], "reef-app-history-what").as_deref(),
        Some("status in-progress → done; criterion 1 met")
    );
    assert_eq!(
        pick(lines[0], "reef-app-history-when").as_deref(),
        Some("2026-11-22 20:59 UTC")
    );
    assert_eq!(pick(lines[0], "reef-app-whole-plan"), None);
    // the second names nobody, and says it came of a whole plan's write
    assert_eq!(pick(lines[1], "reef-app-author"), None);
    assert_eq!(
        pick(lines[1], "reef-app-whole-plan").as_deref(),
        Some("whole plan")
    );
    // and a node never edited since histories were kept says so
    let html = dioxus_ssr::render_element(rsx! {
        HistoryList { entries: Vec::new() }
    });
    assert!(html.contains("reef-app-history-none"), "{html}");
}
