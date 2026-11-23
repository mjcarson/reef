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

1. **next_task** (an MCP tool) claims the next node, records its start and marks it in progress,
   and returns its why, what, acceptance, docs and needs. If open comments are waiting, it
   returns those first.
2. **Claude enters plan mode** itself, with its EnterPlanMode tool, and plans the node.
3. **The user answers Claude's questions** (AskUserQuestion) and **approves the plan**
   (ExitPlanMode), in the terminal.
4. **Claude implements the node**, sets its status and criteria, and commits. A hook captures
   the node's changes as soon as it is marked done, before anything can squash them
   ([C56](../milestones/m7.md)).
5. **The Stop hook** (`reef hook stop`) clears the context and starts the next node once the
   node is done ([C61](../milestones/m8.md)). While Claude waits on the user, the hook lets it
   stop. After a paused node's end, `! reef loop start` moves on the same way
   ([C71](../milestones/m8.md)).

### Design choices

- **The terminal first.** Answering questions and approving plans in the app comes later, and
  is optional ([M9](../milestones/m9.md)).
- **Each node starts with a cleared context.** The context is cleared after each node, or after
  a group of parallel nodes in worktrees ([C60](../milestones/m8.md)), so a node never carries
  the last one's.
- **reef never owns the session.** It doesn't start Claude, wrap its terminal or hold its input.
  It types only into the pane the session already runs in, and only from that session's own
  hook, or from a `! reef loop start` typed in it.
