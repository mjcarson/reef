# The feed loop

## Context

Today the user tells Claude which node to start, and the skill finds ready work by grepping
frontmatter. With the plan in reef, reef can keep a session fed. It hands Claude the next node by
the user's order and waits while Claude plans it and the user answers its questions and approves
the plan. Then Claude implements the node, marks it done, resolves any open comments, and takes
the next.

## The terminal loop

The loop runs in the user's own interactive Claude Code session, so questions and approvals stay
where they are today:

1. **next_task** (an MCP tool) returns the next node: its why, what, acceptance, docs and needs.
   If open comments are waiting, it returns those first.
2. **Claude enters plan mode** itself, with its EnterPlanMode tool, and plans the node.
3. **The user answers Claude's questions** (AskUserQuestion) and **approves the plan**
   (ExitPlanMode), in the terminal.
4. **Claude implements the node**, sets its status and criteria, and commits. A hook captures
   the node's changes as soon as it is marked done, before anything can squash them
   ([C56](../milestones/m7.md)).
5. **The Stop hook** (`reef hook stop`) starts the next round when the node is done. While
   Claude waits on the user, the hook lets it stop.

### Design choices

- **The terminal first.** Answering questions and approving plans in the app comes later, and
  is optional ([M9](../milestones/m9.md)).
- **One loop per session.** It holds a project, an optional plan and the node it is on, so two
  sessions never take the same node ([C39](../milestones/m8.md)).
- **Sessions at once work in their own worktrees**, so two nodes' changes never mix on one branch
  ([X3](code-and-comments.md#nodes-worked-on-at-once-x3)).
- **Pausing stops the loop after the current node**, never in the middle of one.

### Open questions (X4)

- **Does a Stop hook reliably keep Claude going?** It must start the next node only once the
  current one is done, and never block twice under `stop_hook_active`.
- **Does Claude enter plan mode on its own** when told to, and do the reef MCP tools work there?
- **When is a node marked in-progress?** Plan mode blocks writes. Either next_task marks the node
  before Claude enters plan mode, or Claude marks it once the plan is approved.
- **How fast does context grow?** A hook can't clear the context, so X4 measures how much each
  node adds over several.
- **If X4 fails:** fall back to a `/reef-next` command the user types, and consider moving M9
  earlier.

## The MCP tools

Plan mode blocks Bash, so while Claude plans it can't run the CLI.

### Design choices

- **`reef mcp` serves the same operations as the CLI** over stdio: get, put, status, check, next,
  and later next_task, the queue and comments.
- **Read-only tools are marked read-only**, so plan mode allows them.
- **Puts take a node's YAML**, the same keys as frontmatter, and a breaking put returns check's
  messages.

## The user's order

### Design choices

- **Readiness stays derived.** A node is ready when everything it and its groups need is done.
  The user's order never makes a waiting node ready.
- **The queue decides among ready nodes.** next takes the first ready entry in the queue. A
  queued group stands for its members, in plan order.
- **A waiting entry says why.** An entry queued ahead of its needs is skipped, and both
  `reef queue ls` and next_task name the needs it waits on.
- **When the queue runs out**, next falls back to ready work in plan order. A loop can be told to
  stop instead.
- **Per project, spanning its plans**, with `--plan` to narrow a loop to one.
- **The terminal first, then the app.** `reef queue` sets the order ([C24](../milestones/m5.md)),
  and the app's queue panel follows ([C25](../milestones/m5.md)).

## The skill

### Design choices

- **The same rules, through reef.** Draw new work as a node before starting it; mark it done and
  its criteria met in the change that finishes it; record benchmark figures; run check after each
  edit. The central skill does all of this through reef's MCP tools and CLI, not by editing
  files.
- **The mdbook skill stays** for the static mode.
- **One command sets up a repo:** `reef claude install` writes the MCP entry, the Stop hook and
  the skill, and running it twice changes nothing.

## Later, the loop in the app

Optional, once the terminal loop works. reef runs the session itself, headless, and relays its
questions and plans to the app.

### Open questions (X5)

- **`claude -p` with stream-json, driven from Rust**, or an Agent SDK sidecar in TypeScript or
  Python. Which one can pause on AskUserQuestion and ExitPlanMode and wait for an answer from
  elsewhere?
- **How the session's transcript streams into the app**, and how an answer or an approval gets
  back to the session.
