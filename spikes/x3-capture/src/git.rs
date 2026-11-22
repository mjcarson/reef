//! git through its CLI, run in one checkout and reading only what is local to it
//!
//! Nothing here fetches, pushes or names a remote. Every command runs with `-C` on the checkout
//! it was made for, and the variables that would point git at another repo or index are
//! removed, as `central/reef-service/src/repo.rs` does, so a git hook's environment can't
//! redirect a capture.

use anyhow::Context;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tracing::instrument;

use crate::diff::{Diff, parse_git};

/// The environment git reads a repo from instead of the directory it runs in
const GIT_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_PREFIX",
];

/// The trailer a commit names its node with, if a repo uses them
pub const NODE_TRAILER: &str = "Reef-Node";

/// The options every diff is taken with, so a user's config can't change what is captured
///
/// Renames at git's default 50%, Myers with the indent heuristic, three lines of context, full
/// ids, and no external diff or textconv driver.
const DIFF_ARGS: &[&str] = &[
    "-c",
    "core.quotePath=false",
    "diff",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--find-renames=50%",
    "--diff-algorithm=myers",
    "--indent-heuristic",
    "--unified=3",
    "--raw",
    "-z",
    "-p",
    "--no-abbrev",
    "--full-index",
];

/// How a three-way merge of trees came out
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The merged tree, with conflict markers in any file that conflicted
    pub tree: String,
    /// The files that conflicted, if any
    pub conflicts: Vec<String>,
}

/// One checkout's git, through the CLI
#[derive(Debug, Clone)]
pub struct Git {
    /// The checkout's directory, or any directory inside it
    dir: PathBuf,
    /// Whether to ignore the user's and the system's git config, as the tests' repos do
    isolated: bool,
}

impl Git {
    /// Uses git in a checkout, with the user's own config
    ///
    /// # Arguments
    ///
    /// * `dir` - The checkout, or any directory inside it
    pub fn new<P: Into<PathBuf>>(dir: P) -> Self {
        Git {
            dir: dir.into(),
            isolated: false,
        }
    }

    /// Uses git in a checkout without any global or system config, and with a fixed author
    ///
    /// # Arguments
    ///
    /// * `dir` - The checkout, or any directory inside it
    pub fn isolated<P: Into<PathBuf>>(dir: P) -> Self {
        Git {
            dir: dir.into(),
            isolated: true,
        }
    }

    /// The same git, in another directory
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory to run in instead
    pub fn at<P: Into<PathBuf>>(&self, dir: P) -> Self {
        Git {
            dir: dir.into(),
            isolated: self.isolated,
        }
    }

