//! Where a directory's repo is: its main worktree's root, or the directory itself outside git
//!
//! A project names the repo its plans describe, and a working directory picks the project whose
//! repo holds it. A linked worktree (`git worktree add`) belongs to its main repo's project, so
//! a directory in one is located at the main worktree's root.
//!
//! This asks git rather than reading `.git` files itself, so `gitdir:` files, `commondir` and
//! relative paths are all git's to resolve. It runs a process, so callers in async code run it
//! through `spawn_blocking`. It touches no shoal, so the CLI can use it too.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The environment git reads a repo from instead of the directory it runs in
pub(crate) const GIT_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_PREFIX",
    "GIT_NAMESPACE",
];

/// Where a directory was located
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Located {
    /// In a git repo, whose main worktree is rooted here
    Repo(PathBuf),
    /// Outside any git repo, at this canonical path
    Dir(PathBuf),
}

impl Located {
    /// The path located
    pub fn path(&self) -> &Path {
        match self {
            Located::Repo(path) | Located::Dir(path) => path,
        }
    }

    /// Gives up which kind of place this was, keeping its path
    pub fn into_path(self) -> PathBuf {
        match self {
            Located::Repo(path) | Located::Dir(path) => path,
        }
    }
}

/// Locates a directory: its repo's main worktree root, or its canonical path outside git
///
/// # Arguments
///
/// * `dir` - The directory to locate, which must exist
pub fn locate(dir: &Path) -> anyhow::Result<Located> {
    // resolve symlinks and `..`, so two spellings of one directory are one path
    let dir = dir
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("{}: {error}", dir.display()))?;
    // ask git where its worktree and its common dir are, if this is in one
    match git_roots(&dir)? {
        Some((toplevel, common)) => Ok(Located::Repo(main_root(toplevel, &common))),
        None => Ok(Located::Dir(dir)),
    }
}

/// The root of the main worktree, given a worktree's root and the repo's common dir
///
/// A non-bare repo's common dir is the main worktree's `.git`, so a linked worktree maps to the
/// directory holding it. Any other common dir, a bare repo's or a submodule's under its
/// superproject's `.git/modules`, leaves the worktree as it is.
///
/// # Arguments
///
/// * `toplevel` - The root of the worktree the directory is in
/// * `common` - The repo's common dir
fn main_root(toplevel: PathBuf, common: &Path) -> PathBuf {
    // `<main>/.git` names the main worktree as its parent
    if common.file_name().is_some_and(|name| name == ".git")
        && let Some(main) = common.parent()
    {
        return main.to_path_buf();
    }
    toplevel
}

/// Asks git for the worktree root and common dir of the repo a directory is in
///
/// Returns `None` outside a git repo, or when git isn't installed.
///
/// # Arguments
///
/// * `dir` - The canonical directory to ask about
fn git_roots(dir: &Path) -> anyhow::Result<Option<(PathBuf, PathBuf)>> {
    // both paths absolute, and from the directory alone, whatever this process's environment
    let mut git = Command::new("git");
    git.arg("-C").arg(dir).args([
        "rev-parse",
        "--path-format=absolute",
        "--show-toplevel",
        "--git-common-dir",
    ]);
    for var in GIT_ENV {
        git.env_remove(var);
    }
    let output = match git.output() {
        Ok(output) => output,
        // with no git there are no repos, only directories
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => anyhow::bail!("running git in {}: {error}", dir.display()),
    };
    // git refuses a directory outside any repo, which is an answer, not a failure
    if !output.status.success() {
        return Ok(None);
    }
    // one path a line: the worktree's root, then the common dir
    let text = String::from_utf8(output.stdout)
        .map_err(|_| anyhow::anyhow!("git named a path in {} that isn't utf-8", dir.display()))?;
    let mut lines = text.lines();
    match (lines.next(), lines.next()) {
        (Some(toplevel), Some(common)) => {
            // canonical too, so a symlinked checkout is one path however it was reached
            let toplevel = PathBuf::from(toplevel).canonicalize()?;
            let common = PathBuf::from(common).canonicalize()?;
            Ok(Some((toplevel, common)))
        }
        // a bare repo has no worktree, so it holds no working directory
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::main_root;
    use std::path::{Path, PathBuf};

    /// A linked worktree maps to its main worktree, and anything else stays where it is
    #[test]
    fn main_root_follows_a_non_bare_common_dir() {
        // the main worktree, and a linked one whose common dir is the main's `.git`
        let main = PathBuf::from("/repos/reef");
        assert_eq!(main_root(main.clone(), Path::new("/repos/reef/.git")), main);
        assert_eq!(
            main_root("/repos/reef-wt".into(), Path::new("/repos/reef/.git")),
            main
        );
        // a submodule, whose common dir is under its superproject's `.git/modules`
        let sub = PathBuf::from("/repos/reef/vendor/shoal");
        let modules = Path::new("/repos/reef/.git/modules/shoal");
        assert_eq!(main_root(sub.clone(), modules), sub);
    }
}
