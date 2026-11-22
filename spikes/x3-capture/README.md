# X3: capturing a node's changes

This spike picks out one node's changes from the local checkout its work happens in, then
stores them where no squash can take them. Several nodes are worked on at once, and others run
one after another on one branch. Each one's history gets amended, rebased, merged and squashed.

Nothing is pushed or fetched. A CLI run in the node's checkout reads only local data:
- commits
- the index
- the worktree

What it reads goes to a state dir outside every checkout, which stands in for shoal. A Claude
Code hook or a git hook runs the CLI. The CLI is the spike's `capture` command, standing in for
`reef node capture`.

The crate is a workspace of its own on stable, which reef's root workspace excludes.

- **`src/git.rs`** runs git's CLI in one checkout. It removes the `GIT_*` variables that would
  point it at another repo, as `central/reef-service/src/repo.rs` does.
  - `snapshot` writes the worktree as one tree, uncommitted changes and untracked files
    included. It copies the worktree's index, then runs `git add -A` and `git write-tree`
    against the copy, so the user's own index is never touched.
- **`src/gix.rs`** reads the same diff through gix 0.90. It does a tree-to-tree diff with
  renames at 50%, then a Myers line diff with git's slider heuristics, cut into three-line
  hunks.
- **`src/rule.rs`** holds the six candidate rules for the base a node's diff starts from.
- **`src/scenario.rs`** builds 13 scratch repos with no remote and works nodes in them with real
  git. Every rule is tried at each check, against what the node really changed: each path, with
  its blob before the node and after.
  - **`src/matrix.rs`** prints how each rule did.
- **`src/capture.rs`** has `start` and `capture`.
  - `start` is `reef start`. It records the work and pins the start under `refs/reef/start/<node>`.
  - `capture` snapshots the worktree, finds the base, diffs the two and stores the result as
    the node's draft revision, or seals it.
- **`src/hook.rs`** has every trigger: Claude Code's PostToolUse, Stop, WorktreeCreate and
  WorktreeRemove, and git's post-commit and post-rewrite.
- **`src/mcp.rs`** is a stub reef MCP server named `reef`. Its `status` tool takes reef's own
  arguments, so the PostToolUse hook sees `mcp__reef__status` as it would for real.
- **`src/bench.rs`** times the CLI against gix on real commits.
- **`setup.sh`, `drive.sh` and `runs.py`** run live headless Claude Code sessions against a
  scratch repo and write their timelines into [`runs/`](runs).

## Running it

```sh
cargo build --release && cargo test
./target/release/x3-capture matrix --notes          # every rule on every case, and why
# the bench, in a clone with no remote rather than a live repo
git clone -q --no-hardlinks ../.. /tmp/x3/reef && git -C /tmp/x3/reef remote remove origin
./target/release/x3-capture bench --repo /tmp/x3/reef \
  --commit 31155d8 --commit 37f44f1 --commit bf5b248
# the live runs: a repo, two worktrees and three sessions at once, then a squash and a gc
./setup.sh /tmp/x3/live/textkit /tmp/x3/live/state
./drive.sh run /tmp/x3/live/textkit-a A "Add textkit/slug.py with ..." /tmp/x3/live/a.jsonl done-first &
./drive.sh run /tmp/x3/live/textkit-b B "Add textkit/count.py with ..." /tmp/x3/live/b.jsonl done-first &
./drive.sh run-w /tmp/x3/live/textkit N3 "Add textkit/initials.py with ..." /tmp/x3/live/n3.jsonl &
wait
./drive.sh squash /tmp/x3/live/textkit /tmp/x3/live/state A B N3
python3 -I runs.py /tmp/x3/live "Run" runs/new.md a b n3
```

`drive.sh` runs each session under `env -i` on purpose. X4 found that a session started from
inside another one inherits that session's variables and hooks.

## The rules

