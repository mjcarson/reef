//! git in the checkout a node's work happens in, reading only what is local to it
//!
//! - **Reads through gix:** HEAD and its branch, the upstream a branch tracks, the repo's
//!   default branch, any ref or revision, and merge-base, all in this process.
//! - **The pin through gix:** a commit of the start's tree, its parent the start, under
//!   `refs/reef/start/<plan>/<id>`, so no gc prunes either. reef writes it as `reef`, whatever the
//!   user's config says, so no identity needs setting up first. The ref is local, and no push
//!   sends it, since push sends branches.
//! - **A node's diff through gix:** a tree-to-tree diff with renames at 50%, each changed blob
//!   diffed with Myers and git's slider heuristics, cut into hunks with three lines of context,
//!   as `git diff` would show it.
//! - **The carry through gix:** a three-way merge of trees, its conflicted files marked as
//!   `git merge-tree --write-tree` marks them, so it writes git's own tree.
//! - **Trailers through gix:** one trailer's values on each commit HEAD reaches past the commits
//!   hidden, as `git log --format=%(trailers)` parses them, with each commit's author and when it
//!   was authored.
//! - **The snapshots through git's CLI.** gix 0.90 can't stage a worktree, so the worktree's index
//!   is copied, and git adds to the copy and writes it out as a tree. The user's own index and
//!   staging are left as they were. A start takes everything `git add -A` would, and a seal only
//!   what is tracked, and what its base already holds. Every step meets at tree ids in the one
//!   object store, so gix reads the tree git wrote.
//!
//! Nothing here fetches, pushes or names a remote. gix is opened from the directory alone, and
//! git's CLI runs with the variables that would point it at another repo or index removed, as
//! `reef-service`'s `repo` does, so a git hook's environment can't redirect either. Each gix
//! step is tested against git's CLI, which stays its oracle.

use anyhow::Context;
use gix::bstr::{BString, ByteSlice};
use gix::diff::blob::unified_diff::{ConsumeHunk, ContextSize, DiffLineKind, HunkHeader};
use gix::diff::blob::{Algorithm, InternedInput, UnifiedDiff, diff_with_slider_heuristics};
use gix::object::tree::diff::ChangeDetached;
use gix::objs::tree::EntryMode;
use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
use gix::refs::{FullName, Target};
use reef_api::ResolvingCommit;
use reef_api::revision::{FileDiff, FileStatus, Hunk, Line, LineKind};
use reef_model::NodeName;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// The branches taken as a repo's default when `origin/HEAD` names none, in the order tried
const DEFAULT_BRANCHES: &[&str] = &["main", "master"];

/// The name and email reef writes its own commits as
const REEF: (&str, &str) = ("reef", "reef@localhost");

/// How many bytes git looks through for a NUL to call a blob binary
const BINARY_PROBE: usize = 8000;

/// How many paths a seal adds in one `git add`, well under any system's limit on arguments
const ADD_CHUNK: usize = 500;

/// The ref that pins a node's start, within its project's repo: `refs/reef/start/<plan>/<id>`
///
/// # Arguments
///
/// * `node` - The node's full name
///
/// # Examples
///
/// ```
/// use reef::git::pin_ref;
///
/// let node = "reef/central/C30".parse().unwrap();
/// assert_eq!(pin_ref(&node), "refs/reef/start/central/C30");
/// ```
pub fn pin_ref(node: &NodeName) -> String {
    format!("refs/reef/start/{}/{}", node.plan.plan, node.id)
}

/// A pin written, and what its ref pointed at before, so a start refused can put it back
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    /// The ref's full name
    pub name: String,
    /// The commit it now points at
    pub id: String,
    /// What it pointed at before, or `None` if it didn't exist
    pub previous: Option<String>,
}

/// A three-way merge of trees: the tree it wrote, and the files it couldn't resolve
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The merged tree, conflicted files holding git's markers
    pub tree: String,
    /// The paths that still conflict, sorted
    pub conflicts: Vec<String>,
}

/// A commit carrying a trailer, and the values the trailer has on it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trailed {
    /// The commit, its summary, its author and when it was authored
    pub commit: ResolvingCommit,
    /// Each of the trailer's values on it, in the order written
    pub values: Vec<String>,
}

/// One checkout: a worktree and the repo it is in
pub struct Checkout {
    /// gix's view of the repo, opened from the worktree
    repo: gix::Repository,
    /// The worktree's root, canonical
    root: PathBuf,
}

