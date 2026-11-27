# Recording work

- **Start a node where its work happens:** `reef start L2` records your checkout's branch and
  start and marks the node in progress. Each start, a start again included, is kept in the
  node's history, so `reef node log L2` and the app's Timeline say who started it and when.
  `reef work` shows what this checkout is on.
- **Its changes are captured from your checkout,** by the git hooks `reef claude install` writes
  ([Set the repo up for Claude Code](../projects/claude.md)),
  or by hand with `reef node capture`. Each capture is a revision of the node, which the app's
  Changes tab shows.
- **Run a node's tests with `reef test`,** which runs `cargo test` with the same arguments,
  `--` and libtest's after it included, and tells central reef how far they have got:

  ```sh
  reef test --workspace -- --nocapture
  ```

  - **What it shows is cargo's,** colours and progress bar in a terminal, and it exits with
    cargo's code. reef's own options, such as `--conf`, go before `test`.
  - **The total is known before a test runs:** the binaries are built and listed first, so a
    filter, `--ignored` and `--skip` are counted as the run will run them.
  - **The run is told on the node this worktree is on,** the one `reef work` shows, with each
    binary's counts and each failure's output.
  - **Telling central reef is best-effort.** Where the checkout is no registered project's, or
    the server doesn't answer, `reef test` says so once and the tests run as they would have.
- **Comments are threads** on a revision's lines, or on the node:

  ```sh
  reef comment ls                  # the plan's open threads; --node L2, --all for resolved too
  reef comment reply 7 "Fixed in the next push"
  reef comment resolve 7 --reply "No change needed"
  ```

  A commit with a `Reef-Resolves: 7` trailer resolves thread 7 once it is captured. Each comment
  is listed with who wrote it, from their login, and "Claude, for" them where Claude wrote it
  through reef's tools.
