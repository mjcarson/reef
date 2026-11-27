---
id: central
title: Centralized reef
collapsed: false
nodes:
- id: X1
  title: X1 Shoal in reef's build and processes
  status: done
  docs:
  - design/architecture.md#shoal-in-the-build
  description:
    why: Shoal's server needs nightly, a patched glommio, `target-cpu=native` and locked memory, while reef is stable and its server is a dx-built Dioxus fullstack binary.
    what: reef runs shoal in-process by default and connects to an external node when one is configured, and no stable or wasm crate pulls in shoal.
    details: Tried in `spikes/x1-shoal`. One Dioxus 0.7.10 fullstack server, built with an `embedded` feature passed to dx's server build alone, picks the mode at start from whether a node's address is set. `node.sh` runs the external node, shoal's node program over the same tables. A restart costs the same in both modes, since shoal is ready in 40 ms. The findings are in the architecture page's *What X1 found*.
  performance:
    labels:
    - In-process
    - External node
    metrics:
    - name: dx serve restart, edit to a round trip, s
      key: dx-restart-s
      unit: s
      before:
      - 1.76
      - 1.74
      - 1.77
      after:
      - 1.76
      - 1.73
      - 1.77
    - name: shoal start to ready, s
      key: shoal-ready-s
      unit: s
      before:
      - 0.04
      - 0.035
      - 0.045
      after: 0.028
    note: Five rounds of `spikes/x1-shoal/restart.sh` a side, on a 32-thread machine with dx 0.7.10 and nightly 1.100, in debug builds without debuginfo. Each round changes a constant in the server's code and is timed to the first round trip answered by the new build. Both sides are the same build with `embedded`; built on stable without it, external-only restarted in 1.49 s (1.29 to 1.52). Shoal's start is the pool's `start` and `ready` in-process, five rounds, and an external node's launch to `SHOAL_NODE_SERVING`, once; a restart of the app doesn't restart an external node.
  acceptance:
  - feature: A scratch Dioxus fullstack server starts standalone shoal and round-trips a row
    status: met
  - feature: The same build connects to an external node when one is configured, and starts shoal in-process when none is
    status: met
  - integration: '`cargo tree` of the stable workspace and of the wasm client shows no shoal'
    status: met
  - perf: A `dx serve` restart with shoal inside is timed
    status: met
    metric: dx-restart-s
- id: X2
  title: X2 reef's rows in shoal, priced
  status: done
  docs:
  - design/store.md#rows-and-versions
  description:
    why: With no client scans or transactions on shoal's `ScanSupport` branch, nobody knows how reef lists plans, stops two writers clobbering a node, or imports a plan atomically.
    what: 'The layout is chosen and tried: listing partitions, each spec as one JSON field, versions by conditional write under a per-plan lock, revisions behind the plan''s own row. The 202-node plan loads in 0.29 ms.'
    details: Tried in `spikes/x2-rows`, which imports the example book into an in-process pool, reads it back as the book's graph, and times it. Each command starts its own pool, so a run killed midway is followed by one that opens what it left on disk. The findings are in the store page's *What X2 found*.
  performance:
    labels:
    - Book on disk
    - Shoal
    metrics:
    - name: Load the 202-node plan, ms
      key: plan-load-ms
      unit: ms
      before:
      - 2.311
      - 2.24
      - 2.563
      after:
      - 0.285
      - 0.265
      - 0.297
    - name: Load it with its specs as YAML, ms
      key: plan-load-yaml-ms
      unit: ms
      after:
      - 1.204
      - 1.177
      - 1.289
    - name: Load it cold, in a fresh process, ms
      key: plan-load-cold-ms
      unit: ms
      after:
      - 0.808
      - 0.777
      - 0.881
    - name: Rewrite it as a revision, ms
      key: plan-rewrite-ms
      unit: ms
      after:
      - 2.19
      - 1.948
      - 2.359
    - name: Prune the revision it replaced, ms
      key: plan-prune-ms
      unit: ms
      after:
      - 2.022
      - 1.709
      - 2.413
    - name: Put one node, ms
      key: node-put-ms
      unit: ms
      after:
      - 0.151
      - 0.117
      - 0.192
    note: '`spikes/x2-rows` `bench --rounds 10`: ten timed rounds after three warm-up rounds, on a Ryzen 9 9950X with an NVMe disk (XFS), a two-shard in-process pool with 256 MiB, nightly 1.100, release build. A load from shoal is the plan row''s get, the revision''s get, decoding every spec and `Graph::build`. The book''s is mdbook-reef''s walk, parse and `Graph::build`, without rendering. A cold load is a fresh process''s first, off the archive. A rewrite reads the old versions, reserves the revision, writes its 202 rows in one bundle and flips. A prune lists the old revision''s keys, deletes its 202 rows and clears `retired`. A put reads the plan''s row and writes one node over its version.'
  acceptance:
  - feature: A stale `if_matches` write is refused
    status: met
    test: spikes/x2-rows/tests/rows.rs::stale_put_is_refused
  - feature: An import killed midway leaves the old revision readable
    status: met
    test: spikes/x2-rows/tests/rows.rs::killed_import_keeps_old_revision
  - feature: A put made during an import waits on the plan's lock and is refused as stale, never written and lost
    status: met
    test: spikes/x2-rows/tests/rows.rs::put_during_import_is_refused_not_lost
  - integration: 'The rows read back build the book''s graph: the same nodes, order, groups and specs'
    status: met
    test: spikes/x2-rows/tests/rows.rs::rows_build_the_books_graph
  - perf: The example book's 202-node plan is loaded and timed
    status: met
    metric: plan-load-ms
