# central: reef's store and service over shoal

Centralized reef keeps projects, plans and nodes in shoal instead of in a book's markdown. The plan
for it is the book in `../plan`. This workspace is the only part of reef that depends on shoal, so
it is a workspace of its own, which reef's stable root workspace excludes.

```
reef-service   the service and its store: every write checked against the whole plan, reef's
               shoal schema, and the only code that touches shoal; the JSON API's router, and
               `reef-serve`, which serves it alone
reef-shoal     the external node: shoal's node program over reef's schema
```

The service's data types, and the JSON API's requests, answers, routes and changes, are
`reef-api`'s, in reef's stable workspace (`../crates/reef-api`), so the API's clients need neither
shoal nor this workspace. One is `reef` (`../crates/reef`), the terminal's client: `reef project
add|ls|show` and `reef plan new|ls`, which call the address `[api] address` names and say so when
nothing answers there.

## Building

It needs shoal's `ScanSupport` checkout at `../../shoal` and glommio at `../../glommio`, beside
reef. `.cargo/config.toml` sets `-Ctarget-cpu=native` for the host alone, which gxhash wants; never
export it in `RUSTFLAGS`, where a wasm build would get it too.

- **`embedded`** links shoal's engine into reef-service, which needs nightly.
- **Without it**, reef-service is shoal's client half alone and builds on stable.

```sh
cargo +nightly test --workspace         # everything, with the engine (reef-shoal turns `embedded` on)
cargo +stable test -p reef-service      # the client half alone; a workspace-wide stable build fails
```

## Reaching shoal

reef-service starts a shoal pool in its own process, unless reef's config names an external node.
The config is TOML at `$REEF_CONFIG`, else `$XDG_CONFIG_HOME/reef/config.toml`, else
`~/.config/reef/config.toml`. With no file there, every key takes its default.

```toml
[shoal]
address = "127.0.0.1:12100"   # an external node; unset starts shoal in this process
data = "/home/me/.local/share/reef/shoal"   # the in-process pool's data; default $XDG_DATA_HOME/reef/shoal
cores = 2                     # the in-process pool's shards
memory = "256MiB"             # each shard's memory budget

[api]
address = "127.0.0.1:12180"   # where the JSON API listens, and `reef` calls it: loopback only
```

Only one process can open a data directory. `Service::shared` opens the process's service once,
however often it is asked, since `dioxus::serve` reruns its setup after every hot-patch. Stopping an
in-process pool cleanly takes up to 3 s.

An external node is `reef-shoal`, over a shoal config of its own (`reef-shoal/shoal.example.yml`).
Its data paths must be absolute, and never on tmpfs.

Before shoal starts, either way, reef checks its config and names the fix for each problem. It
refuses a memory budget of 0, a relative or tmpfs data path, and a locked-memory limit under
512 KiB. It warns on a limit under 30 MiB for each shard and 30 more, below which glommio runs
without registered io buffers (`ulimit -l unlimited` lifts it).

```sh
cargo +nightly run --release -p reef-shoal -- serve --conf shoal.yml
```

## The service

`Service<B: Backend>` is what the front doors call: the JSON API, and the app's server functions.
It reads and writes through a `Store`, and checks every write against the whole plan with
`mdbook-reef check`'s rules.

- **A write is refused if it adds a problem.** The plan is built as it stands and as the write
  would leave it, and a problem found after that wasn't found before comes back as
  `Checked::Refused`, with check's own message. A stored node's problems sit at its name
  (``reef/central/C11: needs `X`, which is not a node``), so a fix gets through on a plan that
  already has other problems.
- **Puts come in batches** (`Service::put`), each put naming the version its writer read. A
  group with no members yet is work that says neither why nor what, and a member alone names a
  group that isn't a node, so a group goes in with its members. A stale put refuses the whole
  batch as `Checked::Stale` before it is checked. A batch is written in place, not atomically:
  a write refused partway, which only another process could cause, is an error naming what was
  written.
- **A whole plan is checked too** (`Service::write_plan`). An import into an empty plan is
  refused if it has any problem, before a revision is reserved.
- **Docs come from the project's working tree.** A stored node's links resolve from its repo's
  root, so a link to a doc or heading that isn't there refuses the write, and a node can take
  its why and what from a doc. A link that leaves the repo (``doc `../x.md` leaves the repo``)
  refuses it too, and no doc is read through a symlink that leads out of the repo.
- **The check runs under the plan's write lock**, through `Store::put_checked` and
  `Store::write_plan_checked`, so two writes that are fine alone can't break the plan together.
  It builds the plan off the async threads, since its docs are files.
- **`check` and `payload`** give a plan's problems and the viewer's payload. The payload's doc
  links are absolute paths under the plan's `docs/` (`reef_api::paths::docs`), where `doc_page`
  renders each doc as the page mdbook would write for it: a `<main>` holding the doc, each
  heading under mdbook's id, so the viewer's *Read here* cuts its section out unchanged. Only
  markdown is served, so an image a doc shows doesn't load there.
- **Each applied write is a change.** A project registered, a plan listed, a batch put or a whole
  plan written goes to every subscriber of `Service::changes`, after the write's lock is let go,
  so two writers' changes to one plan can arrive in either order; each put names its node's new
  version. `Service::end_changes` closes every subscriber, as a server does before it stops.

The store's own `put` and `write_plan` are unchecked; nothing outside the service and the tests
writes through them.

## The JSON API

`api::router(service)` is a plain axum 0.8 router, the version Dioxus 0.7.10 resolves, so reef-app
can mount it beside its server functions. It takes an `Arc<Service<B>>`, or the `&'static Service`
that `Service::shared` opens. Every route is one service call, and every body and answer is a
`reef-api` type; `reef_api::paths` has every path, and builders that percent-encode ids.

