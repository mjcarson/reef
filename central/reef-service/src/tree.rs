//! Docs read from git's objects at one commit, for a remote project, and the switch between
//! those and a local project's working tree
//!
//! A remote project's clone is bare, so its docs are blobs of the tree of the commit its row says
//! was last fetched of the branch a plan follows. Every read of one snapshot names that commit,
//! so a fetch that lands while a write is checked changes nothing the check reads, and readers
//! never see half of a fetch. The rules a working tree's docs keep hold here too: no path that
//! climbs out or starts from the root is read, and a symlink is followed only while it stays
//! inside the tree.
//!
//! A local project's plan that follows a branch reads a worktree that has that branch checked
//! out, the main one first, so its uncommitted docs link as a working tree's do. With none, it
//! reads the branch's tip from the repo's objects, as a remote project's commit is read. Which
//! of those it is is found as the docs are opened, so one read sees one of them.

use gix::ObjectId;
use reef_api::Project;
use reef_render::paths::join;
use reef_render::{DocSource, Entry, Headings, RepoSource, leaves};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

/// How many symlinks one read follows before it gives up, as a loop of them would never end
const LINK_HOPS: usize = 8;

/// What a path names at a commit, once its symlinks are followed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Found {
    /// A directory, by its tree's id
    Tree(ObjectId),
    /// A file, by its blob's id
    Blob(ObjectId),
}

/// Docs read at one commit of a bare clone
pub struct TreeSource {
    /// The clone, opened without reading any config but its own
    repo: gix::Repository,
    /// The tree of the commit the docs are read at
    tree: ObjectId,
}

impl TreeSource {
    /// Opens a clone's docs at a commit
    ///
    /// # Arguments
    ///
    /// * `clone` - The bare clone
    /// * `commit` - The commit to read at, as its full hex id
    pub fn open(clone: &Path, commit: &str) -> anyhow::Result<TreeSource> {
        // the clone alone, ignoring the account's and the system's git config
        let repo = gix::open_opts(clone, gix::open::Options::isolated()).map_err(|error| {
            anyhow::anyhow!("the clone at {} can't be read: {error}", clone.display())
        })?;
        // the commit its row names, and its tree
        let id = ObjectId::from_hex(commit.as_bytes())
            .map_err(|error| anyhow::anyhow!("`{commit}` is not a commit's id: {error}"))?;
        let tree = repo
            .find_commit(id)
            .and_then(|commit| Ok(commit.tree_id()?.detach()))
            .map_err(|error| {
                anyhow::anyhow!(
                    "the clone at {} has no commit {commit}: {error}",
                    clone.display()
                )
            })?;
        Ok(TreeSource { repo, tree })
    }

    /// The tree or blob a path names at this commit, following symlinks that stay in the tree,
    /// the empty path naming the commit's own tree
    ///
    /// Each part of the path is looked up in turn, so a symlink to a directory is followed as a
    /// working tree would follow it. A link to an absolute path, or one whose target climbs out of
    /// the tree, names nothing, as does a submodule.
    ///
    /// # Arguments
    ///
    /// * `path` - The path from the repo's root
    fn entry(&self, path: &str) -> Option<Found> {
        // a path that climbs out, or starts from somewhere else, is refused as written
        if leaves(path) {
            return None;
        }
        // the parts still to look up, and the directories already walked into
        let mut rest: VecDeque<String> = parts(path);
        let mut walked: Vec<String> = Vec::new();
        // start the walk at the commit's own tree
        let mut at = self.tree;
        let mut tree = self.repo.find_tree(at).ok()?;
        let mut hops = 0;
        while let Some(part) = rest.pop_front() {
            // the entry this part names in the directory walked to
            let entry = tree.lookup_entry([part.as_str()]).ok()??;
            let mode = entry.mode();
            if mode.is_tree() {
                // walk into the directory this part names
                at = entry.object_id();
                tree = self.repo.find_tree(at).ok()?;
                walked.push(part);
            } else if mode.is_blob() {
                // stop at the file, which the path names only if no parts are left after it
                return rest.is_empty().then(|| Found::Blob(entry.object_id()));
            } else if mode.is_link() {
                // a symlink's target, from the directory it is in, then the rest of the path
                hops += 1;
                if hops > LINK_HOPS {
                    return None;
                }
                let target = self.repo.find_blob(entry.object_id()).ok()?.take_data();
                let target = String::from_utf8(target).ok()?;
                if target.starts_with('/') {
                    return None;
                }
                let rest_path = Vec::from(rest).join("/");
                let followed = join(&join(&walked.join("/"), &target), &rest_path);
                // one that leads out of the tree is no doc of it
                if leaves(&followed) {
                    return None;
                }
                // looked up again from the root
                rest = parts(&followed);
                walked.clear();
                at = self.tree;
                tree = self.repo.find_tree(at).ok()?;
            } else {
                // a submodule's commit is in another repo
                return None;
            }
        }
        // name the tree of the directory the path ended at
        Some(Found::Tree(at))
    }