- id: X3
  title: X3 Capturing a node's changes
  status: done
  docs:
  - design/code-and-comments.md#capturing-a-nodes-changes
  description:
    why: Commits are squashed regularly, so git can't hold a node's changes, and nobody knows how reef picks out one node's changes while several are worked on at once.
    what: A node's diff runs from its start, carried over what its base branch brought in, to its worktree. Nodes at once get worktrees, and Stop seals the draft git's hooks keep.
    details: Tried in `spikes/x3-capture`. Six rules for the base of a node's diff ran on 13 scratch repos with no remote, worked with real git. Real git hooks ran in two worktrees. Three headless Claude Code 2.1.330 sessions ran at once, one through `claude -w`. The captures were extracted from each local checkout by the CLI, never pushed or fetched. gix runs every step but the snapshot, which stays `git add -A` and `git write-tree`. The findings are in the code-and-comments page's *What X3 found*.
  performance:
    labels:
    - git CLI
    - gix
    metrics:
    - name: Read X2's commit's diff, ms
      key: git-diff-ms
      unit: ms
      before:
      - 2.78
      - 2.64
      - 3.02
      after:
      - 0.99
      - 0.98
      - 1.03
    - name: Capture a node, process start to stored, ms
      key: node-capture-ms
      unit: ms
      before:
      - 11.76
      - 11.56
      - 12.49
      after:
      - 9.77
      - 9.49
      - 10.04
    - name: Snapshot a worktree as a tree, ms
      key: snapshot-ms
      unit: ms
      before:
      - 2.54
      - 2.45
      - 3.22
    note: One sitting on a Ryzen 9 9950X, release build, in a clone of reef on tmpfs with no remote, checked out at X2's commit. The same steps' medians moved 20 to 30% between sittings, so only one sitting's figures compare. A diff is `x3-capture bench` reading X2's commit `31155d8` (16 files, +5886 -20) from its parent, ten timed rounds after three warm-up rounds. The CLI starts a process each round, and gix opens the repo each round. A capture is `x3-capture capture` of X2's commit and one edit, from its process start to a stored revision, over ten runs after three. Its gix side is the hybrid, gix for every step but the snapshot. The snapshot is the clone's 215 files with one edit and one untracked file, through git's CLI with either backend.
  acceptance:
  - feature: Two nodes worked at once in two worktrees each capture only their own changes
    status: met
    test: spikes/x3-capture/tests/capture.rs::parallel_worktrees_capture_their_own
  - feature: A stub hook captures a node when it is marked done, and a squash afterwards loses nothing
    status: met
    test: spikes/x3-capture/tests/capture.rs::squash_loses_nothing
  - feature: The rule is tried on a scratch repo through amend and rebase before a capture
    status: met
    test: spikes/x3-capture/tests/rules.rs::the_matrix_holds
  - feature: A capture reads only the local checkout, with nothing pushed, fetched or read from a remote
    status: met
    test: spikes/x3-capture/tests/capture.rs::a_dead_remote_is_never_touched
  - integration: The hybrid, gix for every step but the snapshot, takes every case's capture exactly as git's CLI does
    status: met
    test: spikes/x3-capture/tests/rules.rs::hybrid_agrees_on_every_case
  - perf: A capture is timed through the CLI and through gix
    status: met
    metric: node-capture-ms