impl Checkout {
    /// Opens the checkout a directory is in
    ///
    /// # Arguments
    ///
    /// * `dir` - The worktree, or any directory in it
    pub fn open(dir: &Path) -> anyhow::Result<Checkout> {
        // from the directory alone, never from a hook's GIT_DIR
        let repo = gix::discover(dir)
            .with_context(|| format!("{} is not in a git repo", dir.display()))?;
        // a bare repo has no worktree, so no work happens in it
        let root = repo
            .workdir()
            .ok_or_else(|| anyhow::anyhow!("{} is in a bare repo", dir.display()))?
            .canonicalize()?;
        Ok(Checkout { repo, root })
    }

    /// The worktree's root
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The root of the repo's main worktree, which a linked worktree is part of
    ///
    /// A non-bare repo's common dir is the main worktree's `.git`, so its parent is the main
    /// worktree. Any other common dir, a submodule's under its superproject's `.git/modules`,
    /// leaves this worktree as it is.
    pub fn main_root(&self) -> anyhow::Result<PathBuf> {
        // the common dir, as git names it from any worktree of the repo
        let common = self.repo.common_dir().canonicalize()?;
        if common.file_name().is_some_and(|name| name == ".git")
            && let Some(main) = common.parent()
        {
            return Ok(main.to_path_buf());
        }
        Ok(self.root.clone())
    }

    /// Whether a rebase is under way in this worktree, which runs post-commit for each pick
    /// with HEAD half built
    ///
    /// It is read from the worktree's own git dir, where git keeps `rebase-merge` or
    /// `rebase-apply` while it rebases, so a linked worktree's rebase is its own. A mailbox
    /// `git am` applies is no rebase.
    pub fn rebasing(&self) -> bool {
        // gix reads the markers git's own prompt reads
        matches!(
            self.repo.state(),
            Some(
                gix::state::InProgress::Rebase
                    | gix::state::InProgress::RebaseInteractive
                    | gix::state::InProgress::ApplyMailboxRebase
            )
        )
    }

    /// Whether HEAD is a merge commit, with more than one parent
    pub fn head_merges(&self) -> anyhow::Result<bool> {
        // an unborn HEAD is no commit at all
        let Some(head) = self.head()? else {
            return Ok(false);
        };
        let commit = self.repo.find_commit(id(&head)?)?;
        Ok(commit.parent_ids().count() > 1)
    }

    /// The commit HEAD is at, or `None` in a repo with no commit yet
    pub fn head(&self) -> anyhow::Result<Option<String>> {
        // an unborn HEAD names a branch with no commit
        let head = self.repo.head()?;
        Ok(head.id().map(|id| id.detach().to_string()))
    }

    /// The branch checked out, or `None` on a detached HEAD
    pub fn branch(&self) -> anyhow::Result<Option<String>> {
        // only a HEAD that names a branch has one
        let name = self.repo.head_name()?;
        Ok(name
            .filter(|name| name.as_bstr().starts_with(b"refs/heads/"))
            .map(|name| name.shorten().to_string()))
    }

    /// The commit a revision names, such as a branch, a ref or an id, if it names one here
    ///
    /// # Arguments
    ///
    /// * `rev` - The revision
    pub fn commit(&self, rev: &str) -> anyhow::Result<Option<String>> {
        // a name that resolves to nothing is an answer, not a failure
        let Ok(found) = self.repo.rev_parse_single(rev) else {
            return Ok(None);
        };
        // and one that names something no commit peels from is none either
        match found.object()?.peel_to_commit() {
            Ok(commit) => Ok(Some(commit.id.to_string())),
            Err(_) => Ok(None),
        }
    }

    /// The tracking ref a branch's upstream is, as git abbreviates it, if it is set and exists
    /// here
    ///
    /// The ref is read as it is locally: nothing is fetched.
    ///
    /// # Arguments
    ///
    /// * `branch` - The branch's short name, such as `main`
    pub fn upstream(&self, branch: &str) -> anyhow::Result<Option<String>> {
        // the tracking ref the branch's config maps its upstream to, if it has one
        let full: FullName = format!("refs/heads/{branch}").try_into()?;
        let direction = gix::remote::Direction::Fetch;
        let Some(tracking) = self
            .repo
            .branch_remote_tracking_ref_name(full.as_ref(), direction)
        else {
            return Ok(None);
        };
        let tracking = tracking?;
        // an upstream never fetched isn't here to read
        if self.repo.try_find_reference(tracking.as_ref())?.is_none() {
            return Ok(None);
        }
        Ok(Some(tracking.as_ref().shorten().to_string()))
    }

