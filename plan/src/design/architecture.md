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
| `reef-service` | nightly if shoal is in-process | the store and the model: the only code that touches shoal |
| `reef-app` | nightly if shoal is in-process | the Dioxus fullstack app on axum, which also mounts the JSON API |

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
`shoal::server::node::main::<Db>()`, that clients reach over the network.

### Design choices

- **Only `reef-service` and `reef-app` may depend on shoal.** Every other crate stays stable,
  and `cargo tree` of the stable workspace and of the wasm build shows no shoal.
- **In-process or a node program is X1's to decide.** In-process means one binary to run, but
  every `dx serve` restart also restarts shoal, with its locked memory and its start-up time, and
  `reef-service` and `reef-app` need nightly. A node program means a second process, but the app
  restarts alone, and both can stay stable on the client half.
- **A separate `central/` workspace if the server needs nightly.** It uses the stable crates by
  path, so the stable workspace and the committed `reef.js` build stay as they are.

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