- id: X4
  title: X4 The terminal feed loop, tried
  status: done
  docs:
  - design/feed-loop.md#the-terminal-loop
  description:
    why: 'None of it was proven: a Stop hook keeping Claude going, Claude entering plan mode itself, MCP tools in plan mode, or clearing context between nodes when no hook can.'
    what: Claude plans each node in plan mode by itself, and read-only tools work there. The hook acts once per finished node, and typing `/clear` into the session's own zellij or tmux pane starts each node fresh.
    details: 'Tried in `spikes/x4-loop`: a stub reef MCP server built with rmcp 3.11, and stub hooks, fed eight nodes in three sessions of Claude Code 2.1.329 (Opus 5.5, auto mode). One ran inside zellij and two in plain tmux, with a stand-in user answering and approving through tmux. The timelines are in `spikes/x4-loop/runs/`, and the findings are in the feed-loop page''s *What X4 found*.'
  performance:
    labels:
    - Not cleared
    - Cleared
    metrics:
    - name: Context the next node starts with, ktok
      key: context-carried-ktok
      unit: ktok
      before:
      - 57.0
      - 49.1
      - 57.2
      after:
      - 44.2
      - 44.1
      - 44.2
    - name: Context a node adds, from its claim to its end, ktok
      key: context-per-node-ktok
      unit: ktok
      after:
      - 12.8
      - 7.5
      - 21.7
    - name: Stop to the next node's next_task, injected, s
      key: inject-next-s
      unit: s
      after:
      - 14.2
      - 13.3
      - 15.1
    note: 'From the status line''s `current_usage` (input, cache writes and cache reads) in `spikes/x4-loop`''s runs, with Claude Code 2.1.329, Opus 5.5 at xhigh effort and a 1M window. A fresh session is at 44.0k before its first tool call. The next node''s context covers three transitions: A''s injected `/clear` in zellij, A''s typed `/clear` and "next", and B''s injected `/clear` in tmux. *Not cleared* is the node''s end, which is what the next node starts from without a clear; in run C it started 0.5k above. *Cleared* is the first figure after the clear. A node''s own context runs from the first figure after next_task to the last before the next node, over the six nodes with no clear mid-node. Stop to next_task is one transition in zellij and one in tmux. 8 s of it is the injector waiting for a turn that `initialUserMessage` never starts.'
  acceptance:
  - feature: Blocking starts the next node only after the current one is done, and never twice under `stop_hook_active`
    status: met
    test: spikes/x4-loop/src/hook.rs::tests::never_acts_twice_for_one_node
  - feature: Plan mode, AskUserQuestion and ExitPlanMode approval all work in the terminal
    status: met
  - feature: The finding records which reef tools plan mode allows
    status: met
  - feature: Each node starts with a cleared context, reef typing `/clear` into the user's own zellij pane
    status: met
  - feature: 'A way with no multiplexer clears between nodes: the user types `/clear` and one word'
    status: met
  - perf: A node starts with a fresh session's context (44.0k), not what the last node left
    metric: context-carried-ktok
    at-most: 45.0
