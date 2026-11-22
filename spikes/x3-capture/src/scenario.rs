//! Scratch repos that put the rules through how nodes are really worked on
//!
//! Each case builds a repo with no remote, works one or more nodes in it with real git, and at
//! each check says what the node really changed: every path it touched, with the blob the path
//! held before the node and the blob it holds now. Every rule is tried at each check, against
//! the same repo, and a capture is then stored by the chosen rule, as a hook would store it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::capture::{self, Options, Reader};
use crate::diff::Sides;
use crate::git::{Git, NODE_TRAILER};
use crate::rule::Rule;
use crate::store::{Status, Store, Work};

/// The blob id git gives some contents
///
/// # Arguments
///
/// * `data` - The contents
pub fn blob(data: &[u8]) -> String {
    gix::objs::compute_hash(gix::hash::Kind::Sha1, gix::objs::Kind::Blob, data)
        .map(|id| id.to_string())
        .unwrap_or_default()
}

/// `n` numbered lines starting with a word, the body most files start with
///
/// # Arguments
///
/// * `word` - What each line starts with
/// * `n` - How many lines
pub fn lines(word: &str, n: usize) -> String {
    (1..=n).map(|at| format!("{word} {at}\n")).collect()
}

/// What a node really changed: each path, with its blob before the node and after
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expect {
    /// The paths and their two sides
    pub sides: BTreeMap<String, Sides>,
}

impl Expect {
    /// Nothing changed yet
    pub fn new() -> Self {
        Expect::default()
    }

    /// A file the node added
    ///
    /// # Arguments
    ///
    /// * `path` - Its path
    /// * `new` - Its contents
    pub fn added(mut self, path: &str, new: &str) -> Self {
        self.sides.insert(
            path.to_owned(),
            Sides {
                old: None,
                new: Some(blob(new.as_bytes())),
            },
        );
        self
    }

    /// A binary file the node added
    ///
    /// # Arguments
    ///
    /// * `path` - Its path
    /// * `new` - Its bytes
    pub fn added_bytes(mut self, path: &str, new: &[u8]) -> Self {
        self.sides.insert(
            path.to_owned(),
            Sides {
                old: None,
                new: Some(blob(new)),
            },
        );
        self
    }

    /// A file the node changed
    ///
    /// # Arguments
    ///
    /// * `path` - Its path
    /// * `old` - What it held before
    /// * `new` - What it holds now
    pub fn modified(mut self, path: &str, old: &str, new: &str) -> Self {
        self.sides.insert(
            path.to_owned(),
            Sides {
                old: Some(blob(old.as_bytes())),
                new: Some(blob(new.as_bytes())),
            },
        );
        self
    }

    /// A file the node deleted
    ///
    /// # Arguments
    ///
    /// * `path` - Its path
    /// * `old` - What it held before
    pub fn deleted(mut self, path: &str, old: &str) -> Self {
        self.sides.insert(
            path.to_owned(),
            Sides {
                old: Some(blob(old.as_bytes())),
                new: None,
            },
        );
        self
    }

    /// Says how a captured diff's paths differ from these, or nothing if they agree
    ///
    /// # Arguments
    ///
    /// * `got` - The captured diff's sides
    pub fn compare(&self, got: &BTreeMap<String, Sides>) -> Option<String> {
        // paths only one side has, then paths both have with different blobs
        let extra: Vec<&str> = got
            .keys()
            .filter(|path| !self.sides.contains_key(*path))
            .map(String::as_str)
            .collect();
        let missing: Vec<&str> = self
            .sides
            .keys()
            .filter(|path| !got.contains_key(*path))
            .map(String::as_str)
            .collect();
        let wrong: Vec<&str> = self
            .sides
            .iter()
            .filter(|(path, sides)| got.get(*path).is_some_and(|got| got != *sides))
            .map(|(path, _)| path.as_str())
            .collect();
        if extra.is_empty() && missing.is_empty() && wrong.is_empty() {
            return None;
        }
        // name each kind of difference that turned up
        let mut parts = Vec::new();
        if !extra.is_empty() {
            parts.push(format!("also {}", extra.join(", ")));
        }
        if !missing.is_empty() {
            parts.push(format!("lacks {}", missing.join(", ")));
        }
        if !wrong.is_empty() {
            parts.push(format!("wrong side of {}", wrong.join(", ")));
        }
        Some(parts.join("; "))
    }
}

