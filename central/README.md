# central: reef's store and service over shoal

Centralized reef keeps projects, plans and nodes in shoal instead of in a book's markdown. Its own
plan is `reef/central`, kept by central reef itself, and `../plan` is that plan's export (see
[Running it for real](#running-it-for-real)). This workspace is the only part of reef that depends
on shoal, so it is a workspace of its own, which reef's stable root workspace excludes.

```
reef-service   the service and its store: every write checked against the whole plan, reef's
               shoal schema, and the only code that touches shoal; the JSON API's router, and
               `reef-serve`, which serves it alone
reef-shoal     the external node: shoal's node program over reef's schema
reef-app       the UI, a Dioxus 0.7 fullstack app on axum, which serves the JSON API beside its
               pages
```

The service's data types, and the JSON API's requests, answers, routes and changes, are
`reef-api`'s, in reef's stable workspace (`../crates/reef-api`), so the API's clients need neither
shoal nor this workspace. One is `reef` (`../crates/reef`), the terminal's client: `reef project
add|ls|show`, `reef plan new|ls`, `reef node put|get|rm`, `reef page ls|get|put|rm`, `reef
status`, `reef accept`, `reef check`, `reef next`, `reef queue add|move|rm|ls`, `reef start` and
`reef work`, which record and show where a node's work happens, `reef node capture`, which keeps
a node's changes in its worktree as a revision, and `reef import` and `reef export`, which carry a
book's nodes and pages in and out. `reef export --replace` writes
a plan back over the book it lives in, keeping the book's files, summary and `book.toml`. They call
the address `[api] address` names and say so when nothing answers there.

## Building

It needs shoal's `ScanSupport` checkout at `../../shoal` and glommio at `../../glommio`, beside
reef. `.cargo/config.toml` sets `-Ctarget-cpu=native` for the host alone, which gxhash wants; never
export it in `RUSTFLAGS`, where a wasm build would get it too.

- **`embedded`** links shoal's engine into reef-service, which needs nightly.
- **Without it**, reef-service is shoal's client half alone and builds on stable.

```sh
cargo +nightly test --workspace         # everything, with the engine (reef-shoal turns `embedded` on)
cargo +nightly test -p reef-app --features embedded   # and the app's suite, which needs it named
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
- **Docs come from the plan's pages, over the project's working tree.** A stored node's links
  resolve from its repo's root, so a link to a doc or heading that isn't there refuses the write,
  and a node can take its why and what from a doc. A plan keeps markdown pages of its own, which
  lie over the repo: a page hides the repo's file at its path. A link that leaves the repo
  (``doc `../x.md` leaves the repo``) refuses it too, and no doc is read through a symlink that
  leads out of the repo.
- **A page edit is checked as a node write is** (`Service::edit_page`): taking out a page a node
  links to, or the heading it names, or the section its why came from, is refused.
- **A link can name a node.** `node:C28`, or `node:M6#rows` at a heading of its body, in a page
  or in any of a node's markdown, opens that node in the sidebar (its anchor carries
  `data-goto`, which both viewers follow). One naming a node the plan lacks is a problem, so a
  node can't be removed while a page links it. `reef import` writes each link into a book's
  node file so, naming the node that heads the file, and `reef export` writes each back to the
  file its node lands in ([C68](../plan/src/milestones/m6.md)).
- **The check runs under the plan's write lock**, through `Store::put_checked` and
  `Store::write_plan_checked`, so two writes that are fine alone can't break the plan together.
  It builds the plan off the async threads, since its docs are files.
- **`check` and `payload`** give a plan's problems and the viewer's payload. The payload's doc
  links are absolute paths under the plan's `docs/` (`reef_api::paths::docs`), where `doc_page`
  renders each doc as the page mdbook would write for it: a `<main>` holding the doc, each
  heading under mdbook's id, so the viewer's *Read here* cuts its section out unchanged. Only
  markdown is served, so an image a doc shows doesn't load there.
- **Each applied write is a change.** A project registered, a plan listed, a batch put, a page
  edited or a whole plan written goes to every subscriber of `Service::changes`, after the write's lock is let go,
  so two writers' changes to one plan can arrive in either order; each put names its node's new
  version. `Service::end_changes` closes every subscriber, as a server does before it stops.

