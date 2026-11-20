# X4: the terminal feed loop, tried

A stub of reef's MCP server and Claude Code hooks that feed one interactive session node by
node, clearing its context between nodes. reef never starts or owns the session. Once a node is
done, the Stop hook types `/clear` and the next node's kickoff into the session's own zellij or
tmux pane. Without a multiplexer, the hook tells the user to type `/clear`.

- `x4-loop mcp` is the stub MCP server, built with rmcp 3.11. `next_task` claims the next node
  and hands out its why, what, acceptance and the loop's steps. `get_node`, `next` and `check`
  are marked `readOnlyHint`. `set_status` and `note` write.
- `x4-loop hook <event>` logs every hook's input to `events.jsonl`.
  - Stop decides what happens once a node is done (`decide` in `src/hook.rs`).
  - SessionStart adds the node's context after a clear.
- `x4-loop inject` is started by the Stop hook, detached. It waits until `claude agents --json`
  reports the session idle, types `/clear`, then the kickoff.
- `x4-loop status-line` records the session's `context_window` on every update.
- `x4-loop report` prints a run's timeline and the context at each node's start and end.
- The stub plan is three tiny functions for a `textkit` crate: N1 `slugify`, N2 `word_count`
  and N3 `title_case`. Each leaves the user one choice to ask about. N1 also calls every tool
  once while in plan mode.

It is a workspace of its own on stable, which reef's root workspace excludes.

## Modes

The Stop hook acts once per finished node, whatever `stop_hook_active` says. A paused loop and
a finished plan stop there.

| Mode | When a node is done |
|---|---|
| `inject` | The hook lets Claude stop and starts the injector, which types `/clear` and the kickoff into the pane |
| `manual` | The hook lets Claude stop and tells the user, through `systemMessage`, to type `/clear`. SessionStart then names the next node |
| `approval` | The hook blocks, telling Claude to take the next node. The next plan's approval can clear the context, with `showClearContextOnPlanAccept` |

## Running it

```sh
cargo build --release && cargo test
# the scratch repo and the loop's state dir
./setup.sh /tmp/x4/textkit /tmp/x4/state
# a test session on a tmux server of its own, with a scrubbed environment; add `zellij` to
# run claude inside zellij
./drive.sh start x4a /tmp/x4/textkit zellij
./drive.sh screen x4a         # or watch it: tmux -L x4 attach -r -t x4a
./drive.sh type x4a "Take the next reef node: call the x4 next_task tool and follow what it says."
./target/release/x4-loop --dir /tmp/x4/state pause      # stop after the current node
./target/release/x4-loop --dir /tmp/x4/state mode manual
./target/release/x4-loop --dir /tmp/x4/state report
./drive.sh stop x4a
```

**`drive.sh` scrubs the environment on purpose.** A Claude session running in zellij passes
`ZELLIJ_SESSION_NAME` and `ZELLIJ_PANE_ID` to everything it starts. A test session started from
inside another Claude session would inherit that session's pane. Its injector would then type
`/clear` into the wrong session.

## The runs

All three runs used Claude Code 2.1.329 with Opus 5.5 at xhigh effort, a 1M context window, in
auto mode. I drove each session through tmux send-keys, standing in for the user: answering
each question, approving each plan, and choosing when to pause. The full timelines are in
[`runs/`](runs).

- **[Run A](runs/a.md), inside zellij.** `inject` for N1 to N2. A pause during N2. Then
  `manual` for N2 to N3, with `/clear` typed by hand followed by "next".
- **[Run B](runs/b.md), plain tmux.** `approval` with `showClearContextOnPlanAccept`. N1 and N2
  were approved with "clear context", then `inject` for N2 to N3 through tmux. N3 was approved
  without clearing.
- **[Run C](runs/c.md), plain tmux.** `approval` with no clears at all, paused during N2. It
  shows what `stop_hook_active` does in a chain.

The context at each point is the status line's `current_usage`: input, cache writes and cache
reads.