| Rule | The base a node's diff starts from |
|---|---|
| merge-base | `merge-base(head, base branch)` |
| start | The worktree as it was when the node started, uncommitted changes included |
| captured | The newest of the merge-base, the start commit and other nodes' captured heads that the head descends from |
| trailers | The parent of the oldest commit since the merge-base whose `Reef-Node` trailer names the node |
| patch-id | *captured*, also finding other nodes' captured heads a rebase rewrote, by `git patch-id --stable` |
| carried | The start, carried forward over whatever the base branch brought in since it started |

**carried** in full:
1. `reef start` records two things:
   - the start: the worktree as a tree, uncommitted changes included
   - the fork: `merge-base(start, base branch)`
2. At capture, the rule takes the merge-base again. If it is still the fork, nothing came in
   from anyone else, and the base is the start's tree.
3. Otherwise the start is carried forward over what came in. That is a three-way
   `git merge-tree` of the start's tree and the new merge-base's tree, from the fork's tree: the
   same merge a rebase does. All three are trees, so no history needs to survive.

On `main` itself there is no base branch, unless `main` tracks an upstream. Then the base is
the start.

## The matrix

| Case | merge-base | start | captured | trailers | patch-id | carried | gix = CLI |
|---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| Two worktrees at once, main moving on | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Amending its own commit | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Rebased onto main holding another node | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Started from a done need's unmerged head | ✗ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Then the need squash-merged, and rebased past it | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Nodes in a row on one branch | ✗ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| In a row, rebased between them | ✗ | ✗ | ✗ | ✓ | ✓ | ✓ | ✓ |
| Main merged in instead of a rebase | ✓ | ✗ | ✓ | ✗ | ✓ | ✓ | ✓ |
| Amending into the last node's commit | ✗ | ✓ | ✗ | ✗ | ✗ | ✓ | ✓ |
| The last node's work left uncommitted | ✗ | ✓ | ✗ | ✗ | ✗ | ✓ | ✓ |
| In a row on main itself | ✗ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Two at once in one worktree | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✓ |
| In a row, a rebase conflict in the last node's file | ✗ | ✗ | ✗ | ✓ | ✗ | ✗ | ✓ |

`tests/rules.rs` pins every cell, and `matrix --notes` says what each rule got wrong.

- **merge-base** takes in every earlier node's commits when nodes run in a row, or when a node
  starts from a need that isn't merged yet.
- **start** takes in whatever came in from `main` by a rebase or a merge.
- **captured** loses its other nodes' heads to a rebase, an amend, or work left uncommitted.
- **trailers** can't see uncommitted work or an amend into another node's commit. It also takes
  in `main`'s changes when `main` is merged into the branch.
- **patch-id** fixes captured's rebase, but nothing else.
- **carried** fails only where nothing can win, or where a conflict was resolved during the
  node's work:
  - **Two at once in one worktree.** No rule can tell the two nodes' edits apart. So nodes
    worked at the same time each get a worktree.
  - **A rebase conflict in an earlier node's file.** The carry conflicts, and the conflicted
    file's diff holds how the earlier node's change was resolved. Only trailers get it right.
    carried names the conflicted files on the revision (`conflicts`), so the app can say so.

## Revisions, drafts and seals

The trigger rule is **one draft per node, sealed at Stop.**
- **Drafts:** every trigger runs the same capture in the node's own checkout. A node's latest
  revision is its draft until it is sealed. Each draft capture takes the draft's place,
  keeping its revision number, so git's hooks can capture every commit without a revision per
  commit.
- **Seals:** Stop seals the draft once the node is done, and so does a capture asked for by
  hand. Stop seals each done mark once. Marking the node done again, after a round of comment
  fixes, opens a new revision.
- **Unchanged trees:** a draft whose tree is unchanged takes the newer head as its provenance,
  so a sealed revision names the commit that holds it.

