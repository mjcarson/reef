//! Capturing from local checkouts: nodes at once, a squash after, no remote, the start's pin,
//! and gix against the CLI

use std::collections::BTreeSet;
use std::path::Path;

use x3_capture::capture::{self, Backend, Options, pin_ref};
use x3_capture::rule::Rule;
use x3_capture::scenario::{Scratch, lines};
use x3_capture::store::Capture;

/// The paths a capture touched
///
/// # Arguments
///
/// * `capture` - The capture
fn paths(capture: &Capture) -> BTreeSet<String> {
    capture.diff.sides().into_keys().collect()
}

/// A set of paths, for comparing
///
/// # Arguments
///
/// * `paths` - The paths
fn set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|path| path.to_string()).collect()
}

/// Two nodes worked at once in two worktrees each capture only their own changes, even when
/// both capture at the same moment
#[test]
fn parallel_worktrees_capture_their_own() {
    let s = Scratch::new().unwrap();
    let wa = s.worktree("wt-a", "reef/A", "main").unwrap();
    let wb = s.worktree("wt-b", "reef/B", "main").unwrap();
    s.start(&wa, "A", &[]).unwrap();
    s.start(&wb, "B", &[]).unwrap();
    // each commits, main moves on between them, and each leaves something uncommitted
    s.write(&wa, "a/one.txt", b"a\n").unwrap();
    s.commit(&wa, "A: one", Some("A")).unwrap();
    s.write(&s.main, "upstream/1.txt", b"up\n").unwrap();
    s.commit(&s.main, "upstream", None).unwrap();
    s.write(&wb, "b/one.txt", b"b\n").unwrap();
    s.commit(&wb, "B: one", Some("B")).unwrap();
    s.write(&wa, "a/two.txt", b"a2\n").unwrap();
    s.remove(&wb, "b/gone.txt").unwrap();
    // both capture at once, from two threads, as two sessions' hooks would
    let (a, b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| capture::capture(&s.store, "A", &Options::chosen("test")));
        let b = scope.spawn(|| capture::capture(&s.store, "B", &Options::chosen("test")));
        (a.join().unwrap(), b.join().unwrap())
    });
    let a = a.unwrap().expect("A changed");
    let b = b.unwrap().expect("B changed");
    assert_eq!(paths(&a), set(&["a/one.txt", "a/two.txt"]));
    assert_eq!(paths(&b), set(&["b/gone.txt", "b/one.txt"]));
    // both stored, neither over the other
    let state = s.store.load().unwrap();
    assert_eq!(state.captures["A"].len(), 1);
    assert_eq!(state.captures["B"].len(), 1);
}

/// A capture survives its commits being squashed, its branch and worktree deleted, its pin
/// dropped and every unreachable object pruned
#[test]
fn squash_loses_nothing() {
    let s = Scratch::new().unwrap();
    let wa = s.worktree("wt-a", "reef/A", "main").unwrap();
    s.start(&wa, "A", &[]).unwrap();
    s.write(&wa, "a/one.txt", b"a1\n").unwrap();
    s.commit(&wa, "A: one", Some("A")).unwrap();
    let a_keep = lines("a keep", 8).replace("a keep 2", "a keep two");
    s.write(&wa, "a/keep.txt", a_keep.as_bytes()).unwrap();
    s.commit(&wa, "A: keep", Some("A")).unwrap();
    s.done("A").unwrap();
    // captured as the hook would capture it, marked done
    let captured = capture::capture(&s.store, "A", &Options::chosen("test"))
        .unwrap()
        .expect("A changed");
    let head = captured.head.clone().expect("A has a head");
    // a squash-merge, then everything that led to it goes
    s.squash_into_main("reef/A").unwrap();
    let git = s.git(&s.main);
    let wa_str = wa.to_string_lossy().into_owned();
    git.run(&["worktree", "remove", "--force", &wa_str])
        .unwrap();
    git.run(&["branch", "-D", "reef/A"]).unwrap();
    git.delete_ref(&pin_ref("A")).unwrap();
    git.run(&["reflog", "expire", "--expire=now", "--all"])
        .unwrap();
    git.run(&["gc", "-q", "--prune=now"]).unwrap();
    // git no longer has the head the capture read
    assert!(!git.exists(&head).unwrap(), "the squashed head survived gc");
    // and the capture reads back exactly as it was taken
    let state = s.store.load().unwrap();
    assert_eq!(state.captures["A"], vec![captured]);
    // the CLI shows it too, from the state dir alone
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_x3-capture"))
        .args(["--state"])
        .arg(s.store.dir())
        .args(["show", "A"])
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.contains("a/one.txt") && out.contains("a/keep.txt"),
        "{out}"
    );
}