    /// The blob a path names at this commit, following symlinks that stay in the tree
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the repo's root
    fn blob(&self, path: &str) -> Option<ObjectId> {
        // look the path up and keep only a file, since a directory names no doc
        match self.entry(path)? {
            Found::Blob(id) => Some(id),
            Found::Tree(_) => None,
        }
    }

    /// The files and folders of a directory at this commit, by name, its dotfiles left out
    ///
    /// A blob is a file and a tree a folder. A symlink is listed as what it leads to, and left
    /// out if that is out of the tree or nothing, and a submodule is left out.
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory's path from the repo's root, empty for the root
    fn entries(&self, dir: &str) -> Vec<Entry> {
        // find the tree the path names, listing nothing if it names no directory
        let Some(Found::Tree(at)) = self.entry(dir) else {
            return Vec::new();
        };
        // open that tree, listing nothing if it can't be read
        let Ok(tree) = self.repo.find_tree(at) else {
            return Vec::new();
        };
        // gather each entry by name, so the listing comes out sorted, with whether it is a folder
        let mut entries: BTreeMap<String, bool> = BTreeMap::new();
        for entry in tree.iter().flatten() {
            // skip a name that isn't text, or a dotfile, as a book's walk skips them
            let Ok(name) = std::str::from_utf8(entry.filename()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            // tell a folder from a file by the entry's mode
            let mode = entry.mode();
            let folder = if mode.is_tree() {
                true
            } else if mode.is_blob() {
                false
            } else if mode.is_link() {
                // list a symlink as what it leads to, skipping one that leads out of the tree
                // or to nothing
                match self.entry(&join(dir, name)) {
                    Some(Found::Tree(_)) => true,
                    Some(Found::Blob(_)) => false,
                    None => continue,
                }
            } else {
                // skip a submodule, since its files are in another repo
                continue;
            };
            // keep the entry under its name
            entries.insert(name.to_string(), folder);
        }
        // return the entries in name order
        entries
            .into_iter()
            .map(|(name, folder)| Entry { name, folder })
            .collect()
    }
}

/// The parts of a `/`-separated path, with empty and `.` parts left out, as a filesystem reads it
///
/// # Arguments
///
/// * `path` - The path, which must not leave its root
fn parts(path: &str) -> VecDeque<String> {
    // `join` drops empty and `.` parts, and resolves a `..` that stays inside
    join("", path)
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

impl DocSource for TreeSource {
    /// Reads a doc from its blob at this commit, unless its path leaves the repo
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the repo's root
    fn read(&self, path: &str) -> Option<String> {
        // only a blob that stays in the tree is read, and only as text
        let id = self.blob(path)?;
        let data = self.repo.find_blob(id).ok()?.take_data();
        String::from_utf8(data).ok()
    }

    /// A doc's path from the repo's root, or a refusal of one that leaves the repo as written
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the repo's root
    fn place(&self, path: &str) -> Result<String, String> {
        // a symlink out is found when it is read, and so is just not found, as in a working tree
        match leaves(path) {
            true => Err("leaves the repo".to_string()),
            false => Ok(path.to_string()),
        }
    }

    /// The files and folders of a directory at this commit, by name
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory's path from the repo's root
    fn list(&self, dir: &str) -> Vec<Entry> {
        // list the directory's entries at this commit
        self.entries(dir)
    }
}

/// A branch a remote project's plan follows that its clone hasn't fetched yet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotFetched {
    /// The project's name
    pub project: String,
    /// The branch
    pub branch: String,
}

impl std::fmt::Display for NotFetched {
    /// Names the branch, and how to fetch it
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "branch `{}` of project `{}` isn't fetched yet: run `reef project fetch`, or follow \
             another with `reef plan branch`",
            self.branch, self.project
        )
    }
}

