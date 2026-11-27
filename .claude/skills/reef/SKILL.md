---
name: reef
description: Keep this repo's plan, held by central reef, in step with the work, through reef's MCP tools and CLI. Use when planning work, before starting or after finishing a feature, milestone, spike, fix or optimization, when running benchmarks whose results belong in the plan, when a decision adds, moves or drops work, or when the user names a node by its id.
---

# reef: keeping the plan in step with the work

central reef keeps this repo's plan: a graph of nodes, each a piece of work, served by
`reef-serve` and reached through the `reef` CLI and its MCP tools. **The service is the source
of truth.** Every node is YAML with the keys a book's frontmatter takes, and nothing in the repo
is edited to change it. Every write is checked against the whole plan. Update the plan in the
**same change** as the work it describes, never as a follow-up.

The full key reference is in [reference.md](reference.md). Read it before writing
`description`, `acceptance` or `performance`, or a fence.

## 1. Find the plan

- **The working directory picks the project**: the registered project whose repo holds it, its
  main repo's in a linked worktree, or the one registered by the git URL its remotes name.
  `reef project show` names it, and `reef plan ls` lists its plans and the branch each
  follows. Where it has several, the checkout picks one: the plan the session's loop is held
  to, else the one that follows the checkout's branch, else the plan of the node its worktree is
  on. Name another by its name alone: `plan` on a tool, `--plan` on a command.
- **A project registered by its URL reads docs at the commit last fetched**, so a link to a doc
  that isn't pushed and fetched yet is refused. Ask the user to push it, then run
  `reef project fetch`; pushing is theirs to do.
- **Tools first, the CLI where there are none.** Claude Code serves reef's tools as
  `mcp__reef__*` when the repo's `.mcp.json` runs `reef mcp`. Plan mode blocks Bash, but it
  runs `mcp__reef__next`, `mcp__reef__get`, `mcp__reef__history`, `mcp__reef__check` and
  `mcp__reef__comments` without a prompt, since they only read. The others write, so they wait until the plan is approved. Each tool's text is
  what its command prints.
- **With no `mcp__reef__*` tools, the repo isn't set up for them.** `reef claude install`, run
  from the repo's root, writes the MCP entry, Claude Code's and git's hooks and this skill. It
  writes hooks that run on every commit and every turn, so it is the user's to run: suggest it,
  and work through the CLI meanwhile.

  | To | Tool | Command |
  |----|------|---------|
  | Find work | `mcp__reef__next` | `reef next` |
  | Take the loop's next node | `mcp__reef__next_task` | `reef loop take` |
  | Work a group's member here | | `reef loop attach <ID>` |
  | Show this session in the app | | `! reef loop run`, then `! reef loop detach` |
  | Read a node's YAML | `mcp__reef__get` | `reef node get <ID>` |
  | Read its body too | | `reef node get <ID> --md` |
  | See who changed it, and how | `mcp__reef__history` | `reef node log <ID>` |
  | Check the plan | `mcp__reef__check` | `reef check` |
  | Put a node | `mcp__reef__put` | `reef node put <FILE>` |
  | Set its status | `mcp__reef__status` | `reef status <ID> <STATUS>` |
  | Set a criterion | `mcp__reef__accept` | `reef accept <ID> <N> met --test <TEST>` |
  | List a node's criteria | | `reef accept <ID>` |
  | Start its code here | | `reef start <ID>` |
  | Fetch a remote project's docs | | `reef project fetch` |
  | Read or change a page | | `reef page ls`, `reef page get <PATH>`, `reef page put <PATH> <FILE>` |
  | See the user's order | | `reef queue ls` |
  | List open comments | `mcp__reef__comments` | `reef comment ls` |
  | Reply to a comment | `mcp__reef__reply` | `reef comment reply <N> <TEXT>` |
  | Resolve one with no change | `mcp__reef__resolve` | `reef comment resolve <N> --reply <TEXT>` |

- **If nothing answers**, the service isn't running. reef's error says where it looked: tell
  the user, rather than starting one.
- **If reef asks you to sign in**, the service requires it and this `reef` has no token for it:
  ask the user to run `! reef login`. Never ask for a password, nor pass one on a command line.
- **Take work from `next`**: the user's queue first, then ready work in plan order, then each
  piece of work that waits and the needs it waits on.
