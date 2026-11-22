//! reef's git module against git's CLI, which stays its oracle
//!
//! Each case makes scratch repos under cargo's target dir, with no remote, and works them with
//! real git. Every gix read is checked against what git's CLI answers for the same repo, and the
//! snapshot and the pin are read back through the CLI.

use reef::git::{Checkout, Pinned};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs git in a directory as the tests' own author, with no one's config, returning its output
///
/// # Arguments
///
/// * `dir` - The directory to run in
/// * `args` - Git's arguments
fn git(dir: &Path, args: &[&str]) -> String {
    // from the directory alone, as this test's author
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(["-c", "init.defaultBranch=main"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {} in {}: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// What git's CLI answers, or `None` where it fails, for answers a failure gives
///
/// # Arguments
///
/// * `dir` - The directory to run in
/// * `args` - Git's arguments
fn answer(dir: &Path, args: &[&str]) -> Option<String> {
    // the same command, its failure an answer
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).unwrap().trim().to_string())
}

/// A fresh directory for one case, under cargo's target dir
///
/// # Arguments
///
/// * `name` - The case's name
fn fresh(name: &str) -> PathBuf {
    // wipe what an earlier run left
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("git")
        .join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A repo on `main` with a commit of one file
///
/// # Arguments
///
/// * `dir` - Where to make it
fn repo(dir: &Path) -> PathBuf {
    // an empty repo, then a first commit
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q"]);
    commit(dir, "a.txt", "one\n", "first");
    dir.canonicalize().unwrap()
}

/// Writes a file and commits it, returning the commit
///
/// # Arguments
///
/// * `dir` - The worktree
/// * `path` - The file's path in it
/// * `text` - What it holds
/// * `message` - The commit's message
fn commit(dir: &Path, path: &str, text: &str, message: &str) -> String {
    // write it, stage it, commit it
    std::fs::write(dir.join(path), text).unwrap();
    git(dir, &["add", path]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

/// HEAD and its branch read as git reads them, detached and unborn included
#[test]
fn head_and_branch_match_the_cli() {
    let dir = fresh("head");
    let repo = repo(&dir.join("repo"));
    // on a branch
    let checkout = Checkout::open(&repo).unwrap();
    assert_eq!(
        checkout.head().unwrap(),
        answer(&repo, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        checkout.branch().unwrap(),
        answer(&repo, &["symbolic-ref", "--short", "HEAD"])
    );
    assert_eq!(checkout.branch().unwrap().as_deref(), Some("main"));
    // from a directory deep in it, the worktree's root
    std::fs::create_dir_all(repo.join("src/deep")).unwrap();
    let deep = Checkout::open(&repo.join("src/deep")).unwrap();
    assert_eq!(deep.root(), repo);
    assert_eq!(deep.main_root().unwrap(), repo);
    // detached, there is a commit but no branch
    commit(&repo, "b.txt", "two\n", "second");
    git(&repo, &["checkout", "-q", "--detach", "HEAD~1"]);
    let detached = Checkout::open(&repo).unwrap();
    assert_eq!(
        detached.head().unwrap(),
        answer(&repo, &["rev-parse", "HEAD"])
    );
    assert_eq!(detached.branch().unwrap(), None);
    assert_eq!(answer(&repo, &["symbolic-ref", "--short", "HEAD"]), None);
    // and with no commit yet, a branch but no commit
    let unborn = dir.join("unborn");
    std::fs::create_dir_all(&unborn).unwrap();
    git(&unborn, &["init", "-q"]);
    let empty = Checkout::open(&unborn).unwrap();
    assert_eq!(empty.head().unwrap(), None);
    assert_eq!(
        answer(&unborn, &["rev-parse", "--verify", "--quiet", "HEAD"]),
        None
    );
    assert_eq!(empty.branch().unwrap().as_deref(), Some("main"));
}

/// A revision resolves to the commit git's `rev-parse` peels it to, or to none where git finds
/// none
#[test]
fn commit_resolves_as_rev_parse_does() {
    let dir = fresh("commit");
    let repo = repo(&dir.join("repo"));
    commit(&repo, "b.txt", "two\n", "second");
    // a tag, a tracking ref no fetch made, and a tree
    git(&repo, &["tag", "-a", "-m", "v1", "v1", "HEAD~1"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["update-ref", "refs/remotes/origin/main", &head]);
    let tree = git(&repo, &["rev-parse", "HEAD^{tree}"]);
    let checkout = Checkout::open(&repo).unwrap();
    for rev in [
        "main",
        "HEAD",
        "HEAD~1",
        "v1",
        "origin/main",
        "refs/remotes/origin/main",
        head.as_str(),
        tree.as_str(),
        "nope",
        "origin/nope",
    ] {
        // git peels each to a commit, or answers nothing
        let peeled = format!("{rev}^{{commit}}");
        let expected = answer(&repo, &["rev-parse", "--verify", "--quiet", &peeled]);
        assert_eq!(checkout.commit(rev).unwrap(), expected, "{rev}");
    }
}

/// A branch's upstream is the tracking ref `@{upstream}` names, and none when it isn't set or
/// was never fetched
#[test]
fn upstream_matches_at_upstream() {
    let dir = fresh("upstream");
    let repo = repo(&dir.join("repo"));
    // a remote that is never fetched, its tracking ref written by hand
    git(&repo, &["remote", "add", "origin", "/nowhere/at/all"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["update-ref", "refs/remotes/origin/main", &head]);
    git(
        &repo,
        &["branch", "-q", "--set-upstream-to=origin/main", "main"],
    );
    git(&repo, &["branch", "-q", "alone"]);
    // a branch whose upstream's tracking ref is gone
    git(&repo, &["update-ref", "refs/remotes/origin/gone", &head]);
    git(
        &repo,
        &["branch", "-q", "--track", "orphaned", "origin/gone"],
    );
    git(&repo, &["update-ref", "-d", "refs/remotes/origin/gone"]);
    let checkout = Checkout::open(&repo).unwrap();
    for branch in ["main", "alone", "orphaned"] {
        // git abbreviates the tracking ref, and fails for none or one gone
        let spec = format!("{branch}@{{upstream}}");
        let expected = answer(&repo, &["rev-parse", "--abbrev-ref", &spec]);
        assert_eq!(checkout.upstream(branch).unwrap(), expected, "{branch}");
    }
    assert_eq!(
        checkout.upstream("main").unwrap().as_deref(),
        Some("origin/main")
    );
}

/// The default branch is the one `origin/HEAD` names, local if it can be, else `main` or
/// `master`, else none
#[test]
fn default_branch_follows_origin_head_then_the_usual_names() {
    let dir = fresh("default");
    let repo = repo(&dir.join("repo"));
    let head = git(&repo, &["rev-parse", "HEAD"]);
    // with no remote, the local `main`
    let checkout = Checkout::open(&repo).unwrap();
    assert_eq!(checkout.default_branch().unwrap().as_deref(), Some("main"));
    // `origin/HEAD` naming `trunk`, which has no local branch, then one that has
    git(&repo, &["update-ref", "refs/remotes/origin/trunk", &head]);
    git(
        &repo,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );
    assert_eq!(
        Checkout::open(&repo)
            .unwrap()
            .default_branch()
            .unwrap()
            .as_deref(),
        Some("origin/trunk")
    );
    git(&repo, &["branch", "-q", "trunk"]);
    assert_eq!(
        Checkout::open(&repo)
            .unwrap()
            .default_branch()
            .unwrap()
            .as_deref(),
        Some("trunk")
    );
    // `master` where there is no `main`, and nothing where there is neither
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q", "-b", "master"]);
    commit(&other, "a.txt", "one\n", "first");
    let checkout = Checkout::open(&other).unwrap();
    assert_eq!(
        checkout.default_branch().unwrap().as_deref(),
        Some("master")
    );
    git(&other, &["branch", "-q", "-m", "master", "dev"]);
    let checkout = Checkout::open(&other).unwrap();
    assert_eq!(checkout.default_branch().unwrap(), None);
}

/// merge-base agrees with git's for every pair of commits in a history with a merge, and
/// unrelated histories have none
#[test]
fn merge_base_matches_the_cli() {
    let dir = fresh("merge-base");
    let repo = repo(&dir.join("repo"));
    // main and a branch apart, then merged, then each moving on
    let mut commits = vec![git(&repo, &["rev-parse", "HEAD"])];
    git(&repo, &["checkout", "-q", "-b", "side"]);
    commits.push(commit(&repo, "s.txt", "side\n", "side one"));
    git(&repo, &["checkout", "-q", "main"]);
    commits.push(commit(&repo, "m.txt", "main\n", "main one"));
    git(&repo, &["merge", "-q", "--no-edit", "side"]);
    commits.push(git(&repo, &["rev-parse", "HEAD"]));
    commits.push(commit(&repo, "m.txt", "main two\n", "main two"));
    git(&repo, &["checkout", "-q", "side"]);
    commits.push(commit(&repo, "s.txt", "side two\n", "side two"));
    // and a history of its own
    git(&repo, &["checkout", "-q", "--orphan", "alone"]);
    commits.push(commit(&repo, "o.txt", "alone\n", "alone"));
    let checkout = Checkout::open(&repo).unwrap();
    for a in &commits {
        for b in &commits {
            let expected = answer(&repo, &["merge-base", a, b]);
            assert_eq!(checkout.merge_base(a, b).unwrap(), expected, "{a} {b}");
        }
    }
}

/// The snapshot holds the worktree as it is, staged, unstaged and untracked alike but not
/// what is ignored, and leaves the index as it was, in a linked worktree too
#[test]
fn snapshot_writes_the_worktree_and_leaves_the_index() {
    let dir = fresh("snapshot");
    let repo = repo(&dir.join("repo"));
    commit(&repo, ".gitignore", "target/\n", "ignore");
    commit(&repo, "gone.txt", "gone\n", "to delete");
    // a change staged, one not, a file untracked, one deleted, and one ignored
    std::fs::write(repo.join("a.txt"), "one, staged\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    std::fs::write(repo.join("a.txt"), "one, staged, then edited\n").unwrap();
    std::fs::write(repo.join("new.txt"), "untracked\n").unwrap();
    std::fs::remove_file(repo.join("gone.txt")).unwrap();
    std::fs::create_dir_all(repo.join("target")).unwrap();
    std::fs::write(repo.join("target/out"), "ignored\n").unwrap();
    let status = git(&repo, &["status", "--porcelain"]);
    let staged = git(&repo, &["diff", "--cached", "--name-status"]);
    // the tree holds every file as the worktree has it, and nothing ignored or deleted
    let tree = Checkout::open(&repo).unwrap().snapshot().unwrap();
    let files = git(&repo, &["ls-tree", "-r", "--name-only", &tree]);
    assert_eq!(
        files.lines().collect::<Vec<_>>(),
        [".gitignore", "a.txt", "new.txt"]
    );
    let a = git(&repo, &["cat-file", "blob", &format!("{tree}:a.txt")]);
    assert_eq!(a, "one, staged, then edited");
    // the user's index and staging are as they were, and no copy is left behind
    assert_eq!(git(&repo, &["status", "--porcelain"]), status);
    assert_eq!(git(&repo, &["diff", "--cached", "--name-status"]), staged);
    let git_dir = repo.join(".git");
    let left = std::fs::read_dir(&git_dir).unwrap();
    assert!(
        !left
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("reef-"))
    );
    // a linked worktree's snapshot is its own, from its own index
    let linked = dir.join("linked");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );
    let linked = linked.canonicalize().unwrap();
    std::fs::write(linked.join("b.txt"), "linked only\n").unwrap();
    let checkout = Checkout::open(&linked).unwrap();
    assert_eq!(checkout.root(), linked);
    assert_eq!(checkout.main_root().unwrap(), repo);
    let tree = checkout.snapshot().unwrap();
    let files = git(&repo, &["ls-tree", "-r", "--name-only", &tree]);
    assert_eq!(
        files.lines().collect::<Vec<_>>(),
        [".gitignore", "a.txt", "b.txt", "gone.txt"]
    );
    assert_eq!(git(&linked, &["status", "--porcelain"]), "?? b.txt");
}

/// A pin is a commit of the start's tree whose parent is the start, written as reef, and
/// taking it back puts its ref as it was, but never over a later pin
#[test]
fn pin_reads_back_through_the_cli_and_unpin_puts_it_back() {
    let dir = fresh("pin");
    let repo = repo(&dir.join("repo"));
    let start = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("wip.txt"), "uncommitted\n").unwrap();
    let checkout = Checkout::open(&repo).unwrap();
    let tree = checkout.snapshot().unwrap();
    // the first pin of a node finds no ref there
    let name = "refs/reef/start/central/C30";
    let first = checkout
        .pin(name, &tree, &start, "reef: C30 started")
        .unwrap();
    assert_eq!(first.previous, None);
    assert_eq!(git(&repo, &["rev-parse", name]), first.id);
    assert_eq!(
        git(&repo, &["rev-parse", &format!("{name}^{{tree}}")]),
        tree
    );
    assert_eq!(git(&repo, &["rev-parse", &format!("{name}^")]), start);
    let who = git(
        &repo,
        &["log", "-1", "--format=%an <%ae>|%cn <%ce>|%s", name],
    );
    assert_eq!(
        who,
        "reef <reef@localhost>|reef <reef@localhost>|reef: C30 started"
    );
    assert_eq!(
        checkout.ref_id(name).unwrap().as_deref(),
        Some(first.id.as_str())
    );
    assert_eq!(checkout.tree_of(&first.id).unwrap(), tree);
    // a second pin names the first as what it replaced, and taking it back restores the first
    std::fs::write(repo.join("wip.txt"), "more\n").unwrap();
    let again = checkout.snapshot().unwrap();
    let second = checkout
        .pin(name, &again, &start, "reef: C30 started")
        .unwrap();
    assert_eq!(second.previous.as_deref(), Some(first.id.as_str()));
    checkout.unpin(&second).unwrap();
    assert_eq!(git(&repo, &["rev-parse", name]), first.id);
    // a pin taken back after a later one is refused, leaving the later one
    let later = checkout
        .pin(name, &again, &start, "reef: C30 started again")
        .unwrap();
    assert_ne!(later.id, second.id);
    let stale = Pinned {
        id: second.id.clone(),
        ..second
    };
    assert!(checkout.unpin(&stale).is_err());
    assert_eq!(git(&repo, &["rev-parse", name]), later.id);
    // and a pin of a ref that didn't exist is taken back by deleting it
    checkout.unpin(&later).unwrap();
    checkout.unpin(&first).unwrap();
    assert_eq!(
        answer(&repo, &["rev-parse", "--verify", "--quiet", name]),
        None
    );
    assert!(!checkout.has_ref(name).unwrap());
}

#[path = "support/oracle.rs"]
mod oracle;

/// Writes files into a worktree, making their directories, each from its path and its bytes
///
/// # Arguments
///
/// * `dir` - The worktree
/// * `files` - Each file's path and what it holds
fn write_all(dir: &Path, files: &[(&str, &[u8])]) {
    for (path, bytes) in files {
        // its directory first, as git would check it out
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

/// Commits everything in a worktree, returning the commit
///
/// # Arguments
///
/// * `dir` - The worktree
/// * `message` - The commit's message
fn commit_all(dir: &Path, message: &str) -> String {
    // stage it all, then commit it
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

/// A file of numbered lines, each `line N`, for edits that leave most of it alone
///
/// # Arguments
///
/// * `count` - How many lines
fn numbered(count: usize) -> String {
    (1..=count).map(|n| format!("line {n}\n")).collect()
}

/// The diff gix takes is the one git's CLI shows: every kind of change, between commits and
/// between trees, hunk by hunk and line by line
#[test]
fn diff_matches_git_diff() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = fresh("diff");
    let repo = repo(&dir.join("repo"));
    // a base holding every kind of file a change can touch
    let functions = "fn a() {\n    1\n}\n\nfn b() {\n    2\n}\n";
    write_all(
        &repo,
        &[
            ("edited.txt", numbered(40).as_bytes()),
            ("gone.txt", b"to be deleted\n"),
            ("renamed.txt", numbered(20).as_bytes()),
            ("moved/edited.txt", numbered(30).as_bytes()),
            ("run.sh", b"#!/bin/sh\necho hi\n"),
            ("blob.bin", b"\x00\x01\x02 binary\n"),
            ("crlf.txt", b"one\r\ntwo\r\nthree\r\n"),
            ("no newline.txt", b"first\nlast"),
            ("slide.rs", functions.as_bytes()),
            ("deep/er/still/é.txt", b"unicode\n"),
        ],
    );
    symlink("edited.txt", repo.join("link")).unwrap();
    let base = commit_all(&repo, "base");
    // then a change of every kind
    let mut edited = numbered(40).replace("line 3\n", "line three\n");
    edited = edited.replace("line 30\n", "line 30\nline 30.5\n");
    let slid = "fn a() {\n    1\n}\n\nfn c() {\n    3\n}\n\nfn b() {\n    2\n}\n";
    write_all(
        &repo,
        &[
            ("edited.txt", edited.as_bytes()),
            (
                "moved/edited.txt",
                numbered(30).replace("line 2\n", "line two\n").as_bytes(),
            ),
            ("blob.bin", b"\x00\x01\x03 binary, changed\n"),
            ("crlf.txt", b"one\r\n2\r\nthree\r\n"),
            ("no newline.txt", b"first\nlast, still no newline"),
            ("slide.rs", slid.as_bytes()),
            ("new/added.txt", b"a new file\n"),
            ("deep/er/still/é.txt", b"unicode, edited\n"),
        ],
    );
    std::fs::remove_file(repo.join("gone.txt")).unwrap();
    std::fs::rename(repo.join("renamed.txt"), repo.join("now named.txt")).unwrap();
    std::fs::rename(
        repo.join("moved/edited.txt"),
        repo.join("moved/edited too.txt"),
    )
    .unwrap();
    let mut mode = std::fs::metadata(repo.join("run.sh"))
        .unwrap()
        .permissions();
    mode.set_mode(0o755);
    std::fs::set_permissions(repo.join("run.sh"), mode).unwrap();
    std::fs::remove_file(repo.join("link")).unwrap();
    symlink("now named.txt", repo.join("link")).unwrap();
    let head = commit_all(&repo, "head");
    // between the commits, and between their trees, gix's diff is git's
    let checkout = Checkout::open(&repo).unwrap();
    let ours = checkout.diff(&base, &head).unwrap();
    let theirs = oracle::git_diff(&repo, &base, &head);
    assert_eq!(ours, theirs);
    let base_tree = checkout.tree_of(&base).unwrap();
    let head_tree = checkout.tree_of(&head).unwrap();
    assert_eq!(checkout.diff(&base_tree, &head_tree).unwrap(), theirs);
    // and it holds every kind of change it was given
    let path = |path: &str| ours.iter().find(|file| file.path() == path).unwrap();
    assert_eq!(path("gone.txt").status, reef_api::FileStatus::Deleted);
    assert_eq!(path("now named.txt").status, reef_api::FileStatus::Renamed);
    assert_eq!(
        path("moved/edited too.txt").status,
        reef_api::FileStatus::Renamed
    );
    assert_eq!(path("new/added.txt").status, reef_api::FileStatus::Added);
    assert_eq!(path("run.sh").new_mode.as_deref(), Some("100755"));
    assert_eq!(path("link").new_mode.as_deref(), Some("120000"));
    assert!(path("blob.bin").binary && path("blob.bin").hunks.is_empty());
    assert_eq!(path("edited.txt").hunks.len(), 2);
    assert!(
        path("crlf.txt").hunks[0]
            .lines
            .iter()
            .any(|line| line.text == "2\r")
    );
    let last = path("no newline.txt").hunks[0].lines.last().unwrap();
    assert!(last.no_newline, "{last:?}");
    // a diff of a tree with itself is empty
    assert!(checkout.diff(&head, &head).unwrap().is_empty());
}

/// A merge of trees through gix writes the tree `git merge-tree --write-tree` writes, and names
/// the same conflicted files: a clean merge, a conflict on one line, a delete against an edit,
/// an add on both sides, and a rename against an edit
#[test]
fn merge_trees_writes_git_merge_trees_tree() {
    let dir = fresh("merge");
    let repo = repo(&dir.join("repo"));
    write_all(
        &repo,
        &[
            ("shared.txt", numbered(20).as_bytes()),
            ("conflict.txt", b"one\ntwo\nthree\n"),
            ("deleted.txt", b"keep me\n"),
            ("renamed.txt", numbered(20).as_bytes()),
        ],
    );
    let base = commit_all(&repo, "base");
    // each case: what our side and their side change, from the same base, `None` deleting
    type Side = Vec<(&'static str, Option<Vec<u8>>)>;
    let lines = |from: &str, to: &str| Some(numbered(20).replace(from, to).into_bytes());
    let text = |text: &str| Some(text.as_bytes().to_vec());
    let cases: [(&str, Side, Side); 5] = [
        (
            "clean",
            vec![("shared.txt", lines("line 2\n", "two\n"))],
            vec![("shared.txt", lines("line 19\n", "nineteen\n"))],
        ),
        (
            "one-line",
            vec![("conflict.txt", text("one\nours\nthree\n"))],
            vec![("conflict.txt", text("one\ntheirs\nthree\n"))],
        ),
        (
            "delete-edit",
            vec![("deleted.txt", None)],
            vec![("deleted.txt", text("edited\n"))],
        ),
        (
            "add-add",
            vec![("both.txt", text("ours\n"))],
            vec![("both.txt", text("theirs\n"))],
        ),
        (
            "rename-edit",
            vec![
                ("renamed.txt", None),
                ("now renamed.txt", Some(numbered(20).into_bytes())),
            ],
            vec![("renamed.txt", lines("line 10\n", "ten\n"))],
        ),
    ];
    let checkout = Checkout::open(&repo).unwrap();
    let base_tree = checkout.tree_of(&base).unwrap();
    for (name, ours, theirs) in cases {
        // each side a commit off the base, read as its tree
        let side = |files: Side, label: &str| -> String {
            git(&repo, &["checkout", "-q", "--detach", &base]);
            for (path, bytes) in files {
                match bytes {
                    Some(bytes) => write_all(&repo, &[(path, bytes.as_slice())]),
                    None => std::fs::remove_file(repo.join(path)).unwrap(),
                }
            }
            let commit = commit_all(&repo, &format!("{name}: {label}"));
            checkout.tree_of(&commit).unwrap()
        };
        let ours = side(ours, "ours");
        let theirs = side(theirs, "theirs");
        let merged = checkout.merge_trees(&base_tree, &ours, &theirs).unwrap();
        let (tree, conflicts) = oracle::git_merge_tree(&repo, &base_tree, &ours, &theirs);
        assert_eq!(merged.tree, tree, "{name}: gix wrote another tree");
        assert_eq!(
            merged.conflicts, conflicts,
            "{name}: gix named other conflicts"
        );
        // only the clean merge and the rename against an edit merged cleanly
        let clean = matches!(name, "clean" | "rename-edit");
        assert_eq!(
            merged.conflicts.is_empty(),
            clean,
            "{name}: {:?}",
            merged.conflicts
        );
    }
}

/// A seal takes every tracked file as the worktree has it and a staged new one, leaves out
/// untracked files unless its base holds them, and leaves the user's index as it was
#[test]
fn seal_snapshot_takes_what_is_tracked() {
    let dir = fresh("seal");
    let repo = repo(&dir.join("repo"));
    write_all(
        &repo,
        &[(".gitignore", b"*.log\n"), ("b.txt", b"to be deleted\n")],
    );
    commit_all(&repo, "base");
    // a start that took untracked files along, one named like a pattern
    write_all(
        &repo,
        &[
            ("left.txt", b"left by an earlier node\n"),
            ("w*.txt", b"a star in its name\n"),
        ],
    );
    let checkout = Checkout::open(&repo).unwrap();
    let base = checkout.snapshot().unwrap();
    // then the node's work: tracked files edited and deleted, a new file staged, the start's
    // leftovers edited, and untracked and ignored files it never added
    write_all(
        &repo,
        &[
            ("a.txt", b"one, edited\n"),
            ("c.txt", b"staged\n"),
            ("left.txt", b"left, and edited\n"),
            ("wild.txt", b"matches w*.txt as a pattern\n"),
            ("__pycache__/x.cpython-312.pyc", b"\x00bytecode"),
            ("debug.log", b"ignored\n"),
        ],
    );
    std::fs::remove_file(repo.join("b.txt")).unwrap();
    git(&repo, &["add", "c.txt"]);
    let status = git(&repo, &["status", "--porcelain", "--untracked-files=all"]);
    // the seal holds what is tracked and what the base held, as the worktree has them
    let sealed = checkout.seal_snapshot(&base).unwrap();
    let files = git(&repo, &["ls-tree", "-r", "--name-only", &sealed]);
    assert_eq!(
        files.lines().collect::<Vec<_>>(),
        [".gitignore", "a.txt", "c.txt", "left.txt", "w*.txt"]
    );
    let blob = |path: &str| git(&repo, &["cat-file", "blob", &format!("{sealed}:{path}")]);
    assert_eq!(blob("a.txt"), "one, edited");
    assert_eq!(blob("left.txt"), "left, and edited");
    // so its diff from the base is the node's own, and git's CLI shows the same
    let diff = checkout.diff(&base, &sealed).unwrap();
    let paths: Vec<&str> = diff.iter().map(|file| file.path()).collect();
    assert_eq!(paths, ["a.txt", "b.txt", "c.txt", "left.txt"]);
    assert_eq!(diff, oracle::git_diff(&repo, &base, &sealed));
    // and the user's index and staging are as they were, with no copy left behind
    assert_eq!(
        git(&repo, &["status", "--porcelain", "--untracked-files=all"]),
        status
    );
    let left = std::fs::read_dir(repo.join(".git")).unwrap();
    assert!(
        !left
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("reef-"))
    );
}
