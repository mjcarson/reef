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
