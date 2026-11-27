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