| Route | Answer |
|---|---|
| `GET /api/v1/projects` | every `Project` |
| `POST /api/v1/projects` `NewProject` | `AddedProject`: 201, or 409 if the name or repo is taken |
| `GET /api/v1/find-project?dir=` | the `Project` a directory belongs to, or 404 |
| `GET /api/v1/projects/{project}/plans` | every `PlanInfo` |
| `POST /api/v1/projects/{project}/plans` `NewPlan` | `AddedPlan`: 201, or 200 if it was listed |
| `GET …/plans/{plan}` | its `PlanInfo`, naming the revision readers load |
| `GET …/plans/{plan}/nodes` | its `Listing` |
| `POST …/plans/{plan}/nodes` `Puts` | `Checked`: 200, 409 if stale, 422 if it would add a problem |
| `GET …/plans/{plan}/nodes/{id}` | the `StoredNode` |
| `PUT …/plans/{plan}/revision` `PlanWrite` | `Rewrite`: 200, or 422 if it would add a problem |
| `GET …/plans/{plan}/payload` | the viewer's `Payload` |
| `GET …/plans/{plan}/check` | its problems, as check prints them |
| `GET …/plans/{plan}/docs/{*path}` | a doc as its HTML page (`docs/design.html` for `docs/design.md`), or 404 |
| `GET /api/v1/changes?project=&plan=` | server-sent events, one a `Change` |

- **A refusal is an answer**, its outcome the body whatever the status, such as
  ``{"refused":[{"source":"reef/central/C11","line":null,"message":"needs `X`, which is not a node"}]}``.
- **An error is an `ApiError`**, `{"code":…,"message":…}`: `not-registered`, `not-listed` and
  `not-found` are 404, `bad-name` and `bad-request` 400, and `internal` 500. Paths a client sends
  (`repo`, `dir`) must be absolute directories on the service's host.
- **The change stream** sends each change as an event named by its kind (`project-added`,
  `plan-added`, `nodes-put`, `plan-written`), its data the change as JSON. It is live only: a
  client reads what it needs, then follows. One that falls behind by more than 256 changes is
  sent `missed`, with how many, and should read again. `plan` narrows it within a `project`.
- **Localhost only.** `api::bind` refuses any address but a loopback one until users sign in
  (M10).

`reef-serve` serves it alone, until reef-app mounts it. It listens before it opens shoal, so a
taken port or an address that isn't local stops it first, and on SIGINT or SIGTERM it ends the
change stream, waits for open requests and closes the service.

```sh
cargo +nightly run --release -p reef-service --features embedded --bin reef-serve   # in-process
cargo +stable run -p reef-service --bin reef-serve -- --conf reef.toml               # external node
```

`examples/payload_bench.rs` times the example book's 202-node payload, built in process and fetched
over loopback.

## The store

`Store<B: Backend>` reads and writes plans and their nodes by version. Its logic (each plan's lock,
the versions, the full-key checks) is written once, over a backend that only reads rows and makes
conditional writes the way shoal does: `ShoalBackend`, or `MemoryBackend`, its in-memory twin,
which needs no shoal and builds on stable. `tests/store.rs` runs every case against both, so shoal
is the twin's oracle.

- **A put names the version it read**, or `None` for a new node. One that isn't as expected
  comes back as `Put::Stale`, with whether the node exists, is missing or moved on.
- **A plan is listed with an empty current revision**, so a put always has somewhere to go.
- **Writes to a plan take its lock alone, and reads share it**, since reef-service is shoal's
  only writer.
- **Every row read is checked against its full key**, so a partition hash collision is an
  error, never the wrong row.
- **A project names a repo** (`Store::add_project`), located at its main worktree's root, so a
  linked worktree is its main repo. A name or a repo another project has is refused, and
  `Store::find_project` finds the project a working directory belongs to. A plan's project must
  be registered.
- **A whole plan is written as a new revision** (`Store::write_plan`): named `pending` on the
  plan's row, its rows written, then made current by one conditional write, and the revision it
  replaced pruned. A write killed midway leaves the old revision current and whole, and the next
  write prunes what it left. `tests/common/faulty.rs` kills writes where a case asks it to.

## The schema

| Table | Partition | Sort | Holds |
|---|---|---|---|
| `ProjectRow` | the constant `"projects"` | name | the repo's path |
| `PlanRow` | project | plan name | `current`, `pending` and `retired` revisions, and a `version` |
| `NodeRow` | (project, plan, revision) | id | a `version`, status, group, written order, the spec as JSON, the body |

`schema::tests::fingerprint_is_pinned` pins it. Shoal can't migrate a table yet, so a change to a
row needs a fresh data directory, and then a new pin.
