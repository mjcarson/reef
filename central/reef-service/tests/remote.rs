//! A remote project's docs, read from git's objects at one commit of its bare clone
//!
//! Each case makes a repo with git's CLI, clones it bare as the service would keep it, and
//! reads it through [`TreeSource`], which must keep the rules a working tree's docs keep: no
//! path that climbs out, and no symlink followed out of the repo.

// an in-process pool's open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef_render::DocSource;
use reef_service::tree::{Docs, TreeSource};
use std::path::{Path, PathBuf};

use common::git;

/// A repo of docs, and a bare clone of it, made fresh for a case
///
/// Returns the working repo and the clone.
///
/// # Arguments
///
/// * `case` - The case's name, which names its directory
fn cloned(case: &str) -> anyhow::Result<(PathBuf, PathBuf)> {
    // under cargo's target dir, wiped by `git::repo`
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("trees")
        .join(case);
    let work = git::repo(&dir.join("work"))?;
    Ok((work, dir.join("clone.git")))
}

/// Clones a repo bare, as the service keeps a remote project's
///
/// # Arguments
///
/// * `work` - The repo
/// * `clone` - Where the clone goes, wiped first
fn clone_bare(work: &Path, clone: &Path) -> anyhow::Result<()> {
    // whatever an earlier run left there goes first
    if clone.exists() {
        std::fs::remove_dir_all(clone)?;
    }
    let to = clone.to_str().expect("the clone is a utf-8 path");
    git::run(work, &["clone", "-q", "--bare", ".", to])?;
    Ok(())
}

/// A doc is read as its commit holds it, not as a later commit does
#[test]
fn tree_source_reads_a_commits_blobs() -> anyhow::Result<()> {
    // two commits of one doc, and a directory beside it
    let (work, clone) = cloned("reads")?;
    git::write(&work, "docs/design.md", "# Design\n\nThe old words.\n")?;
    git::write(&work, "docs/deep/other.md", "# Other\n")?;
    git::run(&work, &["add", "."])?;
    git::run(&work, &["commit", "-q", "-m", "old"])?;
    let old = git::run(&work, &["rev-parse", "HEAD"])?;
    git::write(&work, "docs/design.md", "# Design\n\nThe new words.\n")?;
    let new = git::commit(
        &work,
        "docs/design.md",
        "# Design\n\nThe new words.\n",
        "new",
    )?;
    clone_bare(&work, &clone)?;
    // each commit reads its own words
    let at_old = TreeSource::open(&clone, &old)?;
    let at_new = TreeSource::open(&clone, &new)?;
    assert!(
        at_old
            .read("docs/design.md")
            .unwrap()
            .contains("The old words")
    );
    assert!(
        at_new
            .read("docs/design.md")
            .unwrap()
            .contains("The new words")
    );
    // a doc in a deeper directory, and one no commit has, or a directory, read as nothing
    assert_eq!(
        at_old.read("docs/deep/other.md").as_deref(),
        Some("# Other\n")
    );
    assert_eq!(at_old.read("docs/missing.md"), None);
    assert_eq!(at_old.read("docs"), None);
    assert_eq!(at_old.read("docs/design.md/more"), None);
    // a path is read as a working tree would read it, its `.` and empty parts dropped
    assert!(at_old.read("./docs//design.md").is_some());
    // headings come from the doc's body, and where a doc is looked for is its path
    let headings = at_old
        .headings("docs/design.md")
        .expect("the doc has headings");
    assert!(
        headings.iter().any(|(id, _, _)| id == "design"),
        "{headings:?}"
    );
    assert_eq!(
        at_old.place("docs/design.md").as_deref(),
        Ok("docs/design.md")
    );
    // the switch the service reads through reads the same
    let docs = Docs::Commit {
        clone: clone.clone(),
        commit: old.clone(),
    };
    assert_eq!(
        docs.source()?.read("docs/design.md"),
        at_old.read("docs/design.md")
    );
    // and a commit the clone doesn't have, or a clone that isn't one, is an error naming it
    let missing = TreeSource::open(&clone, &"0".repeat(40))
        .err()
        .expect("a missing commit");
    assert!(missing.to_string().contains("clone.git"), "{missing}");
    let nowhere = work.join("nowhere.git");
    let unopened = TreeSource::open(&nowhere, &old)
        .err()
        .expect("a clone that isn't there");
    assert!(unopened.to_string().contains("nowhere.git"), "{unopened}");
    Ok(())
}