/// How one rule did at one check
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// Whether the captured diff was exactly the node's changes
    pub pass: bool,
    /// How the rule found its base, and what it got wrong if it did
    pub note: String,
}

/// One check of one node, by every rule
#[derive(Debug, Clone)]
pub struct Checked {
    /// The node checked
    pub node: String,
    /// Each rule's verdict
    pub verdicts: BTreeMap<Rule, Verdict>,
    /// Whether gix read the chosen rule's diff exactly as git's CLI did
    pub gix_agrees: bool,
}

/// A scratch repo with a main worktree, linked worktrees beside it, and a state dir
pub struct Scratch {
    /// The temporary dir everything lives in, removed when this is dropped
    _tmp: tempfile::TempDir,
    /// The main worktree, where `main` is checked out
    pub main: PathBuf,
    /// The state dir, outside every checkout
    pub store: Store,
    /// Each check so far
    pub checks: Vec<Checked>,
}

/// The files every scratch repo starts with
pub fn initial() -> Vec<(&'static str, String)> {
    vec![
        ("README.md", "scratch\n".to_owned()),
        ("shared.txt", lines("shared", 10)),
        ("a/keep.txt", lines("a keep", 8)),
        ("a/old.txt", lines("a old", 12)),
        ("b/keep.txt", lines("b keep", 8)),
        ("b/gone.txt", "b gone\n".to_owned()),
        ("c/keep.txt", lines("c keep", 8)),
        ("d/keep.txt", lines("d keep", 8)),
    ]
}

impl Scratch {
    /// A new repo on `main` with the initial files committed, and no remote
    pub fn new() -> anyhow::Result<Self> {
        // a dir of its own, under the system's temp dir
        let tmp = tempfile::Builder::new().prefix("x3-").tempdir()?;
        let main = tmp.path().join("repo");
        std::fs::create_dir_all(&main)?;
        let store = Store::isolated(tmp.path().join("state"))?;
        let scratch = Scratch {
            _tmp: tmp,
            main: main.canonicalize()?,
            store,
            checks: Vec::new(),
        };
        // the repo, its first files and its first commit
        scratch
            .git(&scratch.main)
            .run(&["init", "-q", "-b", "main"])?;
        for (path, contents) in initial() {
            scratch.write(&scratch.main, path, contents.as_bytes())?;
        }
        scratch.commit(&scratch.main, "initial", None)?;
        Ok(scratch)
    }

    /// Where the scratch repo's dirs are
    pub fn root(&self) -> &Path {
        self._tmp.path()
    }

    /// git in one of the scratch repo's worktrees
    ///
    /// # Arguments
    ///
    /// * `dir` - The worktree
    pub fn git(&self, dir: &Path) -> Git {
        Git::isolated(dir)
    }

    /// Adds a linked worktree on a new branch
    ///
    /// # Arguments
    ///
    /// * `name` - The worktree's dir, beside the main one
    /// * `branch` - The branch to make
    /// * `from` - What the branch starts at
    pub fn worktree(&self, name: &str, branch: &str, from: &str) -> anyhow::Result<PathBuf> {
        let path = self.root().join(name);
        let path_str = path.to_string_lossy().into_owned();
        self.git(&self.main)
            .run(&["worktree", "add", "-q", "-b", branch, &path_str, from])?;
        Ok(path.canonicalize()?)
    }

