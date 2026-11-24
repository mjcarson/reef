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
