//! The triggers: Claude Code's PostToolUse and Stop, and real git hooks in two worktrees

use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

use x3_capture::hook;
use x3_capture::scenario::Scratch;
use x3_capture::store::Store;

/// Runs a Claude Code hook with this JSON on stdin
///
/// # Arguments
///
/// * `store` - The state dir
/// * `event` - The hook's event
/// * `input` - What Claude Code would hand it
fn claude_hook(store: &Store, event: &str, input: Value) {
    let raw = serde_json::to_vec(&input).unwrap();
    hook::run(store, event, &[], &raw, Path::new(".")).unwrap();
}

/// PostToolUse's input for reef's status tool
///
/// # Arguments
///
/// * `cwd` - The session's directory
/// * `id` - The node
/// * `status` - The status it sets
fn status_call(cwd: &Path, id: &str, status: &str) -> Value {
    json!({
        "session_id": "test",
        "hook_event_name": "PostToolUse",
        "cwd": cwd,
        "tool_name": "mcp__reef__status",
        "tool_input": { "id": id, "status": status },
        "tool_response": format!("`{id}` is {status}"),
    })
}

/// How many revisions a node has
///
/// # Arguments
///
/// * `store` - The state dir
/// * `node` - The node
fn revisions(store: &Store, node: &str) -> usize {
    store
        .load()
        .unwrap()
        .captures
        .get(node)
        .map(Vec::len)
        .unwrap_or_default()
}

/// The paths a node's latest capture touched
///
/// # Arguments
///
/// * `store` - The state dir
/// * `node` - The node
fn latest_paths(store: &Store, node: &str) -> BTreeSet<String> {
    store
        .load()
        .unwrap()
        .latest(node)
        .map(|capture| capture.diff.sides().into_keys().collect())
        .unwrap_or_default()
}

/// A set of paths, for comparing
///
/// # Arguments
///
/// * `paths` - The paths
fn set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|path| path.to_string()).collect()
}

/// PostToolUse captures a node once when reef's status tool marks it done, and nothing else
#[test]
fn post_tool_use_captures_on_done() {
    let s = Scratch::new().unwrap();
    let wa = s.worktree("wt-a", "reef/A", "main").unwrap();
    s.start(&wa, "A", &[]).unwrap();
    s.write(&wa, "a/one.txt", b"a\n").unwrap();
    // another tool, or another status, captures nothing
    let mut other = status_call(&wa, "A", "done");
    other["tool_name"] = json!("mcp__reef__get");
    claude_hook(&s.store, "PostToolUse", other);
    claude_hook(
        &s.store,
        "PostToolUse",
        status_call(&wa, "A", "in-progress"),
    );
    assert_eq!(revisions(&s.store, "A"), 0);
    // done captures it, uncommitted work and all
    claude_hook(&s.store, "PostToolUse", status_call(&wa, "A", "done"));
    assert_eq!(revisions(&s.store, "A"), 1);
    assert_eq!(latest_paths(&s.store, "A"), set(&["a/one.txt"]));
    // done again with nothing changed stores no second revision
    claude_hook(&s.store, "PostToolUse", status_call(&wa, "A", "done"));
    assert_eq!(revisions(&s.store, "A"), 1);
    // committed after the done mark, the same tree names the commit that now holds it
    s.commit(&wa, "A: one", Some("A")).unwrap();
    claude_hook(&s.store, "PostToolUse", status_call(&wa, "A", "done"));
    let state = s.store.load().unwrap();
    let draft = state.latest("A").unwrap();
    assert_eq!(draft.revision, 1);
    assert_eq!(draft.head, s.git(&wa).commit("HEAD").unwrap());
}

