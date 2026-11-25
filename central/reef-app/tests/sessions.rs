//! A terminal's own session draws as a row to read: where it runs, where it stands and its
//! loop's node, with none of a runner's buttons
//!
//! The `AttachedRow` component is rendered natively and parsed, so each check is of elements,
//! classes and text, not of how the renderer escapes or closes its tags.

use dioxus::prelude::*;
use reef_api::{Loop, LoopState, Pane, Session, SessionState};
use reef_app::session::AttachedRow;

/// A terminal's session in a Claude Code process with no multiplexer, working
fn attached() -> Session {
    Session {
        id: "k3x9-7".to_string(),
        pane: Pane::Claude { pid: 4242 },
        project: "reef".to_string(),
        plan: None,
        host: "box".to_string(),
        cwd: "/src/reef".into(),
        attached_at: 0,
        seen_at: 0,
        state: SessionState::Working,
        claude: Some("s1".to_string()),
        entries: 3,
        prompts: Vec::new(),
    }
}

/// The loop of its pane, running on C84
fn feeding() -> Loop {
    Loop {
        pane: Pane::Claude { pid: 4242 },
        project: "reef".to_string(),
        plan: None,
        session: Some("s1".to_string()),
        acted: None,
        node: Some("reef/central/C84".parse().expect("a node's name")),
        state: LoopState::Running,
        version: 1,
        started_at: 0,
        width: 1,
        opens: false,
        pid: Some(4242),
        members: Vec::new(),
    }
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

/// A terminal's session shows its host, checkout, pane, state and its loop's node, and has no
/// button to start, pause or stop it
#[test]
fn a_terminals_session_draws_with_no_buttons() {
    // the row, rendered and parsed
    let html = dioxus_ssr::render_element(rsx! {
        AttachedRow { session: attached(), feeding: Some(feeding()), picked: true }
    });
    let doc = roxmltree::Document::parse(&html).expect("the row is well formed");
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "li");
    assert_eq!(
        root.attribute("class"),
        Some("reef-app-runner attached picked")
    );
    assert_eq!(root.attribute("data-session"), Some("k3x9-7"));
    // where it runs and stands, and the node its loop holds
    let said = text(root);
    for part in [
        "box",
        "reef",
        "Claude Code process 4242",
        "working",
        "On C84",
    ] {
        assert!(said.contains(part), "{part} in {said}");
    }
    // and nothing that acts on it
    assert!(
        !root.descendants().any(|n| n.has_tag_name("button")),
        "{html}"
    );
    for act in ["Start", "Pause", "Stop", "Resume"] {
        assert!(!said.contains(act), "{act} in {said}");
    }
    // with no loop, it says it is answered in its terminal
    let html = dioxus_ssr::render_element(rsx! {
        AttachedRow { session: attached(), feeding: None, picked: false }
    });
    assert!(html.contains("answered in its terminal"), "{html}");
    assert!(!html.contains("<button"), "{html}");
}