impl std::error::Error for NotFetched {}

/// Where a plan's docs are read from, which a blocking read opens
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Docs {
    /// A local project's working tree, as it stands
    WorkingTree(PathBuf),
    /// A remote project's clone, at the commit last fetched of its plan's branch
    Commit {
        /// The bare clone
        clone: PathBuf,
        /// The commit to read at
        commit: String,
    },
    /// A local project's plan that follows a branch: a worktree with it checked out, else its tip
    LocalBranch {
        /// The root of the repo's main worktree
        repo: PathBuf,
        /// The branch, without `refs/heads/`
        branch: String,
    },
}

impl Docs {
    /// Where a plan's docs are read from, by its project and the branch it follows
    ///
    /// # Arguments
    ///
    /// * `project` - The plan's project, as the store lists it
    /// * `branch` - The branch the plan follows, or `None` for its project's own
    pub fn of(project: &Project, branch: Option<&str>) -> anyhow::Result<Docs> {
        match (&project.remote, branch) {
            // a remote project's repo is its clone, read at the commit its branch was fetched at
            (Some(remote), branch) => {
                let commit = remote.commit_of(branch).ok_or_else(|| NotFetched {
                    project: project.name.clone(),
                    branch: branch.unwrap_or(&remote.branch).to_string(),
                })?;
                Ok(Docs::Commit {
                    clone: project.repo.clone(),
                    commit: commit.to_string(),
                })
            }
            // a local project's own is its working tree, as it stands
            (None, None) => Ok(Docs::WorkingTree(project.repo.clone())),
            // and a branch of it is found as it is opened
            (None, Some(branch)) => Ok(Docs::LocalBranch {
                repo: project.repo.clone(),
                branch: branch.to_string(),
            }),
        }
    }

    /// Opens the docs to read, off the async threads, since each read is a file's or an object's
    pub fn source(&self) -> anyhow::Result<RepoDocs> {
        match self {
            Docs::WorkingTree(root) => Ok(RepoDocs::WorkingTree(RepoSource::new(root))),
            Docs::Commit { clone, commit } => {
                Ok(RepoDocs::Commit(Box::new(TreeSource::open(clone, commit)?)))
            }
            Docs::LocalBranch { repo, branch } => {
                // a worktree with the branch checked out, the main one first, else its tip
                let opened = open_repo(repo)?;
                if let Some(worktree) = checked_out(&opened, branch)? {
                    return Ok(RepoDocs::WorkingTree(RepoSource::new(&worktree)));
                }
                let commit = branch_tip(&opened, branch)?;
                Ok(RepoDocs::Commit(Box::new(TreeSource::open(repo, &commit)?)))
            }
        }
    }
}

/// Opens a local project's repo, reading no config but its own
///
/// # Arguments
///
/// * `repo` - The root of its main worktree
fn open_repo(repo: &Path) -> anyhow::Result<gix::Repository> {
    gix::open_opts(repo, gix::open::Options::isolated())
        .map_err(|error| anyhow::anyhow!("the repo at {} can't be read: {error}", repo.display()))
}

