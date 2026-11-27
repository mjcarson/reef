# Test runs

A test run is reported to central reef as it goes, so the app can show how far a node's tests
have got. The service runs no tests: a client runs `cargo test` on the user's own machine and
reports to the routes in [the JSON API](api.md), `POST /api/v1/test-runs` to open a run and
`POST /api/v1/test-runs/{run}` for each report.

- **Kept in memory,** as runners' sessions are. A run lives as long as the service does. A
  report for a run the service doesn't hold, as after a restart, is answered 404, and its client
  opens it again with everything it holds.
- **A run names its client's id for it:** 1 to 64 lowercase letters, digits and dashes. Opening
  an id the service holds replaces that run.
- **On the node its checkout is on, or on none.** A run names its project, and the plan and node
  of its checkout if it is on one. A node needs its plan, and must be a node of it.
- **Each binary's counts, times and state.** A report gives every test binary as it stands:
  how many tests it holds, whether that total is only a guess, how many passed, failed and were
  ignored, and whether it is waiting, running, done or won't run. Its times, and how long the run
  has gone, are milliseconds since the run started by the client's clock, so a host whose clock
  disagrees with the service's still times its binaries right.
- **Failures come once each,** a report giving those since the last. A run keeps its first 200,
  each output cut to its last 8,000 characters, where the panic is, though its binaries count
  every one. Reading the run gives each output; a project's listing leaves them out.
- **A run that ended stays ended.** Passed, failed or stopped, a later report keeps what it adds
  but not a state that says it runs again. One whose client hasn't reported in a minute reads as
  gone.
- **Told as `test-run-changed`,** naming the run and the plan of its node. A run on no node
  reaches every plan's followers.
- **A project keeps its latest 32 runs,** letting go of the oldest that ended or went silent,
  and the last run of each command that ended passed or failed, for at most 64 commands, which a
  later run of that command is estimated from.

## The client: `reef test`

`reef test [ARGS]` (`crates/reef/src/test_run`) runs `cargo test [ARGS]` as typed and reads its
output as it comes.

- **Built and listed first.** `cargo test <cargo's args> --no-run --message-format=json` builds
  every test binary. Each `compiler-artifact` built as a test names its executable, package and
  target, and is run with `--list --format terse` and libtest's arguments as soon as it is built,
  from its package's directory, so a filter, `--ignored`, `--skip` and `--exact` count what the
  run will run. A target its manifest gives `harness = false` isn't libtest's, so it is never run
  to list it, and its count is a guess. Each package with doc tests gets a binary of them, guessed
  from the project's last finished run of the same command until their `running N tests` line.
  The build is skipped for `--no-run`, `--doc` and a `--message-format` of the user's own.
- **The build shows what cargo shows while it builds,** but not its own `Executable` lines,
  nor its errors, which the run that follows says again. Rendered warnings come with that run,
  which replays them for every fresh unit. The build's `Finished` line is kept and shown in
  place of the run's own, so it says how long the build took, after the warnings, as
  `cargo test` would.
- **A pty where a terminal watches.** With `reef test`'s standard output and error both a
  terminal, cargo writes both its streams to one pty, sized as the terminal and resized with it,
  output processing off so its bytes pass as written. Elsewhere, as under a tool, each stream has
  a pipe, and goes to `reef test`'s own. cargo keeps the terminal's standard input and process
  group, so Ctrl-C reaches it as ever, and `reef test` passes on any interrupt, terminate or
  hangup it gets.
- **What is read.** cargo's `Running <target> (<path>)`, matched to a built binary by its
  executable's file name, and `Doc-tests <crate>`; libtest's `running N tests`, each
  `test <name> ... ok|FAILED|ignored`, the `---- <name> stdout ----` blocks under `failures:`,
  and `test result:`, whose counts have the last word. Lines are read as a terminal shows them,
  escapes left out and a carriage return's overdrawn text gone. With `--nocapture`, a failure's
  output is what it printed between its name and its `FAILED`.
- **Two pipes can arrive out of order,** so a libtest line waits for cargo to name its binary, and
  cargo's line naming the next waits for the running binary's result. A binary that says
  `running N tests` while the last still owes its result shows the last one died: it ends, not
  run if it had tests left, and the run fails.
- **Reports are best-effort, beside the tests.** The run opens on the project the working
  directory picks, and the node `reef work` finds there. Where that fails, `reef test` says so
  once on standard error and only runs the tests. Each change is reported, at most one each half
  second and at least one each 15 seconds, with the failures since the last report that went. A
  failed report is dropped, and a 404 opens the run again with everything held. The last report
  is given 3 seconds once cargo exits.
- **It exits with cargo's code,** or, where a signal ended cargo, ends by that signal too, so a
  shell stops what it was running, as it would have for cargo.