/// A capture never reaches for a remote: one whose URL leads nowhere changes nothing
#[test]
fn a_dead_remote_is_never_touched() {
    let s = Scratch::new().unwrap();
    // a remote that would fail any fetch or push, with nothing fetched from it
    let git = s.git(&s.main);
    git.run(&["remote", "add", "origin", "/nonexistent/x3-remote.git"])
        .unwrap();
    let wa = s.worktree("wt-a", "reef/A", "main").unwrap();
    s.start(&wa, "A", &[]).unwrap();
    s.write(&wa, "a/one.txt", b"a\n").unwrap();
    s.commit(&wa, "A: one", Some("A")).unwrap();
    s.write(&wa, "a/two.txt", b"uncommitted\n").unwrap();
    // nothing pushed, nothing fetched, and every rule still answers
    for rule in Rule::ALL {
        let options = Options {
            rule,
            backend: Backend::Cli,
            trigger: "test".to_owned(),
            seal: false,
            patch_id: false,
        };
        let taken = capture::take(&s.store, "A", options.rule, options.backend).unwrap();
        assert_eq!(
            taken.diff.sides().into_keys().collect::<BTreeSet<_>>(),
            set(&["a/one.txt", "a/two.txt"]),
            "{}",
            rule.name()
        );
    }
    // and no tracking ref ever appeared
    assert!(
        git.commit("origin/main").unwrap().is_none(),
        "something fetched"
    );
}

/// Every scratch case runs with no remote at all
#[test]
fn no_case_has_a_remote() {
    for case in x3_capture::scenario::CASES {
        // `run` fails a case that has a remote
        x3_capture::scenario::run(case).unwrap();
    }
}

/// The start's pin is what lets a rewritten history still find its start after a gc
#[test]
fn the_pin_keeps_the_start() {
    let s = Scratch::new().unwrap();
    let wt = s.worktree("wt", "work", "main").unwrap();
    s.start(&wt, "C", &[]).unwrap();
    s.write(&wt, "c/one.txt", b"c1\n").unwrap();
    s.commit(&wt, "C: one", Some("C")).unwrap();
    s.done("C").unwrap();
    // D starts on C's commit, with something uncommitted left in its start
    s.write(&wt, "c/left.txt", b"left\n").unwrap();
    s.start(&wt, "D", &[]).unwrap();
    s.write(&wt, "d/one.txt", b"d1\n").unwrap();
    s.commit(&wt, "C's left and D's one", Some("D")).unwrap();
    // main moves on, the branch is rebased, and the old commits lose every reflog entry
    s.write(&s.main, "upstream/1.txt", b"up\n").unwrap();
    s.commit(&s.main, "upstream", None).unwrap();
    let git = s.git(&wt);
    git.run(&["rebase", "-q", "main"]).unwrap();
    git.run(&["reflog", "expire", "--expire=now", "--all"])
        .unwrap();
    git.run(&["gc", "-q", "--prune=now"]).unwrap();
    // pinned, the start's tree survives, and D's changes are its own
    let taken = capture::take(&s.store, "D", Rule::Carried, Backend::Cli).unwrap();
    assert_eq!(
        taken.diff.sides().into_keys().collect::<BTreeSet<_>>(),
        set(&["d/one.txt"])
    );
    // unpinned and pruned, the start is gone and the rule can't carry it
    git.delete_ref(&pin_ref("D")).unwrap();
    git.delete_ref(&pin_ref("C")).unwrap();
    git.run(&["gc", "-q", "--prune=now"]).unwrap();
    assert!(capture::take(&s.store, "D", Rule::Carried, Backend::Cli).is_err());
}

