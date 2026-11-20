# Architecture

## Context

reef today is one book at a time. `mdbook-reef` walks a book's `src`, parses each node's
frontmatter, renders its markdown, resolves its doc links on disk, and embeds the whole plan as
JSON in every page that draws it. The viewer, `reef-view`, reads that JSON from the page and
recomputes status and readiness in the browser. Centralized reef keeps the model and the viewer.
It replaces where the nodes come from and who serves them.

The crates, once this plan is done:

| Crate | Builds on | Does |
|---|---|---|
| `reef-model` | stable, native and wasm | nodes, the checked graph, views (unchanged) |
| `reef-layout` | stable | the layout (unchanged) |
| `reef-render` | stable | markdown rendering, doc links, why/what extraction over a `DocSource` ([C1](#a-model-any-source-can-feed)) |
| `reef-view` | stable, wasm | the viewer, plus a `mount` entry point ([C12](app.md#the-viewer-inside-the-app)) |
| `mdbook-reef` | stable | the static mode, now over `reef-render`'s filesystem source |
| `reef-api` | stable | the JSON API's request and response types |
| `reef` | stable | the CLI, the MCP server and the hooks, all HTTP clients of the JSON API |
| `reef-service` | nightly; stable built external-only | the store and the model: the only code that touches shoal, in-process or external ([C57](../milestones/m2.md)) |
| `reef-app` | nightly; stable built external-only | the Dioxus fullstack app on axum, which also mounts the JSON API |
| `reef-shoal` | nightly | the external node: shoal's node program over reef's schema ([C57](../milestones/m2.md)) |

## Shoal in the build

reef builds against shoal's `ScanSupport` branch, a local checkout used by path. Shoal is not on
crates.io, and its glommio is a path dependency too. Shoal's server needs nightly Rust
(`#![feature(trivial_bounds)]` in shoal-core), a patched glommio checked out at `../glommio`,
`-Ctarget-cpu=native` for gxhash, and locked memory. reef's workspace is stable, `mdbook-reef`
should stay publishable, and the viewer's wasm build must never pull shoal in.

Shoal's client half needs none of that. `shoal` with `default-features = false` and a
`#[shoal::db(client)]` schema builds on stable 1.98, with or without `-Ctarget-cpu=native`. Its
tree holds tokio, kanal and gxhash, but no glommio and no shoal-core.

Shoal can run inside a tokio program (`ShoalPool::<Db>::start(conf)` then `pool.ready(timeout)`,
as `shoal/examples/tmdb.rs` does), or as a node program, a three-line binary calling
`shoal::server::node::main::<Db>()`, that clients reach over the network. Either way the client
is the same: an in-process pool is reached over loopback TCP too.

### Design choices

- **Both: in-process unless an external node is configured.** With no node's address in its
  config, reef-service starts a pool in its own process and connects to the address it binds.
  With one, it connects to that node. Only start-up differs, so the rest of the service sees one
  client. A restart costs the same either way ([What X1 found](#what-x1-found)).
- **Only `reef-service`, `reef-app` and `reef-shoal` depend on shoal.** Every other crate stays
  stable, and `cargo tree` of the stable workspace and of the wasm build shows no shoal.
- **The engine is a feature, `embedded`, given to the server build alone.** It turns on
  `shoal/server` and swaps `#[shoal::db(client)]` for `#[shoal::db]`. It can't be a Cargo
  default, because dx keeps default features on in the wasm build too, so dx builds the server
  with `@server --platform server --features embedded`. Without it, the service is external-only
  and builds on stable.
- **One schema, two halves.** reef's tables and its one `#[shoal::db]` struct are a module of
  reef-service: the whole schema with `embedded`, the client half alone without it. reef-shoal
  depends on reef-service with `embedded`, for the schema and for the config checks
  ([C10](../milestones/m2.md)), so the client and the node can't disagree on the fingerprint.
- **reef's config names shoal.** It is TOML at `$REEF_CONFIG`, else
  `$XDG_CONFIG_HOME/reef/config.toml`. Under `[shoal]`, an `address` means an external node, and
  `data`, `cores` and `memory` set the in-process pool's data directory, shards and per-shard
  budget. An external node reads a shoal config of its own (`reef-shoal serve --conf shoal.yml`).
- **A separate `central/` workspace, on nightly.** It holds reef-service, reef-app and reef-shoal,
  and uses the stable crates by path. A workspace with an `embedded` feature locks shoal-core and
  glommio even while the feature is off, so it needs both checkouts beside it. The stable
  workspace and the committed `reef.js` build stay as they are.
- **`-Ctarget-cpu=native` for the host only.** It goes in `central/.cargo/config.toml` under
  `[target.x86_64-unknown-linux-gnu]`, never in an exported `RUSTFLAGS`, which dx passes to the
  wasm build too.

### What X1 found

Tried in `spikes/x1-shoal`, a scratch Dioxus 0.7.10 fullstack server that runs shoal either way
and answers a round trip on a plain JSON route and through a server function.

- **A restart costs the same in both modes.** A `dx serve` restart, from an edit to the first
  round trip from the new build, took 1.76 s in both modes (five rounds each). An in-process
  pool is ready 40 ms after it starts, which is lost in dx's rebuild and relaunch. Built on stable
  without the engine, external-only took 1.49 s.
- **Data survives the server being killed.** dx kills its server on every rebuild. A row written
  before survives SIGKILL and SIGTERM alike, and an external node's stop and start. The data
  directory's lock is a file lock, released with the process.
- **Killing dx alone orphans its server.** The orphan keeps the data directory locked, and the
  next start fails with `StorageDirectoryLocked`. Anything that runs `dx serve` stops its whole
  process group.
- **`dioxus::serve` runs its closure again after a hot-patch**, so the store opens once and
  never starts a second pool. `--hot-patch` itself failed to build the wasm client under dx
  0.7.10 ("exceptions proposal not enabled"), so it is untried with shoal.
- **An exported `RUSTFLAGS=-Ctarget-cpu=native` breaks the wasm client.** wasm32 then reports no
  target features. wasm-bindgen leaves its externref table out, and its CLI fails to find
  `__externref_table_alloc`. dx keeps the host-only flag from `.cargo/config.toml`.
- **The pool starts on a thread of its own.** `start` and `ready` block, and shoal's error can
  carry a panic payload that isn't `Sync`. `cores` must be set, or a shard is pinned to every CPU
  but cpu 0.
- **Dioxus's logger wins.** `dioxus::serve` installs its subscriber before the closure runs, so
  an in-process pool doesn't call shoal's `trace::setup`, and its spans go to Dioxus's logger.
- **Shoal's data paths must be absolute.** A relative one fails to open with EBADF, and `SHOAL_*`
  variables can't set a nested key, so an external node's config is written out with the path
  filled in ([C10](../milestones/m2.md)).
- **A get that finds nothing is an error.** `send_one` reports it as `QueryDidNotSucceed`
  (shoal's known issue 55), which the store reads as no row ([C7](../milestones/m2.md)).
- **The JSON API shares the app's port.** A plain axum route on `dioxus::server::router` answers
  beside the server functions ([C13](../milestones/m4.md)).

### Open questions

- Whether the CLI starts the server when it isn't running, or says so and stops. Only one
  process can open shoal's data directory at a time.

## A model any source can feed

`Graph::build(Vec<Node>)` in `reef-model` is already independent of where nodes come from, and
`Payload { nodes }` is what the viewer reads. What sits above it is not:

- `Plan::load` in `crates/mdbook-reef/src/plan.rs` walks files with `std::fs`;
- the why, the what and the design choices are taken from linked docs read off disk;
- links follow mdbook's path conventions (`paths.rs`), and every node's provenance is a file and
  line.

### Design choices

- **`reef-render` gets the rendering and doc resolution**, over a `DocSource` trait that reads a
  doc by path and lists its headings. It has a filesystem source, which mdbook-reef uses, and an
  in-memory one. The service gets a repo and plan-pages source ([C14](app.md#docs-in-central-mode)).
- **The example book is the test.** Its payload JSON must be byte-identical before and after
  the move, and `mdbook-reef check` must print the same.
- **Names, not just ids.** A node's full name is `project/plan/id` ([C2](../milestones/m1.md)).
  A bare id still resolves within its own plan, so stored specs read like frontmatter. `needs`
  across plans are not supported at first.

## One service, two front doors

The app, the CLI, the MCP server and the hooks all change the same plans. If each touched shoal,
versions, validation, atomic writes and derived state would each be written several times.

### Design choices

- **One service layer.** `reef-service` is the only code that opens shoal. Each put is checked
  against the whole plan with `mdbook-reef check`'s rules, and a put that breaks the plan is
  refused with check's message.
- **A write is refused only if it adds a problem.** The plan is built as it stands and as the
  write would leave it, and only problems found after that weren't found before refuse it.
  A stored node's problems sit at its name, with no line, so they hold still across writes,
  and a fix gets through on a plan that already has other problems.
- **Puts come in batches, checked as one change.** A group with no members is work that says
  neither why nor what, and a member alone names a group that isn't a node, so neither could
  go in first. Each put names the version its writer read. The check runs under the plan's
  write lock, so two writes fine alone can't break the plan together.
- **Docs come from the project's working tree.** A stored node's links resolve from its repo's
  root, where import puts them ([C26](../milestones/m6.md)), so a link to a missing doc or
  heading is refused, and a why or a what can still come from a doc.
- **Two front doors onto it.** A plain JSON API on axum (typed in `reef-api`) for the CLI, the
  MCP server and the hooks; Dioxus server functions for the app. Both run in one axum server,
  and the CLI never depends on the app's wire format.
- **Each new kind of data adds three things:** an operation on the service, a JSON route and a
  server function. This holds for the queue, work records, comments and the loop's state.
- **Localhost only** until [M10](../milestones/m10.md).

## The CLI

The terminal is where Claude and the user already work, and Claude's loop must not wait for the
app.

### Design choices

- **`reef` is stable and small:** an HTTP client of the JSON API, with no shoal and no Dioxus in
  its tree.
- **It takes the same YAML as frontmatter.** `reef node put` reads a `NodeSpec`, and
  `reef node get` prints one, so the skill's rules and the reference carry over unchanged.
- **The working directory picks the project:** the registered project whose repo holds it. A
  linked worktree picks its main repo's project (`git rev-parse --git-common-dir`).
  `--project` and `--plan` override.
