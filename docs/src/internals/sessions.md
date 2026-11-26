# Sessions' plans and runners

## Plans and choices kept

Each node keeps every plan its sessions asked approval for, with its verdict and feedback, and
every question with its choices and the answer picked. reef's Stop and SessionEnd hooks read them
from each session's transcript, whether it ran in a terminal or under `reef loop run`, so a repo
set up before SessionEnd was added needs `reef claude install` run again. `GET
/api/v1/projects/{project}/plans/{plan}/nodes/{id}/decisions` lists a node's.

## A loop the app runs

`reef loop run`, run from a repo's root, attaches this checkout to central reef as a runner.
Each node then gets a fresh `claude -p` session over stream-json, spawned here, on the user's own
machine. The service runs nothing: it keeps each runner's transcript and the prompts waiting on
it in memory, and the app's sessions panel shows them, starts, pauses and stops the runner's
loop, and answers its prompts.

```sh
reef loop run                                   # wait for the app to start the loop
reef loop run --start --plan central            # or start it now, narrowed to a plan
reef loop run --permission-mode auto -- --model sonnet --effort low
```

- **The repo is set up as for the terminal loop,** by `reef claude install`, since each session
  runs its `.mcp.json` and hooks. They find the runner's loop through `REEF_PANE`, which names
  its pane, `reef:<id>`. Nothing of the runner's own `CLAUDE*`, `ZELLIJ*` or `TMUX*` reaches them.
- **One node at a time.** A loop on a runner's pane is one node wide. Once nothing else is ready,
  the runner ends its session and waits for the app to start the loop again.
- **Every prompt waits for the app,** a tool's permission among them, since headless Claude Code
  starts in `default` mode unless `--permission-mode` says otherwise. Allow rules in the repo's
  settings still allow what they name.
- **Ctrl-C ends it:** its session ends, its loop lets go of its node, and the app shows it
  detached. A runner unheard from for a minute shows as gone. One whose service restarted
  attaches again with the transcript it holds.
- **Who did each act is kept,** though the session is only in memory (C89). A session says who
  attached it, and each order who gave it from the app. Its project's loops' history, read at
  `GET …/projects/{project}/loops/history` and by `reef loop log`, keeps who attached it, let go
  of it, answered its prompts and replied in it, beside each loop's acts. A report, sent each
  turn, keeps nothing.

## A terminal's own session

`reef loop run` typed in a Claude Code session, as `! reef loop run`, attaches that session
instead of spawning one. It sees `CLAUDE_CODE_SESSION_ID` and `CLAUDE_PID` outside a runner's
`reef:` pane, attaches a session on the terminal's pane (its zellij or tmux pane, else
`claude:<pid>`) under an id of its own, starts a follower and returns.

- **The follower** is `reef hook follow`, started in a process group of its own like the
  injector, its output in `$XDG_STATE_HOME/reef/follow.log`. Every quarter second it reads the
  complete lines Claude Code has added to the session's transcript,
  `~/.claude/projects/*/<session>.jsonl` (or under `$CLAUDE_CONFIG_DIR`). A last line with no
  newline yet is held back until it has one.
- **Each line is read as the hooks read decisions.** Typed messages, Claude's text, tool calls
  and their output become entries. An AskUserQuestion or ExitPlanMode call is an `asked` entry,
  and its result an `answered` entry. Lines Claude Code injects (`isMeta`), subagents' lines
  (`isSidechain`) and records already read (by `uuid`) are skipped. `turn_duration` marks the
  session idle. Nothing is raised as a prompt, so nothing waits on the app.
- **Reports go through the runner's relay:** one report at a time, each answered before the
  next. A failed report keeps its entries for the next one, so each entry arrives once and in
  order. The follower long-polls for orders, as a runner does, which keeps the session live; it
  carries none out. A 404 attaches the session again with the latest 2,000 entries.
- **The app can't act on it.** Every start, pause, stop, answer or reply on a session whose pane
  isn't a runner's is refused with a 409, `in-terminal`. A pane shows one terminal session:
  attaching another on it detaches the one before, and that one stays detached however its
  follower reports.
- **Each Claude Code process has an inbox,** `$XDG_STATE_HOME/reef/follow/<CLAUDE_PID>.jsonl`, a
  JSON event a line:
  - `started` names the follower process and the session's id, so a second `! reef loop run`
    finds the session attached already;
  - `moved` is written by the SessionStart hook for every session the process starts, with its
    id and transcript;
  - `detach` is written by `! reef loop detach`, which then waits up to 5 s for the follower to
    remove the inbox.
- **A clear moves the session to a new file.** On a `moved` naming a new Claude Code session,
  the follower reads the old file to its end, adds a `session` entry for the new one, and
  follows the new file from its start. A `moved` for the session it already follows (a startup or
  a compaction) is ignored.
- **It ends** on a `detach`, once `CLAUDE_PID` is gone (a zombie counts as gone), or once another
  session has replaced it on its pane. It reads the file to its end, notes why it ended, reports,
  detaches the session and removes the inbox.