/// Stop captures each node done in its own worktree since its last capture, once
#[test]
fn stop_captures_each_done_node_once() {
    let s = Scratch::new().unwrap();
    let wa = s.worktree("wt-a", "reef/A", "main").unwrap();
    let wb = s.worktree("wt-b", "reef/B", "main").unwrap();
    s.start(&wa, "A", &[]).unwrap();
    s.start(&wb, "B", &[]).unwrap();
    s.write(&wa, "a/one.txt", b"a\n").unwrap();
    s.write(&wb, "b/one.txt", b"b\n").unwrap();
    // mid-node, Stop does nothing
    let stop = |cwd: &Path| json!({ "session_id": "test", "cwd": cwd, "stop_hook_active": false });
    claude_hook(&s.store, "Stop", stop(&wa));
    assert_eq!(revisions(&s.store, "A"), 0);
    // both done; A's session stops, and only A is captured, from A's worktree
    s.done("A").unwrap();
    s.done("B").unwrap();
    claude_hook(&s.store, "Stop", stop(&wa));
    assert_eq!(revisions(&s.store, "A"), 1);
    assert_eq!(revisions(&s.store, "B"), 0);
    // a second Stop takes nothing new, and B's own session takes B
    s.write(&wa, "a/after.txt", b"after done\n").unwrap();
    claude_hook(&s.store, "Stop", stop(&wa));
    assert_eq!(revisions(&s.store, "A"), 1);
    claude_hook(&s.store, "Stop", stop(&wb));
    assert_eq!(latest_paths(&s.store, "B"), set(&["b/one.txt"]));
    // marked done again, as a round of comment fixes ends, A is captured again
    s.done("A").unwrap();
    claude_hook(&s.store, "Stop", stop(&wa));
    assert_eq!(revisions(&s.store, "A"), 2);
    assert_eq!(
        latest_paths(&s.store, "A"),
        set(&["a/after.txt", "a/one.txt"])
    );
}

/// Installs the spike's git hooks in the repo's common hooks dir, shared by every worktree
///
/// # Arguments
///
/// * `s` - The scratch repo
fn install_git_hooks(s: &Scratch) {
    let hooks = s.main.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for event in ["post-commit", "post-rewrite"] {
        let script = format!(
            "#!/bin/sh\nexec '{}' --state '{}' hook {event} \"$@\"\n",
            env!("CARGO_BIN_EXE_x3-capture"),
            s.store.dir().display()
        );
        let path = hooks.join(event);
        std::fs::write(&path, script).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }
}

/// The event log's entries of one kind
///
/// # Arguments
///
/// * `store` - The state dir
/// * `kind` - The kind
fn events(store: &Store, kind: &str) -> Vec<Value> {
    store
        .events()
        .unwrap()
        .into_iter()
        .filter(|event| event["kind"] == kind)
        .collect()
}