- id: X6
  title: X6 reef-view inside Dioxus 0.7
  status: done
  docs:
  - design/app.md#the-viewer-inside-the-app
  description:
    why: reef pins wasm-bindgen 0.2.130 with its own wasm-bindgen-cli while dx brings its own, and reef-view starts itself from a script tag. Nobody knows if one app can hold both.
    what: reef-app links reef-view into its wasm, the page-load start off, and draws through `mount(element, payload, view)`. Its handle swaps payloads live. Both builds bind with wasm-bindgen 0.2.130.
    details: Tried in `spikes/x6-view`, a Dioxus 0.7.10 fullstack app. Its server reads a book into the viewer's payload and serves the book's rendered pages, and its client mounts reef-view and swaps payloads. `check.py` drives it in headless Chromium. reef-view gained `mount`, a `Handle` that swaps payloads, and the default `autostart` feature. The findings are in the app page's *What X6 found*.
  performance:
    labels:
    - Without reef-view
    - With reef-view
    metrics:
    - name: The app's client wasm, KiB
      key: app-wasm-kib
      unit: KiB
      before: 2446.5
      after: 2925.5
    - name: The same, gzipped, KiB
      key: app-wasm-gz-kib
      unit: KiB
      before: 657.5
      after: 831.5
    note: The `_bg.wasm` of `spikes/x6-view`'s client from `dx build --release`, one build a side, with dx 0.7.10, its wasm-opt `-Oz` (binaryen 129) and nightly 1.100. *Without* is the same app with reef-view taken out and its `draw` stubbed. Gzipped with `gzip -9`. For comparison, `reef.js` loaded beside the app instead is 815.9 KiB, 305.6 KiB gzipped.
  acceptance:
  - feature: A minimal Dioxus 0.7 app draws the example book's plan through `mount`
    status: met
    test: spikes/x6-view/check.py
  - feature: Saved view state, *Read here* doc fetches and the sidebar all work inside it
    status: met
    test: spikes/x6-view/check.py
  - feature: A second payload redraws without a reload
    status: met
    test: spikes/x6-view/check.py
  - integration: '`dx build` and `cargo xtask wasm` agree on one wasm-bindgen version, or the finding says which pin moves'
    status: met
- id: C66
  title: C66 A double-click opens a collapsed group in reef-view
  status: done
  description:
    why: reef-view takes the pointer on a press, so the browser sends the double-click to the diagram's SVG, and a double-click on a collapsed group finds no box and does nothing.
    what: A double-click on a collapsed group opens it in the static mode's viewer, as it does in reef-app's canvas.
    details: Found while building C55, whose canvas opens the box the double-click's press landed on. The viewer now keeps what its last press landed on and opens that, if it is a collapsed group still drawn. A double-click on a toggle still does nothing more, since each press toggled it. reef.js is rebuilt from it. On the example book's plan page, with every group collapsed, a double-click on the first collapsed group drew 43 boxes and then 43 with the old bundle, and 43 and then 49 with the new one, in headless Chromium. `check.py`'s step 9 does the same through `mount`, and a double-click on a toggle leaves 43.
  acceptance:
  - feature: A double-click on a collapsed group in the example book opens it
    status: met
    test: spikes/x6-view/check.py
