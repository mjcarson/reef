//! Docs read from git's objects at one commit, for a remote project, and the switch between
//! those and a local project's working tree
//!
//! A remote project's clone is bare, so its docs are blobs of the tree of the commit its row says
//! was last fetched. Every read of one snapshot names that commit, so a fetch that lands while a
//! write is checked changes nothing the check reads, and readers never see half of a fetch. The
//! rules a working tree's docs keep hold here too: no path that climbs out or starts from the
//! root is read, and a symlink is followed only while it stays inside the tree.

use gix::ObjectId;
use reef_api::Project;
use reef_render::paths::join;
use reef_render::{DocSource, Headings, RepoSource, leaves};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// How many symlinks one read follows before it gives up, as a loop of them would never end
const LINK_HOPS: usize = 8;

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

    /// The blob a path names at this commit, following symlinks that stay in the tree
    ///
    /// Each part of the path is looked up in turn, so a symlink to a directory is followed as a
    /// working tree would follow it. A link to an absolute path, or one whose target climbs out of
    /// the tree, names nothing, as does a directory or a submodule.
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the repo's root
    fn blob(&self, path: &str) -> Option<ObjectId> {
        // a path that climbs out, or starts from somewhere else, is refused as written
        if leaves(path) {
            return None;
        }
        // the parts still to look up, and the directories already walked into
        let mut rest: VecDeque<String> = parts(path);
        let mut walked: Vec<String> = Vec::new();
        let mut tree = self.repo.find_tree(self.tree).ok()?;
        let mut hops = 0;
        while let Some(part) = rest.pop_front() {
            // the entry this part names in the directory walked to
            let entry = tree.lookup_entry([part.as_str()]).ok()??;
            let mode = entry.mode();
            if mode.is_tree() {
                // a directory, walked into, if the path goes on
                tree = self.repo.find_tree(entry.object_id()).ok()?;
                walked.push(part);
            } else if mode.is_blob() {
                // a file, which is the doc only if the path ends here
                return rest.is_empty().then(|| entry.object_id());
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
                tree = self.repo.find_tree(self.tree).ok()?;
            } else {
                // a submodule's commit is in another repo
                return None;
            }
        }
        // a path that ends at a directory names no doc
        None
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
}

/// Where a project's docs are read from, which a blocking read opens
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Docs {
    /// A local project's working tree, as it stands
    WorkingTree(PathBuf),
    /// A remote project's clone, at the commit last fetched
    Commit {
        /// The bare clone
        clone: PathBuf,
        /// The commit to read at
        commit: String,
    },
}

impl Docs {
    /// Where a project's docs are read from
    ///
    /// # Arguments
    ///
    /// * `project` - The project, as the store lists it
    pub fn of(project: &Project) -> Docs {
        match &project.remote {
            // a remote project's repo is its clone, read at its row's commit
            Some(remote) => Docs::Commit {
                clone: project.repo.clone(),
                commit: remote.commit.clone(),
            },
            None => Docs::WorkingTree(project.repo.clone()),
        }
    }

    /// Opens the docs to read, off the async threads, since each read is a file's or an object's
    pub fn source(&self) -> anyhow::Result<RepoDocs> {
        match self {
            Docs::WorkingTree(root) => Ok(RepoDocs::WorkingTree(RepoSource::new(root))),
            Docs::Commit { clone, commit } => {
                Ok(RepoDocs::Commit(Box::new(TreeSource::open(clone, commit)?)))
            }
        }
    }
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
}
