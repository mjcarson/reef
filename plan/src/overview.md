---
id: central
title: Centralized reef
collapsed: false
nodes:
  - id: X1
    title: "X1 Shoal in reef's build and processes"
    status: done
    docs: [design/architecture.md#shoal-in-the-build]
    description:
      why: "Shoal's server needs nightly, a patched glommio, `target-cpu=native` and locked memory, while reef is stable and its server is a dx-built Dioxus fullstack binary."
      what: "reef runs shoal in-process by default and connects to an external node when one is configured, and no stable or wasm crate pulls in shoal."
      details: "Tried in `spikes/x1-shoal`. One Dioxus 0.7.10 fullstack server, built with an `embedded` feature passed to dx's server build alone, picks the mode at start from whether a node's address is set. `node.sh` runs the external node, shoal's node program over the same tables. A restart costs the same in both modes, since shoal is ready in 40 ms. The findings are in the architecture page's *What X1 found*."
    acceptance:
      - feature: A scratch Dioxus fullstack server starts standalone shoal and round-trips a row
        status: met
      - feature: "The same build connects to an external node when one is configured, and starts shoal in-process when none is"
        status: met
      - integration: "`cargo tree` of the stable workspace and of the wasm client shows no shoal"
        status: met
      - perf: "A `dx serve` restart with shoal inside is timed"
        metric: dx-restart-s
        status: met
    performance:
      labels: [In-process, External node]
      note: "Five rounds of `spikes/x1-shoal/restart.sh` a side, on a 32-thread machine with dx 0.7.10 and nightly 1.100, in debug builds without debuginfo. Each round changes a constant in the server's code and is timed to the first round trip answered by the new build. Both sides are the same build with `embedded`; built on stable without it, external-only restarted in 1.49 s (1.29 to 1.52). Shoal's start is the pool's `start` and `ready` in-process, five rounds, and an external node's launch to `SHOAL_NODE_SERVING`, once; a restart of the app doesn't restart an external node."
      metrics:
        - name: "dx serve restart, edit to a round trip, s"
          key: dx-restart-s
          unit: s
          before: [1.76, 1.74, 1.77]
          after: [1.76, 1.73, 1.77]
        - name: "shoal start to ready, s"
          key: shoal-ready-s
          unit: s
          before: [0.040, 0.035, 0.045]
          after: 0.028
  - id: X2
    title: "X2 reef's rows in shoal, priced"
    docs: [design/store.md#rows-and-versions]
    description:
      why: "With no client scans or transactions on shoal's `ScanSupport` branch, nobody knows how reef lists plans, stops two writers clobbering a node, or imports a plan atomically."
      what: "The row layout is chosen (constant listing partitions, the spec as one serialized field, versions by conditional write, revisions behind a pointer row) and a 202-node plan is timed through it."
    acceptance:
      - feature: "A stale `if_matches` write is refused"
      - feature: An import killed midway leaves the old revision readable
      - perf: "The example book's 202-node plan is loaded and timed (plan-load-ms)"
  - id: X3
    title: "X3 Capturing a node's changes"
    docs: [design/code-and-comments.md#capturing-a-nodes-changes]
    description:
      why: "Commits are squashed regularly, so git can't hold a node's changes, and nobody knows how reef picks out one node's changes while several are worked on at once."
      what: "The capture rule is chosen: a worktree per node or trailers, the base a node's diff starts from, which hook captures it, and whether reef reads git through its CLI or gix."
    acceptance:
      - feature: "Two nodes worked at once in two worktrees each capture only their own changes"
      - feature: "A stub hook captures a node when it is marked done, and a squash afterwards loses nothing"
      - feature: "The rule is tried on a scratch repo through amend and rebase before a capture"
  - id: X4
    title: "X4 The terminal feed loop, tried"
    docs: [design/feed-loop.md#the-terminal-loop]
    description:
      why: "None of it is proven: a Stop hook keeping Claude going, Claude calling EnterPlanMode itself, MCP tools usable in plan mode, or how context grows across nodes."
      what: "A stub hook and MCP server drive two nodes in one session, settling the hook's replies, the tools plan mode allows, pausing, and how much context grows."
    acceptance:
      - feature: "Blocking starts the next node only after the current one is done, and never twice under `stop_hook_active`"
      - feature: Plan mode, AskUserQuestion and ExitPlanMode approval all work in the terminal
      - feature: The finding records which reef tools plan mode allows
  - id: X6
    title: "X6 reef-view inside Dioxus 0.7"
    docs: [design/app.md#the-viewer-inside-the-app]
    description:
      why: "reef pins wasm-bindgen 0.2.130 with its own wasm-bindgen-cli while dx brings its own, and reef-view starts itself from a script tag. Nobody knows if one app can hold both."
      what: "The plan knows how reef-app loads the viewer (linked into its wasm, or `reef.js` beside it), what `mount` takes and returns, and whether a payload swaps live."
    acceptance:
      - feature: "A minimal Dioxus 0.7 app draws the example book's plan through `mount`"
      - feature: "Saved view state, *Read here* doc fetches and the sidebar all work inside it"
      - feature: A second payload redraws without a reload
      - integration: "`dx build` and `cargo xtask wasm` agree on one wasm-bindgen version, or the finding says which pin moves"
---
# Centralized reef

reef keeps a plan as markdown inside a project's docs. This book plans the move to a
centralized planner that holds every project's plans outside the projects themselves, in shoal.

## Why

- **Plans clutter a project's docs.** Milestones, spikes and found work sit beside the pages
  that describe the code, and they are a poor home for planning that runs for months.
- **reef can't show a node's code.** Showing the diff behind each node needs the plan outside
  the repository, and the diff kept outside git too, since commits are squashed regularly.
- **Nothing keeps Claude busy.** Someone has to tell Claude which node is next, every time,
  and the order of ready work is whatever the plan happens to say.

## What

A local reef server, backed by shoal, holds many projects with several concurrent plans each.
Through it:

- the user orders the ready work, and Claude takes it in that order;
- a Claude session in the terminal is fed the next node: it plans in plan mode, the user answers
  its questions and approves the plan, Claude implements it, then resolves any open comments;
- each node keeps the diff that did its work, captured when it is done, and the user comments
  on the changed lines.

## The decisions

1. **Local first, shared later.** A single-user server on localhost reads repos from local
   paths and runs shoal standalone. Users, sign-in and remote repos are an optional later group
   ([M10](milestones/m10.md)).
2. **Shoal is the source of truth.** Nodes are written through a `reef` CLI and MCP tools that
   take the same YAML keys as today's frontmatter, and through the app. mdbook plans import into
   it and export back.
3. **Terminal first.** A reef MCP server and a Stop hook feed the user's normal Claude Code
   session. Answering Claude's questions and approving its plans in the app comes later and
   is optional ([M9](milestones/m9.md)).
4. **This plan lives in its own book.** It sits at `plan/`, apart from `docs/`.
   [C29](milestones/m6.md) imports it into centralized reef once that runs.
5. **The user orders the work.** Whether a node is ready still comes from its `needs`. Among
   ready nodes, a queue the user orders decides what Claude takes next. The queue is per
   project and spans its plans. It is set from the CLI first and by dragging in the app later
   ([M5](milestones/m5.md)).
6. **The UI is a Dioxus 0.7 fullstack app.** `reef-app` runs on axum. The same server answers a
   plain JSON API for the CLI, the MCP server and the hook. Both go through one service layer,
   the only code that touches shoal. The existing viewer is mounted inside the app until an
   optional native port. mdbook-reef and its viewer stay, as the static mode.
7. **reef keeps each node's changes.** Git is read once, by a Claude Code hook when a node is
   completed, and never used to store them, since commits are squashed regularly. Nodes worked
   on at the same time each get their own worktree
   ([X3](design/code-and-comments.md#capturing-a-nodes-changes)).
8. **Shoal in-process, unless an external node is configured.** The server starts shoal itself
   by default, and connects to a separate shoal node when its config names one
   ([X1](design/architecture.md#shoal-in-the-build)).

The spikes below settle what the milestones can't decide alone: how shoal fits reef's build
([X1](design/architecture.md#shoal-in-the-build)), reef's rows in shoal
([X2](design/store.md#rows-and-versions)), how a node's changes are captured
([X3](design/code-and-comments.md#capturing-a-nodes-changes)), whether the terminal loop works
([X4](design/feed-loop.md#the-terminal-loop)), and whether the viewer runs inside Dioxus
([X6](design/app.md#the-viewer-inside-the-app)).

```reef
root = "central"
page = "plan.md"
height = "75vh"
```
