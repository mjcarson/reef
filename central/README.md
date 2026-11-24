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

It needs shoal's `ScanSupport` checkout at `../../shoal` and glommio at `../../glommio`, beside
reef. `.cargo/config.toml` sets `-Ctarget-cpu=native` for the host alone, which gxhash wants; never
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
address = "127.0.0.1:12180"   # where the JSON API listens over plain HTTP, and `reef` calls it: loopback only

[api.tls]                     # other hosts, over TLS, beside it; needs `[auth] required`
address = "0.0.0.0:12443"     # any address
cert = "/etc/reef/fullchain.pem"   # the certificate chain, as PEM, the server's own first
key = "/etc/reef/privkey.pem"      # its private key, as PEM

[auth]
required = true               # every caller signs in; default false, which answers anyone

[remotes]
clones = "/home/me/.local/share/reef/clones"   # remote projects' clones; default $XDG_DATA_HOME/reef/clones
fetch_every = "5m"            # how often every remote project is fetched, in s, m or h; "0s" never
hosts = ["github.com"]        # the hosts a remote may be cloned from; default any
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
  leads out of the repo. A project registered by its git URL reads its docs from the service's
  clone at the commit last fetched instead ([Projects from remote
  repos](#projects-from-remote-repos)).
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
- **A line's thread follows its line** (`Store::follow_threads`). Each capture follows the
  node's threads to its latest revision before it is told, by comparing each two revisions'
  stored hunks, and a thread keeps where its line went in each. One whose line a revision
  deleted, edited or no longer shows is outdated from it on. The threads a capture moved are a
  change of their own (`threads-followed`).
- **A change resolves a thread** (`Store::resolve_by_trailers`). `reef node capture` reads the
  `Reef-Resolves: <n>` trailers on the node's commits past its start and the base branch, and
  each thread of the plan one names is resolved and linked to the revision the capture kept, or
  the latest it matched. A commit's change, by its author, author date and summary, resolves a
  thread once, so a rebase never resolves one the user reopened. `reef comment resolve <n>`
  links a revision by hand (`Service::resolve_thread`), over the version it read. Each thread
  written is `thread-marked`.
- **A terminal loop is kept by its pane** (`Service::edit_loop`): zellij's or tmux's pane, else
  the Claude Code process, `CLAUDE_PID`, which every `/clear` keeps. A loop feeds a registered
  project, or one listed plan of it, holds one node at a time, which no other loop may take,
  and keeps the node whose end it acted on and whether it is paused. Each session that starts
  in its pane takes it over, so a clear's new session keeps it. Each loop written is
  `loop-changed`, to its project.

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
| `POST /api/v1/projects` `NewProject` | `AddedProject`: 201, 409 if the name or repo is taken, or 502 if its `remote` can't be fetched |
| `GET /api/v1/find-project?dir=&remote=` | the `Project` a directory belongs to, or one of its checkout's remotes was registered by, or 404 |
| `POST /api/v1/projects/{project}/fetch` | `Fetched`: 200 whether its branch moved or not, 422 for a local project, or 502 if its remote can't be fetched |
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
| `POST …/plans/{plan}/work/{id}/captures` `AddCapture` | `Captured`: 200 if kept or unchanged, the node's threads followed to its latest revision and its trailers' threads resolved, 409 if started again since, 422 if not started |
| `GET …/plans/{plan}/work/{id}/revisions` | every `Revision` of the node, oldest first, its files without their hunks |
| `GET …/plans/{plan}/work/{id}/revisions/{n}` | the `Revision`, or 404 |
| `GET …/plans/{plan}/work/{id}/revisions/{n}/files/{*path}` | the `FileDiff`, its hunks and lines, or 404 if the revision doesn't list it |
| `GET …/plans/{plan}/comments` | every `Thread` of the plan, by number, its comments rendered, a line's with where it was `followed` and the revision it went `outdated` in |
| `GET …/plans/{plan}/nodes/{id}/comments` | every `Thread` on the node, by number |
| `POST …/plans/{plan}/nodes/{id}/comments` `NewThread` | `Commented`: 200, 422 if the node is not in its plan or the line not in its revisions |
| `GET …/plans/{plan}/comments/{n}` | the `Thread`, or 404 |
| `POST …/plans/{plan}/comments/{n}/replies` `NewReply` | `Commented`: 200, or 422 if no thread starts at `n` |
| `PUT …/plans/{plan}/comments/{n}/state` `MarkThread` | `Marked`: 200, 409 if stale, 422 if no thread starts at `n` |
| `POST …/plans/{plan}/comments/{n}/resolution` `ResolveThread` | `Resolved`: 200, 409 if stale, 422 if no thread starts at `n` or the project has no such revision |
| `GET /api/v1/loops` | every `Loop`, by pane |
| `POST /api/v1/loops` `EditLoop` | `LoopEdited`: 200 if written, 409 if the pane's loop feeds another project or plan or another loop holds the node, 422 if the pane has no loop or the node isn't the loop's or in its plan |
| `GET /api/v1/projects/{project}/loops` | every `Loop` feeding the project, by pane |
| `GET /api/v1/changes?project=&plan=` | server-sent events, one a `Change` |
| `POST /api/v1/tokens` `SignIn` | `IssuedToken`: 201, 401 if no user has that name and password, the same words for either, or 429 with `Retry-After` if too many failed lately |
| `GET /api/v1/tokens?user=` | every `TokenInfo` of the caller's, the one it sent marked `current`, or of the user an admin names; 401 or 403 otherwise |
| `DELETE /api/v1/tokens/current` | `SignedOut`: the token the caller sent revoked, or 401 if it sent none |
| `DELETE /api/v1/tokens/{id}` | `SignedOut`: the token revoked, or 404 unless it is the caller's, or any to an admin |
| `GET /api/v1/me` | `Me`: the user the caller's token signs in, and whether sign-in is required |
| `GET /api/v1/users` | every `User` to an admin, none while there are none, 401 or 403 to anyone else |
| `POST /api/v1/users` `NewUser` | `UserAdded`: 201, 409 if the name is taken, 401 or 403 unless an admin asks, or the first user is added from the service's own host |
| `DELETE /api/v1/users/{name}` | `UserRemoved`: 200 with how many tokens were revoked, 404 if no user has the name, 409 if they are the last admin, 401 or 403 unless an admin asks |
| `PUT /api/v1/users/{name}/password` `PasswordChange` | `PasswordChanged`: 200, the caller's own with its `current`, or another's by an admin, which revokes their tokens; 403 if `current` doesn't match, 404 if no user has the name, 429 if too many failed lately |
| `GET /sign-in?next=` | the browser's sign-in page, plain HTML that runs no script |
| `POST /sign-in` (a form) | the sign-in cookie set and 303 to `next`, a path of this origin, or the form again with a 401, or a 429 if too many failed lately |
| `POST /sign-out` | the cookie's token revoked, the cookie cleared, and 303 to `/sign-in` |

- **A refusal is an answer**, its outcome the body whatever the status, such as
  ``{"refused":[{"source":"reef/central/C11","line":null,"message":"needs `X`, which is not a node"}]}``.
- **An error is an `ApiError`**, `{"code":…,"message":…}`: `not-registered`, `not-listed` and
  `not-found` are 404, `bad-name` and `bad-request` 400, `unauthorized` 401, `forbidden` 403,
  `too-many-tries` 429, with `retry_after` in seconds, `internal` 500, and `fetch-failed` 502, a
  remote git couldn't fetch. A `repo` a client sends
  must be an absolute directory on the service's host. A `dir` must be absolute, and is matched
  against local projects only if the service's host has it, so a checkout on another host finds
  its project by its `remote`s alone.
- **A whole plan or a page is a big body.** `PUT …/revision` and `POST …/pages` take up to
  48 MiB, past axum's default of 2 MiB, which the example book's pages alone exceed.
- **The change stream** sends each change as an event named by its kind (`project-added`,
  `project-fetched`, `plan-added`, `nodes-put`, `pages-put`, `plan-written`, `queue-changed`,
  `work-recorded`), its data the change as JSON. It is live only: a
  client reads what it needs, then follows. One that falls behind by more than 256 changes is
  sent `missed`, with how many, and should read again. `plan` narrows it within a `project`, and
  a project's own changes reach it only if they concern every plan, as `project-fetched` does.
- **One layer signs callers in.** `api::serve` wraps the router in `auth::guard`, as reef-app
  wraps its whole router. It reads a token from `Authorization: Bearer`, else the `reef_sign_in`
  cookie, and hands each route its `Caller`. With `[auth] required` off, the default, it lets
  everything through. On, everything but signing in and the app's static files needs a user: a
  page a browser opens is sent to `/sign-in?next=…`, and anything else is answered 401 with
  `WWW-Authenticate: Bearer`. A change stream opened with a token ends once the token signs
  nobody in: revoked, expired, or its user removed or given a new password by an admin.
- **A write from a page of another origin is refused,** 403, with sign-in on or off, as Go's
  `CrossOriginProtection` judges it (`origin::judge`): a `Sec-Fetch-Site` other than
  `same-origin` or `none`, or without one, an `Origin` that isn't the scheme, host and port the
  request was sent to. A request with neither, as the CLI, the MCP server and the hooks send,
  goes on, and so does every read.
- **The first user comes from the service's own host**, over loopback, and is an admin; after
  that only an admin adds users, whether sign-in is required or not, so users can be added before
  it is turned on.
- **Plain HTTP on localhost, TLS beside it.** `api::bind` refuses any address but a loopback one
  for plain HTTP, and `tls::bind` adds a listener over TLS on any address where `[api.tls]` names
  one, refused unless sign-in is required. `api::serve_router` serves one router on both, and
  each request knows its `Peer`: where it came from, and whether over TLS.

`reef-serve` serves it alone, until reef-app mounts it. It listens before it opens shoal, so a
taken port, an address that isn't local or a certificate that can't be read stops it first, and
on SIGINT or SIGTERM it ends the change stream, waits for open requests and closes the service.

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
reef claude install                        # the MCP entry, the hooks and the skill, for the loop
```

- **An edit goes through reef, then out to the book.** `reef node get|put`, `reef status`,
  `reef accept` and `reef page get|put` change the plan, each checked. `reef export plan --replace`
  then writes it back over `../plan`, only the files whose text changed, and the book is committed
  in the same change. `plan/` is never edited by hand: the next export writes over it.
- **A node's body goes through reef too.** `reef node get <ID> --md` prints a node as a node
  file, its `# version:` line inside the frontmatter, and `reef node put` of that file writes its
  body as well. YAML alone keeps the body stored. A body's links lead from the repo's root, as
  the plan keeps them ([C69](../plan/src/milestones/m6.md)).
- **One command sets a repo up for the loop.** `reef claude install` writes `.mcp.json`, running
  `reef mcp`, and `.claude/settings.json`, running `reef hook …` on SessionStart, Stop, SessionEnd,
  PostToolUse after reef's status tool, WorktreeCreate and WorktreeRemove. Both are committed, so
  every worktree checks them out. `.claude/settings.local.json` enables the server, allows its tools
  and offers a clear at a plan's approval. It stays out of git, and WorktreeCreate copies it into
  each worktree it makes. git's post-commit and post-rewrite go where git runs hooks,
  `core.hooksPath`'s dir if set, a shell hook already there keeping its own lines. Then trusting the
  folder is the only prompt. Running it again changes nothing, so run it after a reinstall too.
- **The skill comes from the binary.** `reef skill`, and `reef claude install`, write
  `crates/reef/assets/skill` to `../.claude/skills/reef`, over the mdbook skill, so it names the
  tools and commands of the reef that wrote it. Run it again after a reinstall that changed the
  skill. `mdbook-reef install` and `mdbook-reef skill` write the mdbook skill there instead, so
  pass `--no-skill` to the first, and skip the second.
- **A new binary is a reinstall and a restart.** Install both again, then
  `systemctl --user restart reef`. Stop the unit before running reef-app over the same data. A
  `reef` newer than the service it calls says the service has no route for what it asked.
- **A changed row needs a fresh data directory**, since shoal can't migrate a table: stop the
  unit, move `~/.local/share/reef/shoal` aside, start it, then `reef project add reef` and
  `reef import plan --plan central` again. Export first if the book is behind. A table added
  needed none when C30 added `WorkRow`: its binary opened a data directory the one before it
  wrote, read the imported plan back, and kept work records beside it. C32's two tables and
  C34's `CommentRow` needed none either, nor did C37's resolutions, which a thread's row keeps
  only once it has one, so a row kept before reads back with none. C39's `LoopRow` moved the
  schema's pin but needed none: its binary read back the nodes and comments C37's wrote. Nor did
  C48's `UserRow` and `TokenRow`: its binary read back a plan the one before it wrote. Nor did
  C49's `RemoteRow`, which keeps a remote project's URL beside its `ProjectRow` rather than in
  it.

### Signing in

Sign-in is off unless reef's config asks for it, as reef's own unit runs with no config at all.
To turn it on, add `[auth] required = true` to the config the service reads, restart it, then:

```sh
reef user add ada                  # from the service's host: the first user, an admin
reef login --user ada              # its token, kept for every `reef` this user runs here
reef whoami                        # whom the service takes this `reef` for
reef user add bob                  # as an admin; `reef user ls` lists them
reef user passwd                   # your own password, with your current one
reef user passwd bob               # as an admin: bob's, which revokes his tokens
reef user rm bob                   # as an admin: bob and his tokens, never the last admin
reef token ls                      # your tokens, this reef's marked; `--user bob` as an admin
reef token revoke 3f9c0a1b2d4e5f60 # one by its id; `--others` every one but this reef's
reef logout                        # the token revoked at the service, and forgotten here
```

- **A password is asked for on the terminal** with echo off, never taken as an argument, or read
  from `--password-file PATH` (`-` for standard input). The service keeps only its argon2id hash.
- **A login keeps a token, never the password,** in `$XDG_CONFIG_HOME/reef/credentials.toml`,
  readable by its owner alone, a token for each server by its root (`http://127.0.0.1:12180`,
  or the `https://` one `[api] url` names).
  Every request reads it again, so the MCP server, the hooks and a runner started before the
  login send it too. `--credentials PATH` names another file, `$REEF_TOKEN` gives a token
  outright, and `$REEF_CREDENTIALS` names a file, in that order, before the default.
- **A browser signs in at `/sign-in`,** which sets an HttpOnly, SameSite=Strict cookie for 30
  days, `Secure` when it signs in over TLS. The app's bar names whom it is signed in as, beside a
  Sign out, and the name opens `/account`: your password, your tokens, each revoked from there,
  and, for an admin, every user, whose password they set or whom they remove.
- **Repeated wrong passwords are slowed.** A name may fail 5 times, and an address 20, an IPv6
  one by its /64; then each try waits, from a second, doubling to 5 minutes, refused with 429
  before its password is checked. A success clears the name, and failures are forgotten an hour
  after the last. Your own password change is slowed the same way.
- **A lost admin is recovered on the service's host,** from the store itself, never over the
  API: stop the service, which holds its data directory, then

  ```sh
  systemctl --user stop reef
  reef-serve recover ada            # a new password, asked twice; ada made an admin
  systemctl --user start reef
  ```

  It revokes ada's tokens, and adds ada as an admin if no user has the name. `--password-file`
  reads the password from a file, and `--conf` names the service's config. With shoal on a node
  of its own, it runs while the service does, which then trusts a revoked token for up to a
  minute.
- **A refused `reef` says how to sign in**, naming the service's address and the file its token
  came from.

### Serving other hosts

The loopback port stays plain HTTP, for the service's own host. Other hosts reach the JSON API,
and the app, over TLS on a second listener, which sign-in must be required for:

```toml
[api]
address = "127.0.0.1:12180"         # this host's CLI, hooks and MCP server, as before

[api.tls]
address = "0.0.0.0:12443"
cert = "/etc/reef/fullchain.pem"
key = "/etc/reef/privkey.pem"

[auth]
required = true
```

The first admin is added on the service's host, over loopback, with `reef user add`. A `reef` on
another host names the service by its root, and a root to trust its certificate by if the system
doesn't already:

```toml
[api]
url = "https://reef.example:12443"   # wins over `address`
ca = "/home/me/.config/reef/ca.pem"  # beside the system's roots, and `$SSL_CERT_FILE`'s
```

- **The certificate is read as the server starts,** so a renewed one is served from the next
  restart. A chain or key that can't be read, or a key that isn't the certificate's, stops it,
  naming the key that set the file.
- **A `reef` refusing the certificate says so,** with every cause, rather than that the service
  isn't running.
- **A self-signed certificate must be a leaf.** `openssl req -x509` makes a CA unless told
  otherwise, and rustls refuses a CA as a server's own (`CaUsedAsEndEntity`), so add
  `-addext basicConstraints=critical,CA:FALSE`, or sign it with a CA of your own and name that
  CA as `ca`.
- **Only an admin registers what is on the service's host,** once sign-in is required: a
  directory (`--repo`), or a remote that names a path on it, `file://` or bare. A user who isn't
  one is refused before the path is looked at. `[remotes] hosts` limits the hosts a remote may
  be cloned from, for every caller.
- **Known limits:** no HSTS, since the loopback port is plain HTTP on a name the browser may
  share; no reading of a reverse proxy's `X-Forwarded-*`, so a proxy in front makes every caller
  look local; and on the plain port, with sign-in off, a page whose name a DNS rebinding points at
  loopback is the same origin as the service (C80).

### Projects from remote repos

A project can be registered by its git URL rather than a directory on the service's host, for a
service its users don't share a disk with. The service keeps a clone of one branch of it, and
reads that project's docs from its clone at the commit it last fetched. A node's changes still
come from the user's own checkout, as `reef start` and `reef node capture` read them there.

```sh
reef project add shoal --url git@github.com:org/shoal.git          # its default branch
reef project add shoal --url https://github.com/org/shoal --branch dev
reef project fetch                  # from a clone of it, or --project shoal from anywhere
reef project ls                     # shoal  git@github.com:org/shoal.git (`main` at 6376987)
```

- **The clone is bare, shallow and one branch**, made by `git init --bare` in `[remotes] clones`
  and filled by the fetch every later fetch runs: the branch's tip alone, over its own ref, so a
  force-push is followed. Its directory is named by its project and the moment it was made, so a
  registration retried after a failure never meets the clone that one left. It needs git 2.29.
- **Docs are read from git's objects at the commit the project's row keeps**, through gix, never
  a working tree. Every read of one moment sees one commit, and a fetch lands for readers only
  once its commit is written. A symlink is followed while it stays in the tree, and a submodule's
  files aren't read.
- **A fetch is asked for, or on an interval.** `reef project fetch` (`POST …/fetch`) fetches one
  project; `[remotes] fetch_every`, five minutes unless set, fetches every remote project, and
  the first round runs as the server starts. A branch that moved is `project-fetched`, which the
  app's plan page reloads on.
- **A link is checked against what was fetched.** A node linking a doc that isn't pushed and
  fetched yet is refused, so push, then `reef project fetch`, before linking a new doc.
- **A clone of the repo finds the project.** Every `reef` sends the URLs its checkout's remotes
  fetch from, users and passwords left out, and the service matches them by host and path,
  whatever their scheme, user, port, trailing `/` or `.git`. A directory in a local project's repo
  on the service's host finds that project first.
- **Git runs as the service's account.** A private remote is reached through that account's ssh
  keys or credential helpers. A URL with a password, a user over http(s), or a relative path is
  refused, as is any transport but file, git, ssh, http and https, since every caller sees a
  project's URL and `ext::` would run a command. Git never prompts, and stops after 5 minutes.
- **A path on the service's host is an admin's to name,** with sign-in required, and
  `[remotes] hosts`, if set, names the only hosts a remote may be cloned from.
- **Known limits:** a clone is never removed, since a project can't be; moving `[remotes] clones`
  leaves the paths projects keep behind; a doc kept in LFS reads as its pointer; and with sign-in
  off, anyone who can reach the loopback port can have the service clone any repo its account
  can read, or read any directory with `--repo`.

### Plans and choices kept

Each node keeps every plan its sessions asked approval for, with its verdict and feedback, and
every question with its choices and the answer picked. reef's Stop and SessionEnd hooks read them
from each session's transcript, whether it ran in a terminal or under `reef loop run`, so a repo
set up before SessionEnd was added needs `reef claude install` run again. `GET
/api/v1/projects/{project}/plans/{plan}/nodes/{id}/decisions` lists a node's.

### A loop the app runs

`reef loop run`, run from a repo's root, attaches this checkout to central reef as a runner.
Each node then gets a fresh `claude -p` session over stream-json, spawned here, on the user's own
machine. The service runs nothing: it keeps each runner's transcript and the prompts waiting on
it in memory, and the app's sessions panel shows them, starts, pauses and stops the runner's
loop, and answers its prompts.

```sh
reef loop run                                   # wait for the app to start the loop
reef loop run --start --plan central            # or start it now, narrowed to a plan
reef loop run --permission-mode auto -- --model sonnet --effort low
```

- **The repo is set up as for the terminal loop,** by `reef claude install`, since each session
  runs its `.mcp.json` and hooks. They find the runner's loop through `REEF_PANE`, which names
  its pane, `reef:<id>`. Nothing of the runner's own `CLAUDE*`, `ZELLIJ*` or `TMUX*` reaches them.
- **One node at a time.** A loop on a runner's pane is one node wide. Once nothing else is ready,
  the runner ends its session and waits for the app to start the loop again.
- **Every prompt waits for the app,** a tool's permission among them, since headless Claude Code
  starts in `default` mode unless `--permission-mode` says otherwise. Allow rules in the repo's
  settings still allow what they name.
- **Ctrl-C ends it:** its session ends, its loop lets go of its node, and the app shows it
  detached. A runner unheard from for a minute shows as gone. One whose service restarted
  attaches again with the transcript it holds.

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
- **The Changes tab shows a node's captured revisions,** the latest unless another is picked,
  and a file's diff once it is opened, fetched then and drawn as a table of its hunks. A
  capture the plan's stream announces reads the tab again (`live::WorkChanges`) without
  reading the plan again.
- **Comments are threads on a diff's lines or on the node.** A click on a line's numbers opens a
  box under it, and its threads show under it, in each revision their line was followed to.
  The Comments tab holds the node's threads, a box for a general one, and how many are open,
  and the toolbar counts and lists the plan's open threads, each named where its line is now
  and marked if it is outdated. A thread a change resolved names that revision and its commit,
  or what it was reopened after. The canvas reads the plan's threads once a change, for all
  three.

- **A node's Decisions tab** lists every plan its sessions asked approval for, with its verdict
  and feedback, and every question with the choice picked, each saying where and when it was
  answered, and follows the plan's stream for new ones.
- **The sessions panel** opens from the bar, whose button counts the live sessions and the
  prompts waiting. It lists each runner, its state and its loop's node, with buttons that start,
  pause and stop the loop, and a terminal's loops to read. The picked runner's transcript follows
  the project's stream, and each prompt it waits on is a card to answer, beside a box for a
  reply when Claude's turn ends mid-node. A question's card offers its choices and a box for the
  user's own words, and a plan's shows the plan to approve, or to reject with feedback that
  Claude plans again from.

- **It is signed in to as reef-serve is.** Its whole router, pages, server functions and the
  JSON API, sits under `auth::guard`, and `/app/me` reads the `Caller` the guard found, so the bar
  says whom the server takes the browser for, with a Sign out, once sign-in is required. A
  browser whose sign-in lapses finds its change stream closed and its calls answered 401, and
  is offered the sign-in page again, which brings it back where it was.
- **It listens where the CLI looks:** `[api] address`, loopback only, so it takes reef-serve's
  place, and over TLS beside it where `[api.tls]` says. Only one process can open shoal's data
  directory, so run one or the other. Under
  `dx serve`, dx names the server's address and proxies its own port to it; point `[api] address`
  at dx's port to reach it from the CLI.
- **It is served by `api::serve_router`, not `dioxus::serve`,** which would listen on `PORT`
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
env -u RUSTFLAGS dx build --release @server --platform server --features embedded   # timed
REEF_CONFIG=reef.toml ../target/dx/reef-app/debug/web/server                 # serve the build
cargo +nightly test -p reef-app --features embedded                          # the suite
python check.py                                                              # the browser checks
python check.py --release --only=c33,c35                                     # the diff's timing
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
  and state, and a mark writes it again by version; a reply is a row of its own. Where a line's
  thread was followed to sits on that row too, which a capture's follow writes by version, and
  so does each revision linked to it as one that answered it, which a resolve writes.
- **Every loop is a row of one partition** (`Store::edit_loop`), under its pane's key, so a
  pane keeps one loop. Every change reads them all under one lock and writes the one row, a
  start `if_absent` and anything else by version, so two loops taking one node at once are
  taken one at a time. A stop deletes the row by version.

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
| `CommentRow` | (project, plan) | number, padded | a `version`, and the comment as JSON: its node, time and body, and a thread's target, trail, state and resolutions or a reply's thread |
| `LoopRow` | the constant `"loops"` | the pane's key | a `version`, and the loop as JSON: its project, plan, session, node, the node it acted on, its state and when it started |
| `UserRow` | the constant `"users"` | name | a `version`, and the user as JSON: their password's argon2id hash, whether they are an admin, and when they were added |
| `TokenRow` | the constant `"tokens"` | the token's SHA-256 | a `version`, and what it signs in as JSON: its user, kind, label, and when it was issued and expires |

`schema::tests::fingerprint_is_pinned` pins it. Shoal can't migrate a table yet, so a change to a
row needs a fresh data directory, and then a new pin.