- id: C100
  title: C100 A node's menu on the canvas
  status: done
  needs:
  - C25
  - C55
  docs:
  - design/app.md#the-node-menu
  description:
    why: Queuing a node, setting its status or opening one of its tabs means selecting it and working through the sidebar, and a right-click on a box opens the browser's own menu.
    what: Right-clicking a box or group on a plan's canvas opens a menu that queues it first or last or takes it off, sets its status, opens its sidebar at a tab, and copies its id.
    details: |
      `central/reef-app/src/canvas/menu.rs`: `NodeMenu`, opened by `View::menu`. The SVG now keeps
      what a non-primary press landed on as its `last_hit` too, and its `oncontextmenu` opens the menu
      for a box, group frame or group head (`Hit::Item` or `Hit::Toggle`), preventing the browser's
      menu; a right-click on nothing leaves the browser's alone. The menu is HTML beside `ChartTip`,
      placed as it is and kept inside the view, so the boxes' markup and `tests/diagram.rs`'s parity
      are unchanged. A fixed backdrop under it closes it on the next press, wheel or right-click, so a
      press outside never selects or pans; Escape closes it from the menu or the canvas, before the
      canvas's Escape deselects, and closing gives the view the keyboard back.

      Its items: Queue first, Queue last (`QueueEdit::Add` at `Top`/`Bottom`) and, for a queued node,
      Off the queue, through the page's `Queue` as the sidebar's buttons are; a status chip each for
      to do, in progress, blocked and done, the node's own marked, none for a group; a button for each
      sidebar tab (`Tab::ALL`), which selects the node at that tab; and Copy id, through `dom::copy`
      (`navigator.clipboard`, with web-sys's `Navigator` and `Clipboard` features), which says so
      where the page has no clipboard. A status pick stays open until it is answered and says why
      where it wrote nothing.

      `calls::set_status` (`POST /app/status`) reads the plan, refuses a group (`StatusSet::Group`)
      and a node the plan lacks, writes nothing for a status the node has (`Unchanged`), and otherwise
      puts the node back over the version it read with the new status, as `reef status` does,
      answering `Applied`, `Stale` or `Refused`. `tests/app.rs::a_status_is_set_over_the_version_read`
      covers it. `check.py::c100_the_node_menu` right-clicks W1 to queue it last, then first, then
      off, checking the panel and `reef queue ls` each time; finds G1's head offering the queue and no
      status; closes a menu with Escape and with a press on another box, which selects nothing; blocks
      L1 and sets it back to do, read through `reef node get`; opens W2's Comments tab; and reads L1's
      id back from the clipboard.
  acceptance:
  - feature: Right-clicking a node opens its menu in place of the browser's, and Queue first or Queue last puts it at that end of the plan's queue, `reef queue ls` agreeing
    status: met
    test: central/reef-app/check.py::c100_the_node_menu
  - feature: The menu takes a queued node off the queue, and offers that only for a queued node
    status: met
    test: central/reef-app/check.py::c100_the_node_menu
  - feature: The menu sets a node's status, and a group's menu offers no status
    status: met
    test: central/reef-app/check.py::c100_the_node_menu
  - integration: A status set through the app is written over the version read, an unchanged one writes nothing, and a group's is refused
    status: met
    test: central/reef-app/tests/app.rs::a_status_is_set_over_the_version_read
  - feature: The menu opens the node's sidebar at the tab picked, and copies the node's id
    status: met
    test: central/reef-app/check.py::c100_the_node_menu
  - feature: Escape, or a press outside the menu, closes it and selects nothing
    status: met
    test: central/reef-app/check.py::c100_the_node_menu
- id: C101
  title: C101 A semantic Changes view
  status: done
  needs:
  - C33
  - C35
  docs:
  - design/app.md#the-semantic-changes-view
  description:
    why: The Changes tab shows a revision a file at a time as git's hunks, so reviewing it means opening every file and working out which type or function each hunk changes.
    what: The Changes tab groups each file's changes by struct, enum or trait with its methods, then loose functions, each tagged new, changed or removed, with a toggle back to the classic view.
    details: |
      **Outlined when captured.** `crates/reef/src/outline/`: `outline_files` runs in
      `capture::take` right after `Checkout::diff`, which stays equal to git's oracle, reading each
      side's blob through `Checkout::blob`. A `Grammar` a language (`rust.rs`, through
      tree-sitter 0.27.2 and tree-sitter-rust 0.24.2, pinned) walks a side's tree into items:
      structs, enums, unions and traits, impl blocks owned by their type's last name with the trait
      they implement, their members, inline modules owning their loose items, and loose fns,
      consts, statics, types and macros, each starting at the attributes and comments written above
      it. Items are matched across sides by owner, kind, name, trait and occurrence; one on a side
      alone is new or removed if it holds a changed line. A side with a syntax error, a binary,
      symlinked, non-UTF-8 or over-1-MiB side, or more than 500 changed items leaves the file
      without an outline.

      **Kept as JSON.** `reef_api::outline` holds `Outline`, `Item`, `Owner` and `Span`, and the
      pure `attribute` (each line to its deepest item, a blank change to the change beside it),
      `groups` (by owner, a type's tag its definition's) and `segments`. `FileDiff` and
      `FileSummary` gain a serde-defaulted `outline`, so rows needed no schema change; the store's
      `file_row` drops it with a too-large file's hunks. The capture recounts its kept items with
      `attribute` over them alone, as the app draws them.

      **The view.** `central/reef-app/src/side/outline.rs`: `OutlineFile` lists a file's groups
      from the revision's listing and fetches its hunks once one opens; `OutlineView` draws each
      open group's members with their own runs of lines through `diff::hunk_table`, which the
      classic `Diff` now shares, so threads and the comment box work alike, and a thread on a line
      no item holds keeps the "Other lines" group. "Show in diff" opens the group holding its line,
      once (`Review.applied`). `View::diff_mode` holds the Semantic/Classic toggle at the tab's top
      right, kept in localStorage. `check.py`'s C33, C35, C36 and C56 pick Classic first.
  performance:
    labels:
    - Diff alone
    - Diff and outline
    metrics:
    - name: C100's diff read for a capture, ms
      key: capture-diff-ms
      unit: ms
      before:
      - 1.214
      - 1.21
      - 1.234
      after:
      - 19.738
      - 19.667
      - 19.847
    note: '`cargo run --release -p reef --example diff_bench -- --commit b51a32b --rounds 20` on this machine: C100''s commit from its parent, 13 files +788 −22, of which 7 Rust files outline into 22 items, the checkout open, 3 warm-up rounds then 20 timed. Medians with the lowest and highest. Outlining alone took 18.4 ms (18.3–18.6).'
  acceptance:
  - unit: A Rust file's outline names each changed item with its owner, trait and lines, glues its attributes and comments, and is left out when either side doesn't parse
    status: met
    test: crates/reef/src/outline/mod.rs::tests::edits_outline_by_their_items
  - unit: A line goes to its deepest item, a blank line to the change beside it, and a type's group takes its definition's tag
    status: met
    test: crates/reef-api/src/outline.rs::tests::lines_go_to_their_deepest_item
  - integration: A capture keeps each Rust file's outline with the file and its summary, and a file too large to keep has none
    status: met
    test: central/reef-service/tests/cli.rs::a_capture_outlines_its_rust_files
  - integration: An outlined file draws its groups, kinds, tags and counts, each member with only its own lines, and every thread on its lines once
    status: met
    test: central/reef-app/tests/outline.rs::an_open_group_draws_its_members_lines
  - feature: The Changes tab opens in the semantic view, each Rust file's changes grouped by type and loose function and tagged, and a group opens to lines that take comments
    status: met
    test: central/reef-app/check.py::c101_semantic_changes
  - feature: A toggle at the tab's top right switches to the classic view and back, kept across nodes and reloads
    status: met
    test: central/reef-app/check.py::c101_semantic_changes
  - perf: Reading C100's commit's diff and outlining it, as a capture does, takes at most 50 ms
    metric: capture-diff-ms
    at-most: 50.0
