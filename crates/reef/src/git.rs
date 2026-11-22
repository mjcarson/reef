//! git in the checkout a node's work happens in, reading only what is local to it
//!
//! - **Reads through gix:** HEAD and its branch, the upstream a branch tracks, the repo's
//!   default branch, any ref or revision, and merge-base, all in this process.
//! - **The pin through gix:** a commit of the start's tree, its parent the start, under
//!   `refs/reef/start/<plan>/<id>`, so no gc prunes either. reef writes it as `reef`, whatever the
//!   user's config says, so no identity needs setting up first. The ref is local, and no push
//!   sends it, since push sends branches.
//! - **The snapshot through git's CLI.** gix 0.90 can't stage a worktree, so the worktree's index
//!   is copied, and `git add -A` and `git write-tree` run against the copy. The user's own index
//!   and staging are left as they were. Every step meets at tree ids in the one object store, so
//!   gix reads the tree git wrote.
//!
//! Nothing here fetches, pushes or names a remote. gix is opened from the directory alone, and
//! git's CLI runs with the variables that would point it at another repo or index removed, as
//! `reef-service`'s `repo` does, so a git hook's environment can't redirect either. Each gix
//! step is tested against git's CLI, which stays its oracle.

use anyhow::Context;
use gix::bstr::BString;
use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
use gix::refs::{FullName, Target};
use reef_model::NodeName;
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

    /// Writes the worktree as it is now, committed or not, as one tree
    ///
    /// The worktree's index is copied and everything is added to the copy, untracked files too
    /// and ignored ones not, so the user's own index and staging are left as they were.
    pub fn snapshot(&self) -> anyhow::Result<String> {
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
        // add everything from the worktree's root, then write the copy out as a tree
        let tree = self
            .git(&["add", "-A"], Some(&copy))
            .and_then(|_| self.git(&["write-tree"], Some(&copy)));
        // the copy goes whether or not the tree was written
        let _ = std::fs::remove_file(&copy);
        tree
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
        // from the worktree alone, in the C locale so its messages read the same everywhere
        let mut git = Command::new("git");
        git.arg("-C").arg(&self.root).args(args).env("LC_ALL", "C");
        for var in GIT_ENV {
            git.env_remove(var);
        }
        if let Some(index) = index {
            git.env("GIT_INDEX_FILE", index);
        }
        let output = git
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("running git {}", args.join(" ")))?;
        // a failure carries git's own message
        if !output.status.success() {
            anyhow::bail!(
                "git {} failed in {}: {}",
                args.join(" "),
                self.root.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
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