- **A node's work record comes from its checkout** (`Service::start_work`). `reef start` reads
  the worktree it runs in through gix, writes its worktree as a tree through git's CLI, pins the
  start under `refs/reef/start/<plan>/<id>`, and sends what it read. The service keeps it, a
  node only if it is in its plan, and reads no checkout itself. A capture's SHAs
  (`Service::add_capture`) join the start whose pinned commit they name.
- **A node's changes come from its checkout too.** `reef node capture` diffs the node's start,
  carried over what its base branch brought in, to its worktree as a seal takes it: tracked
  files as they are, untracked ones only where the base has them. gix takes every step but the
  snapshot. The service keeps each capture as the node's next revision (`Service::add_capture`),
  unless its latest already holds it, and tells the change stream of it (`revision-captured`).
- **Comments are threads on a node's work** (`Service::start_thread`): on a line of one of its
  revisions, which must hold that line, or on the node as a whole. Replies follow in a thread
  (`Service::reply`), and a thread is marked open or resolved over the version its writer read
  (`Service::mark_thread`). A comment's body is markdown, read back rendered with raw HTML
  escaped, and each comment and mark is a change (`comment-added`, `thread-marked`).

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
| `GET …/plans/{plan}/docs/{*path}` | a doc as its HTML page (`docs/design.html` for `docs/design.md`), the plan's page or the repo's file, or 404 |
| `GET …/plans/{plan}/pages` | its own pages: a `PageListing` |
| `POST …/plans/{plan}/pages` `PageEdit` | `PageEdited`: 200, 409 if stale, 422 if it would add a problem |
| `GET …/plans/{plan}/pages/{*path}` | the `StoredPage`, or 404 |
| `GET /api/v1/projects/{project}/work` | every `WorkRecord` of its nodes, by node |
| `GET …/plans/{plan}/work/{id}` | the node's `WorkRecord`, or 404 if it wasn't started |
| `POST …/plans/{plan}/work/{id}` `StartWork` | `Started`: 200, 409 if stale, 422 if the node is not in its plan |
| `POST …/plans/{plan}/work/{id}/captures` `AddCapture` | `Captured`: 200 if kept or unchanged, 409 if started again since, 422 if not started |
| `GET …/plans/{plan}/work/{id}/revisions` | every `Revision` of the node, oldest first, its files without their hunks |
| `GET …/plans/{plan}/work/{id}/revisions/{n}` | the `Revision`, or 404 |
| `GET …/plans/{plan}/work/{id}/revisions/{n}/files/{*path}` | the `FileDiff`, its hunks and lines, or 404 if the revision doesn't list it |
| `GET …/plans/{plan}/comments` | every `Thread` of the plan, by number, its comments rendered |
| `GET …/plans/{plan}/nodes/{id}/comments` | every `Thread` on the node, by number |
| `POST …/plans/{plan}/nodes/{id}/comments` `NewThread` | `Commented`: 200, 422 if the node is not in its plan or the line not in its revisions |
| `GET …/plans/{plan}/comments/{n}` | the `Thread`, or 404 |
| `POST …/plans/{plan}/comments/{n}/replies` `NewReply` | `Commented`: 200, or 422 if no thread starts at `n` |
| `PUT …/plans/{plan}/comments/{n}/state` `MarkThread` | `Marked`: 200, 409 if stale, 422 if no thread starts at `n` |
| `GET /api/v1/changes?project=&plan=` | server-sent events, one a `Change` |

- **A refusal is an answer**, its outcome the body whatever the status, such as
  ``{"refused":[{"source":"reef/central/C11","line":null,"message":"needs `X`, which is not a node"}]}``.
- **An error is an `ApiError`**, `{"code":…,"message":…}`: `not-registered`, `not-listed` and
  `not-found` are 404, `bad-name` and `bad-request` 400, and `internal` 500. Paths a client sends
  (`repo`, `dir`) must be absolute directories on the service's host.