    /// Writes a file in a worktree, making its dirs
    ///
    /// # Arguments
    ///
    /// * `dir` - The worktree
    /// * `path` - The file, from the worktree's root
    /// * `contents` - What it holds
    pub fn write(&self, dir: &Path, path: &str, contents: &[u8]) -> anyhow::Result<()> {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Deletes a file in a worktree
    ///
    /// # Arguments
    ///
    /// * `dir` - The worktree
    /// * `path` - The file, from the worktree's root
    pub fn remove(&self, dir: &Path, path: &str) -> anyhow::Result<()> {
        std::fs::remove_file(dir.join(path))?;
        Ok(())
    }

    /// Moves a file in a worktree, writing new contents at its new path
    ///
    /// # Arguments
    ///
    /// * `dir` - The worktree
    /// * `from` - Where it was
    /// * `to` - Where it goes
    /// * `contents` - What it holds there
    pub fn rename(&self, dir: &Path, from: &str, to: &str, contents: &[u8]) -> anyhow::Result<()> {
        self.remove(dir, from)?;
        self.write(dir, to, contents)
    }

    /// Commits everything in a worktree, naming the node in a trailer as C31's hook would
    ///
    /// # Arguments
    ///
    /// * `dir` - The worktree
    /// * `message` - The commit's message
    /// * `node` - The node to name, if any
    pub fn commit(&self, dir: &Path, message: &str, node: Option<&str>) -> anyhow::Result<()> {
        let git = self.git(dir);
        git.run(&["add", "-A"])?;
        let trailer = node.map(|node| format!("{NODE_TRAILER}: x3/plan/{node}"));
        let mut args = vec!["commit", "-q", "-m", message];
        if let Some(trailer) = &trailer {
            args.extend(["--trailer", trailer]);
        }
        git.run(&args)?;
        Ok(())
    }

    /// Amends the last commit with everything in a worktree, keeping its message
    ///
    /// # Arguments
    ///
    /// * `dir` - The worktree
    pub fn amend(&self, dir: &Path) -> anyhow::Result<()> {
        let git = self.git(dir);
        git.run(&["add", "-A"])?;
        git.run(&["commit", "-q", "--amend", "--no-edit"])?;
        Ok(())
    }

    /// Squash-merges a branch into `main` in the main worktree, as a PR's squash would
    ///
    /// # Arguments
    ///
    /// * `branch` - The branch to squash in
    pub fn squash_into_main(&self, branch: &str) -> anyhow::Result<()> {
        let git = self.git(&self.main);
        git.run(&["merge", "-q", "--squash", branch])?;
        git.run(&["commit", "-q", "-m", &format!("{branch}, squashed")])?;
        Ok(())
    }

    /// Starts a node in a worktree, with `main` as its base branch
    ///
    /// # Arguments
    ///
    /// * `dir` - The worktree
    /// * `node` - The node
    /// * `needs` - The nodes it needs
    pub fn start(&self, dir: &Path, node: &str, needs: &[&str]) -> anyhow::Result<Work> {
        let needs: Vec<String> = needs.iter().map(|need| need.to_string()).collect();
        capture::start(&self.store, &self.git(dir), node, Some("main"), &needs)
    }

    /// Marks a node done
    ///
    /// # Arguments
    ///
    /// * `node` - The node
    pub fn done(&self, node: &str) -> anyhow::Result<()> {
        self.store.set_status(node, Status::Done)
    }

    /// Tries every rule on a node now, then stores its capture by the chosen rule
    ///
    /// # Arguments
    ///
    /// * `node` - The node
    /// * `expect` - What it really changed
    pub fn check(&mut self, node: &str, expect: &Expect) -> anyhow::Result<()> {
        let state = self.store.load()?;
        let work = state
            .work
            .get(node)
            .ok_or_else(|| anyhow::anyhow!("{node} wasn't started"))?;
        let git = self.git(&work.worktree);
        // every rule, against the repo as it is now
        let mut verdicts = BTreeMap::new();
        for rule in Rule::ALL {
            let verdict = match capture::take_in(&git, &state, work, rule, Reader::Cli) {
                Ok(taken) => match expect.compare(&taken.diff.sides()) {
                    None => Verdict {
                        pass: true,
                        note: taken.base.how,
                    },
                    Some(wrong) => Verdict {
                        pass: false,
                        note: format!("{}: {wrong}", taken.base.how),
                    },
                },
                Err(error) => Verdict {
                    pass: false,
                    note: format!("{error:#}"),
                },
            };
            verdicts.insert(rule, verdict);
        }
        // gix must read the chosen rule's diff exactly as the CLI did
        let gix_agrees = match (
            capture::take_in(&git, &state, work, Rule::CHOSEN, Reader::Cli),
            capture::take_in(&git, &state, work, Rule::CHOSEN, Reader::Gix),
        ) {
            (Ok(cli), Ok(gix)) => cli.diff.sorted() == gix.diff.sorted(),
            _ => false,
        };
        // store it as a hook would, so later nodes see this one's capture
        capture::capture(&self.store, node, &Options::chosen("scenario"))?;
        self.checks.push(Checked {
            node: node.to_owned(),
            verdicts,
            gix_agrees,
        });
        Ok(())
    }
}

/// One case: its name, what it shows, and how to build and check it
pub struct Case {
    /// Its short name, as the matrix and the tests use it
    pub name: &'static str,
    /// What it puts the rules through
    pub title: &'static str,
    /// Builds the repo and checks its nodes
    pub run: fn(&mut Scratch) -> anyhow::Result<()>,
}

/// Two worktrees from `main` worked at once, `main` moving on meanwhile
///
/// # Arguments
///
/// * `s` - The scratch repo
fn parallel(s: &mut Scratch) -> anyhow::Result<()> {
    // a worktree and a branch a node
    let wa = s.worktree("wt-a", "reef/A", "main")?;
    let wb = s.worktree("wt-b", "reef/B", "main")?;
    s.start(&wa, "A", &[])?;
    s.start(&wb, "B", &[])?;
    // commits from both, and from main between them
    s.write(&wa, "a/one.txt", b"one\n")?;
    s.commit(&wa, "A: one", Some("A"))?;
    s.write(&s.main, "upstream/1.txt", b"up\n")?;
    s.commit(&s.main.clone(), "upstream 1", None)?;
    let b_keep = lines("b keep", 8).replace("b keep 4", "b keep four");
    s.write(&wb, "b/keep.txt", b_keep.as_bytes())?;
    s.commit(&wb, "B: keep", Some("B"))?;
    let a_keep = lines("a keep", 8).replace("a keep 2", "a keep two");
    s.write(&wa, "a/keep.txt", a_keep.as_bytes())?;
    let a_new = lines("a old", 12).replace("a old 6", "a old six");
    s.rename(&wa, "a/old.txt", "a/new.txt", a_new.as_bytes())?;
    s.commit(&wa, "A: keep, rename", Some("A"))?;
    s.remove(&wb, "b/gone.txt")?;
    s.commit(&wb, "B: gone", Some("B"))?;
    // uncommitted: an untracked file, an unstaged edit, a binary file and a staged one
    s.write(&wa, "a/untracked.txt", b"u\n")?;
    s.write(&wa, "a/one.txt", b"one\nmore\n")?;
    s.write(&wb, "b/blob.bin", b"\0\x01bin")?;
    s.write(&wb, "b/staged.txt", b"staged\n")?;
    s.git(&wb).run(&["add", "b/staged.txt"])?;
    // each node's changes, and only its own
    s.check(
        "A",
        &Expect::new()
            .added("a/one.txt", "one\nmore\n")
            .modified("a/keep.txt", &lines("a keep", 8), &a_keep)
            .deleted("a/old.txt", &lines("a old", 12))
            .added("a/new.txt", &a_new)
            .added("a/untracked.txt", "u\n"),
    )?;
    s.check(
        "B",
        &Expect::new()
            .modified("b/keep.txt", &lines("b keep", 8), &b_keep)
            .deleted("b/gone.txt", "b gone\n")
            .added_bytes("b/blob.bin", b"\0\x01bin")
            .added("b/staged.txt", "staged\n"),
    )
}

/// A node amends its own last commit before it is captured
///
/// # Arguments
///
/// * `s` - The scratch repo
fn amend(s: &mut Scratch) -> anyhow::Result<()> {
    let wa = s.worktree("wt-a", "reef/A", "main")?;
    s.start(&wa, "A", &[])?;
    s.write(&wa, "a/one.txt", b"1\n")?;
    s.commit(&wa, "A: one", Some("A"))?;
    // the amend changes the commit and adds a file to it
    s.write(&wa, "a/one.txt", b"1\n2\n")?;
    s.write(&wa, "a/two.txt", b"two\n")?;
    s.amend(&wa)?;
    s.check(
        "A",
        &Expect::new()
            .added("a/one.txt", "1\n2\n")
            .added("a/two.txt", "two\n"),
    )
}

/// B is rebased onto a `main` that holds A, squash-merged, and more besides
///
/// # Arguments
///
/// * `s` - The scratch repo
fn rebase(s: &mut Scratch) -> anyhow::Result<()> {
    let wa = s.worktree("wt-a", "reef/A", "main")?;
    let wb = s.worktree("wt-b", "reef/B", "main")?;
    s.start(&wa, "A", &[])?;
    s.start(&wb, "B", &[])?;
    s.write(&wa, "a/one.txt", b"a\n")?;
    s.commit(&wa, "A: one", Some("A"))?;
    s.write(&wb, "b/one.txt", b"b\n")?;
    s.commit(&wb, "B: one", Some("B"))?;
    s.check("A", &Expect::new().added("a/one.txt", "a\n"))?;
    s.done("A")?;
    // A lands on main as one commit, then main moves on
    s.squash_into_main("reef/A")?;
    let main = s.main.clone();
    s.write(&main, "upstream/1.txt", b"up\n")?;
    s.commit(&main, "upstream 1", None)?;
    // B catches up by rebasing, then carries on
    s.git(&wb).run(&["rebase", "-q", "main"])?;
    s.write(&wb, "b/two.txt", b"b2\n")?;
    s.commit(&wb, "B: two", Some("B"))?;
    s.check(
        "B",
        &Expect::new()
            .added("b/one.txt", "b\n")
            .added("b/two.txt", "b2\n"),
    )
}

/// B starts from A's head, A done but not yet merged
///
/// # Arguments
///
/// * `s` - The scratch repo
fn need_unmerged(s: &mut Scratch) -> anyhow::Result<()> {
    let wa = s.worktree("wt-a", "reef/A", "main")?;
    s.start(&wa, "A", &[])?;
    s.write(&wa, "a/one.txt", b"a1\n")?;
    s.commit(&wa, "A: one", Some("A"))?;
    s.write(&wa, "a/two.txt", b"a2\n")?;
    s.commit(&wa, "A: two", Some("A"))?;
    s.check(
        "A",
        &Expect::new()
            .added("a/one.txt", "a1\n")
            .added("a/two.txt", "a2\n"),
    )?;
    s.done("A")?;
    // B's worktree starts where A left off
    let wb = s.worktree("wt-b", "reef/B", "reef/A")?;
    s.start(&wb, "B", &["A"])?;
    s.write(&wb, "b/one.txt", b"b1\n")?;
    s.commit(&wb, "B: one", Some("B"))?;
    s.write(&wb, "b/two.txt", b"b2\n")?;
    s.check(
        "B",
        &Expect::new()
            .added("b/one.txt", "b1\n")
            .added("b/two.txt", "b2\n"),
    )
}

/// The same, then A is squash-merged and B rebased onto `main` past A's commits
///
/// # Arguments
///
/// * `s` - The scratch repo
fn need_merged(s: &mut Scratch) -> anyhow::Result<()> {
    let wa = s.worktree("wt-a", "reef/A", "main")?;
    s.start(&wa, "A", &[])?;
    s.write(&wa, "a/one.txt", b"a1\n")?;
    s.commit(&wa, "A: one", Some("A"))?;
    s.write(&wa, "a/one.txt", b"a1\na1b\n")?;
    s.commit(&wa, "A: one again", Some("A"))?;
    s.check("A", &Expect::new().added("a/one.txt", "a1\na1b\n"))?;
    s.done("A")?;
    let wb = s.worktree("wt-b", "reef/B", "reef/A")?;
    s.start(&wb, "B", &["A"])?;
    s.write(&wb, "b/one.txt", b"b1\n")?;
    s.commit(&wb, "B: one", Some("B"))?;
    // A lands on main as one commit, main moves on, and B moves onto it, leaving A's commits
    s.squash_into_main("reef/A")?;
    let main = s.main.clone();
    s.write(&main, "upstream/1.txt", b"up\n")?;
    s.commit(&main, "upstream 1", None)?;
    s.git(&wb)
        .run(&["rebase", "-q", "--onto", "main", "reef/A"])?;
    s.write(&wb, "b/two.txt", b"b2\n")?;
    s.commit(&wb, "B: two", Some("B"))?;
    s.check(
        "B",
        &Expect::new()
            .added("b/one.txt", "b1\n")
            .added("b/two.txt", "b2\n"),
    )
}

/// Works C to done, on branch `work` in a worktree of its own
///
/// # Arguments
///
/// * `s` - The scratch repo
fn c_on_work(s: &mut Scratch) -> anyhow::Result<PathBuf> {
    let wt = s.worktree("wt", "work", "main")?;
    s.start(&wt, "C", &[])?;
    s.write(&wt, "c/one.txt", b"c1\n")?;
    s.commit(&wt, "C: one", Some("C"))?;
    let c_keep = lines("c keep", 8).replace("c keep 3", "c keep three");
    s.write(&wt, "c/keep.txt", c_keep.as_bytes())?;
    s.commit(&wt, "C: keep", Some("C"))?;
    s.check(
        "C",
        &Expect::new().added("c/one.txt", "c1\n").modified(
            "c/keep.txt",
            &lines("c keep", 8),
            &c_keep,
        ),
    )?;
    s.done("C")?;
    Ok(wt)
}

/// C then D, one after the other on one branch
///
/// # Arguments
///
/// * `s` - The scratch repo
fn in_a_row(s: &mut Scratch) -> anyhow::Result<()> {
    let wt = c_on_work(s)?;
    s.start(&wt, "D", &[])?;
    s.write(&wt, "d/one.txt", b"d1\n")?;
    s.commit(&wt, "D: one", Some("D"))?;
    let d_keep = lines("d keep", 8).replace("d keep 5", "d keep five");
    s.write(&wt, "d/keep.txt", d_keep.as_bytes())?;
    s.check(
        "D",
        &Expect::new().added("d/one.txt", "d1\n").modified(
            "d/keep.txt",
            &lines("d keep", 8),
            &d_keep,
        ),
    )
}

/// The same, with the branch rebased onto a newer `main` between C's capture and D's
///
/// # Arguments
///
/// * `s` - The scratch repo
fn in_a_row_rebased(s: &mut Scratch) -> anyhow::Result<()> {
    let wt = c_on_work(s)?;
    s.start(&wt, "D", &[])?;
    s.write(&wt, "d/one.txt", b"d1\n")?;
    s.commit(&wt, "D: one", Some("D"))?;
    // main moves on, and the whole branch, C's commits too, is rebased onto it
    let main = s.main.clone();
    s.write(&main, "upstream/1.txt", b"up\n")?;
    s.commit(&main, "upstream 1", None)?;
    s.git(&wt).run(&["rebase", "-q", "main"])?;
    s.write(&wt, "d/two.txt", b"d2\n")?;
    s.commit(&wt, "D: two", Some("D"))?;
    let d_keep = lines("d keep", 8).replace("d keep 5", "d keep five");
    s.write(&wt, "d/keep.txt", d_keep.as_bytes())?;
    s.check(
        "D",
        &Expect::new()
            .added("d/one.txt", "d1\n")
            .added("d/two.txt", "d2\n")
            .modified("d/keep.txt", &lines("d keep", 8), &d_keep),
    )
}

/// B merges `main` in instead of rebasing
///
/// # Arguments
///
/// * `s` - The scratch repo
fn main_merged(s: &mut Scratch) -> anyhow::Result<()> {
    let wb = s.worktree("wt-b", "reef/B", "main")?;
    s.start(&wb, "B", &[])?;
    s.write(&wb, "b/one.txt", b"b1\n")?;
    s.commit(&wb, "B: one", Some("B"))?;
    let main = s.main.clone();
    s.write(&main, "upstream/1.txt", b"up\n")?;
    s.commit(&main, "upstream 1", None)?;
    s.git(&wb).run(&["merge", "-q", "--no-edit", "main"])?;
    s.write(&wb, "b/two.txt", b"b2\n")?;
    s.commit(&wb, "B: two", Some("B"))?;
    s.check(
        "B",
        &Expect::new()
            .added("b/one.txt", "b1\n")
            .added("b/two.txt", "b2\n"),
    )
}

/// D's first change is amended into C's last commit
///
/// # Arguments
///
/// * `s` - The scratch repo
fn amend_into_last(s: &mut Scratch) -> anyhow::Result<()> {
    let wt = c_on_work(s)?;
    s.start(&wt, "D", &[])?;
    s.write(&wt, "d/one.txt", b"d1\n")?;
    s.amend(&wt)?;
    s.check("D", &Expect::new().added("d/one.txt", "d1\n"))
}

/// C is done with some of its work uncommitted when D starts, and one commit holds both
///
/// # Arguments
///
/// * `s` - The scratch repo
fn left_uncommitted(s: &mut Scratch) -> anyhow::Result<()> {
    let wt = s.worktree("wt", "work", "main")?;
    s.start(&wt, "C", &[])?;
    s.write(&wt, "c/one.txt", b"c1\n")?;
    s.commit(&wt, "C: one", Some("C"))?;
    s.write(&wt, "c/two.txt", b"c2\n")?;
    s.check(
        "C",
        &Expect::new()
            .added("c/one.txt", "c1\n")
            .added("c/two.txt", "c2\n"),
    )?;
    s.done("C")?;
    // D starts with C's last file still uncommitted, then commits both together
    s.start(&wt, "D", &[])?;
    s.write(&wt, "d/one.txt", b"d1\n")?;
    s.commit(&wt, "C's two and D's one", Some("D"))?;
    s.check("D", &Expect::new().added("d/one.txt", "d1\n"))
}

/// C then D, worked on `main` itself, with no other branch
///
/// # Arguments
///
/// * `s` - The scratch repo
fn on_main(s: &mut Scratch) -> anyhow::Result<()> {
    let main = s.main.clone();
    s.start(&main, "C", &[])?;
    s.write(&main, "c/one.txt", b"c1\n")?;
    s.commit(&main, "C: one", Some("C"))?;
    s.check("C", &Expect::new().added("c/one.txt", "c1\n"))?;
    s.done("C")?;
    s.start(&main, "D", &[])?;
    s.write(&main, "d/one.txt", b"d1\n")?;
    s.commit(&main, "D: one", Some("D"))?;
    s.check("D", &Expect::new().added("d/one.txt", "d1\n"))
}

/// A and B worked at once in one worktree, their commits interleaved
///
/// # Arguments
///
/// * `s` - The scratch repo
fn two_at_once(s: &mut Scratch) -> anyhow::Result<()> {
    let wt = s.worktree("wt", "work", "main")?;
    s.start(&wt, "A", &[])?;
    s.start(&wt, "B", &[])?;
    s.write(&wt, "a/one.txt", b"a1\n")?;
    s.commit(&wt, "A: one", Some("A"))?;
    s.write(&wt, "b/one.txt", b"b1\n")?;
    s.commit(&wt, "B: one", Some("B"))?;
    s.write(&wt, "a/two.txt", b"a2\n")?;
    s.commit(&wt, "A: two", Some("A"))?;
    s.check(
        "A",
        &Expect::new()
            .added("a/one.txt", "a1\n")
            .added("a/two.txt", "a2\n"),
    )?;
    s.check("B", &Expect::new().added("b/one.txt", "b1\n"))
}

/// C and D in a row, rebased onto a `main` that changed the line C changed
///
/// # Arguments
///
/// * `s` - The scratch repo
fn rebase_conflict(s: &mut Scratch) -> anyhow::Result<()> {
    let wt = s.worktree("wt", "work", "main")?;
    s.start(&wt, "C", &[])?;
    let shared = lines("shared", 10);
    let by_c = shared.replace("shared 5", "shared five, by C");
    s.write(&wt, "shared.txt", by_c.as_bytes())?;
    s.commit(&wt, "C: shared", Some("C"))?;
    s.check("C", &Expect::new().modified("shared.txt", &shared, &by_c))?;
    s.done("C")?;
    s.start(&wt, "D", &[])?;
    s.write(&wt, "d/one.txt", b"d1\n")?;
    s.commit(&wt, "D: one", Some("D"))?;
    // main changes the same line, and the rebase's conflict is resolved by hand
    let main = s.main.clone();
    let upstream = shared.replace("shared 5", "shared five, upstream");
    s.write(&main, "shared.txt", upstream.as_bytes())?;
    s.commit(&main, "upstream shared", None)?;
    let git = s.git(&wt);
    let _ = git.run(&["rebase", "-q", "main"]);
    let resolved = shared.replace("shared 5", "shared five, by C and upstream");
    s.write(&wt, "shared.txt", resolved.as_bytes())?;
    git.run(&["add", "shared.txt"])?;
    git.run_env(&["rebase", "--continue"], &[("GIT_EDITOR", "true")])?;
    s.write(&wt, "d/two.txt", b"d2\n")?;
    s.commit(&wt, "D: two", Some("D"))?;
    s.check(
        "D",
        &Expect::new()
            .added("d/one.txt", "d1\n")
            .added("d/two.txt", "d2\n"),
    )
}

/// Every case, in the matrix's order
pub const CASES: &[Case] = &[
    Case {
        name: "parallel",
        title: "Two worktrees at once, main moving on",
        run: parallel,
    },
    Case {
        name: "amend",
        title: "Amending its own commit",
        run: amend,
    },
    Case {
        name: "rebase",
        title: "Rebased onto main holding another node",
        run: rebase,
    },
    Case {
        name: "need-unmerged",
        title: "Started from a done need's unmerged head",
        run: need_unmerged,
    },
    Case {
        name: "need-merged",
        title: "Then the need squash-merged, and rebased past it",
        run: need_merged,
    },
    Case {
        name: "in-a-row",
        title: "Nodes in a row on one branch",
        run: in_a_row,
    },
    Case {
        name: "in-a-row-rebased",
        title: "In a row, rebased between them",
        run: in_a_row_rebased,
    },
    Case {
        name: "main-merged",
        title: "Main merged in instead of a rebase",
        run: main_merged,
    },
    Case {
        name: "amend-into-last",
        title: "Amending into the last node's commit",
        run: amend_into_last,
    },
    Case {
        name: "left-uncommitted",
        title: "The last node's work left uncommitted",
        run: left_uncommitted,
    },
    Case {
        name: "on-main",
        title: "In a row on main itself",
        run: on_main,
    },
    Case {
        name: "two-at-once",
        title: "Two at once in one worktree",
        run: two_at_once,
    },
    Case {
        name: "rebase-conflict",
        title: "In a row, a rebase conflict in the last node's file",
        run: rebase_conflict,
    },
];

/// One case's outcome: whether each rule passed every check in it
pub struct Outcome {
    /// The case
    pub case: &'static Case,
    /// Each check made in it
    pub checks: Vec<Checked>,
}

impl Outcome {
    /// Whether a rule passed every check in the case
    ///
    /// # Arguments
    ///
    /// * `rule` - The rule
    pub fn passed(&self, rule: Rule) -> bool {
        self.checks.iter().all(|check| {
            check
                .verdicts
                .get(&rule)
                .is_some_and(|verdict| verdict.pass)
        })
    }

    /// Whether gix agreed with the CLI at every check
    pub fn gix_agrees(&self) -> bool {
        self.checks.iter().all(|check| check.gix_agrees)
    }
}

/// Runs one case in a fresh scratch repo
///
/// # Arguments
///
/// * `case` - The case
pub fn run(case: &'static Case) -> anyhow::Result<Outcome> {
    let mut scratch = Scratch::new()?;
    (case.run)(&mut scratch).map_err(|error| anyhow::anyhow!("{}: {error:#}", case.name))?;
    // no case may need a remote
    if !scratch.git(&scratch.main).remotes()?.is_empty() {
        anyhow::bail!("{} has a remote", case.name);
    }
    Ok(Outcome {
        case,
        checks: scratch.checks,
    })
}