/// git's own hooks capture the right node in each of two worktrees, and a local squash of a
/// node's commits leaves its capture whole
#[test]
fn git_hooks_capture_each_worktree() {
    let s = Scratch::new().unwrap();
    let wa = s.worktree("wt-a", "reef/A", "main").unwrap();
    let wb = s.worktree("wt-b", "reef/B", "main").unwrap();
    install_git_hooks(&s);
    s.start(&wa, "A", &[]).unwrap();
    s.start(&wb, "B", &[]).unwrap();
    // each commit is captured, in its own worktree, as its own node
    s.write(&wa, "a/one.txt", b"a1\n").unwrap();
    s.commit(&wa, "A: one", Some("A")).unwrap();
    s.write(&wb, "b/one.txt", b"b1\n").unwrap();
    s.commit(&wb, "B: one", Some("B")).unwrap();
    s.write(&wa, "a/two.txt", b"a2\n").unwrap();
    s.commit(&wa, "A: two", Some("A")).unwrap();
    s.write(&wa, "a/three.txt", b"a3\n").unwrap();
    s.commit(&wa, "A: three", Some("A")).unwrap();
    // three commits keep one draft current, not three revisions
    assert_eq!(revisions(&s.store, "A"), 1);
    assert!(!s.store.load().unwrap().latest("A").unwrap().sealed);
    assert_eq!(
        latest_paths(&s.store, "A"),
        set(&["a/one.txt", "a/three.txt", "a/two.txt"])
    );
    assert_eq!(latest_paths(&s.store, "B"), set(&["b/one.txt"]));
    let before = s.store.load().unwrap().latest("A").unwrap().diff.sides();
    // squash A's three commits into one, locally, with an interactive rebase
    let start = s.store.load().unwrap().work["A"].start.clone();
    s.git(&wa)
        .run_env(
            &["rebase", "-q", "-i", &start],
            &[
                ("GIT_SEQUENCE_EDITOR", "sed -i -e '2,$s/^pick/squash/'"),
                ("GIT_EDITOR", "true"),
            ],
        )
        .unwrap();
    let commits = s
        .git(&wa)
        .run(&["rev-list", "--count", &format!("{start}..HEAD")])
        .unwrap();
    assert_eq!(commits, "1", "the rebase didn't squash");
    // post-commit ran mid-rebase and was skipped; post-rewrite ran once it ended
    let commits = events(&s.store, "post-commit");
    assert!(
        commits
            .iter()
            .any(|event| event["body"]["mid_operation"] == true),
        "{commits:#?}"
    );
    let rewrites = events(&s.store, "post-rewrite:rebase");
    assert_eq!(rewrites.len(), 1, "{rewrites:#?}");
    // the squashed history has the same tree, so the capture is the same, whole
    let after = s.store.load().unwrap().latest("A").unwrap().diff.sides();
    assert_eq!(before, after);
    // and no capture ever came from a half-built HEAD
    for capture in &s.store.load().unwrap().captures["A"] {
        assert!(
            capture
                .diff
                .sides()
                .keys()
                .all(|path| path.starts_with("a/")),
            "r{} by {}",
            capture.revision,
            capture.trigger
        );
    }
    // done, the session's Stop seals the draft as it is
    s.done("A").unwrap();
    let stop = json!({ "session_id": "test", "cwd": wa, "stop_hook_active": false });
    claude_hook(&s.store, "Stop", stop);
    let state = s.store.load().unwrap();
    let sealed = state.latest("A").unwrap();
    assert_eq!((sealed.revision, sealed.sealed), (1, true));
    assert_eq!(sealed.sealed_by.as_deref(), Some("Stop"));
    // a commit after the seal, as a round of comment fixes makes, starts a new draft
    s.write(&wa, "a/fix.txt", b"fix\n").unwrap();
    s.commit(&wa, "A: fix", Some("A")).unwrap();
    let state = s.store.load().unwrap();
    let draft = state.latest("A").unwrap();
    assert_eq!((draft.revision, draft.sealed), (2, false));
    assert!(draft.diff.sides().contains_key("a/fix.txt"));
}

/// `claude -w <node>` asks reef for the node's worktree, which reef makes and starts the node
/// in, and its removal takes a last draft first, keeping the branch
#[test]
fn worktree_hooks_make_and_keep_a_node() {
    let s = Scratch::new().unwrap();
    // WorktreeCreate answers with the path, where Claude Code would have put its own
    let input = json!({ "session_id": "test", "cwd": s.main, "name": "N7" });
    let raw = serde_json::to_vec(&input).unwrap();
    let path = hook::run(&s.store, "WorktreeCreate", &[], &raw, Path::new("."))
        .unwrap()
        .expect("WorktreeCreate answers with a path");
    let path = std::path::PathBuf::from(path);
    assert_eq!(path, s.main.join(".claude/worktrees/N7"));
    assert_eq!(s.git(&path).branch().unwrap().as_deref(), Some("reef/N7"));
    // the node is started there, so a capture knows where it is
    let state = s.store.load().unwrap();
    assert_eq!(state.work["N7"].worktree, path);
    // work left uncommitted is captured before the worktree goes
    s.write(&path, "n7.txt", b"seven\n").unwrap();
    let input = json!({ "session_id": "test", "cwd": s.main, "worktree_path": path });
    let raw = serde_json::to_vec(&input).unwrap();
    hook::run(&s.store, "WorktreeRemove", &[], &raw, Path::new(".")).unwrap();
    assert!(!path.exists(), "the worktree is still there");
    assert_eq!(latest_paths(&s.store, "N7"), set(&["n7.txt"]));
    assert!(
        s.git(&s.main).commit("reef/N7").unwrap().is_some(),
        "the branch went too"
    );
}