/// The files gix and the CLI are compared on, beyond what the cases cover
///
/// # Arguments
///
/// * `s` - The scratch repo
/// * `dir` - The worktree to write them in
fn awkward_files(s: &Scratch, dir: &Path) {
    // several hunks in one long file, and an edit at each end
    let long = lines("long", 60);
    let long_edited = long
        .replace("long 1\n", "long one\n")
        .replace("long 20\n", "long twenty\n")
        .replace("long 40\n", "")
        .replace("long 60\n", "long 60\nlong 61\n");
    s.write(dir, "long.txt", long.as_bytes()).unwrap();
    s.write(dir, "nonl.txt", b"tail").unwrap();
    s.write(dir, "crlf.txt", b"one\r\ntwo\r\nthree\r\n")
        .unwrap();
    s.write(dir, "emptied.txt", b"was here\n").unwrap();
    s.write(dir, "empty.txt", b"").unwrap();
    s.write(dir, "run.sh", b"echo run\n").unwrap();
    s.write(dir, "moved.txt", lines("moved", 20).as_bytes())
        .unwrap();
    s.write(dir, "blob.bin", b"\0\x01\x02").unwrap();
    s.commit(dir, "awkward files", None).unwrap();
    // then every kind of change to them, left uncommitted
    s.write(dir, "long.txt", long_edited.as_bytes()).unwrap();
    s.write(dir, "nonl.txt", b"tail\nmore").unwrap();
    s.write(dir, "crlf.txt", b"one\r\nTWO\r\nthree\r\n")
        .unwrap();
    s.write(dir, "emptied.txt", b"").unwrap();
    s.write(dir, "empty.txt", b"now full\n").unwrap();
    s.rename(
        dir,
        "moved.txt",
        "moved/here.txt",
        lines("moved", 20)
            .replace("moved 9", "moved nine")
            .as_bytes(),
    )
    .unwrap();
    s.write(dir, "blob.bin", b"\0\x01\x03").unwrap();
    s.write(dir, "new.txt", b"brand new\n").unwrap();
    s.remove(dir, "README.md").unwrap();
    let run = dir.join("run.sh");
    let mut perms = std::fs::metadata(&run).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&run, perms).unwrap();
}

/// gix reads the same files, hunks and lines as git's CLI, however awkward the files
#[test]
fn gix_matches_cli() {
    let s = Scratch::new().unwrap();
    let main = s.main.clone();
    awkward_files(&s, &main);
    // the committed state against the worktree as it is now
    let git = s.git(&main);
    let base = git.commit("HEAD").unwrap().unwrap();
    let tree = git.snapshot().unwrap();
    let cli = git.diff(&base, &tree).unwrap();
    let gix = x3_capture::gix::diff(&x3_capture::gix::open(&main).unwrap(), &base, &tree).unwrap();
    // every kind of change turned up, and both readers agree on each
    assert_eq!(cli.files.len(), 10, "{cli:#?}");
    for (cli, gix) in cli.sorted().files.iter().zip(&gix.sorted().files) {
        assert_eq!(cli, gix, "{}", cli.path());
    }
    assert_eq!(cli.sorted(), gix.sorted());
}

/// The three trees of one merge: a base, and each side's changes to it
///
/// # Arguments
///
/// * `s` - The scratch repo
/// * `ours` - Our side's changes, made in the worktree
/// * `theirs` - Their side's changes, made in the worktree
fn merge_case(
    s: &Scratch,
    ours: &dyn Fn(&Scratch, &Path),
    theirs: &dyn Fn(&Scratch, &Path),
) -> (String, String, String) {
    let main = s.main.clone();
    let git = s.git(&main);
    // the base is the scratch repo's first commit
    let base = git.tree("HEAD").unwrap();
    // each side is the worktree after its changes, snapshotted, then put back
    let side = |change: &dyn Fn(&Scratch, &Path)| {
        change(s, &main);
        let tree = git.snapshot().unwrap();
        git.run(&["reset", "-q", "--hard"]).unwrap();
        git.run(&["clean", "-q", "-fd"]).unwrap();
        tree
    };
    let ours = side(ours);
    let theirs = side(theirs);
    (base, ours, theirs)
}

