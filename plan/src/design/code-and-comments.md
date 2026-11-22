# Code and comments

## Context

With the plan outside the repository, reef can show what each node changed. The user can then
comment on the changed lines, and Claude resolves those comments after the node it is on.

Git can't be the record of that. Commits are squashed regularly, so soon after a node is done its
commits, their SHAs and any trailers on them are gone. So reef reads git while a node is worked
on and when it is completed, and keeps what it read. That needs four things reef doesn't have:
- a way to pick out one node's changes while several are worked on at once
- hooks that capture them before a squash
- a store for what it captured
- comments that stay attached to their lines

## Capturing a node's changes

### Design choices

- **reef keeps the changes, and only reads git.** A node's diff is extracted from git when it is
  captured, and stored in shoal as a revision of the node's changes. Nothing is read back from
  git later, so a squash, a rebase or a deleted branch loses nothing reef shows.
- **Extracted from the local checkout, by reef's CLI.** `reef node capture` runs in the node's
  worktree and reads only local git data: its commits, its index and its worktree.
  - Nothing is pushed or fetched, and no remote is read.
  - The JSON API receives only what the CLI extracted. reef-service never reads a checkout, so
    capture is the same once the server is shared ([M10](../milestones/m10.md)).
- **A node's diff runs from its start, carried forward, to its worktree as it is.**
  - `reef start` records two things: the worktree as a tree, uncommitted changes included, and
    where it left its base branch.
  - A capture snapshots the worktree the same way: a copy of the index, `git add -A` and
    `git write-tree`. It diffs that from the start.
  - Sometimes the base branch has brought something in since, by a rebase or a merge. Then the
    start is first carried over it with a three-way `git merge-tree` of trees, the merge a
    rebase does.
  - Nothing depends on commits a squash removes, or on trailers.
- **The start is pinned** under `refs/reef/start/<node>`, so a gc after a rebase can't prune it.
  The ref is local, and no push sends it.
- **One draft, sealed at Stop** ([C56](../milestones/m7.md)).
  - git's post-commit and post-rewrite hooks, and PostToolUse on reef's status tool, keep a
    node's latest revision current as its draft.
  - The Stop hook seals the draft once the node is done, before it hands Claude the next node
    ([C41](../milestones/m8.md)).
  - Marking the node done again, after a round of comment fixes, seals a revision of its own.
  - `reef node capture` seals by hand, for work done outside a session.
- **reef makes a node's worktree.** Its WorktreeCreate hook replaces Claude Code's own, so
  `claude -w <node>` gets `.claude/worktrees/<node>` on branch `reef/<node>`, with the node
  started in it ([C60](../milestones/m8.md)). `reef start` starts a node in the worktree it runs in.
- **A work record** ([C30](../milestones/m7.md)).
  - `reef start` records the node's worktree, branch, base branch, start and fork, and pins the
    start under `refs/reef/start/<plan>/<id>`, as a commit of the start's tree whose parent is
    the start.
  - The base branch is `--base`, else the branch `origin/HEAD` names, else `main` or `master`.
    On the base branch itself, it is the upstream that branch tracks, or none.
  - A node has one record, in its project's partition, so a capture finds the node its
    worktree is on, the one started there last, across the project's plans.
  - Each capture adds the SHAs it read, naming its start by the pinned commit, so a capture of
    a start since replaced adds nothing. The SHAs are provenance only, and may stop resolving
    once squashed.