/// No path that leaves the repo is read, nor a symlink that leads out of it or never ends, nor
/// a submodule, while a symlink that stays inside is followed
#[test]
fn tree_source_refuses_paths_that_leave_and_links_out() -> anyhow::Result<()> {
    // a doc, symlinks to it, out of the repo and in a loop, and a submodule
    let (work, clone) = cloned("refuses")?;
    git::write(&work, "docs/design.md", "# Design\n")?;
    git::write(&work, "../outside.md", "# Outside\n")?;
    let link = |target: &str, path: &str| std::os::unix::fs::symlink(target, work.join(path));
    link("design.md", "docs/link.md")?;
    link("docs", "guide")?;
    link("../outside.md", "out.md")?;
    link("/etc/hostname", "absolute.md")?;
    link("../../outside.md", "docs/climbs.md")?;
    link("second.md", "first.md")?;
    link("first.md", "second.md")?;
    std::fs::write(work.join("binary.md"), [0xff, 0xfe, 0x00])?;
    git::run(&work, &["add", "."])?;
    let start = git::run(&work, &["rev-parse", "HEAD"])?;
    let gitlink = format!("160000,{start},vendor");
    git::run(&work, &["update-index", "--add", "--cacheinfo", &gitlink])?;
    git::run(&work, &["commit", "-q", "-m", "links"])?;
    let commit = git::run(&work, &["rev-parse", "HEAD"])?;
    clone_bare(&work, &clone)?;
    let tree = TreeSource::open(&clone, &commit)?;
    // a path that climbs out, or starts from the root, is refused as written
    for path in ["../outside.md", "docs/../../outside.md", "/etc/hostname"] {
        assert_eq!(tree.read(path), None, "{path}");
        assert_eq!(
            tree.place(path),
            Err("leaves the repo".to_string()),
            "{path}"
        );
    }
    // a symlink to a file or a directory inside the repo is followed
    assert_eq!(tree.read("docs/link.md").as_deref(), Some("# Design\n"));
    assert_eq!(tree.read("guide/design.md").as_deref(), Some("# Design\n"));
    assert_eq!(tree.read("guide/link.md").as_deref(), Some("# Design\n"));
    // one that leads out, by climbing or from the root, or round in a loop, is no doc
    for path in ["out.md", "docs/climbs.md", "absolute.md", "first.md"] {
        assert_eq!(tree.read(path), None, "{path}");
        assert!(tree.place(path).is_ok(), "{path}");
    }
    // a submodule's files are another repo's, and a blob that isn't text is no doc
    assert_eq!(tree.read("vendor"), None);
    assert_eq!(tree.read("vendor/docs/design.md"), None);
    assert_eq!(tree.read("binary.md"), None);
    Ok(())
}

/// A directory at a commit lists its blobs as files and its trees as folders, each symlink as
/// what it leads to, and leaves out a link out of the tree, a submodule and a dotfile
#[test]
fn tree_source_lists_a_directory_as_a_working_tree_would() -> anyhow::Result<()> {
    // make a work tree and the path its bare clone will go to
    let (work, clone) = cloned("lists")?;
    // write a file, a folder, a dotfile, and a file outside the work tree
    git::write(&work, "docs/design.md", "# Design\n")?;
    git::write(&work, "docs/guide/setup.md", "# Setup\n")?;
    git::write(&work, "docs/.hidden.md", "# Hidden\n")?;
    git::write(&work, "../outside.md", "# Outside\n")?;
    // link to the file, to the folder, out of the tree, and to nothing at all
    let link = |target: &str, path: &str| std::os::unix::fs::symlink(target, work.join(path));
    link("design.md", "docs/link.md")?;
    link("guide", "docs/also")?;
    link("../../outside.md", "docs/out.md")?;
    link("missing.md", "docs/dangling.md")?;
    // stage it all, then add a submodule's gitlink at the commit it started from
    git::run(&work, &["add", "."])?;
    let start = git::run(&work, &["rev-parse", "HEAD"])?;
    let gitlink = format!("160000,{start},docs/vendor");
    git::run(&work, &["update-index", "--add", "--cacheinfo", &gitlink])?;
    // commit it and open its tree from a bare clone, as a remote project's docs are read
    git::run(&work, &["commit", "-q", "-m", "a folder"])?;
    let commit = git::run(&work, &["rev-parse", "HEAD"])?;
    clone_bare(&work, &clone)?;
    let tree = TreeSource::open(&clone, &commit)?;
    // list the folder by name, each as a file or folder
    let listed: Vec<(String, bool)> = tree
        .list("docs")
        .into_iter()
        .map(|entry| (entry.name, entry.folder))
        .collect();
    // expect each link as what it leads to, and no link out, dangling link, dotfile or submodule
    let want = [
        ("also", true),
        ("design.md", false),
        ("guide", true),
        ("link.md", false),
    ];
    let want: Vec<(String, bool)> = want
        .into_iter()
        .map(|(name, folder)| (name.to_string(), folder))
        .collect();
    assert_eq!(listed, want);
    // make sure the root lists its folders too
    let root: Vec<String> = tree.list("").into_iter().map(|entry| entry.name).collect();
    assert!(root.contains(&"docs".to_string()), "{root:?}");
    // make sure a folder reached through a link lists as the folder it leads to
    assert_eq!(tree.list("docs/also"), tree.list("docs/guide"));
    // make sure a file, a path that leaves, a missing path and a submodule list nothing
    for dir in ["docs/design.md", "..", "/etc", "docs/gone", "docs/vendor"] {
        assert!(tree.list(dir).is_empty(), "{dir}");
    }
    // make sure the docs a commit opens as list the folder the same way
    let docs = Docs::Commit {
        clone: clone.clone(),
        commit,
    };
    assert_eq!(docs.source()?.list("docs"), tree.list("docs"));
    Ok(())
}