- **A whole plan or a page is a big body.** `PUT …/revision` and `POST …/pages` take up to
  48 MiB, past axum's default of 2 MiB, which the example book's pages alone exceed.
- **The change stream** sends each change as an event named by its kind (`project-added`,
  `plan-added`, `nodes-put`, `pages-put`, `plan-written`, `queue-changed`, `work-recorded`), its
  data the change as JSON. It is live only: a
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

## Running it for real

reef's own plan lives in a reef-serve that runs as a systemd user unit, its data in the default
`~/.local/share/reef/shoal`, with no config file. `../plan` is the plan's export, which git keeps:
it is the copy a fresh data directory is imported from, and the book `mdbook-reef` still builds.

```sh
cargo install --locked --path ../crates/reef --target-dir ../target          # the terminal's client
cargo +nightly install --locked --path reef-service --features embedded --bin reef-serve --target-dir target
```

`~/.config/systemd/user/reef.service`:

```ini
[Unit]
Description=reef's central planner

[Service]
ExecStart=%h/.cargo/bin/reef-serve
Restart=on-failure
TimeoutStopSec=10

[Install]
WantedBy=default.target
```

A locked-memory limit of at least 30 MiB a shard, and 30 more, keeps glommio's registered io
buffers, so a session whose `ulimit -l` is lower needs `LimitMEMLOCK=` in the unit too.

```sh
systemctl --user daemon-reload && systemctl --user enable --now reef
journalctl --user -u reef                  # what it said starting, and any warning
reef project add reef                      # from the repo's root, once
reef import plan --plan central            # the plan, from its book
```

- **An edit goes through reef, then out to the book.** `reef node get|put`, `reef status`,
  `reef accept` and `reef page get|put` change the plan, each checked. `reef export plan --replace`
  then writes it back over `../plan`, only the files whose text changed, and the book is committed
  in the same change. `plan/` is never edited by hand: the next export writes over it.
- **A node's body goes through reef too.** `reef node get <ID> --md` prints a node as a node
  file, its `# version:` line inside the frontmatter, and `reef node put` of that file writes its
  body as well. YAML alone keeps the body stored. A body's links lead from the repo's root, as
  the plan keeps them ([C69](../plan/src/milestones/m6.md)).
- **A new binary is a reinstall and a restart.** Install both again, then
  `systemctl --user restart reef`. Stop the unit before running reef-app over the same data. A
  `reef` newer than the service it calls says the service has no route for what it asked.
- **A changed row needs a fresh data directory**, since shoal can't migrate a table: stop the
  unit, move `~/.local/share/reef/shoal` aside, start it, then `reef project add reef` and
  `reef import plan --plan central` again. Export first if the book is behind. A table added
  needed none when C30 added `WorkRow`: its binary opened a data directory the one before it
  wrote, read the imported plan back, and kept work records beside it. C32's two tables and
  C34's `CommentRow` needed none either.

## The app

`reef-app` is a Dioxus 0.7.10 fullstack app. Its front page lists every project with its plans,
and each plan opens at `/plans/{project}/{plan}`, drawn by reef-view linked into the client's
wasm, with the node the viewer selects in a panel beside it that edits its YAML. A plan's page
follows the change stream, so each put shows up live. Its server functions sit under `/app/`,
each a call on the service, and its server mounts `api::router` beside them, so one port serves
the pages and the JSON API alike.

- **The editor saves as `reef node put` does,** through `reef_api::puts_for`, at the version it
  read: another writer's change since comes back stale with the newer version, and a save that
  would add a problem comes back with check's messages.

- **It listens where the CLI looks:** `[api] address`, loopback only, so it takes reef-serve's
  place. Only one process can open shoal's data directory, so run one or the other. Under
  `dx serve`, dx names the server's address and proxies its own port to it; point `[api] address`
  at dx's port to reach it from the CLI.
- **It is served by its own `axum::serve`, not `dioxus::serve`,** which would listen on `PORT`
  or 8080. It stops on SIGINT or SIGTERM, ending the change stream first.
- **`embedded` goes to the server build alone,** since dx keeps default features on in the wasm
  build too: `@server --platform server --features embedded`. `tests/tree.rs` checks the wasm
  client's tree holds no shoal, glommio or reef-service.
