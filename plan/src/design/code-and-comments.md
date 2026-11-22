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
  - A capture snapshots the worktree as a seal takes it, on a copy of the index: `git add -u`
    takes every tracked file as it is, an untracked file is added only where the diff's base
    holds it, and `git write-tree` writes the copy out. It diffs that from the start.
  - Sometimes the base branch has brought something in since, by a rebase or a merge. Then the
    start is first carried over it with a three-way `git merge-tree` of trees, the merge a
    rebase does.
  - Nothing depends on commits a squash removes, or on trailers.
- **The start is pinned** under `refs/reef/start/<node>`, so a gc after a rebase can't prune it.
  The ref is local, and no push sends it.
- **A seal leaves out untracked files no commit or add took** ([C32](../milestones/m7.md)). A node's work is
  what it commits or stages, so a `__pycache__` its run left, which no ignore file names, stays
  out of its changes, and a new file counts once it is added.
  - A start still takes untracked files, so work an earlier node left is the start's. An
    untracked file the base holds is kept, so a start's leftover isn't shown as deleted, and an
    edit of one shows as an edit.
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

- **A node with two needs done but not merged** would start from a merge of their heads, which
  `reef start` would make. That wasn't tried.

## A node's diff

### Design choices

- **Stored as revisions.** Each capture is a revision of the node's changes: files, hunks and
  lines. Each line has an anchor a comment can name: its revision, its file, and its number on
  the side it is on, the base for a removed line and the worktree for any other. The latest
  revision is a draft until it is sealed. Until git's hooks keep a draft ([C56](../milestones/m7.md)),
  every capture `reef node capture` takes is sealed.
- **Rows, not bucket objects** ([C32](../milestones/m7.md)). A revision is a row in its node's partition,
  listing its files without their hunks, and each file is a row in the revision's partition,
  sorted by path, holding its hunks and lines as JSON. Listing a node's revisions reads one
  partition, and opening a file reads one row. A bucket would need `ScanSupport`'s client,
  which nothing in reef uses yet, and a second store for the twin.
  - A file whose row would pass 32 MiB, half a frame, is kept without its hunks and says so,
    as a binary file is kept, its line counts still taken.
  - The capture's request carries every file, so it takes up to 48 MiB, as a plan's write does.
- **Numbered for the node, and never reset.** Revisions count from 1, and a node started again
  keeps the ones it had, since comments name them, so its next is numbered after its latest.
- **A capture its latest revision holds adds none.** One whose base and tree are the latest
  revision's adds its SHAs to the work record, naming that revision, and nothing else.
- **Loaded when opened.** The Changes tab lists revisions and files, and a file's diff is
  fetched and drawn only when it is opened, so a large node doesn't freeze the app.
- **Served as JSON and through server functions.** `…/work/{id}/revisions` lists a node's
  revisions, `…/revisions/{n}` reads one, and `…/revisions/{n}/files/{*path}` one file. The
  app's `revisions` and `revision_file` server functions read the same.
- **Checked against git when captured.** A captured diff must match `git diff` of the same base
  and head at the time it is taken.

## Comments

### Design choices

- **On a line, or on the node as a whole.** A thread is on a line of a file in one of the
  node's revisions, named by its anchor, or on the node itself, as a general comment on its
  work, as a merge request's overview discussion is. Each is threaded, open or resolved, in a
  table of their own ([C34](../milestones/m7.md)).
- **A line comment names a line its revision holds.** One whose revision, file, side or number
  the node's stored revisions don't have is refused, and nothing is written.
- **Numbered in the plan.** Every comment, a reply included, takes the plan's next number, and
  a thread is named by its first comment's, so `Reef-Resolves: <n>` names one thread whichever
  node's worktree its commit is captured in.
- **Markdown, rendered on read.** A comment is kept as written and rendered as it is read, its
  raw HTML shown as text and any link that could run script left out, so the app shows the HTML
  as it is and Claude reads the markdown.
- **Marked by version.** A thread is marked open or resolved over the version its writer read,
  so a mark made against a thread since changed is stale and changes nothing.
- **Comments follow the code between revisions** ([C36](../milestones/m7.md)). When a later revision of a
  node's changes moves a commented line, the comment moves with it. reef maps the line by
  comparing the two stored revisions' hunks, not git's history, a revision at a time from the
  one the line was last in. A thread keeps where it was started, where its line went in each
  later revision, and the revision that lost it.
  - **Blobs tie two revisions together.** A worktree file the next revision holds unchanged has
    the line where it was. One that is the next revision's base has it as that base's line of
    the same number, which is what a round of fixes after the node's work landed on its base
    branch sees.
  - **Otherwise the two sides are aligned as a patience diff aligns them.** A line of a base
    both revisions share is matched by its number there, and any other by its text, so an
    edited line matches nothing. The lines between two hunks, which neither revision holds,
    match nothing either.
  - **Followed only to a line the next revision shows**, on the same side, since that is where
    its thread is drawn: an added or context line to an added or context line, and a removed
    line only while the next revision still removes it. So a round of fixes after the work
    landed, which shows only its own changes, takes a thread along only where its hunks show
    the line.
  - **Outdated rather than lost.** A line deleted, edited or no longer shown outdates its
    thread from that revision on, and it is followed no further. It is still drawn in each
    revision it was followed through, and listed in the Comments tab and the toolbar, marked
    outdated, open until it is resolved.
  - **Resolved threads follow too**, so one reopened is where its line is.
  - **A thread started on an older revision is followed at once**, under the lock a capture's
    follow takes, so neither misses the other.
  - **Each capture follows, and says so.** A capture follows its node's threads before the
    change stream hears of it, a capture that keeps no revision included, so one a failed
    follow left behind catches up. Each thread it writes moves on a version, so a mark made
    before it is stale, and the stream names them in a `threads-followed` change.
- **Resolved by a change.** A `Reef-Resolves: <comment>` trailer, read when its commit is
  captured, resolves the comment and links it to that revision, so the user sees which change
  answered which comment. `reef comment resolve` does the same by hand.
- **Claude takes open comments between nodes.** After it finishes a node, next_task hands back
  the open comments before the next node ([C43](../milestones/m8.md)).