- **Cleared by typing into the pane.** No hook output, tool or command clears an interactive
  session. Once a node is done and another is next, the Stop hook lets Claude stop and starts an
  injector in the background, `reef hook clear` ([C61](../milestones/m8.md)). The injector waits
  until the session is idle, then types `/clear` and the kickoff into the session's own zellij
  or tmux pane, found from the hook's `ZELLIJ_SESSION_NAME` and `ZELLIJ_PANE_ID`, or `TMUX`'s
  socket and `TMUX_PANE`.
  - **Idle as `claude agents --json` says,** for the interactive session of the Claude Code
    process `CLAUDE_PID` names. That process keeps its pid across a clear, and lists the clear's
    new session within 0.25 s, so the injector waits on the new session before the kickoff.
  - **It types only while the loop is between nodes:** running, holding the node whose end it
    acted on, its Claude Code process running the session it waits on. Otherwise it quits.
  - **In its own process group,** with nothing piped back to Claude Code, and what it did
    appended to `$XDG_STATE_HOME/reef/clear.log`.
  - **A start between nodes hands off as Stop does** ([C71](../milestones/m8.md)). Stop acts on a node's
    end once, so after a paused loop's node, or once the injector stepped aside, `! reef loop
    start` starts the same injector in zellij or tmux, handed the session and process from
    `CLAUDE_CODE_SESSION_ID` and `CLAUDE_PID`. Elsewhere it says `/clear`, then `next`. A start
    naming a pane with `--pane` types nothing, since that pane's session isn't known.
  - **It waits out the start's own turn.** Claude Code answers a `!` command with a turn of its
    own, busy from the command's Enter to the reply's end. The injector starts inside that turn,
    so it waits it out as it waits out the turn that ended a node.
- **A draft is never typed over.** The injector reads the pane's input box with its colours,
  through zellij's `dump-screen --ansi` or tmux's `capture-pane -e`: the last `❯` line right
  under a rule, up to the rule that closes it. It waits while the box holds anything that isn't
  faint, since Claude Code draws its own placeholder and suggested prompt faint. A pane showing
  no box, such as a dialog's or a shell's, is never typed into.
  - **Enter only on its own line.** After typing, it reads the box back, and presses Enter only
    if the box holds what it typed, so a key the user pressed meanwhile never sends their line.
  - **It steps aside** once a turn starts that it didn't type, such as the user sending their
    draft, or after ten minutes. The user's own `/clear` and `next`, or `! reef loop start`,
    then start the next node.
- **Without a multiplexer, the user types `/clear` and `next`.** The Stop hook's message says
  which node is next, and SessionStart(clear) names it to Claude, so one word after the clear
  starts it. A clear mid-node, a plan approval's, is told nothing.
- **next_task starts the node,** before plan mode, since plan mode stops writes. It records the
  start in the session's checkout as `reef start` does, so the node's changes are captured as its
  own, and marks it in progress ([C40](../milestones/m8.md)). A call before the node is done hands out the
  same node, and a pane with no loop is refused, since starting a loop is the user's to do.
- **The hook acts once per finished node.** It records the node whose end it acted on, and never
  acts on it again. `stop_hook_active` stays true across a whole chain, so it can't be the
  guard.
- **One loop per pane** ([C39](../milestones/m8.md)). A clear gives the session a new id, so a loop
  follows its pane. Each session that starts in a pane takes its loop, a clear's included,
  through the SessionStart hook (`reef hook session-start`). Two loops never take the same
  node.
  - **The pane is the multiplexer's, else the Claude Code process.** A zellij pane is named by
    `ZELLIJ_SESSION_NAME` and `ZELLIJ_PANE_ID`, and a tmux pane by `TMUX`'s socket and
    `TMUX_PANE`. Without either, the pane is `CLAUDE_PID`, the Claude Code process, which a
    clear keeps. Claude Code 2.1.327 passes it to its Bash commands and to its SessionStart and
    Stop hooks, beside `CLAUDE_CODE_SESSION_ID`, the session's current id.
  - **The MCP server finds its pane the same way, else by its parent.** It inherits its pane's
    multiplexer variables, but Claude Code 2.1.330 hands an MCP server `CLAUDE_CODE_SESSION_ID`
    and not `CLAUDE_PID`. It starts the server itself, so the server's parent is the Claude Code
    process `CLAUDE_PID` names to Bash.
  - **Run in the session's own pane.** `reef loop start`, typed as `! reef loop start`, finds
    its pane and its session from Claude Code's variables, or `--pane` names one.
  - **A loop keeps** its project, the one plan it narrows to if any, its session, the node it
    holds and the node whose end it acted on. `reef loop pause` lets it finish its node, `reef
    loop start` sets it running again, moving on when it is between nodes, `reef loop stop` ends
    it and lets go of its node, and `reef loop ls` lists the project's.
  - **Kept by central reef**, all loops in one partition, so whether another loop holds a node
    is one read under one lock.
- **Sessions at once work in their own worktrees**, which reef makes, so two nodes' changes never
  mix on one branch ([X3](code-and-comments.md#nodes-worked-on-at-once-x3)).
- **Pausing stops the loop after the current node**, never in the middle of one.

### What X4 found

Tried in `spikes/x4-loop`: a stub reef MCP server, built with rmcp 3.11, and stub hooks fed
eight nodes in three sessions of Claude Code 2.1.329, running Opus 5.5 in auto mode. One session
ran inside zellij and two in plain tmux. A stand-in user answered and approved through tmux.

- **Claude takes the loop's steps unprompted.** In all eight nodes it called next_task, then
  EnterPlanMode, asked with AskUserQuestion, and called ExitPlanMode. Answers and approvals
  worked in the terminal, in zellij and in tmux.
- **Plan mode lets read-only MCP tools through, and not writes.**
  - Tools marked `readOnlyHint` ran without a prompt, every time.
  - A write tool asked for permission even though the allow list names it, or Claude held it
    back on its own.
  - Refusing the prompt interrupts the turn, and no Stop hook runs, so the loop waits on the
    user.
  - After approval, in auto mode, writes ran without a prompt.
- **`stop_hook_active` stays true across a whole chain.** After a block, tool calls, an answer
  and a plan approval, it was still true at the next node's end. Only a typed prompt or a clear
  resets it. A hook that lets Claude stop whenever the flag is true would end the loop after two
  nodes.
- **A block reads as an error.** The terminal shows *Stop hook reported a problem* with the
  reason. A `systemMessage` on a stop that is let through shows as *Stop says: …*.
- **Typing into the pane clears the context.**
  - zellij: `zellij --session S action write-chars --pane-id P`, then `write 13`. tmux:
    `tmux send-keys -t P`.
  - `claude agents --json` reported the session idle 0.1 s after the Stop, and `/clear` went in
    0.5 s after it.
  - The next node started at 44.1k to 44.2k, a fresh session's context, against 49.1k to 57.2k
    without the clear.
- **`initialUserMessage` doesn't start a turn after an interactive `/clear`.**
  - SessionStart(clear)'s `additionalContext` reaches Claude: a bare "next" started the next
    node.
  - Its `initialUserMessage` started nothing within 8 s, so the injector types the kickoff
    itself. With that wait, a Stop takes 13 s to 15 s to reach the next node's next_task.
- **A clear gives the session a new id.**
  - SessionEnd (`reason: clear`) is followed at once by SessionStart (`source: clear`), with a
    new `session_id` and transcript.
  - A plan approval's clear does the same.
- **Plan approval's clear clears the planning, not the last node.**
  - With `showClearContextOnPlanAccept`, the dialog's first option is *Yes, clear context and
    use auto mode*.
  - After it, the node is implemented with only the plan and SessionStart's
    `additionalContext`. It ended 8k to 9k lower than the same node without the clear.
  - The next node's planning still carries the last node.
- **Setting a repo up:**
  - The trust dialog lists the local allow rules and the hooks, and defaults to *No, exit*.
  - `enabledMcpjsonServers` in `.claude/settings.local.json` skipped the MCP server's prompt.
- **A node adds 7.5k to 21.7k of context**, with a median of 12.8k. A fresh session starts at
  44.0k.
- **Everything a Claude session starts inherits its pane's variables,** this session's hooks
  included. An injector started with another session's environment types into that session.
  The spike's test sessions scrub their environment for this reason.

### What C61 found

Tried with Claude Code 2.1.330, running Sonnet 5.5, on a scratch reef-serve. One loop ran in
zellij, one in plain tmux and one with neither, each on a private tmux server with a scrubbed
environment. A stand-in user approved through tmux.

- **Typing into the pane clears between nodes, in both.** In zellij and in tmux, a node's end
  cleared the session and started the next node's next_task with nothing typed. The new
  session's first turn ran at 44.4k, a fresh session's context. It began with SessionStart's
  context naming the node, then `/clear`, then the kickoff.
- **Claude Code draws its own text in the box faint** (SGR 2): the placeholder (`Try "…"`), and
  the suggested prompt that appears a second or two after a turn ends. A plain read of the
  screen takes either for a draft.
- **A named session's top rule holds its name,** as `──── write-a-txt-file ─`, so a rule is
  dashes end to end around any label.
- **A draft held the injector until it was gone.** A draft typed while a node ran was still in
  the box, untouched, after the node's end. Erased, the injector cleared and kicked off within
  3 s. Sent instead, the injector stepped aside.
- **Without a multiplexer, `/clear` and `next` started the next node** through SessionStart's
  context.

### What C71 found

Tried with Claude Code 2.1.330, running Sonnet 5.5 at low effort, on a scratch reef-serve. One
loop ran in zellij, one in plain tmux and one with neither, each on a private tmux server with a
scrubbed environment.

- **A `!` command is one busy turn.** `claude agents --json`, polled every 0.12 s, showed the
  session busy from the command's Enter until Claude had answered its output, with no idle look
  between. `! sleep 2` was busy for 2.9 s and `! true` for 0.8 s.
- **A start between nodes cleared and moved on, in both.** In zellij and in tmux, `! reef loop
  start` after a paused node's end had Claude answer the start in a line, then cleared the
  session and took the next node with nothing more typed. Each new session's first turn ran at
  44.4k, a fresh session's, against 51.6k to 52.5k at the old session's end.
- **So did one after the injector stepped aside.** A draft left in the box held Stop's
  injector, and sending it made the injector step aside. `! reef loop start` then cleared and
  moved on the same way.
- **Without a multiplexer, the start said `/clear`, then `next`,** and those two started the
  next node at 44.4k.

### Open questions

- **Approval's clear as well?** It drops a node's planning before its implementation, 8k to 9k
  for the stub's nodes. Whether `reef claude install` turns it on is
  [C42](../milestones/m8.md)'s call.

## The MCP tools

Plan mode blocks Bash, so while Claude plans it can't run the CLI.

### Design choices

- **`reef mcp` serves the same operations as the CLI** over stdio: get, put, status, accept,
  check and next, and later next_task, the queue and comments. rmcp's `#[tool_router]` serves
  them, as in X4's stub. Each tool runs its command's own function, so its text is what the
  command prints, and the working directory Claude Code starts it in picks the project.