- **gix for every step but the snapshot.** The diff, the carry, merge-base and ancestry run
  in-process through gix, behind reef's own git module, and so do `reef start`'s reads and its
  pin ([C30](../milestones/m7.md)).
  - The snapshot stays `git add -A` and `git write-tree`, since gix can't stage a worktree. Any
    git version can do that, so reef needs a git binary but no recent one.
  - Every step meets at tree ids in the one object store, so gix reads the tree git just wrote.
  - Each gix step is tested against the CLI, which stays as its oracle. gix is pinned, since it
    is 0.x and breaks often.
  - Moving the snapshot to gix as well, so no git binary is needed, is [C70](../milestones/m7.md), and
    optional.
  - Not git2: it builds libgit2 from C, and falls short of git on filter drivers and newer
    index and repo formats ([What X3 found](#what-x3-found)).

### Nodes worked on at once (X3)

- **Each in its own worktree.** No rule can tell apart two nodes' edits in one worktree,
  trailers included, so nodes worked on at the same time each get a worktree and a branch.
  Every capture finds its node from the work record of the worktree it runs in.
- **Nodes in a row share a branch**, as this repo's do. Each node's start is where the last one
  left off, so its diff holds only its own changes, through rebases between them.
- **A node whose need is done but not merged** starts from that need's head. Its start then
  holds the need's work, and its diff only its own. Once the need is squash-merged and the node
  rebased past it, the carry follows.
- **No trailers.** The start carried forward did better than `Reef-Node` trailers everywhere
  except a rebase conflict, so nothing adds them.

### What X3 found

`spikes/x3-capture` tried six rules for the base of a node's diff. Each ran on 13 scratch repos
with no remote, worked with real git, and was checked against what each node really changed:
every path, with its blob before and after. Real git hooks ran in two worktrees. Three headless
Claude Code 2.1.330 sessions ran at once, one through `claude -w`. The repos were then
squash-merged and pruned.

| Case | merge-base | start | captured | trailers | patch-id | carried |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| Two worktrees at once, main moving on | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Amending its own commit | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Rebased onto main holding another node | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ |
| Started from a done need's unmerged head | ✗ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Then the need squash-merged, and rebased past it | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ |
| Nodes in a row on one branch | ✗ | ✓ | ✓ | ✓ | ✓ | ✓ |
| In a row, rebased between them | ✗ | ✗ | ✗ | ✓ | ✓ | ✓ |
| Main merged in instead of a rebase | ✓ | ✗ | ✓ | ✗ | ✓ | ✓ |
| Amending into the last node's commit | ✗ | ✓ | ✗ | ✗ | ✗ | ✓ |
| The last node's work left uncommitted | ✗ | ✓ | ✗ | ✗ | ✗ | ✓ |
| In a row on main itself | ✗ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Two at once in one worktree | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| In a row, a rebase conflict in the last node's file | ✗ | ✗ | ✗ | ✓ | ✗ | ✗ |

The rules:
- *merge-base* is `merge-base(head, base branch)`.
- *start* is the worktree when the node started.
- *captured* is the newest ancestor among the merge-base, the start and other nodes' captured
  heads.
- *trailers* is the parent of the oldest commit naming the node.
- *patch-id* is *captured* that also finds rewritten heads by `git patch-id`.
- *carried* is the start, carried forward.

- **carried is the one rule that held** wherever reef must work. Only *start* handled an amend
  into another node's commit and work left uncommitted, and only *carried* also handled rebases
  and merges.
  - **Two at once in one worktree** defeats every rule, which is why nodes at once get
    worktrees.
  - **A rebase conflict in an earlier node's file** puts its resolution in this node's diff.
    The capture names the conflicted files on the revision.
- **The pin is needed.** After a rebase, a gc with the pin gone prunes the start's tree, and the
  capture can't be taken.
- **Stop is what sees everything.**
  - Told to mark the node done first, as this repo does, Claude marked it done before it
    committed. PostToolUse then fires before anything after the mark, such as the plan's
    export.
  - git's post-commit captured every commit, in each worktree, from the repo's common hooks
    dir. A rebase runs it for each pick with HEAD half built, so it skips mid-rebase, and
    post-rewrite runs once the rebase ends.
  - A local squash left the captured tree, and so the diff, unchanged.
- **Every hook runs in headless `-p`:** PostToolUse on an MCP tool, with `tool_input` the
  tool's arguments, Stop, WorktreeCreate, and git's hooks under Claude's own `git commit`.
  Claude Code doesn't remove its worktrees in `-p`, so WorktreeRemove never ran live.
- **Without reef's WorktreeCreate hook, Claude Code bases its worktree on `origin/HEAD`** and
  fetches it, by its docs. reef's hook bases the worktree on the session's HEAD, and fetches
  nothing.
- **A squash loses nothing.** Three nodes were squash-merged, their branches, worktrees and pins
  deleted, every reflog expired and git pruned. The heads the captures read were gone, and every
  capture read back byte for byte.
- **gix matched the CLI on every step it took over.**
  - Every case's diff matched, renames, mode changes, binary files, CRLF and missing final
    newlines included.
  - Its tree merge wrote git's own tree, conflict markers included, for a conflict on one line,
    a delete against an edit, an add on both sides, and a rename against an edit.
  - Its merge-bases and ancestry matched for every pair of commits in a history with a merge.
  - In every case, the hybrid took the same capture as the CLI.
  - It read X2's commit (+5886 -20) in 0.99 ms against the CLI's 2.78 ms.
  - A whole capture, from process start to stored, took 9.77 ms against 11.76 ms.
  - It adds 51 gix crates and no C.
- **git2 would need no git binary, but brings its own gaps.**
  - git2 0.20 builds libgit2 1.9.7 and zlib from C, even with every feature off.
  - libgit2 runs only its `crlf` and `ident` filters, never git-lfs's clean filter, which
    gix's filter crate does run.
  - It fails on a split or sparse index.
  - It refuses repo extensions it doesn't know, reftable's `refStorage` included.

### Open questions

- **Untracked files that aren't ignored are captured**, as `git add -A` would commit them. In
  every live run, Python's `__pycache__` landed in the revision though no commit took it.
  Whether a seal leaves out untracked files the node's commits never took is
  [C32](../milestones/m7.md)'s to decide.
- **A node with two needs done but not merged** would start from a merge of their heads, which
  `reef start` would make. That wasn't tried.
- **A round of fixes after the node's work has landed** on its base branch, and its branch was
  rebased onto it, shows only that round's changes. Whether comments follow onto that revision
  is [C36](../milestones/m7.md)'s.

## A node's diff

### Design choices

- **Stored as revisions.** Each capture is a revision of the node's changes: files, hunks and
  lines. Each line has an anchor a comment can name: its revision, file and line. The latest
  revision is a draft until it is sealed.
- **Loaded when opened.** The Changes tab lists revisions and files, and a file's diff is
  fetched and drawn only when it is opened, so a large node doesn't freeze the app.
- **Checked against git when captured.** A captured diff must match `git diff` of the same base
  and head at the time it is taken.

### Open questions

- Whether a revision's files are rows, one per file, or objects in a shoal bucket. A row is
  bounded by a frame, 64 MiB by default, and the client on `ScanSupport` can put and get a
  bucket's objects.

## Comments

### Design choices

- **Anchored to node, revision, file and line**, threaded, each open or resolved, in a table of
  their own.
- **Comments follow the code between revisions.** When a later revision of a node's changes
  moves a commented line, the comment moves with it. reef maps the line by comparing the two
  stored revisions, not git's history. When the line is gone, the comment is marked outdated
  rather than lost.
- **Resolved by a change.** A `Reef-Resolves: <comment>` trailer, read when its commit is
  captured, resolves the comment and links it to that revision, so the user sees which change
  answered which comment. `reef comment resolve` does the same by hand.
- **Claude takes open comments between nodes.** After it finishes a node, next_task hands back
  the open comments before the next node ([C43](../milestones/m8.md)).