/// gix's three-way merge of trees writes the tree `git merge-tree` writes, conflicts and their
/// markers included, and names the same conflicted files
#[test]
fn gix_merges_trees_as_git_does() {
    // each case: its name, the files it must conflict on, then our change and their change
    type Change = Box<dyn Fn(&Scratch, &Path)>;
    let shared = lines("shared", 10);
    let cases: Vec<(&str, &[&str], Change, Change)> = vec![
        (
            "apart",
            &[],
            Box::new(|s, d| s.write(d, "a/new.txt", b"ours\n").unwrap()),
            Box::new(|s, d| s.write(d, "b/new.txt", b"theirs\n").unwrap()),
        ),
        (
            "same line",
            &["shared.txt"],
            Box::new({
                let shared = shared.clone();
                move |s, d| {
                    let ours = shared.replace("shared 5", "shared five, ours");
                    s.write(d, "shared.txt", ours.as_bytes()).unwrap()
                }
            }),
            Box::new({
                let shared = shared.clone();
                move |s, d| {
                    let theirs = shared.replace("shared 5", "shared five, theirs");
                    s.write(d, "shared.txt", theirs.as_bytes()).unwrap()
                }
            }),
        ),
        (
            "both the same",
            &[],
            Box::new({
                let shared = shared.clone();
                move |s, d| {
                    let both = shared.replace("shared 2", "shared two");
                    s.write(d, "shared.txt", both.as_bytes()).unwrap()
                }
            }),
            Box::new({
                let shared = shared.clone();
                move |s, d| {
                    let both = shared.replace("shared 2", "shared two");
                    s.write(d, "shared.txt", both.as_bytes()).unwrap()
                }
            }),
        ),
        (
            "lines apart in one file",
            &[],
            Box::new({
                let shared = shared.clone();
                move |s, d| {
                    let ours = shared.replace("shared 1\n", "shared one\n");
                    s.write(d, "shared.txt", ours.as_bytes()).unwrap()
                }
            }),
            Box::new({
                let shared = shared.clone();
                move |s, d| {
                    let theirs = shared.replace("shared 9", "shared nine");
                    s.write(d, "shared.txt", theirs.as_bytes()).unwrap()
                }
            }),
        ),
        (
            "renamed by ours, edited by theirs",
            &[],
            Box::new(|s, d| {
                let old = lines("a old", 12);
                s.rename(d, "a/old.txt", "a/moved.txt", old.as_bytes())
                    .unwrap()
            }),
            Box::new(|s, d| {
                let edited = lines("a old", 12).replace("a old 7", "a old seven");
                s.write(d, "a/old.txt", edited.as_bytes()).unwrap()
            }),
        ),
        (
            "deleted by ours, edited by theirs",
            &["b/gone.txt"],
            Box::new(|s, d| s.remove(d, "b/gone.txt").unwrap()),
            Box::new(|s, d| s.write(d, "b/gone.txt", b"b kept and changed\n").unwrap()),
        ),
        (
            "added by both, apart",
            &["c/both.txt"],
            Box::new(|s, d| s.write(d, "c/both.txt", b"ours\n").unwrap()),
            Box::new(|s, d| s.write(d, "c/both.txt", b"theirs\n").unwrap()),
        ),
    ];
    for (name, conflicts, ours, theirs) in cases {
        let s = Scratch::new().unwrap();
        let (base, ours, theirs) = merge_case(&s, ours.as_ref(), theirs.as_ref());
        let cli = s.git(&s.main).merge_trees(&base, &ours, &theirs).unwrap();
        let repo = x3_capture::gix::open(&s.main).unwrap();
        let gix = x3_capture::gix::merge_trees(&repo, &base, &ours, &theirs).unwrap();
        // git conflicted where the case says, and gix agrees on the files and the whole tree
        assert_eq!(cli.conflicts, conflicts, "{name}: git's conflicts");
        assert_eq!(cli.conflicts, gix.conflicts, "{name}: conflicts");
        assert_eq!(cli.tree, gix.tree, "{name}: tree");
    }
}

/// gix finds the same merge-bases and ancestors as git's CLI, across every pair of commits in
/// a history with branches and a merge
#[test]
fn gix_walks_history_as_git_does() {
    let s = Scratch::new().unwrap();
    // a branch off main, main moving on, and the branch merging main in
    let wb = s.worktree("wt-b", "reef/B", "main").unwrap();
    s.write(&wb, "b/one.txt", b"b1\n").unwrap();
    s.commit(&wb, "B: one", None).unwrap();
    s.write(&s.main, "upstream/1.txt", b"up\n").unwrap();
    s.commit(&s.main, "upstream", None).unwrap();
    s.git(&wb)
        .run(&["merge", "-q", "--no-edit", "main"])
        .unwrap();
    s.write(&wb, "b/two.txt", b"b2\n").unwrap();
    s.commit(&wb, "B: two", None).unwrap();
    s.write(&s.main, "upstream/2.txt", b"up2\n").unwrap();
    s.commit(&s.main, "upstream again", None).unwrap();
    // every commit in either branch
    let git = s.git(&s.main);
    let all = git.run(&["rev-list", "--all"]).unwrap();
    let commits: Vec<&str> = all.lines().collect();
    assert!(commits.len() >= 6, "{commits:?}");
    let repo = x3_capture::gix::open(&s.main).unwrap();
    for a in &commits {
        for b in &commits {
            assert_eq!(
                git.merge_base(a, b).unwrap(),
                x3_capture::gix::merge_base(&repo, a, b).unwrap(),
                "merge-base of {a} and {b}"
            );
            assert_eq!(
                git.is_ancestor(a, b).unwrap(),
                x3_capture::gix::is_ancestor(&repo, a, b).unwrap(),
                "is {a} an ancestor of {b}"
            );
        }
    }
    // and a branch's name resolves to the commit git resolves it to
    assert_eq!(
        git.commit("main").unwrap(),
        x3_capture::gix::commit(&repo, "main").unwrap()
    );
}