/// Several branches fetch into one shallow clone, each over its own ref, so each plan's docs
/// read at its own branch's tip, and a branch the remote hasn't fails alone
#[tokio::test]
async fn several_branches_fetch_into_one_clone() -> anyhow::Result<()> {
    // a repo whose `main` and `b2` each hold a doc of their own
    let (work, clone) = cloned("branches")?;
    let main = git::run(&work, &["branch", "--show-current"])?;
    git::write(&work, "docs/main.md", "# Main\n")?;
    git::run(&work, &["add", "."])?;
    git::run(&work, &["commit", "-q", "-m", "main"])?;
    let main_tip = git::run(&work, &["rev-parse", "HEAD"])?;
    git::run(&work, &["checkout", "-q", "-b", "b2"])?;
    git::run(&work, &["rm", "-q", "docs/main.md"])?;
    git::write(&work, "docs/b2.md", "# B2\n")?;
    git::run(&work, &["add", "."])?;
    git::run(&work, &["commit", "-q", "-m", "b2"])?;
    let b2_tip = git::run(&work, &["rev-parse", "HEAD"])?;
    // a clone made as the service makes one, fetching `main`, then `b2` beside it
    if clone.exists() {
        std::fs::remove_dir_all(&clone)?;
    }
    let url = format!("file://{}", work.display());
    reef_service::remote::make(&clone, &main).await?;
    for branch in [main.as_str(), "b2"] {
        reef_service::remote::fetch(&clone, &url, branch).await?;
    }
    assert_eq!(reef_service::remote::tip(&clone, &main).await?, main_tip);
    assert_eq!(reef_service::remote::tip(&clone, "b2").await?, b2_tip);
    // each branch's tip reads its own docs, and not the other's
    let at_main = TreeSource::open(&clone, &main_tip)?;
    let at_b2 = TreeSource::open(&clone, &b2_tip)?;
    assert_eq!(at_main.read("docs/main.md").as_deref(), Some("# Main\n"));
    assert_eq!(at_main.read("docs/b2.md"), None);
    assert_eq!(at_b2.read("docs/b2.md").as_deref(), Some("# B2\n"));
    assert_eq!(at_b2.read("docs/main.md"), None);
    // a branch the remote hasn't fails, and leaves the others where they were
    assert!(
        reef_service::remote::fetch(&clone, &url, "nope")
            .await
            .is_err()
    );
    assert_eq!(reef_service::remote::tip(&clone, "b2").await?, b2_tip);
    Ok(())
}

/// A local project's plan reads a worktree with its branch checked out, else the branch's tip
/// from the repo's objects, and a branch since deleted says how to follow another
#[test]
fn a_local_branch_reads_its_worktree_else_its_tip() -> anyhow::Result<()> {
    // a repo on its first branch, and `b2` beside it with a doc of its own
    let (work, _) = cloned("local-branch")?;
    git::run(&work, &["checkout", "-q", "-b", "b2"])?;
    git::write(&work, "docs/b2.md", "# B2\n")?;
    git::run(&work, &["add", "."])?;
    git::run(&work, &["commit", "-q", "-m", "b2"])?;
    git::run(&work, &["checkout", "-q", "-"])?;
    // `b2` isn't checked out, so its tip is read, and an uncommitted file is no doc of it
    git::write(&work, "docs/loose.md", "# Loose\n")?;
    let docs = Docs::LocalBranch {
        repo: work.clone(),
        branch: "b2".to_string(),
    };
    let source = docs.source()?;
    assert_eq!(source.read("docs/b2.md").as_deref(), Some("# B2\n"));
    assert_eq!(source.read("docs/loose.md"), None);
    // checked out in the main worktree, its files are read as they stand
    git::run(&work, &["checkout", "-q", "b2"])?;
    let source = docs.source()?;
    assert_eq!(source.read("docs/loose.md").as_deref(), Some("# Loose\n"));
    // and a branch since deleted names how to follow another
    let gone = Docs::LocalBranch {
        repo: work,
        branch: "gone".to_string(),
    };
    let error = gone.source().err().expect("a deleted branch can't be read");
    assert!(error.to_string().contains("reef plan branch"), "{error}");
    Ok(())
}
