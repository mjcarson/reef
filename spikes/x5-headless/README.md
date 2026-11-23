# X5: headless Claude, relayed

A stub host that drives a headless Claude Code session from Rust and answers what the session
would otherwise ask at a terminal: its permission prompts, its AskUserQuestion and its
ExitPlanMode. It stands in for reef relaying them to the app. It asks one thing: can `claude -p`
with stream-json, driven from Rust, pause on a question and a plan and wait for an answer from
elsewhere, or does that take an Agent SDK sidecar?

It can, with nothing but the CLI. The Agent SDKs drive this same protocol over the same CLI, so a
sidecar would add a Node or Python runtime and change nothing.

- `x5-headless drive` spawns `claude -p --input-format stream-json --output-format stream-json
  --verbose --permission-prompt-tool stdio` in a repo, with a scrubbed environment, and sends the
  first message as the SDK writes a user's.
  - Every line either side writes is logged, with the time, to `--log`.
  - Each `control_request` it is sent (`subtype: can_use_tool`) is answered on a task of its own,
    so a held question blocks nothing:
    - AskUserQuestion gets its first option, after `--wait` seconds.
    - The first `--rejects` ExitPlanModes are refused with feedback, and the next is approved.
    - Anything else is allowed as asked.
  - At the first `result`, it sends `--then` if one was given, and otherwise closes stdin.
- `x5-headless report` lays a run out as a table, with X4's hook events merged in by time.
- The session runs in X4's scratch repo (`../x4-loop/setup.sh`), whose stub MCP server and hooks
  are wired in through the repo's own `.mcp.json` and `.claude/settings.json`. X4's stub is put in
  `manual` mode, so its Stop hook types into nothing. Its next_task tells Claude to enter plan
  mode, ask the user a question, and call ExitPlanMode.

It is a workspace of its own on stable, which reef's root workspace excludes.

## Running it

```sh
cargo build --release
(cd ../x4-loop && cargo build --release && ./setup.sh /tmp/x5/textkit /tmp/x5/state \
  && ./target/release/x4-loop --dir /tmp/x5/state mode manual)
./target/release/x5-headless drive --repo /tmp/x5/textkit --log runs/a.jsonl \
  --then "Say in one line which node you just finished."
./target/release/x5-headless drive --repo /tmp/x5/textkit --log runs/b.jsonl --wait 200 --mode auto
./target/release/x5-headless report runs/a.jsonl --events /tmp/x5/state/events.jsonl
```

## The protocol

What the session writes, one JSON object a line:

| `type` | When |
|---|---|
| `system` `hook_started`, `hook_response` | Around each hook that runs: SessionStart's, before `init` |
| `system` `init` | Each turn's start: `session_id`, `permissionMode`, `mcp_servers`, `tools`, `model` |
| `system` `status` | The permission mode changed: `plan` after EnterPlanMode, back after an approval |
| `system` `informational` | A Stop hook's `systemMessage`, as `Stop says: …` |
| `assistant` | A message: `text` and `tool_use` blocks |
| `user` | Tool results: `tool_result` blocks |
| `control_request` | A prompt: `request_id`, then `request.subtype` `can_use_tool`, `tool_name`, `display_name`, `input`, `tool_use_id`; `requires_user_interaction: true` for a question or a plan, `mcp_server` and `decision_reason_type` for an MCP tool |
| `result` | The turn's end: `subtype` `success`, `num_turns`, `total_cost_usd` (the session's, so far), `permission_denials`, `result` (the last text) |
| `rate_limit_event` | The account's limits |

What the host writes back:

```json
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"…"}]},"parent_tool_use_id":null,"session_id":""}
{"type":"control_response","response":{"subtype":"success","request_id":"…","response":{"behavior":"allow","updatedInput":{…}}}}
{"type":"control_response","response":{"subtype":"success","request_id":"…","response":{"behavior":"deny","message":"…"}}}
```

- **A question's answer is its input with `answers` added**, keyed by each question's text, the
  label picked as its value. Claude reads back `Your questions have been answered:
  "<question>"="<label>". You can now continue with these answers in mind.`
- **A plan is approved by allowing ExitPlanMode,** its `input.plan` the plan's markdown. Claude
  reads `User has approved your plan…`, with the plan file's path under `~/.claude/plans/`, and
  the mode goes back to what it was before plan mode.
- **A plan is rejected by denying ExitPlanMode with feedback.** Claude reads the message as the
  tool's error, edits its plan file, and calls ExitPlanMode again with the revised plan. The
  denial is counted in the `result`'s `permission_denials`.

## The runs

Both ran Claude Code 2.1.330 with Sonnet 5.5 at low effort, in X4's `textkit` repo. Their logs are
in [`runs/`](runs), and their timelines in [`runs/a.md`](runs/a.md) and [`runs/b.md`](runs/b.md).

- **Run A**, the account's default mode, answered at once: one plan rejected, then one approved,
  then a second message once the first turn ended. N1 was done in 27 s for $0.16, and the second
  message cost $0.005 more.
- **Run B**, `--permission-mode auto`, its question held for 200 s before it was answered, one
  plan rejected, then one approved. N2 was done 227 s after the start, for $0.15.

## What it found

- **No handshake.** The first user message starts the session: no `initialize` control request
  is needed when nothing is registered through it.
- **The repo's own setup runs.** `.mcp.json`'s server connected, its `source` `project`, with
  `enabledMcpjsonServers` in the local settings. SessionStart (`startup`), UserPromptSubmit,
  PreToolUse, PostToolUse, PermissionRequest, Stop and SessionEnd all ran from the repo's
  `.claude/settings.json`.
- **Headless starts in `default` mode**, not the account's `auto`, unless `--permission-mode`
  says otherwise. Rules in the local settings still allow what they name: `Edit` and `Bash(cargo
  test:*)` never prompted once the plan was approved.
- **EnterPlanMode needs no answer.** It ran without a `control_request`, and `system` `status`
  said `permissionMode: plan`.
- **Plan mode asks for every write, allowed or not.** X4's stub `note` tool, allowed by
  `mcp__x4`, still came to the host while planning, as X4 found it prompt in the terminal. So
  reef's write tools come to the app while a node is planned, beside its question.
- **Prompts can come at once.** The `note` and the question were asked within 0.1 s, each with
  its own `request_id`, and each was answered on its own.
- **Stop runs before the `result`.** Its `systemMessage` arrives first, as `system`
  `informational`, then the `result`. So by the time the host reads the `result`, the Stop hook
  has already acted on the node's end, and the loop says what comes next.
- **A second message carries on the same session.** After the `result`, a user message written to
  stdin started a new turn under the same `session_id`, its `init` repeated, with the whole
  conversation still there.
- **Closing stdin ends the session cleanly.** SessionEnd ran, and the process exited 0 within
  0.4 s.
- **A question can wait minutes.** Run B's question was answered 200 s after it was asked. The
  session sent nothing meanwhile and timed nothing out, and carried on from the answer at once.
- **The mode comes back after an approval.** Started in `auto`, the session went to `plan` at
  EnterPlanMode and back to `auto` once its plan was approved. Started in `default`, it came back
  to `default`.
- **Plan files land in the runner's `~/.claude/plans/`**, as an interactive session's do.

The logs, `runs/*.jsonl`, stay out of git, since they name the machine and the account's own
connectors. `report` writes the timelines with the repo, the scratch directory, the home
directory and the user's name replaced.