- **In a terminal loop, take work from `next_task`.** Once the user starts a loop in the
  session's pane (`! reef loop start`), reef feeds the session one node at a time:
  1. **Call `mcp__reef__next_task` before plan mode,** since it writes. It takes the next node,
     the user's queue first, records its start in this checkout, marks it `in-progress`, and
     returns it whole, with its open comment threads and the queue's entries it passed over and
     why. Called again before the node is done, it returns the same node. With nothing to take,
     it says what everything waits on: tell the user, and stop.
     - **A done node with comments that want an answer comes back first,** as a round of them:
       next_task takes it again, starts it again here and marks it `in-progress`, and returns
       its threads and the steps that finish the round. Plan the round as a node, and put those
       steps in the plan: the fix is committed as a `git commit --fixup` of the node's commit,
       its `Reef-Resolves` trailer naming each thread it answers, and squashed in with the
       `git rebase --autosquash` next_task names, unless a remote holds that commit already.
     - **Answer a thread that needs no change** with `mcp__reef__resolve`, saying how, or ask
       its writer with `mcp__reef__reply`, which leaves it waiting on them. Both write as
       Claude, so a thread whose last word is yours never calls for another round.
  2. **Plan it in plan mode:** read its docs and needs, then enter plan mode (EnterPlanMode).
     End the plan with the steps next_task gives for finishing it, the commit among them:
     approving the plan may clear the context, leaving the plan alone.
  3. **Ask what the plan needs answered** (AskUserQuestion), and exit plan mode (ExitPlanMode) for
     the user's approval.
  4. **Implement it, and finish it** as section 5 says: its status, its criteria, the commit.
  5. **End the turn once it is committed,** rather than taking the next node in the same
     context. The Stop hook (`reef hook stop`) then names the next node, or a round of a done
     node's comments, or says the loop is paused or nothing else is ready. In a zellij or tmux pane, reef itself types `/clear` and
     asks for the next node once Claude is idle and the input box is empty. Elsewhere, the user
     types `/clear` and `next`. After a paused node's end, or once reef stepped aside, the user's
     `! reef loop start` moves on the same way. Either way, the next node starts in a fresh
     context, with next_task again. When a `! reef loop start` says reef clears the context,
     answer it in a line and leave the next node to the fresh context.
- **A loop started wider than one node works a group** (`! reef loop start --width 2`):
  - **In the loop's own session,** next_task takes the group. Work only its first node, in this
    checkout. Each other node of the group, a *member*, gets a session of its own in its own
    worktree. In zellij or tmux reef opens a pane for each, running `claude -w <ID>`. Elsewhere
    the user runs that, or `! reef loop attach <ID>` in a Claude session already open.
  - **In a member's session,** call next_task as above: it returns that member's node, started
    in its worktree already. After a `! reef loop attach`, first enter the worktree it names with
    EnterWorktree (its `path`), then call next_task. Work and commit in that worktree only.
  - **Once a node is done and the rest of its group isn't,** Stop says what the group waits on:
    end the turn. A member's session can be closed then. The Stop that finds the whole group
    done clears the loop's own pane, whichever session it ran in.
  - **After the group's clear, next_task may ask for each member's branch to be merged**
    (`git merge reef/<ID>`). Merge each into the loop's checkout, resolving any conflict as the
    repo's own instructions say, then call next_task again for the next group.