- **dx passes an exported `RUSTFLAGS` to the wasm build,** where `-Ctarget-cpu=native` leaves
  wasm32 without reference types, so unset it first.

```sh
cd reef-app
env -u RUSTFLAGS dx serve @server --platform server --features embedded     # develop
env -u RUSTFLAGS dx build @server --platform server --features embedded     # build
REEF_CONFIG=reef.toml ../target/dx/reef-app/debug/web/server                 # serve the build
cargo +nightly test -p reef-app --features embedded                          # the suite
python check.py                                                              # the browser checks
```

`tests/app.rs` serves the app's router over shoal in-process and puts plans through the CLI.
`check.py` drives the built server in headless Chromium with Playwright: it starts the server
itself, on a config and data directory of its own under `target/tmp`, and stops its process
group at the end.

## The store

`Store<B: Backend>` reads and writes plans, their nodes and their pages by version. Its logic (each plan's lock,
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
- **A plan's pages are a revision of their own.** A whole-plan write that carries pages writes
  them under its new revision and makes them current in the same flip, so an import lands whole.
  The plan's row names the pages' revision apart from its nodes', so a batch of node writes
  leaves the pages where they are rather than copying them, and each table is pruned only of
  the revisions it doesn't read. A page edit (`Store::edit_page_checked`) is one row's
  conditional write in the pages' revision. A plan's pages hold at most 32 MiB, and a bulk write
  is sent in bundles of at most 256 rows and 16 MiB, under shoal's 64 MiB frame.
- **A project's queue is one row an entry** (`Store::edit_queue`), sorted by a rank string
  (`rank`), so an edit writes one row. A move writes its new row, then deletes its old one; of a
  node's two rows the later write wins, and the next edit prunes the other. A rank grown past 32
  characters spreads the queue anew, an entry at a time, last first.
- **A node's work record is one row** (`Store::start_work`), in its project's partition by
  `plan/id`, so the node a worktree is on is the one started there last, across the project's
  plans. A start is written where no record is, or over the version of the one it replaces, and
  is later than every start before it. A capture's SHAs (`Store::add_capture`) join the start
  its pinned commit names, and no other.
- **A node's revisions are a row each, and their files a row each** (`Store::revisions`), the
  files written first and the revision `if_absent` after them, so a revision that reads has
  every file it lists. They count from 1 for the node, through restarts. A file whose row would
  pass 32 MiB is kept without its hunks.
- **A plan's comments are a row each** (`Store::threads`), in the plan's partition by number,
  so a node's threads and the plan's are each one read. Every comment, a reply included, takes
  the number after the plan's last, through restarts. A thread's first comment holds its target
  and state, and a mark writes it again by version; a reply is a row of its own.

## The schema

| Table | Partition | Sort | Holds |
|---|---|---|---|
| `ProjectRow` | the constant `"projects"` | name | the repo's path |
| `PlanRow` | project | plan name | `current`, `pending` and `retired` revisions, the pages' `pages_current` and `pages_retired`, and a `version` |
| `NodeRow` | (project, plan, revision) | id | a `version`, status, group, written order, the spec as JSON, the body |
| `PageRow` | (project, plan, revision) | path | a `version`, and the page's markdown |
| `QueueRow` | project | rank | the plan and id of the node queued, and the `seq` it was written at |
| `WorkRow` | project | `plan/id` | a `version`, and the node's work record as JSON: its worktree, branch, base branch, start, start tree, fork and pin, and each capture's SHAs |
| `RevisionRow` | (project, `plan/id`) | number, padded | a `version`, and the revision as JSON: its SHAs, conflicts and files without their hunks |
| `RevisionFileRow` | (project, `plan/id`, number) | path | the file's diff as JSON, its hunks and lines |
| `CommentRow` | (project, plan) | number, padded | a `version`, and the comment as JSON: its node, time and body, and a thread's target and state or a reply's thread |

`schema::tests::fingerprint_is_pinned` pins it. Shoal can't migrate a table yet, so a change to a
row needs a fresh data directory, and then a new pin.