    /// The repo's default branch: the one `origin/HEAD` names, else `main` or `master`
    ///
    /// The local branch is taken where there is one, else the tracking ref, such as
    /// `origin/main`. `None` means the repo has neither.
    pub fn default_branch(&self) -> anyhow::Result<Option<String>> {
        // `origin/HEAD` names the remote's default, as a clone leaves it
        if let Some(head) = self.repo.try_find_reference("refs/remotes/origin/HEAD")?
            && let Some(target) = head.target().try_name()
            && let Some(name) = target.as_bstr().strip_prefix(b"refs/remotes/origin/")
        {
            let name = String::from_utf8_lossy(name).into_owned();
            // its local branch if this repo has one, else the tracking ref itself
            if self.has_ref(&format!("refs/heads/{name}"))? {
                return Ok(Some(name));
            }
            return Ok(Some(format!("origin/{name}")));
        }
        // else the first of the usual names that is a local branch
        for name in DEFAULT_BRANCHES {
            if self.has_ref(&format!("refs/heads/{name}"))? {
                return Ok(Some((*name).to_string()));
            }
        }
        Ok(None)
    }

    /// Whether a ref exists here
    ///
    /// # Arguments
    ///
    /// * `name` - The ref's full name
    pub fn has_ref(&self, name: &str) -> anyhow::Result<bool> {
        Ok(self.repo.try_find_reference(name)?.is_some())
    }

    /// The commit a ref points at, peeled, if the ref exists
    ///
    /// # Arguments
    ///
    /// * `name` - The ref's full name
    pub fn ref_id(&self, name: &str) -> anyhow::Result<Option<String>> {
        // a ref that isn't there points at nothing
        let Some(mut found) = self.repo.try_find_reference(name)? else {
            return Ok(None);
        };
        Ok(Some(found.peel_to_id()?.detach().to_string()))
    }

    /// The best common ancestor of two commits, if they have one
    ///
    /// # Arguments
    ///
    /// * `a` - One commit's full id
    /// * `b` - The other's
    pub fn merge_base(&self, a: &str, b: &str) -> anyhow::Result<Option<String>> {
        // two commits with no history in common have no base, which is an answer
        Ok(self
            .repo
            .merge_base(id(a)?, id(b)?)?
            .map(|base| base.detach().to_string()))
    }

    /// Whether one commit is the other, or one of its ancestors
    ///
    /// # Arguments
    ///
    /// * `ancestor` - The commit that may be an ancestor, its full id
    /// * `of` - The commit whose history it may be in, its full id
    pub fn is_ancestor(&self, ancestor: &str, of: &str) -> anyhow::Result<bool> {
        // a commit is in its own history, and an ancestor is its descendant's merge-base
        if ancestor == of {
            return Ok(true);
        }
        Ok(self.merge_base(ancestor, of)?.as_deref() == Some(ancestor))
    }

    /// Adds a linked worktree on a branch, made at a commit if the branch isn't there yet
    ///
    /// Through git's CLI, since gix can't add a worktree. Nothing is fetched.
    ///
    /// # Arguments
    ///
    /// * `path` - Where the worktree goes, which must not be there yet
    /// * `branch` - The branch it checks out, by its short name
    /// * `from` - The commit a new branch is made at
    pub fn add_worktree(&self, path: &Path, branch: &str, from: &str) -> anyhow::Result<()> {
        // a branch kept from an earlier worktree is checked out again as it is
        let add = [
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("--quiet"),
        ];
        let args: Vec<&OsStr> = match self.has_ref(&format!("refs/heads/{branch}"))? {
            true => add
                .into_iter()
                .chain([path.as_os_str(), OsStr::new(branch)])
                .collect(),
            false => add
                .into_iter()
                .chain([OsStr::new("-b"), OsStr::new(branch), path.as_os_str()])
                .chain([OsStr::new(from)])
                .collect(),
        };
        self.git_bytes(args, None)?;
        Ok(())
    }

    /// Removes a linked worktree of the repo, keeping its branch
    ///
    /// git refuses a worktree with changes it would lose, which is left as it is.
    ///
    /// # Arguments
    ///
    /// * `path` - The worktree's root
    pub fn remove_worktree(&self, path: &Path) -> anyhow::Result<()> {
        // never forced, so nothing uncommitted is lost
        let args = [
            OsStr::new("worktree"),
            OsStr::new("remove"),
            path.as_os_str(),
        ];
        self.git_bytes(args, None)?;
        Ok(())
    }

    /// Deletes a local branch, as a worktree's start that was refused leaves the one it made
    ///
    /// # Arguments
    ///
    /// * `branch` - The branch's short name
    pub fn delete_branch(&self, branch: &str) -> anyhow::Result<()> {
        self.git(&["branch", "--quiet", "-D", branch], None)?;
        Ok(())
    }