- **Before starting work, read its node** by id with `get`. Read its `needs`, its group, and
  its `docs` page (`reef page get <PATH>`, or the repo's file at that path): they say what the
  work is gated on and how it was meant to be done.

## 2. Before starting: draw the work

If the work has no node, add one.

- **How:** put its YAML. YAML with no `# version:` line is a new node. To change a node, put
  back what `get` printed, edited, keeping its `# version:` line. A put over a version since
  changed is refused as stale: get it again and redo the edit. A put that adds a problem writes
  nothing and gives check's messages: fix them and put again. With the CLI, write the YAML to a
  scratch file outside the repo, never into it.
- **Where:** a node of its own naming its group in `group:`. A new group can be put with its
  members inline in its `nodes:`, each of which goes in as a node of its own. Don't make up a
  design page just to have something to link: with no page, leave out `docs`.
- **Which group:** adding work to a group whose members are all done reopens it, and everything
  that needs that group waits again. Do that only when the work really gates what comes after;
  otherwise put it in the open group that comes next, or a new group.
- **`id`:** unique across the whole plan. Follow its existing scheme (look at sibling ids, e.g.
  `C38`, `X4`, `M8`).
- **`title`:** short. Keep any id prefix the plan uses (`C38 The skill for central reef`).
- **`needs`:** only what directly gates it. Skip an arrow a longer path already implies.
  Naming a group means all of it. Never name the node's own group.
- **`helps` and `optional: true`:** for work that helps but does not gate.
- **`docs`:** a page's path from the repo's root, such as `design/feed-loop.md#the-skill`.
  The plan's own pages (`reef page ls`) lie over the repo's files at their paths. Add a
  `#heading` anchor only if the page holds more than this work, and only one that exists. The
  sidebar shows that section, and takes the design choices from its "Design choices" section,
  or write `decisions`.
- **Links:** in a body or a page, `node:<ID>` (`node:M6#rows` for a heading of its body) links a
  node, rather than a path to a file.
- **`description`:** always write `why` and `what`, even when the node links a doc. Check
  refuses work that has neither written nor found in its doc, and a doc's opening paragraph
  rarely says either plainly. One or two plain sentences each, about 30 words at most:
  - `why`: the problem or gap. What is slow, wrong, missing or unknown without this work, or for
    a spike, what the plan can't decide until it is answered.
  - `what`: what will be true or possible once it is done. The outcome, not the steps.
  - Name the specific thing (the shard's loop, a scan's page), never "improves performance".
  - `details` holds anything longer. Write only what is known: no invented figures.
- **`acceptance`:** write the criteria now, before the code: how will it be known to be done?
  One per line, each `unit`, `integration`, `performance` or `feature`. Add `test:` with the
  test's path once the test exists.
- **Performance targets before there are figures:** check requires a criterion's `metric` to
  be a metric in `performance` that has figures. So until the work is measured, write the target
  as text only, naming the metric's key: `perf: "A repeated scan answers in at most 2 ms (repeat-scan-ms)"`.
  Add `metric:` and `at-most:`/`at-least:` in the change that records the figures (step 4).
- **`status`:** leave it `todo` (the default) while the work is only planned.
- **A node's body** is the markdown after a node file's frontmatter, which YAML alone keeps.
  To change it, `reef node get <ID> --md` prints the node as a file, its `# version:` line
  inside the frontmatter, and `reef node put <FILE>` of that file writes the body too.

## 3. While working

- **When the code starts,** run `reef start <ID>` in the checkout where it happens. It records
  the node's start, so its changes are captured as its own, and marks it `in-progress`. Where
  that can't run, set `in-progress` with `status`. A node `next_task` took is started already.
- A decision that adds, moves or drops work changes the plan when the decision is made: add the
  node, change its `needs`, or ask the user to remove it.
- Split a node when only part of it waits on something.
- New work found along the way (a bug, a follow-up) gets its own node, not a note in a body.
- Use status `blocked` when it is stuck, and say on what in its `details`.

## 4. When benchmarks are run

Record the figures in the node's `performance` key, by a put: `before` and `after` for each
metric, as a number or `[median, lo, hi]`, with `better: higher|lower`. Give the same `key` to
the same metric on other nodes so a group can draw its trend. A sweep uses
`points: [[x, before, after], ...]`. Say how they were measured in `note` (what, how many
rounds, on what), and name the two sides with `labels` when "Before"/"After" says too little.
Never invent figures: only write what was measured. Then turn any text-only performance
criterion into one with `metric` (the metric's `key`, or its `name`) and its
`at-most`/`at-least` target, so whether it is met is derived.

## 5. When finishing

In the same change that finishes the work:

- `status` `done` on the node. Never set a status on a group: a group is done when its members
  are, and "ready" is derived from `needs`.
- A commit that answers a comment thread names it in a `Reef-Resolves: <N>` trailer, which
  resolves the thread once the commit is captured.
- Each acceptance criterion gets `met` or `unmet` with `accept`, numbered from 1 in the order
  its `acceptance` lists them, and the test that checks it. Leave a performance criterion with
  a `metric` and `at-least`/`at-most` alone: its status is derived from the `after` figure.
- If the result moved later work (it unblocked, dropped or added something), update those nodes.
- If what was built differs from what was planned, update `description.what` to say what it does.
- If the repo keeps an export of the plan as a book, its own instructions say how to write it.

## 6. Always check

Every write is checked against the whole plan, and one that would add a problem is refused. Run
`check` after a round of edits anyway, and fix every problem it lists before you finish: unknown
ids, cycles, a node that needs its own group, duplicate ids, doc links to missing files or
headings, work with no `why` or `what`, and malformed `description`, `acceptance` or
`performance`.

## Don't

- Don't edit a book `reef export` wrote, by hand: the next export writes over it.
- Don't remove a node. There is no tool for it, and `reef node rm` is the user's to run.
- Don't strike or delete a finished node: mark it done, so the plan shows what was done.
- Don't draw the plan as a mermaid flowchart; add nodes, and a ```reef fence in a body or page.
