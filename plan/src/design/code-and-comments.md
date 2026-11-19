# Code and comments

## Context

With the plan outside the repository, reef can show what each node changed. The user can then
comment on the changed lines, and Claude resolves those comments after the node it is on.

Git can't be the record of that. Commits are squashed regularly, so soon after a node is done its
commits, their SHAs and any trailers on them are gone. reef reads git once, when a node is
completed, and keeps what it read. That needs four things reef doesn't have: a way to pick out
one node's changes while several are worked on at once, a hook that captures them before a
squash, a store for what it captured, and comments that stay attached to their lines.

## Capturing a node's changes

### Design choices

- **reef keeps the changes; git is only read.** A node's diff is extracted from git when it is
  captured and stored in shoal as a revision of the node's changes. Nothing is read back from
  git later, so a squash, a rebase or a deleted branch loses nothing reef shows.
- **A Claude Code hook captures a node when it is completed** ([C56](../milestones/m7.md)),
  before anything can squash it. It captures again once Claude has resolved the node's comments,
  so each round of fixes is a revision of its own. `reef node capture` does the same by hand,
  for work done outside a session.
- **Captured where the changes are.** The hook and the CLI run in the session's worktree,
  extract the diff there, and send it to the JSON API. reef-service never reads a checkout for
  a diff, so capture is unchanged once the server is shared ([M10](../milestones/m10.md)).
- **A work record** ([C30](../milestones/m7.md)): `reef start` and `reef done` record the node's
  worktree, branch and base, and each capture records the SHAs it read. The SHAs are provenance
  only, and may stop resolving once squashed.
- **git through its CLI or gix** is X3's to decide. The CLI is what users run; gix avoids a
  subprocess per diff.

### Nodes worked on at once (X3)

- **A worktree per node, if X3 confirms it.** Each node worked on at the same time gets its own
  git worktree and branch, so two nodes' work never interleaves on one branch. A node's changes
  are then its branch's diff from where it left its base branch (`git merge-base`) to its head,
  plus anything not yet committed.
- **Trailers as the fallback.** If worktrees don't hold up, nodes share a branch, and each
  commit carries `Reef-Node: project/plan/id` from a prepare-commit-msg hook
  ([C31](../milestones/m7.md)). The capture reads them before a squash merges them.

### Open questions

- Which hook captures: a PostToolUse hook on reef's tool that marks a node done, or the Stop hook
  before it hands Claude the next node ([C41](../milestones/m8.md)). Either way, the capture must
  finish before the next node starts.
- What a node's worktree starts from when a node it needs is done but not yet merged into the
  base branch: the base branch alone, or the done needs' branches too.
- Whether reef makes each node's worktree (`reef start`), or Claude Code's own worktree support
  does.

## A node's diff

### Design choices

- **Stored as revisions.** Each capture is a revision of the node's changes: files, hunks and
  lines, each line with an anchor a comment can name: its revision, file and line.
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