/// The worktree of a repo that has a branch checked out, the main one first, if one has
///
/// # Arguments
///
/// * `repo` - The repo, opened
/// * `branch` - The branch, without `refs/heads/`
fn checked_out(repo: &gix::Repository, branch: &str) -> anyhow::Result<Option<PathBuf>> {
    // the full name each worktree's HEAD would name
    let wanted = format!("refs/heads/{branch}");
    for worktree in repo.worktrees_including_main()? {
        // one that can't be read, or whose checkout is gone, has nothing checked out
        let Ok(worktree) = worktree else {
            continue;
        };
        let Some(dir) = worktree.workdir().filter(|dir| dir.is_dir()) else {
            continue;
        };
        let head = worktree.head_name().ok().flatten();
        if head.is_some_and(|head| head.as_bstr() == wanted.as_bytes()) {
            return Ok(Some(dir.to_path_buf()));
        }
    }
    Ok(None)
}

/// The commit a local branch is at, as its full hex id
///
/// # Arguments
///
/// * `repo` - The repo, opened
/// * `branch` - The branch, without `refs/heads/`
fn branch_tip(repo: &gix::Repository, branch: &str) -> anyhow::Result<String> {
    // a branch since deleted says how to follow another
    let name = format!("refs/heads/{branch}");
    let Some(mut found) = repo.try_find_reference(name.as_str())? else {
        anyhow::bail!(
            "the repo at {} has no branch `{branch}`: follow another with `reef plan branch`",
            repo.workdir().unwrap_or(repo.git_dir()).display()
        );
    };
    Ok(found.peel_to_id()?.detach().to_string())
}

/// A local project's repo's branches, by name, in the order git keeps them
///
/// # Arguments
///
/// * `repo` - The root of its main worktree
pub fn local_branches(repo: &Path) -> anyhow::Result<Vec<String>> {
    // every ref under `refs/heads/`, by its short name
    let opened = open_repo(repo)?;
    let mut names = Vec::new();
    for found in opened.references()?.local_branches()? {
        let found = found.map_err(|error| anyhow::anyhow!("reading a branch: {error}"))?;
        names.push(found.name().shorten().to_string());
    }
    Ok(names)
}

/// Whether a local project's repo has a branch, for a plan to follow it
///
/// # Arguments
///
/// * `repo` - The root of its main worktree
/// * `branch` - The branch, without `refs/heads/`
pub fn has_branch(repo: &Path, branch: &str) -> anyhow::Result<bool> {
    // a branch is a ref under `refs/heads/`
    let opened = open_repo(repo)?;
    let name = format!("refs/heads/{branch}");
    Ok(opened.try_find_reference(name.as_str())?.is_some())
}

/// A project's docs, opened: its working tree, or its clone at a commit
pub enum RepoDocs {
    /// A local project's working tree
    WorkingTree(RepoSource),
    /// A remote project's clone at a commit, boxed since gix's repo is large
    Commit(Box<TreeSource>),
}

impl DocSource for RepoDocs {
    /// Reads a doc from the working tree or the commit
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the repo's root
    fn read(&self, path: &str) -> Option<String> {
        match self {
            RepoDocs::WorkingTree(source) => source.read(path),
            RepoDocs::Commit(source) => source.read(path),
        }
    }

    /// The headings of a doc in the working tree or at the commit
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the repo's root
    fn headings(&self, path: &str) -> Option<Headings> {
        match self {
            RepoDocs::WorkingTree(source) => source.headings(path),
            RepoDocs::Commit(source) => source.headings(path),
        }
    }

    /// Where a doc is looked for in the repo, or why it can't be there
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the repo's root
    fn place(&self, path: &str) -> Result<String, String> {
        match self {
            RepoDocs::WorkingTree(source) => source.place(path),
            RepoDocs::Commit(source) => source.place(path),
        }
    }

    /// The files and folders of a directory of the working tree or at the commit
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory's path from the repo's root
    fn list(&self, dir: &str) -> Vec<Entry> {
        // list the directory from the working tree or the commit, whichever the docs are read from
        match self {
            RepoDocs::WorkingTree(source) => source.list(dir),
            RepoDocs::Commit(source) => source.list(dir),
        }
    }
}