- **Read-only tools are marked `readOnlyHint`**, so plan mode runs them without a prompt. Plan
  mode asks for every other tool, or Claude holds it back, even when the allow list names it
  ([X4](#what-x4-found)). So every tool Claude needs while planning only reads, and next_task,
  which claims, is called before plan mode.
- **Puts take a node's YAML**, the same keys as frontmatter, and a breaking put returns check's
  messages.
- **A failed write is a tool error, not a protocol error.** A stale or refused write, or a
  service that isn't running, comes back as the tool's result marked `isError`, carrying the
  message. Claude Code shows a protocol error opaquely, so Claude couldn't read what to fix.
- **No removal tool.** Nothing in the loop removes a node, and `reef node rm` stays on the
  terminal, where the user runs it.

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
- **Done and missing entries stay until taken off.** next passes over an entry that is done or
  started without a word, and lists one that is blocked, or whose node or plan is gone, with the
  waiting ones it skipped ([C22](../milestones/m5.md)).
- **The report says when the queue ordered it.** next's report reads as it always has until the
  queue puts something first or passes something over, and names nodes in full only across
  several plans.
- **Per project, spanning its plans**, with `--plan` to narrow a loop to one. Narrowed, next
  leaves the queue's entries of other plans out, and prints one plan's report as before.
- **The terminal first, then the app.** `reef queue` sets the order ([C24](../milestones/m5.md)),
  and the app's queue panel follows ([C25](../milestones/m5.md)).

## The skill

### Design choices

- **The same rules, through reef.** Draw new work as a node before starting it; mark it done and
  its criteria met in the change that finishes it; record benchmark figures; run check after each
  edit. The central skill does all of this through reef's MCP tools and CLI, not by editing
  files.
- **The mdbook skill stays** for the static mode, under the same name and in the same place,
  `.claude/skills/reef` at the repo's root, so a repo holds one or the other. `reef skill`
  writes the central one over it ([C38](../milestones/m8.md)), and `mdbook-reef skill` the
  static one back.
- **Tools first, the CLI where there are none.** While planning, the read-only tools read the
  plan. The writes wait for the plan's approval. A session without reef's MCP server does the
  same through the CLI, which alone also puts a node's body and a page, and records a start.
- **Shipped in the binary.** The skill's text is built into `reef`, so the skill a repo gets
  names the tools and commands of the reef that wrote it. A test fails when the MCP server has
  a tool the skill doesn't name, or when the skill names a tool, a command or a flag reef doesn't
  have. A new tool such as next_task is written into the skill in the change that adds it.
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
