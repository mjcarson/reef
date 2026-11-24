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