    /// The directory this runs in
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A git command in this checkout, its environment cleared of anything naming another repo
    pub fn command(&self) -> Command {
        let mut git = Command::new("git");
        git.arg("-C").arg(&self.dir);
        // nothing may point this command at a repo other than the checkout's own
        for var in GIT_ENV {
            git.env_remove(var);
        }
        // the C locale keeps git's messages and output stable
        git.env("LC_ALL", "C");
        // the tests' repos ignore whatever config the machine has
        if self.isolated {
            git.env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "x3")
                .env("GIT_AUTHOR_EMAIL", "x3@localhost")
                .env("GIT_COMMITTER_NAME", "x3")
                .env("GIT_COMMITTER_EMAIL", "x3@localhost");
        }
        git
    }

    /// Runs a command, failing with its stderr if it fails
    ///
    /// # Arguments
    ///
    /// * `git` - The command to run
    /// * `what` - What it was, for the error
    fn output(git: &mut Command, what: &str) -> anyhow::Result<Output> {
        let output = git
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("running git {what}"))?;
        // a failure carries git's own message
        if !output.status.success() {
            anyhow::bail!(
                "git {what} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(output)
    }

    /// Runs git with these arguments and returns its stdout, trimmed
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments after `git -C <dir>`
    pub fn run(&self, args: &[&str]) -> anyhow::Result<String> {
        let output = Self::output(self.command().args(args), &args.join(" "))?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// Runs git with these arguments and environment, returning its stdout, trimmed
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments after `git -C <dir>`
    /// * `env` - Extra variables to set
    pub fn run_env(&self, args: &[&str], env: &[(&str, &str)]) -> anyhow::Result<String> {
        let mut git = self.command();
        git.args(args);
        for (key, value) in env {
            git.env(key, value);
        }
        let output = Self::output(&mut git, &args.join(" "))?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// Runs git and says only whether it succeeded, for commands whose exit code is the answer
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments after `git -C <dir>`
    pub fn succeeds(&self, args: &[&str]) -> anyhow::Result<bool> {
        let status = self
            .command()
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .with_context(|| format!("running git {}", args.join(" ")))?;
        Ok(status.success())
    }

    /// The root of the worktree this runs in
    pub fn toplevel(&self) -> anyhow::Result<PathBuf> {
        let root = self.run(&["rev-parse", "--show-toplevel"])?;
        Ok(PathBuf::from(root).canonicalize()?)
    }

    /// The branch checked out here, if HEAD isn't detached
    pub fn branch(&self) -> anyhow::Result<Option<String>> {
        // `symbolic-ref` fails on a detached HEAD, which is an answer
        let mut git = self.command();
        git.args(["symbolic-ref", "--quiet", "--short", "HEAD"]);
        let output = git.stdin(Stdio::null()).output()?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ))
    }

    /// Resolves a revision to a commit, if it names one that exists
    ///
    /// # Arguments
    ///
    /// * `rev` - The revision, such as HEAD or a SHA
    pub fn commit(&self, rev: &str) -> anyhow::Result<Option<String>> {
        // `--verify --quiet` answers nothing for a name that doesn't resolve
        let spec = format!("{rev}^{{commit}}");
        let mut git = self.command();
        git.args(["rev-parse", "--verify", "--quiet", &spec]);
        let output = git.stdin(Stdio::null()).output()?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ))
    }

    /// Whether an object exists in this repo at all
    ///
    /// # Arguments
    ///
    /// * `id` - The object's id
    pub fn exists(&self, id: &str) -> anyhow::Result<bool> {
        self.succeeds(&["cat-file", "-e", id])
    }

    /// The tree a commit or tree-ish holds
    ///
    /// # Arguments
    ///
    /// * `rev` - The commit or tree
    pub fn tree(&self, rev: &str) -> anyhow::Result<String> {
        self.run(&["rev-parse", &format!("{rev}^{{tree}}")])
    }

    /// The best common ancestor of two commits, if they have one
    ///
    /// # Arguments
    ///
    /// * `a` - One commit
    /// * `b` - The other
    pub fn merge_base(&self, a: &str, b: &str) -> anyhow::Result<Option<String>> {
        // no common ancestor is exit 1 with no output, not a failure
        let mut git = self.command();
        git.args(["merge-base", a, b]);
        let output = git.stdin(Stdio::null()).output()?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ))
    }

    /// Whether `ancestor` is `head` or one of its ancestors
    ///
    /// # Arguments
    ///
    /// * `ancestor` - The commit that may be an ancestor
    /// * `head` - The commit whose history is searched
    pub fn is_ancestor(&self, ancestor: &str, head: &str) -> anyhow::Result<bool> {
        self.succeeds(&["merge-base", "--is-ancestor", ancestor, head])
    }

    /// Where a file of this worktree's git dir is, as an absolute path
    ///
    /// # Arguments
    ///
    /// * `name` - The file under the git dir, such as `index`
    pub fn git_path(&self, name: &str) -> anyhow::Result<PathBuf> {
        let path = self.run(&["rev-parse", "--path-format=absolute", "--git-path", name])?;
        Ok(PathBuf::from(path))
    }

    /// Whether a rebase, a merge, a cherry-pick or a revert is under way here
    pub fn mid_operation(&self) -> anyhow::Result<bool> {
        // each leaves a marker in the worktree's git dir until it ends
        for marker in [
            "rebase-merge",
            "rebase-apply",
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
        ] {
            if self.git_path(marker)?.exists() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Writes the worktree as it is now, committed or not, as one tree
    ///
    /// The worktree's index is copied and everything is added to the copy, untracked files
    /// too and ignored ones not, so the user's own index and staging are left as they were.
    /// The tree is written to the object store, where git can diff it like any other.
    #[instrument(name = "Git::snapshot", skip_all, err(Debug))]
    pub fn snapshot(&self) -> anyhow::Result<String> {
        // the copy lives beside the worktree's own index, in its git dir
        let index = self.git_path("index")?;
        let copy = self.git_path(&format!("reef-snapshot-{}.index", std::process::id()))?;
        // a repo with nothing staged yet has no index to copy
        if index.exists() {
            std::fs::copy(&index, &copy).with_context(|| format!("copying {}", index.display()))?;
        }
        let copy_str = copy.to_string_lossy().into_owned();
        let env = [("GIT_INDEX_FILE", copy_str.as_str())];
        // add everything from the worktree's root, then write the copy out as a tree
        let root = self.toplevel()?;
        let tree = self
            .at(&root)
            .run_env(&["add", "-A"], &env)
            .and_then(|_| self.at(&root).run_env(&["write-tree"], &env));
        // the copy goes whether or not the tree was written
        let _ = std::fs::remove_file(&copy);
        tree
    }

    /// Makes a commit of a tree without touching any branch, and returns it
    ///
    /// # Arguments
    ///
    /// * `tree` - The tree to commit
    /// * `parent` - Its parent, if any
    /// * `message` - Its message
    pub fn commit_tree(
        &self,
        tree: &str,
        parent: Option<&str>,
        message: &str,
    ) -> anyhow::Result<String> {
        let mut args = vec!["commit-tree", tree, "-m", message];
        if let Some(parent) = parent {
            args.extend(["-p", parent]);
        }
        // reef's own commits name reef, whatever the user's config says
        self.run_env(
            &args,
            &[
                ("GIT_AUTHOR_NAME", "reef"),
                ("GIT_AUTHOR_EMAIL", "reef@localhost"),
                ("GIT_COMMITTER_NAME", "reef"),
                ("GIT_COMMITTER_EMAIL", "reef@localhost"),
            ],
        )
    }

    /// Points a ref at an object, creating it if needed
    ///
    /// # Arguments
    ///
    /// * `name` - The ref's full name
    /// * `id` - What it points at
    pub fn update_ref(&self, name: &str, id: &str) -> anyhow::Result<()> {
        self.run(&["update-ref", name, id])?;
        Ok(())
    }

    /// Deletes a ref, if it exists
    ///
    /// # Arguments
    ///
    /// * `name` - The ref's full name
    pub fn delete_ref(&self, name: &str) -> anyhow::Result<()> {
        // a ref that is already gone is fine
        let _ = self.succeeds(&["update-ref", "-d", name])?;
        Ok(())
    }

    /// Merges two trees against a base tree without touching the worktree, as a rebase would
    ///
    /// # Arguments
    ///
    /// * `base` - The tree both sides started from
    /// * `ours` - One side
    /// * `theirs` - The other side
    pub fn merge_trees(&self, base: &str, ours: &str, theirs: &str) -> anyhow::Result<Merged> {
        // `--merge-base` lets all three be trees; exit 1 means it conflicted, still with a tree
        let mut git = self.command();
        git.args([
            "merge-tree",
            "--write-tree",
            "--no-messages",
            "--name-only",
            &format!("--merge-base={base}"),
            ours,
            theirs,
        ]);
        let output = git.stdin(Stdio::null()).output()?;
        let conflicted = match output.status.code() {
            Some(0) => false,
            Some(1) => true,
            _ => anyhow::bail!(
                "git merge-tree failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        };
        // the tree is the first line; the conflicted files follow it, a line each
        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = text.lines();
        let tree = lines.next().unwrap_or_default().trim().to_owned();
        let conflicts = if conflicted {
            lines
                .take_while(|line| !line.is_empty())
                .map(str::to_owned)
                .collect()
        } else {
            Vec::new()
        };
        Ok(Merged { tree, conflicts })
    }

    /// The diff between two tree-ish objects, through git's CLI
    ///
    /// # Arguments
    ///
    /// * `base` - The tree or commit the diff starts from
    /// * `head` - The tree or commit it ends at
    #[instrument(name = "Git::diff", skip(self), err(Debug))]
    pub fn diff(&self, base: &str, head: &str) -> anyhow::Result<Diff> {
        // one command gives every file's record and its patch
        let mut args: Vec<&str> = DIFF_ARGS.to_vec();
        args.extend([base, head]);
        let output = Self::output(self.command().args(&args), "diff")?;
        parse_git(&output.stdout)
    }

    /// The commits on the first-parent line from `base` (left out) to `head`, newest first
    ///
    /// # Arguments
    ///
    /// * `base` - Where the line stops, if anywhere
    /// * `head` - Where it starts
    pub fn first_parents(&self, base: Option<&str>, head: &str) -> anyhow::Result<Vec<String>> {
        // a range when there is a base, else the whole history
        let range = match base {
            Some(base) => format!("{base}..{head}"),
            None => head.to_owned(),
        };
        let out = self.run(&["rev-list", "--first-parent", &range])?;
        Ok(out.lines().map(str::to_owned).collect())
    }

    /// The nodes each commit names in its `Reef-Node` trailers
    ///
    /// # Arguments
    ///
    /// * `commits` - The commits to read
    pub fn trailers(&self, commits: &[String]) -> anyhow::Result<BTreeMap<String, Vec<String>>> {
        let mut found = BTreeMap::new();
        // nothing to read for no commits
        if commits.is_empty() {
            return Ok(found);
        }
        // one record a commit: its id, a tab, then its trailers' values joined by commas
        let format =
            format!("--format=%H%x09%(trailers:key={NODE_TRAILER},valueonly,separator=%x2C)");
        let mut args = vec!["log", "--no-walk=unsorted", format.as_str()];
        args.extend(commits.iter().map(String::as_str));
        for line in self.run(&args)?.lines() {
            // the id, then the nodes named, if any
            let (id, nodes) = line.split_once('\t').unwrap_or((line, ""));
            let nodes = nodes
                .split(',')
                .map(str::trim)
                .filter(|node| !node.is_empty())
                .map(str::to_owned)
                .collect();
            found.insert(id.to_owned(), nodes);
        }
        Ok(found)
    }

    /// The stable patch-id of each commit's own change, which a rebase without conflicts keeps
    ///
    /// # Arguments
    ///
    /// * `commits` - The commits to identify
    pub fn patch_ids(&self, commits: &[String]) -> anyhow::Result<BTreeMap<String, String>> {
        let mut ids = BTreeMap::new();
        // nothing to identify for no commits
        if commits.is_empty() {
            return Ok(ids);
        }
        // each commit's patch against its first parent, fed to patch-id
        let mut args = vec!["log", "-p", "--no-walk=unsorted", "--format=commit %H"];
        args.extend(commits.iter().map(String::as_str));
        let patches = Self::output(self.command().args(&args), "log -p")?;
        let mut child = self
            .command()
            .args(["patch-id", "--stable"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("running git patch-id")?;
        // write the patches, then close stdin so patch-id finishes
        child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("patch-id has no stdin"))?
            .write_all(&patches.stdout)?;
        let output = child.wait_with_output()?;
        // one line a commit with a change: the patch-id, then the commit
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some((patch, commit)) = line.split_once(' ') {
                ids.insert(commit.to_owned(), patch.to_owned());
            }
        }
        Ok(ids)
    }

    /// The remotes this repo names, which a capture never needs
    pub fn remotes(&self) -> anyhow::Result<Vec<String>> {
        let out = self.run(&["remote"])?;
        Ok(out.lines().map(str::to_owned).collect())
    }
}