- id: M14
  title: M14 Clients that keep up
  docs:
  - ../../docs/src/central/using/updates.md
  nodes:
  - id: C116
    title: C116 A static reef in every release
    status: done
    needs:
    - C115
    description:
      why: A release's `bin/reef` is a glibc build nobody downloads, and no `reef` binary knows which commit it was built from, so nothing can tell an old client from a new one.
      what: Every release ships a static musl `reef` for plain x86-64, stamped with its commit and that commit's time, which `reef version` prints.
      details: |
        - **The stamp** (`reef_api::Build`, `reef::update::BUILD`): `REEF_COMMIT` and
          `REEF_COMMITTED` (unix seconds) read with `option_env!`, and the target from
          `std::env::consts` and `cfg!(target_env)`. `reef version [--json]` prints it before any
          config is read.
        - **The build** (`reefadm::compile`): the stable build splits in two. reefadm stays a plain
          glibc build; `reef` builds for `x86_64-unknown-linux-musl` with the stamp and
          `CARGO_PROFILE_RELEASE_STRIP=symbols`. The commit's time is probed where reef's checkout
          is fetched (`git show -s --format=%ct`).
        - **The toolchain**: `musl-gcc` from `musl-tools` (ring and tree-sitter compile C), and the
          musl target added to stable.
    acceptance:
    - unit: An unstamped build prints `(not stamped)`, and a stamped one its commit and date, as text and as JSON
      status: met
      test: crates/reef/src/update.rs::tests::a_build_prints_its_stamp_or_its_lack; crates/reef-api/src/build.rs::tests::a_stamp_needs_the_commit_and_its_time
    - unit: reefadm builds reefadm for glibc and reef for musl with the stamp and the strip, and installs `bin/reef` from the musl target
      status: met
      test: crates/reefadm/src/compile.rs::tests::the_central_builds_take_the_class_in_a_directory_of_its_own; crates/reefadm/tests/steps.rs::a_team_server_deploys_on_a_fresh_host
    - unit: A host without musl-gcc or stable's musl target gets them, and a ready host installs nothing
      status: met
      test: crates/reefadm/src/toolchain.rs::tests::a_ready_host_installs_nothing; crates/reefadm/src/toolchain.rs::tests::a_bare_host_gets_everything_after_asking
    - feature: A musl build here is static and `reef version --json` prints its stamp
      status: met
      test: 'by hand: REEF_COMMIT=dc5e68c… REEF_COMMITTED=1795809468 CARGO_PROFILE_RELEASE_STRIP=symbols cargo +stable build --release --locked --target x86_64-unknown-linux-musl -p reef built in 24 s; file says static-pie linked, stripped, ldd says statically linked, and reef version --json printed the commit and time; that binary read europa''s project over HTTPS'
  - id: C117
    title: C117 Central reef serves its client
    status: done
    needs:
    - C116
    description:
      why: A new machine has to build `reef` from source to reach central reef, and a client can't tell whether the server it calls has a newer one.
      what: Central reef serves the stamped `reef` from its release, and its build, to anyone, and names that build in a header on every API response.
      details: |
        - **Finding it** (`reef_service::served::Served::find`, beside shoal's `client.rs`):
          `[client] reef`, else `bin/reef` beside the running release's, asked `version --json` once
          with a timeout and kept only if stamped. Otherwise it keeps why (`NoClient`), which
          reef-serve and reef-app print as they start, with the build they serve when they do.
        - **Routes**: `GET /api/v1/client` answers the `Build`, `GET /api/v1/client/reef` the bytes.
          Both answer without a token, so a machine can fetch it before signing in.
        - **The header**: `reef-client: <commit> <committed>` on every API response, when a client
          is served.
        - **Config**: a `[client]` table, `reef` for the service, `update` for the CLI, since the two
          may share one file.
    acceptance:
    - integration: A service given a stamped client serves its build and its bytes, and names it in a header on API responses
      status: met
      test: central/reef-service/tests/sign_in.rs::memory::the_served_client_is_its_build_its_bytes_and_a_header
    - integration: Both client routes answer without a token on a server that requires sign-in
      status: met
      test: central/reef-service/tests/sign_in.rs::memory::the_client_routes_answer_without_a_token
    - integration: With no client, the route says not found and responses carry no header
      status: met
      test: central/reef-service/tests/sign_in.rs::memory::with_no_client_the_route_says_not_found
    - unit: An unstamped build is refused as the served client
      status: met
      test: central/reef-service/src/served.rs::tests::only_a_stamped_client_is_served
  - id: C118
    title: C118 reef updates itself
    status: done
    needs:
    - C117
    description:
      why: After central reef is upgraded every user must reinstall `reef` by hand, and an old one only finds out through a missing route.
      what: A stamped `reef` that sees its server serve a newer build replaces itself in the background, and `reef update` does it by hand, a reefadm release's own copy left to reefadm.
      details: |
        - **Noticing** (`update::Watch`): `Client::send_as` reads `reef-client` off every answer. A
          stamped build older than it, with `[client] update` on, a binary reef may replace and no
          install of that commit failed within the hour, spawns `reef update --auto` once a process
          (an `AtomicBool`), in its own process group, `--conf` and `--credentials` passed on, its
          stderr `update.log`, and a thread reaps it so a long-lived `reef mcp` leaves no zombie.
        - **`reef update [--check] [--force]`**: newer only by commit time; an older build, or the
          same, with `--force`; an unstamped build by hand always. Hidden `--auto` is quiet and
          newer-only, takes `update.lock` with `File::try_lock`, and keeps `update.toml` so a failed
          commit waits an hour. Hidden `--into PATH` aims it at another file, for the tests.
        - **An old server**: `GET /api/v1/client` answering 404, 401, or 200 with no JSON (reef-app's
          page) is no client.
        - **Installing** (`update::install`): streamed with `Response::chunk()` to
          `.reef-update-<pid>` beside the target, its length checked, chmod 0755 and fsync, run with
          `version --json` to check its commit, then renamed over the target, a link followed. Any
          failure removes the new file. A run that hits `ETXTBSY` (a fork elsewhere holding the
          file just written) is tried again, here and in C117's `Served::read`, whose test hit it.
        - **A reefadm release's own `reef`** (`may_replace`): `releases/<name>/bin/reef` with
          `release.toml` beside `bin`, through any link, is refused, saying to run `reefadm upgrade`.
        - **Replaced while running**: `update::own_exe` strips Linux's ` (deleted)`; the updater's,
          the follower's and the injector's spawns use it.
        - **The skill**: SessionStart adds to its context, when the repo's `.claude/skills/reef`
          isn't the one this binary writes (`skill::stale`), to suggest `reef claude install`.

        **Tried live** (2026-11-27): `reefadm --host` deployed a temp commit of this tree to an
        ubuntu:26.04 LXD container, whose reef-app said it hands out its release's `reef`. A second
        container fetched it with curl and no token, signed in, and ran it. After `reefadm upgrade`
        to a later commit, its next `reef whoami` updated it in the background within a second
        (`update.log`). A copy with `[client] update = false` and an unstamped build stayed put,
        and the unstamped one updated by `reef update` by hand. The server's previous release's
        `reef` stayed put and `reef update` there said to use `reefadm upgrade`. A `reef mcp`
        running across a swap (`/proc` naming its binary ` (deleted)`) still answered, and under
        strace spawned its updater from the stripped path.
    acceptance:
    - unit: Builds order by commit time, stamped or not, older, equal and newer
      status: met
      test: crates/reef/src/update.rs::tests::builds_order_by_their_commits_time
    - unit: The header reads as a commit and its time, and anything else as nothing
      status: met
      test: crates/reef/src/update.rs::tests::the_header_reads_as_a_commit_and_its_time; crates/reef-api/src/build.rs::tests::a_stamp_needs_the_commit_and_its_time
    - unit: A binary in a reefadm release is never replaced
      status: met
      test: crates/reef/src/update.rs::tests::a_releases_own_reef_is_never_replaced
    - unit: A replaced binary's path drops Linux's ` (deleted)`
      status: met
      test: crates/reef/src/update.rs::tests::a_replaced_binary_drops_deleted
    - unit: An install swaps in a binary that says the wanted commit, and refuses one that doesn't, leaving the original
      status: met
      test: crates/reef/src/update.rs::tests::an_install_swaps_in_the_commit_wanted_and_refuses_another
    - integration: '`reef update --force` against a server with a client swaps the target'
      status: met
      test: central/reef-service/tests/cli.rs::update_swaps_a_binary_for_the_served_client; central/reef-service/tests/cli.rs::update_says_when_the_service_hands_out_no_client
    - feature: A stamped reef on another host follows its server's upgrade by itself, and reefadm's own copy, an unstamped build and `update = false` stay put
      status: met
      test: 'by hand: two ubuntu:26.04 LXD containers; reefadm --host deploy then upgrade of c1 to a later temp commit; c2''s curl-fetched reef updated itself on its next whoami (update.log), a copy with update = false, an unstamped build and c1''s previous release''s reef stayed put (sha256 unchanged), and a reef mcp running across a swap answered and spawned its updater from the stripped path under strace'
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
- a Claude session in the terminal is fed the next node, its context cleared: it plans in plan
  mode, the user answers its questions and approves the plan, Claude implements it, then
  resolves any open comments;
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
   session. reef never owns the session: it clears the context between nodes by typing `/clear`
   into the session's own zellij or tmux pane, or asks the user to
   ([X4](design/feed-loop.md#what-x4-found)). Answering Claude's questions and approving its
   plans in the app comes later and is optional ([M9](milestones/m9.md)).
4. **This plan lives in central reef.** It is `reef/central`, and `plan/`, apart from `docs/`,
   is its export, written back over itself by `reef export plan --replace` in the change that
   edits it ([C29](milestones/m6.md)).
5. **The user orders the work.** Whether a node is ready still comes from its `needs`. Among
   ready nodes, a queue the user orders decides what Claude takes next. The queue is per
   project and spans its plans. It is set from the CLI first and by dragging in the app later
   ([M5](milestones/m5.md)).
6. **The UI is a Dioxus 0.7 fullstack app.** `reef-app` runs on axum. The same server answers a
   plain JSON API for the CLI, the MCP server and the hook. Both go through one service layer,
   the only code that touches shoal. The app draws the plan with Dioxus components of its own
   ([C55](design/app.md#the-canvas-in-dioxus)), from reef-draw as reef-view does, after first
   mounting the existing viewer ([X6](design/app.md#what-x6-found)). mdbook-reef and its viewer
   stay, as the static mode.
7. **reef keeps each node's changes.** Commits are squashed regularly, so git is only read and
   never used to store them. reef's CLI reads it in the node's local checkout, and nothing is
   pushed or fetched. A node's diff runs from where it started, carried over what its base
   branch brought in since. Nodes worked on at the same time each get their own worktree
   ([X3](design/code-and-comments.md#what-x3-found)).
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