| Run | Node | At claim | At end | The next node starts at |
|---|---|---:|---:|---:|
| A | N1 | 44,216 | 57,048 | 44,153, after the injected `/clear` |
| A | N2 | 45,105 | 57,153 | 44,128, after a typed `/clear` and "next" |
| A | N3 | 47,854 | 69,516 | |
| B | N1 | 44,956 | 49,018 | 49,864, no clear between nodes |
| B | N2 | 49,864 | 49,106 | 44,153, after the injected `/clear` (tmux) |
| B | N3 | 45,103 | 64,931 | |
| C | N1 | 44,936 | 57,602 | 58,115, no clear |
| C | N2 | 58,115 | 65,656 | |

A fresh session is at 43,984 to 43,992 before its first tool call. In run B, N1 and N2 were each
approved with "clear context", so they were implemented after a clear. The first figure after
each of those clears was about 34,000, and the next was back to about 46,500. That is why they
end lower than they started.

## What it found

The plan records these in `plan/src/design/feed-loop.md`, under *What X4 found*.

- **Claude takes the loop's steps unprompted.** In all eight nodes it called next_task, then
  EnterPlanMode, asked with AskUserQuestion, and called ExitPlanMode. Some nodes had one
  question, some two, and one a follow-up. Answers and approvals worked in the terminal, in
  zellij and in tmux.
- **Plan mode lets read-only MCP tools through, and not writes.**
  - `get_node`, `next` and `check` ran in plan mode without a prompt, every time.
  - `note` asked for permission in plan mode in runs A and C, even though `mcp__x4` is in the
    allow list. In run B, Claude held it back on its own.
  - Refusing the prompt interrupts the turn, and no Stop hook runs, so the loop waits on the
    user.
  - After approval, in auto mode, `note` and `set_status` ran without a prompt.
- **`stop_hook_active` stays true across a whole chain.** It was false on every Stop that came
  after a clear or a typed prompt. In run C it was true at N2's end, after a block, tool calls,
  an answer and a plan approval. A hook that lets Claude stop whenever the flag is true would
  end the loop after two nodes. `decide` acts once per finished node instead.
- **A block reads as an error.** The terminal shows *Stop hook reported a problem*, with the
  reason under it. A `systemMessage` on a stop that is let through shows as *Stop says: …*.
- **Typing into the pane clears the context.**
  - zellij: `zellij --session S action write-chars --pane-id P`, then `write 13`. tmux:
    `send-keys -t $TMUX_PANE`.
  - `claude agents --json` listed the session as idle 0.1 s after the Stop, and `/clear` went in
    0.5 s after it.
  - The next node then started at the context of a fresh session.
- **`initialUserMessage` doesn't start a turn after an interactive `/clear`.**
  - SessionStart(clear) fired every time, and its `additionalContext` reached Claude: a bare
    "next" started N3.
  - Its `initialUserMessage` started nothing within 8 s, after an injected or a typed `/clear`.
  - The injector types the kickoff itself. Its 8 s wait is most of the 13.3 s and 15.1 s from a
    Stop to the next node's next_task.
- **`/clear` gives the session a new id**, and so does a plan approval's clear. SessionEnd
  (`reason: clear`) is followed by SessionStart (`source: clear`) with a new `session_id` and
  transcript. A loop keyed on the session id loses its session at every clear.
- **Plan approval's clear clears the planning, not the last node.**
  - With `showClearContextOnPlanAccept: true` in local settings, the dialog's first option is
    *Yes, clear context (6% used) and use auto mode*.
  - The new session holds the plan, and SessionStart's `additionalContext` names the node.
    Claude implemented the node and called `set_status`. N1 ended at 49.0k, against 57.0k and
    57.6k in the runs without the clear.
  - ExitPlanMode gets no PostToolUse when approved this way.
  - The next node's planning still carries the last node's context: in run B, N2 started
    at 49.9k.
- **Setting a repo up:**
  - The trust dialog lists the local allow rules and the hooks, and defaults to *No, exit*.
  - `enabledMcpjsonServers` in `.claude/settings.local.json` skipped the MCP server's approval
    prompt.
  - The hooks and the status line in `.claude/settings.json` load once the folder is trusted.
- **A node adds 7.5k to 21.7k of context** (median 12.8k) between its claim and its end. That
  covers the six nodes without a mid-node clear.
