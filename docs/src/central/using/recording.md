# Recording work

- **Start a node where its work happens:** `reef start L2` records your checkout's branch and
  start and marks the node in progress. `reef work` shows what this checkout is on.
- **Its changes are captured from your checkout,** by the git hooks `reef claude install` writes
  ([Set the repo up for Claude Code](../projects/claude.md)),
  or by hand with `reef node capture`. Each capture is a revision of the node, which the app's
  Changes tab shows.
- **Comments are threads** on a revision's lines, or on the node:

  ```sh
  reef comment ls                  # the plan's open threads; --node L2, --all for resolved too
  reef comment reply 7 "Fixed in the next push"
  reef comment resolve 7 --reply "No change needed"
  ```

  A commit with a `Reef-Resolves: 7` trailer resolves thread 7 once it is captured.