    /// Adds a pattern to the repo's own `info/exclude`, which every worktree of it reads, unless
    /// a line there names it already, and says whether it added it
    ///
    /// # Arguments
    ///
    /// * `pattern` - The pattern, as an ignore file writes one
    pub fn exclude(&self, pattern: &str) -> anyhow::Result<bool> {
        // the common dir's, so one line covers every worktree
        let info = self.repo.common_dir().join("info");
        let file = info.join("exclude");
        let kept = match std::fs::read_to_string(&file) {
            Ok(kept) => kept,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error).with_context(|| format!("reading {}", file.display())),
        };
        if kept.lines().any(|line| line.trim() == pattern) {
            return Ok(false);
        }
        // appended on a line of its own, after whatever the file ends with
        std::fs::create_dir_all(&info)?;
        let mut text = kept;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(pattern);
        text.push('\n');
        std::fs::write(&file, text).with_context(|| format!("writing {}", file.display()))?;
        Ok(true)
    }

    /// The directory git runs this worktree's hooks from: `core.hooksPath`'s if it is set, else
    /// the common dir's `hooks`, which every worktree of the repo runs
    pub fn hooks_dir(&self) -> anyhow::Result<PathBuf> {
        // git resolves it as it does to run a hook, a relative `core.hooksPath` included
        let dir = self.git(
            &["rev-parse", "--path-format=absolute", "--git-path", "hooks"],
            None,
        )?;
        Ok(PathBuf::from(dir))
    }

    /// Whether git ignores a path in this worktree, by any of its ignore files or the repo's
    /// `info/exclude`, whether or not the path is there
    ///
    /// # Arguments
    ///
    /// * `path` - The path, from the worktree's root
    pub fn ignores(&self, path: &str) -> anyhow::Result<bool> {
        // check-ignore answers in its status: 0 is ignored, 1 is not, and anything else failed
        let mut git = Command::new("git");
        git.arg("-C")
            .arg(&self.root)
            .args(["check-ignore", "--quiet", "--", path])
            .env("LC_ALL", "C");
        for var in GIT_ENV {
            git.env_remove(var);
        }
        let output = git
            .stdin(Stdio::null())
            .output()
            .context("running git check-ignore")?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => anyhow::bail!(
                "git check-ignore failed in {}: {}",
                self.root.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        }
    }

    /// Reads one trailer off each commit a head reaches and none of the hidden commits do,
    /// oldest first, leaving out commits that don't carry it
    ///
    /// A trailer is read as git reads one: from the message's last paragraph, its token matched
    /// whatever its case.
    ///
    /// # Arguments
    ///
    /// * `head` - The commit to walk back from
    /// * `hidden` - The commits whose history is left out, such as a start and a merge-base
    /// * `token` - The trailer's token, such as `Reef-Resolves`
    pub fn trailers(
        &self,
        head: &str,
        hidden: &[&str],
        token: &str,
    ) -> anyhow::Result<Vec<Trailed>> {
        // every commit the head reaches past the hidden ones, as `git log head ^hidden` lists
        let hidden = hidden
            .iter()
            .map(|commit| id(commit))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let walk = self.repo.rev_walk([id(head)?]).with_hidden(hidden).all()?;
        let mut trailed = Vec::new();
        for info in walk {
            let commit = info?.object()?;
            let message = commit.message()?;
            // the trailer's values in the message's last paragraph, if it has a body
            let values: Vec<String> = message
                .body()
                .map(|body| {
                    body.trailers()
                        .filter(|trailer| trailer.token.eq_ignore_ascii_case(token.as_bytes()))
                        .map(|trailer| trailer.value.to_str_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            if values.is_empty() {
                continue;
            }
            // who wrote it, and when, which a rebase or an amend keeps
            let author = commit.author()?;
            trailed.push(Trailed {
                commit: ResolvingCommit {
                    commit: commit.id.to_string(),
                    summary: message.summary().to_str_lossy().into_owned(),
                    author: author.email.to_str_lossy().into_owned(),
                    authored: author.time()?.seconds,
                },
                values,
            });
        }
        // the walk comes newest first, and the oldest commit answers first
        trailed.reverse();
        Ok(trailed)
    }

    /// Whether an object is in the repo's store
    ///
    /// # Arguments
    ///
    /// * `object` - The object's full id
    pub fn has_object(&self, object: &str) -> anyhow::Result<bool> {
        Ok(self.repo.has_object(id(object)?))
    }

    /// The tree a commit holds
    ///
    /// # Arguments
    ///
    /// * `commit` - The commit's full id
    pub fn tree_of(&self, commit: &str) -> anyhow::Result<String> {
        let commit = self.repo.find_commit(id(commit)?)?;
        Ok(commit.tree_id()?.detach().to_string())
    }

    /// Writes the worktree as it is now, committed or not, as one tree: a start's snapshot
    ///
    /// The worktree's index is copied and everything is added to the copy, untracked files too
    /// and ignored ones not, so the user's own index and staging are left as they were.
    pub fn snapshot(&self) -> anyhow::Result<String> {
        // add everything from the worktree's root, then write the copy out as a tree
        self.on_index_copy(|copy| {
            self.git(&["add", "-A"], Some(copy))?;
            self.git(&["write-tree"], Some(copy))
        })
    }

    /// Writes the worktree as a seal takes it, as one tree: what is tracked, and what its base
    /// already holds
    ///
    /// Every tracked file is taken as it is in the worktree, a deleted one gone and a staged new
    /// one in. An untracked file that no commit or add took is left out, such as a `__pycache__`
    /// no ignore file names, unless the base holds it already: a start takes untracked files, so
    /// one it took that is still there is kept rather than shown as deleted. The user's own index
    /// and staging are left as they were.
    ///
    /// # Arguments
    ///
    /// * `base` - The tree the node's diff starts from
    pub fn seal_snapshot(&self, base: &str) -> anyhow::Result<String> {
        // the base's tree, which says which untracked files to keep
        let base = self.repo.find_object(id(base)?)?.peel_to_tree()?;
        self.on_index_copy(|copy| {
            // every tracked file as the worktree has it
            self.git(&["add", "-u"], Some(copy))?;
            // the untracked files that aren't ignored, of which only the base's are kept
            let others = self.git_bytes(
                ["ls-files", "--others", "--exclude-standard", "-z"].map(OsStr::new),
                Some(copy),
            )?;
            let mut kept = Vec::new();
            for path in others
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
            {
                let path = gix::path::from_bstr(path.as_bstr())?;
                if base.lookup_entry_by_path(&path)?.is_some() {
                    kept.push(path.into_owned());
                }
            }
            // added as the paths they are, never as patterns
            for chunk in kept.chunks(ADD_CHUNK) {
                let mut args: Vec<&OsStr> = ["--literal-pathspecs", "add", "--"]
                    .map(OsStr::new)
                    .to_vec();
                args.extend(chunk.iter().map(|path| path.as_os_str()));
                self.git_bytes(args, Some(copy))?;
            }
            self.git(&["write-tree"], Some(copy))
        })
    }

    /// Runs a step against a copy of the worktree's index, which goes once the step is done
    ///
    /// # Arguments
    ///
    /// * `step` - What to run, given the copy's path
    fn on_index_copy<T, F>(&self, step: F) -> anyhow::Result<T>
    where
        F: FnOnce(&Path) -> anyhow::Result<T>,
    {
        // the copy lives beside the worktree's own index, in its own git dir
        let index = self.repo.index_path();
        let copy = self
            .repo
            .git_dir()
            .join(format!("reef-snapshot-{}.index", std::process::id()));
        // a repo with nothing staged yet has no index to copy
        if index.exists() {
            std::fs::copy(&index, &copy).with_context(|| format!("copying {}", index.display()))?;
        }
        let done = step(&copy);
        // the copy goes whether or not the step worked
        let _ = std::fs::remove_file(&copy);
        done
    }

    /// The diff between two trees or commits, as `git diff` would show it, files sorted by path
    ///
    /// Renames are found at 50% similarity, as git finds them by default, and copies are not.
    /// Trees are walked into and submodules left out, and a file with a NUL in its first 8000
    /// bytes on either side is binary, with no hunks.
    ///
    /// # Arguments
    ///
    /// * `base` - The full id of the tree or commit the diff starts from
    /// * `head` - The full id of the tree or commit it ends at
    pub fn diff(&self, base: &str, head: &str) -> anyhow::Result<Vec<FileDiff>> {
        // either side may be a commit, which is peeled to its tree
        let tree = |object: &str| -> anyhow::Result<gix::Tree<'_>> {
            Ok(self.repo.find_object(id(object)?)?.peel_to_tree()?)
        };
        let base = tree(base)?;
        let head = tree(head)?;
        // paths tracked in full, and renames found as git finds them by default
        let mut options = gix::diff::Options::default();
        options
            .track_path()
            .track_rewrites(Some(gix::diff::Rewrites {
                copies: None,
                percentage: Some(0.5),
                limit: 1000,
                track_empty: false,
            }));
        let changes = self.repo.diff_tree_to_tree(&base, &head, options)?;
        // each change that is a file, sorted by the path it is shown under
        let mut files = Vec::with_capacity(changes.len());
        for change in changes {
            if let Some(file) = file_diff(&self.repo, change)? {
                files.push(file);
            }
        }
        files.sort_by(|a, b| a.path().cmp(b.path()));
        Ok(files)
    }

    /// Merges two trees against the tree both started from, as
    /// `git merge-tree --write-tree --merge-base` does
    ///
    /// Conflicted files hold markers labelled with each side's tree id and no ancestor label, as
    /// git's do, so a conflicted merge writes the same tree git would.
    ///
    /// # Arguments
    ///
    /// * `base` - The tree both sides started from
    /// * `ours` - One side
    /// * `theirs` - The other side
    pub fn merge_trees(&self, base: &str, ours: &str, theirs: &str) -> anyhow::Result<Merged> {
        // label each side with its tree id, as git labels a merge of bare trees
        let labels = gix::merge::blob::builtin_driver::text::Labels {
            ancestor: None,
            current: Some(ours.into()),
            other: Some(theirs.into()),
        };
        let options = self.repo.tree_merge_options()?;
        let mut outcome =
            self.repo
                .merge_trees(id(base)?, id(ours)?, id(theirs)?, labels, options)?;
        let tree = outcome.tree.write()?.detach().to_string();
        // the files that still hold markers, or whose change to the tree couldn't be resolved
        let how = gix::merge::tree::TreatAsUnresolved::default();
        let mut conflicts: Vec<String> = outcome
            .conflicts
            .iter()
            .filter(|conflict| conflict.is_unresolved(how))
            .flat_map(|conflict| [conflict.ours.location(), conflict.theirs.location()])
            .map(|path| path.to_string())
            .collect();
        conflicts.sort();
        conflicts.dedup();
        Ok(Merged { tree, conflicts })
    }

    /// Pins a start: a commit of its tree whose parent is its commit, under a ref
    ///
    /// The ref is pointed at the commit whatever it pointed at before, which the answer names
    /// so [`Checkout::unpin`] can put it back.
    ///
    /// # Arguments
    ///
    /// * `name` - The ref's full name
    /// * `tree` - The start's tree
    /// * `parent` - The start's commit
    /// * `message` - The commit's message
    pub fn pin(
        &self,
        name: &str,
        tree: &str,
        parent: &str,
        message: &str,
    ) -> anyhow::Result<Pinned> {
        // reef's own commit, as reef, whatever the user's config says
        let signature = gix::actor::Signature {
            name: REEF.0.into(),
            email: REEF.1.into(),
            time: gix::date::Time::now_local_or_utc(),
        };
        let commit = gix::objs::Commit {
            tree: id(tree)?,
            parents: std::iter::once(id(parent)?).collect(),
            author: signature.clone(),
            committer: signature.clone(),
            encoding: None,
            message: message.into(),
            extra_headers: Vec::new(),
        };
        let pinned = self.repo.write_object(&commit)?.detach();
        // the ref points at it, whatever it pointed at before
        let edit = RefEdit {
            change: Change::Update {
                log: LogChange {
                    mode: RefLog::AndReference,
                    force_create_reflog: false,
                    message: BString::from(message),
                },
                expected: PreviousValue::Any,
                new: Target::Object(pinned),
            },
            name: name.try_into()?,
            deref: false,
        };
        let previous = self.edit_ref(edit, &signature)?;
        Ok(Pinned {
            name: name.to_string(),
            id: pinned.to_string(),
            previous,
        })
    }

    /// Puts a pin's ref back as it was before the pin, if it still points at the pin
    ///
    /// # Arguments
    ///
    /// * `pinned` - The pin to take back
    pub fn unpin(&self, pinned: &Pinned) -> anyhow::Result<()> {
        let signature = gix::actor::Signature {
            name: REEF.0.into(),
            email: REEF.1.into(),
            time: gix::date::Time::now_local_or_utc(),
        };
        // only while it is still this pin, so a later start's pin is never undone
        let expected = PreviousValue::MustExistAndMatch(Target::Object(id(&pinned.id)?));
        let change = match &pinned.previous {
            Some(previous) => Change::Update {
                log: LogChange {
                    mode: RefLog::AndReference,
                    force_create_reflog: false,
                    message: BString::from("reef: a start refused, its pin put back"),
                },
                expected,
                new: Target::Object(id(previous)?),
            },
            None => Change::Delete {
                expected,
                log: RefLog::AndReference,
            },
        };
        let edit = RefEdit {
            change,
            name: pinned.name.as_str().try_into()?,
            deref: false,
        };
        self.edit_ref(edit, &signature)?;
        Ok(())
    }

    /// Applies one ref edit as reef, returning what the ref pointed at before it
    ///
    /// # Arguments
    ///
    /// * `edit` - The edit
    /// * `signature` - Who the reflog names, if one is written
    fn edit_ref(
        &self,
        edit: RefEdit,
        signature: &gix::actor::Signature,
    ) -> anyhow::Result<Option<String>> {
        // the reflog, where git keeps one for this ref, names reef
        let mut time = gix::date::parse::TimeBuf::default();
        let edits = self
            .repo
            .edit_references_as([edit], Some(signature.to_ref(&mut time)))?;
        // the edit comes back with the value it found under the ref's lock
        Ok(edits
            .first()
            .and_then(|edit| edit.change.previous_value())
            .and_then(|previous| previous.try_id().map(|id| id.to_string())))
    }

    /// Runs git's CLI in the worktree's root, returning its output trimmed
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments after `git -C <root>`
    /// * `index` - The index file to use instead of the worktree's own, if any
    fn git(&self, args: &[&str], index: Option<&Path>) -> anyhow::Result<String> {
        // its output as text, trimmed
        let out = self.git_bytes(args.iter().map(OsStr::new), index)?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// Runs git's CLI in the worktree's root, returning its output as it wrote it
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments after `git -C <root>`, which may be paths that aren't utf-8
    /// * `index` - The index file to use instead of the worktree's own, if any
    fn git_bytes<'a, I>(&self, args: I, index: Option<&Path>) -> anyhow::Result<Vec<u8>>
    where
        I: IntoIterator<Item = &'a OsStr>,
    {
        // what is run, for a message, with at most a few of its arguments
        let args: Vec<&OsStr> = args.into_iter().collect();
        let named = || {
            let shown: Vec<String> = args
                .iter()
                .take(4)
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            let more = if args.len() > 4 { " …" } else { "" };
            format!("{}{more}", shown.join(" "))
        };
        // from the worktree alone, in the C locale so its messages read the same everywhere
        let mut git = Command::new("git");
        git.arg("-C").arg(&self.root).args(&args).env("LC_ALL", "C");
        for var in GIT_ENV {
            git.env_remove(var);
        }
        if let Some(index) = index {
            git.env("GIT_INDEX_FILE", index);
        }
        let output = git
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("running git {}", named()))?;
        // a failure carries git's own message
        if !output.status.success() {
            anyhow::bail!(
                "git {} failed in {}: {}",
                named(),
                self.root.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(output.stdout)
    }
}

/// Gathers the hunks gix's unified diff hands over, each line numbered on its sides
#[derive(Default)]
struct Hunks {
    /// The hunks so far
    hunks: Vec<Hunk>,
}

impl ConsumeHunk for Hunks {
    type Out = Vec<Hunk>;

    /// Numbers one hunk's lines on each side and keeps it
    ///
    /// # Arguments
    ///
    /// * `header` - Where the hunk starts and how long it is on each side
    /// * `lines` - Its lines, each with its kind and its bytes, newline included
    fn consume_hunk(
        &mut self,
        header: HunkHeader,
        lines: &[(DiffLineKind, &[u8])],
    ) -> std::io::Result<()> {
        // git starts an empty side at the line before it
        let start = |start: u32, len: u32| {
            if len == 0 {
                start.saturating_sub(1)
            } else {
                start
            }
        };
        let old_start = start(header.before_hunk_start, header.before_hunk_len);
        let new_start = start(header.after_hunk_start, header.after_hunk_len);
        // number each line from where its sides start
        let mut old_no = old_start.max(1);
        let mut new_no = new_start.max(1);
        let mut out = Vec::with_capacity(lines.len());
        for (kind, bytes) in lines {
            let kind = match kind {
                DiffLineKind::Context => LineKind::Context,
                DiffLineKind::Add => LineKind::Added,
                DiffLineKind::Remove => LineKind::Removed,
            };
            // a line is on the sides its kind says, and moves each one's count on
            let (old, new) = match kind {
                LineKind::Context => (Some(old_no), Some(new_no)),
                LineKind::Added => (None, Some(new_no)),
                LineKind::Removed => (Some(old_no), None),
            };
            if old.is_some() {
                old_no += 1;
            }
            if new.is_some() {
                new_no += 1;
            }
            // a token keeps its newline, and the last line of a file may have none
            let no_newline = !bytes.ends_with(b"\n");
            let text = bytes.strip_suffix(b"\n").unwrap_or(bytes);
            out.push(Line {
                kind,
                old,
                new,
                text: String::from_utf8_lossy(text).into_owned(),
                no_newline,
            });
        }
        self.hunks.push(Hunk {
            old_start,
            old_lines: header.before_hunk_len,
            new_start,
            new_lines: header.after_hunk_len,
            lines: out,
        });
        Ok(())
    }

    /// Hands back every hunk
    fn finish(self) -> Self::Out {
        self.hunks
    }
}

/// One side of a changed entry: its mode and its blob
struct Side {
    /// The entry's mode
    mode: EntryMode,
    /// The blob it held
    id: gix::ObjectId,
}

/// Whether git would call a blob binary: a NUL in its first 8000 bytes
///
/// # Arguments
///
/// * `data` - The blob's contents
fn is_binary(data: &[u8]) -> bool {
    data[..data.len().min(BINARY_PROBE)].contains(&0)
}

/// Diffs one file's two sides into hunks, or into none when either side is binary
///
/// # Arguments
///
/// * `repo` - The repo the blobs are in
/// * `old` - The base side, if any
/// * `new` - The head side, if any
fn hunks(
    repo: &gix::Repository,
    old: Option<&Side>,
    new: Option<&Side>,
) -> anyhow::Result<(bool, Vec<Hunk>)> {
    // a missing side is empty
    let read = |side: Option<&Side>| -> anyhow::Result<Vec<u8>> {
        match side {
            Some(side) => Ok(repo.find_blob(side.id)?.detach().data),
            None => Ok(Vec::new()),
        }
    };
    let before = read(old)?;
    let after = read(new)?;
    // git shows a binary file as a line saying it changed, and no hunks
    if is_binary(&before) || is_binary(&after) {
        return Ok((true, Vec::new()));
    }
    // identical contents, as a mode change or a pure rename leaves, have no hunks
    if before == after {
        return Ok((false, Vec::new()));
    }
    // a line diff, with git's slider heuristics, cut into hunks with three lines around each
    let input = InternedInput::new(before.as_slice(), after.as_slice());
    let diff = diff_with_slider_heuristics(Algorithm::Myers, &input);
    let hunks =
        UnifiedDiff::new(&diff, &input, Hunks::default(), ContextSize::symmetrical(3)).consume()?;
    Ok((false, hunks))
}

/// Turns one of gix's tree changes into a file's diff, if it is a file at all
///
/// # Arguments
///
/// * `repo` - The repo the blobs are in
/// * `change` - The change to turn
fn file_diff(repo: &gix::Repository, change: ChangeDetached) -> anyhow::Result<Option<FileDiff>> {
    // a side as the change names it
    let side = |mode: EntryMode, id: gix::ObjectId| Some(Side { mode, id });
    // the change's kind, paths and sides
    let (status, old_path, new_path, old, new) = match change {
        ChangeDetached::Addition {
            location,
            entry_mode,
            id,
            ..
        } => (
            FileStatus::Added,
            None,
            Some(location.to_string()),
            None,
            side(entry_mode, id),
        ),
        ChangeDetached::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => (
            FileStatus::Deleted,
            Some(location.to_string()),
            None,
            side(entry_mode, id),
            None,
        ),
        ChangeDetached::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => (
            FileStatus::Modified,
            Some(location.to_string()),
            Some(location.to_string()),
            side(previous_entry_mode, previous_id),
            side(entry_mode, id),
        ),
        ChangeDetached::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            location,
            ..
        } => (
            FileStatus::Renamed,
            Some(source_location.to_string()),
            Some(location.to_string()),
            side(source_entry_mode, source_id),
            side(entry_mode, id),
        ),
    };
    // trees are walked into and submodules left alone, as git's diff of files does
    let is_file = |side: &Option<Side>| {
        side.as_ref()
            .is_none_or(|side| side.mode.is_blob_or_symlink())
    };
    if !is_file(&old) || !is_file(&new) {
        return Ok(None);
    }
    let (binary, hunks) = hunks(repo, old.as_ref(), new.as_ref())?;
    // the mode git writes, such as 100644
    let octal = |side: &Side| side.mode.kind().as_octal_str().to_string();
    Ok(Some(FileDiff {
        status,
        old_path,
        new_path,
        old_mode: old.as_ref().map(octal),
        new_mode: new.as_ref().map(octal),
        old_id: old.as_ref().map(|side| side.id.to_string()),
        new_id: new.as_ref().map(|side| side.id.to_string()),
        binary,
        too_large: false,
        hunks,
    }))
}

/// Parses a full object id
///
/// # Arguments
///
/// * `object` - The id, in hex
fn id(object: &str) -> anyhow::Result<gix::ObjectId> {
    gix::ObjectId::from_hex(object.trim().as_bytes())
        .with_context(|| format!("`{object}` is not an object id"))
}
