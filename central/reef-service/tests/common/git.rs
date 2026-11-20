//! Git repos and worktrees for the suite, made under cargo's target dir
//!
//! Every command runs from its directory alone, whatever repo the tests themselves run in, and
//! names its own author, so no one's git config is needed.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs git with its arguments in a directory, failing on anything but success
///
/// # Arguments
///
/// * `dir` - The directory to run in
/// * `args` - Git's arguments
fn git(dir: &Path, args: &[&str]) -> anyhow::Result<()> {
    // from the directory alone, as this test's author, with no one's config
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=reef",
            "-c",
            "user.email=reef@example.invalid",
        ])
        .args(["-c", "init.defaultBranch=main"])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "git {} in {}: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

/// Makes a fresh repo with one commit, so a worktree can be added from it
///
/// # Arguments
///
/// * `dir` - Where to make it, wiped first
pub fn repo(dir: &Path) -> anyhow::Result<PathBuf> {
    // wipe what an earlier run left, and start an empty repo there
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    git(dir, &["init", "-q"])?;
    // a worktree needs a commit to check out
    git(dir, &["commit", "-q", "--allow-empty", "-m", "start"])?;
    Ok(dir.canonicalize()?)
}

/// Adds a linked worktree of a repo, on a branch of its own
///
/// # Arguments
///
/// * `repo` - The main worktree
/// * `dir` - Where to add the linked one, which must not exist
pub fn worktree(repo: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    // a branch named for the worktree, so two never share one
    let branch = dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("{} has no name", dir.display()))?;
    let path = dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("{} is not a utf-8 path", dir.display()))?;
    git(repo, &["worktree", "add", "-q", "-b", branch, path])?;
    Ok(dir.canonicalize()?)
}