| Trigger | When it fires | Which node | What it does |
|---|---|---|---|
| PostToolUse on `mcp__reef__status` | Claude marks a node done | `tool_input.id` | Draft |
| Stop | Claude's turn ends | Each node done in this worktree whose done mark isn't sealed | Seals |
| post-commit | Every commit | The worktree's last started node | Draft, skipped mid-rebase or mid-merge |
| post-rewrite | After an amend or a rebase, a local squash included | The same | Draft |
| WorktreeCreate | `claude -w <node>` | The name | Makes `.claude/worktrees/<node>` on `reef/<node>` and starts the node there |
| WorktreeRemove | Claude Code removes that worktree | The worktree's node | A last draft, then removes the worktree, keeping the branch |

Both git hooks go in the repo's common `hooks/` dir and run in every linked worktree. Each
worktree finds its own node from the work records, keyed by worktree root.

## The live runs

Each run used Claude Code 2.1.330 with Opus 5.5, headless (`claude -p`), on the scratch
`textkit` package from `setup.sh`. Each run had a stand-in node per session, and no remote. The
timelines are in [`runs/`](runs).

- **[Run 1](runs/1.md), two worktrees at once, before drafts and seals.**
  - Both sessions committed first, then marked the node done.
  - post-commit captured each node, each with only its own file.
  - PostToolUse and Stop found nothing new.
  - The run showed a flaw: with nothing stored, Stop found the node still pending at every
    turn. Seals came from this.
- **[Run 2](runs/2.md), three sessions at once.**
  - A and B ran in their worktrees and were told to mark done first, as this repo does. N3 was
    started with `claude -w N3`, and reef's WorktreeCreate hook made its worktree.
  - PostToolUse took A's and B's uncommitted work at the done mark. post-commit found the commit
    unchanged. Stop sealed it.
  - The run showed a gap: the sealed revision still named the head from the done mark, before
    the commit.
- **[Run 3](runs/3.md), the same on the final code.**
  - Each node has one revision, sealed by Stop, naming its node's commit.
  - `drive.sh squash` then squash-merged all three into `main`. It deleted their branches,
    worktrees and pins, expired every reflog and ran `gc --prune=now`.
  - The three heads were gone from git. The state dir was byte-for-byte what it had been
    before.

The three runs cost $1.52 in all, $0.17 to $0.21 a session.

## The timings

`bench`, ten timed rounds after three warm-up rounds, with a release build. It ran on a Ryzen 9
9950X (32 threads), in a clone of reef on tmpfs with no remote. Each figure is a commit's diff
from its parent, read as a capture would:
- the CLI starts a process each round
- gix opens the repo each round
- *gix, repo open* reuses one open repo

| Step | Median, ms | Lowest | Highest |
|---|---:|---:|---:|
| X2's `31155d8` (16 files, +5886 -20): git CLI | 2.62 | 2.56 | 2.98 |
| `31155d8`: gix | 1.05 | 1.03 | 1.15 |
| `31155d8`: gix, repo open | 0.87 | 0.86 | 1.24 |
| X4's `37f44f1` (19 files, +3741 -35): git CLI | 2.40 | 2.36 | 2.67 |
| `37f44f1`: gix | 0.98 | 0.95 | 1.01 |
| `37f44f1`: gix, repo open | 0.80 | 0.79 | 0.81 |
| `bf5b248` (8 files, three copies of the 816 KiB `reef.js`): git CLI | 18.31 | 17.69 | 18.97 |
| `bf5b248`: gix | 7.04 | 6.95 | 8.30 |
| `bf5b248`: gix, repo open | 6.84 | 6.81 | 6.95 |
| Snapshot of the clone (336 files, 2 edited, 1 untracked) | 2.78 | 2.66 | 3.16 |

A whole capture of X2's commit is `x3-capture capture`, from its process start to a stored
revision. It covers the snapshot, the rule, the diff, the patch-id and writing 5,887 lines of
diff to the state file, over ten runs after three warm-up runs:

