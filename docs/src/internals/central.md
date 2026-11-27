# The central workspace

Centralized reef keeps projects, plans and nodes in shoal instead of in a book's markdown. Its own
plan is `reef/central`, kept by central reef itself, and `plan/` is that plan's export (see
[reef's own plan](own-plan.md)). This workspace is the only part of reef that depends
on shoal, so it is a workspace of its own, which reef's stable root workspace excludes.

This part of the book is the service's reference: how it is built, what it checks, its JSON API
and its store. To deploy a server, use one, add projects and plans to it or administer it, read
[Central reef](../central/overview.md) instead.

```
reef-service   the service and its store: every write checked against the whole plan, reef's
               shoal schema, and the only code that touches shoal; the JSON API's router, and
               `reef-serve`, which serves it alone
reef-shoal     the external node: shoal's node program over reef's schema
reef-app       the UI, a Dioxus 0.7 fullstack app on axum, which serves the JSON API beside its
               pages
```

The service's data types, and the JSON API's requests, answers, routes and changes, are
`reef-api`'s, in reef's stable workspace (`crates/reef-api`), so the API's clients need neither
shoal nor this workspace. One is `reef` (`crates/reef`), the terminal's client: `reef project
add|fetch|ls|show`, `reef plan new|ls`, `reef node put|get|rm`, `reef page ls|get|put|rm`, `reef
status`, `reef accept`, `reef check`, `reef next`, `reef queue add|move|rm|ls`, `reef start` and
`reef work`, which record and show where a node's work happens, `reef node capture`, which keeps
a node's changes in its worktree as a revision and resolves the comments its commits'
`Reef-Resolves` trailers name, `reef comment ls|reply|resolve`, which list a plan's open threads,
reply in one and resolve one by hand, `reef loop
start|pause|stop|ls`, which keep the terminal loop of the pane a session runs in, `--width`
setting how many nodes it works at once, `reef loop take`, which takes that loop's next node and
starts it, as the MCP tool next_task does, or the next group for a loop wider than one, or a
done node again for a round of its open comments, `reef
loop attach`, which has an open session work a group's member, `reef hook session-start`, which
hands the loop to each session that starts there and names the next node to a clear between
nodes, `reef hook stop`, which seals a node done and says once what comes after the loop's node
or its group, `reef hook clear`, which Stop, or a `reef loop start` between nodes, starts in the
background to type `/clear` and the next node's kickoff into a zellij or tmux pane once Claude is
idle, `reef hook worktree-create` and `worktree-remove`, which make a node's worktree for
`claude -w <id>` with the node started there, and take it away again, and `reef import` and
`reef export`, which carry a book's nodes and pages in and out. `reef export --replace` writes a plan back over the book it lives in, keeping the
book's files, summary and `book.toml`. They call the address `[api] address` names and say so
when nothing answers there.

## Building

It needs shoal's `ScanSupport` checkout and glommio's beside reef's, at `../shoal` and
`../glommio` from reef's root (`central/Cargo.toml` names them from `central`).
`central/sources.toml` pins the commit of each that this one builds against, and the nightly it
builds on, which is what reefadm fetches and installs; move a `rev` there when the checkout
beside reef moves. `.cargo/config.toml` sets `-Ctarget-cpu=native` for the host alone, which gxhash wants; never
export it in `RUSTFLAGS`, where a wasm build would get it too.

- **`embedded`** links shoal's engine into reef-service, which needs nightly.
- **Without it**, reef-service is shoal's client half alone and builds on stable.
- **Shoal's shards need a bigger stack than std's default.** glommio starts each on a thread of
  2 MiB, which an unoptimised build of reef's thirteen tables overflows. reef-serve, reef-app and
  reef-shoal set `RUST_MIN_STACK` to 8 MiB first thing in `main`, unless it is set already
  (`pool::widen_stacks`), and `.cargo/config.toml`'s `[env]` does the same for `cargo test` and
  `cargo run`.

```sh
cargo +nightly test --workspace         # everything, with the engine (reef-shoal turns `embedded` on)
cargo +nightly test -p reef-app --features embedded   # and the app's suite, which needs it named
cargo +stable test -p reef-service      # the client half alone; a workspace-wide stable build fails
```