| Reader | Median, ms | Lowest | Highest |
|---|---:|---:|---:|
| git CLI | 19.95 | 16.26 | 20.96 |
| gix | 18.00 | 17.63 | 18.74 |

## What it found

The plan records these in `plan/src/design/code-and-comments.md`, under *What X3 found*.

- **The base is the start, carried forward.** carried is the one rule that held through every
  case reef must support:
  - nodes at once in worktrees
  - nodes in a row on one branch, including a rebase between them
  - amends, rebases and merges of `main`
  - work left uncommitted
  - a start from a need that isn't merged yet, then after that need is squash-merged

  It needs only C30's work record: the start's tree, the fork, and the base branch. It needs no
  trailer on any commit, and no history that a squash would take.
- **A worktree for each node at the same time, and none needed for nodes in a row.** Two nodes'
  edits in one worktree can't be told apart by any rule, trailers included. Nodes in a row can
  share a branch, which is how this repo works.
- **Trailers are not needed.** carried beat them on uncommitted work, amends into another
  node's commit, and a merge of `main`. They won only on a rebase conflict in an earlier node's
  file, which carried names instead.
- **The start is pinned, or a gc can take it.** After a rebase, only `refs/reef/start/<node>`
  keeps the start's tree and commit. `tests/capture.rs::the_pin_keeps_the_start` shows the
  capture failing once the pin is gone and git is pruned. The ref is local, and no push sends
  it, since push sends branches.
- **Stop seals, everything else keeps a draft current.**
  - PostToolUse on the done mark fires before the commit when Claude marks done first, as this
    repo's convention has it. Anything after it, such as the plan's export, would be missed if
    it sealed there.
  - git's post-commit captures work done outside any session, and every commit before a local
    squash.
  - Stop fires once the turn ends, so its seal holds everything. It runs before the hook starts
    the next node.
- **git's hooks work from every worktree**, from the repo's common hooks dir.
  - A rebase runs post-commit for each pick, with HEAD half built. The hook skips while
    `rebase-merge`, `MERGE_HEAD` or the like is there.
  - post-rewrite then runs once, after the rebase ends.
  - A local interactive squash left the capture's tree, and so its diff, the same.
- **Every hook ran in headless `-p`**: PostToolUse on an MCP tool, Stop, WorktreeCreate, and
  git's hooks under Claude's own `git commit`.
  - PostToolUse's `tool_input` is the tool's arguments: `{"id": "A", "status": "done"}`.
  - With `-p`, Claude Code doesn't remove its worktrees, so WorktreeRemove never ran live. The
    tests cover it.
- **reef can make the node's worktree.** A WorktreeCreate hook replaces Claude Code's own
  `git worktree add` and answers with the path. So `claude -w <node>` gets a worktree from reef,
  with the node started in it, in one step. Without the hook, by Claude Code's docs:
  - it makes the branch `worktree-<name>`
  - it bases the worktree on `origin/HEAD`, or on HEAD with no remote
  - it fetches the default branch first if it hasn't in a day

  reef's hook fetches nothing.
- **The CLI, not gix.**
  - gix read every case's diff exactly as the CLI did. That covers renames, a rename with an
    edit, mode changes, binary files, CRLF, a file with no final newline, emptied and filled
    files, and several hunks in one file.
  - gix took 2.5 to 2.7 times less time per diff, but the diff is a small share of a capture:
    19.95 ms against 18.00 ms whole.
  - The snapshot and the carry are git's CLI either way: `add -A` against a copy of the
    index, and `merge-tree`.
  - The CLI is what users run, and gix would add 50 gix crates to reef's CLI.
- **Untracked files that aren't ignored are captured.** That is what `git add -A` would commit,
  so the repo's `.gitignore` decides. In the live runs, Python's `__pycache__` landed in every
  node's revision, though no session committed it.
- **No remote is needed, or ever touched.** Every case runs with no remote. One test gives the
  repo a remote whose URL leads nowhere, and every rule still answers with nothing fetched.
