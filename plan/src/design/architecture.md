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
| `reef-render` | stable | markdown rendering, doc links, why/what extraction over a `DocSource` ([C1](#a-model-any-source-can-feed)); a book's node files read, and the links in a node moved to another file ([C26](../milestones/m6.md)) |
| `reef-view` | stable, wasm | the viewer, plus a `mount` entry point ([C12](app.md#the-viewer-inside-the-app)) |
| `mdbook-reef` | stable | the static mode, now over `reef-render`'s filesystem source |
| `reef-api` | stable | the service's data types, and the JSON API's requests, answers, paths and changes ([C53](../milestones/m3.md)) |
| `reef` | stable | the CLI, the MCP server and the hooks, all HTTP clients of the JSON API, and the only code that reads a node's checkout ([C30](../milestones/m7.md)) |
| `reef-service` | nightly; stable built external-only | the store and the model: the only code that touches shoal, in-process or external ([C57](../milestones/m2.md)); the JSON API's router, and `reef-serve`, which serves it alone |
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
  in-memory one. The service reads a project's repo through `RepoSource`, which no link may
  leave ([C14](app.md#docs-in-central-mode)), with the plan's own pages `Layered` over it
  ([C28](../milestones/m6.md)).
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
- **Docs come from the project's working tree, or its fetched commit.** A stored node's links
  resolve from its repo's root, where import puts them ([C26](../milestones/m6.md)), so a link to a missing
  doc or heading is refused, and a why or a what can still come from a doc. A project registered
  by its git URL ([C49](../milestones/m10.md)) reads them from the service's bare clone at the commit last
  fetched, through gix, so every read of one moment sees one commit, and a fetch lands whole.
- **Two front doors onto it.** A plain JSON API on axum (typed in `reef-api`) for the CLI, the
  MCP server and the hooks; Dioxus server functions for the app. Both run in one axum server,
  and the CLI never depends on the app's wire format.
- **The service's data types are the API's.** A node as read, written and put, the outcome of a
  put or a whole-plan write, a project and a plan live in `reef-api` with serde, and reef-service
  re-exports them, so the service, the JSON API and the app's server functions share one set.
  A change to one of them changes the wire.
- **Changes are server-sent events,** live only: a client reads what it needs, then follows the
  stream, and one that falls behind is told how many changes it missed.
- **Each new kind of data adds three things:** an operation on the service, a JSON route and a
  server function. This holds for the queue, work records, comments and the loop's state.
- **Signed in when reef's config asks, plain HTTP on localhost, and TLS beside it.** One layer
  signs callers in for both front doors ([Users and sign-in](#users-and-sign-in)), and refuses
  writes from other origins. The loopback port serves plain HTTP to this host, and other hosts
  are served over TLS on a listener beside it ([Served beyond localhost](#served-beyond-localhost)).

## Users and sign-in

Beyond one machine, reef can't tell who is editing, and anyone who reaches its port can read and
write every plan ([C48](../milestones/m10.md)). Users sign in, and one axum layer guards both front doors. It
comes first in [M10](../milestones/m10.md), since an author on every write ([C50](../milestones/m10.md)) and serving beyond
localhost ([C51](../milestones/m10.md)) both build on it.

### Design choices

- **Off unless reef's config asks for it.** `[auth] required = true` turns sign-in on. Off, the
  default, every caller is answered as before, so a single user's server on localhost, reef's own
  among them, runs with no login at all.
- **One layer for both front doors.** `auth::guard` in reef-service wraps the JSON API in
  reef-serve, and reef-app's whole router: its pages, its server functions, the JSON API it
  mounts and Dioxus's static files. It finds who is calling, from `Authorization: Bearer` or the
  sign-in cookie, and hands every route a `Caller`, which the app's server functions read through
  Dioxus's request context.
- **Everything is guarded but signing in.** With sign-in on, a caller who isn't signed in reaches
  only the sign-in page and its form, the token route, `/api/v1/me`, the users' routes, which
  judge their callers themselves, and the app's stylesheet and static files. A page a browser
  opens is sent to `/sign-in?next=…`. The app's server functions, the JSON API, a fetched doc
  and the change stream are answered 401, with `WWW-Authenticate: Bearer`.
- **Users in shoal, passwords as argon2id hashes.** A user is a row under their name, and a token
  a row under its SHA-256 ([Users and tokens](store.md#users-and-tokens)). Hashing runs off the
  async threads, two at a time, since each takes 19 MiB and a core.
- **A password is traded for a token once.** `reef login` asks for the password on the terminal
  with echo off, or reads `--password-file`, and keeps only the token the service issues, 32
  random bytes shown once. The CLI, the MCP server, the hooks and a runner's sessions send it as
  `Authorization: Bearer`, read again for each request, so a process started before the login
  sends it too.
- **Tokens kept a server each, in a file of the user's own.**
  `$XDG_CONFIG_HOME/reef/credentials.toml`, readable by its owner alone, keyed by the server's
  root. `--credentials` names another file, `$REEF_TOKEN` gives a token outright, and
  `$REEF_CREDENTIALS` names a file, in that order, so a test or a script decides for itself.
- **A browser signs in on the service's own page.** `/sign-in` is plain HTML reef-service
  serves, so reef-serve has one too, under a policy that lets no script run. Its form sets an
  HttpOnly, SameSite=Strict cookie for 30 days, `Secure` when it is set over TLS, and goes on
  only to a path of this origin.
- **The first user comes from the service's own host,** over loopback, and is an admin. After
  that only an admin
  adds or lists users. This holds whether sign-in is on or off, so users can be added before it
  is turned on, and nobody can plant an admin while it is off.
- **A wrong name and a wrong password are told alike.** A name no user has is checked against a
  hash made for the purpose, so it takes as long, and both get the same words.
- **Tokens looked up are trusted for a minute,** in the service's memory, so a request costs no
  read of shoal. reef-service is shoal's only writer, so a revoke drops its token at once.
- **What sign-in leaves to later.** A cookie ignores ports, so a page on another loopback port is
  same-site and its posts carry the cookie: [C51](../milestones/m10.md)'s origin check refuses them
  ([Served beyond localhost](#served-beyond-localhost)). Changing a password, removing a user,
  listing and revoking tokens and slowing repeated failures came with [C77](../milestones/m10.md)
  ([Managing users and tokens](#managing-users-and-tokens)), and every write's author came with
  [C50](../milestones/m10.md) ([Who did what](#who-did-what)).

### What C48 found

- **Thirteen tables overflow a shard's stack in a debug build.** glommio starts each of shoal's
  shards on a thread with std's default 2 MiB stack, and adding `UserRow` and `TokenRow` pushed an
  unoptimised shard past it, so every embedded test aborted. The servers set `RUST_MIN_STACK` to
  8 MiB first thing in `main`, before any thread starts, and `central/.cargo/config.toml` does
  the same for `cargo test` and `cargo run`.
- **No fresh data directory.** A data directory the binary before C48 wrote read back under
  C48's, which added the two tables beside it.
- **Server functions run while a page renders see the guard's caller,** since Dioxus's request
  context carries the request's extensions, so the bar says whom the server takes the browser for
  from the first paint.
- **The sign-in page's policy blocks a script's fetch,** as `default-src 'none'` should, so
  nothing injected into the page could call the API with its cookie.

## Served beyond localhost

reef bound loopback alone, spoke plain HTTP, and checked no request's origin, so another host
couldn't reach it, and any page a browser opened could post to it on localhost. Anyone who
reached it could also have it read or clone what was on its host ([C51](../milestones/m10.md)).

### Design choices

- **Plain HTTP on loopback, TLS beside it.** `[api] address` stays plain HTTP and loopback only,
  for the service's own host: its CLI, hooks and MCP server, and its first admin. `[api.tls]`
  adds a second listener on any address that speaks TLS with the PEM chain and key it names,
  through rustls over ring. reef's own unit, and every test that binds loopback, run as before.
- **TLS only with sign-in required.** Other hosts reaching a service that answers anyone would
  let anyone on the network write, so `tls::bind` refuses `[api.tls]` unless `[auth] required`
  is set.
- **One router on both listeners, each request knowing its peer.** `api::serve_router` serves
  it on both, under one stop, for reef-serve and reef-app alike. Every request carries a `Peer`:
  the address it came from, so the first user is still added from a loopback peer alone, and
  whether it came over TLS, so the sign-in cookie is `Secure` there.
- **Handshakes run beside each other.** `TlsListener` hands axum a connection once its
  handshake is done, and runs each handshake as a task of its own, at most 128 at once, under
  10 s, so a client that opens a connection and says nothing never holds up the next.
- **Writes judged by their origin, with no token.** As Go's `CrossOriginProtection` judges
  them, every write a browser sends, with sign-in on or off: a `Sec-Fetch-Site` of `same-origin`
  or `none` goes on, and any other is refused 403; without one, an `Origin` must name the
  scheme the connection speaks and the host and port the request was sent to. A request with
  neither is no browser's, as the CLI's, and goes on, and so does every read, which no page of
  another origin can read the answer to without CORS. One check in the guard covers the JSON
  API, the app's server functions, whose decoder reads a body of any type, and the sign-in form.
- **The CLI calls a root.** `[api] url` names the service on another host, and wins over
  `address`, and `[api] ca` a PEM root its certificate is trusted by beside the system's. A
  login is kept by the root, one spelling of it. The service accepts both keys and leaves them
  be, so one file serves both.
- **What is on the service's host is an admin's.** With sign-in required, registering a project
  by a directory, or by a remote that names a path on the host, `file://` or bare, takes an
  admin, judged before the path is looked at. `[remotes] hosts`, if set, names the only hosts a
  remote may be cloned from, for every caller, so nobody points the service's git at its
  internal network.
- **What it leaves to later.** A renewed certificate is read at the next restart. There is no
  HSTS, since the loopback port is plain HTTP on a name a browser may share, and no reading of a
  reverse proxy's forwarded headers. With sign-in off, a page whose name a DNS rebinding points
  at loopback is the plain port's own origin, which [C80](../milestones/m10.md) closes.

### What C51 found

- **Tried live on a scratch `reef-serve`,** TLS on every interface beside a loopback port, over a
  data directory of its own:
  - the first user, asked for over TLS from the host's LAN address, was refused as not from its
    host, and added over loopback;
  - a `reef` on `[api] url` by the host's name signed in, registered a project, put and read a
    node, and kept its login by the `https://` root;
  - a put naming another origin, another port or plain `http` was refused 403, and one naming the
    service's own origin, or none, was applied;
  - a user who isn't an admin was refused a directory, `/etc` and a `file://` remote, and a
    remote off `[remotes] hosts` was refused for an admin too.
- **A self-signed certificate must be a leaf.** `openssl req -x509` marks its certificate a CA,
  which rustls refuses as a server's own (`CaUsedAsEndEntity`); one made with
  `basicConstraints=critical,CA:FALSE` is served.
- **The app's own writes pass the check unchanged.** Every browser check of the app's drags,
  saves, comments and sessions passed with it on, since Chromium says `Sec-Fetch-Site:
  same-origin` for each, and a form a page on another loopback port posts, the cookie and all,
  is answered 403.
- **A refused certificate is a connect error too,** so the CLI says the service isn't running
  only when the connection was refused, and gives every cause otherwise.
- **No fresh data directory.** C51 adds no table and changes no row, so the schema is as it was,
  and the scratch server's plan read back across its restarts.

## Managing users and tokens

C48 gave reef users and tokens, but a password couldn't be changed, a user couldn't be removed, a
token could be revoked only by its own logout, a sole admin who lost their password couldn't be got
back, an open change stream outlived its token, and nothing slowed a guesser ([C77](../milestones/m10.md)).

### Design choices

- **A token is named by an id cut from its hash,** the first 16 hex digits of its SHA-256, so a
  listing shows no token and nothing a token could be rebuilt from. Everyone lists their own
  tokens, the one they sent marked, and revokes any of them; an admin lists and revokes anyone's.
  An id the caller may not see is answered as no token at all, 404.
- **Your own password needs your current one, and leaves your tokens,** so hooks and other
  machines keep working, and `reef token revoke --others` is there if it leaked. An admin setting
  another user's needs no current one, and revokes every token of theirs, since whoever held the
  old password shouldn't stay signed in.
- **An admin removes a user, never the last admin.** The user's row goes under the users' lock,
  over its version, then every token of theirs. A token issued before its user was added signs
  nobody in, so a new user given an old name inherits nothing, even from a removal whose token
  deletes stopped partway ([Users and tokens](store.md#users-and-tokens)).
- **A revoke reaches whatever trusted the token.** Each revoke, removal and admin reset drops the
  user's tokens from the minute-long lookup cache, before and after the rows go, and bumps a
  counter every open change stream watches, so a stream opened with a token that signs nobody in
  ends at once, and its browser's reconnect is refused. Each stream also looks every minute, for a
  browser token's expiry.
- **Failures slowed by name and by address, before any hash.** A name may fail 5 times and an
  address 20, an IPv6 one by its /64, which one host can fill; then each try waits, from a
  second, doubling to 5 minutes, answered 429 with `Retry-After`, its password unchecked even when
  right. Each try counts as a failure as it is let through, and a success takes it back, so a
  burst sent at once is judged as if one came after another. A success clears the name, failures
  lapse an hour after the last, and at most 4096 keys of each kind are kept, the lapsed and then
  the oldest going first. The token route, the sign-in form and your own password change share it.
  Slowing rather than locking out means a guesser can't keep a user out for long.
- **A lost admin is recovered from the store, never over the API.** `reef-serve recover NAME`
  opens the store as the service does, asks for a new password twice with echo off, sets it, makes
  the user an admin and revokes their tokens, or adds them as an admin if no user has the name.
  Whoever can open the store holds the data already; a loopback caller may not, since any user of
  the host reaches loopback. An in-process pool locks the data directory, so the service is
  stopped first, and the command says so when it isn't. With shoal on a node of its own it runs
  beside the service, which then trusts a revoked token for up to a minute.
- **One prompt for both binaries.** Asking for a password on the terminal with echo off moved from
  the CLI into `reef_api::prompt`, behind a `prompt` feature the browser's build never turns on.
- **The app's account page** ([The shell](app.md#the-shell)) holds your password, your tokens
  and, for an admin, every user, through server functions that call the service's own methods, as
  the JSON API's routes do.

### What C77 found

- **Tried live on a scratch `reef-serve`,** sign-in required, over a data directory the binary
  before C77 wrote, which read back unchanged, since C77 adds no table and changes no row:
  - a wrong current password was refused and the right one changed ada's, her token still
    signing in; a second login and `reef token revoke --others` left one token;
  - bob's change stream, opened with his token, ended 16 ms after an admin revoked it, and his put
    was refused;
  - five wrong logins for bob, then the right password, answered 429 with `Retry-After: 1`;
  - an admin removed bob, and the last admin stayed;
  - `reef-serve recover ada`, run while the service held the data, said to stop it first; run
    after, it set ada's password, made her an admin and revoked her token, and a name nobody had
    was added as an admin.
- **shoal's lock error reaches the binary by its name.** The pool passes shoal's start error on in
  its `Debug` form, so `recover` knows a held data directory by `StorageDirectoryLocked`, as well
  as by shoal's words for it.
- **A page's buttons do nothing before its script runs,** so the browser check clicks until a
  field opens before it types. No password field sits in a `form`, so an early submit sends
  nothing.

## Who did what

With several users, edits, comments and work records said nothing of whose they were
([C50](../milestones/m10.md)). Each now carries its author, and the app shows it.

### Design choices

- **The author comes from the token, never the body.** The guard's `Caller` names the user its
  token or cookie signs in, and `Caller::author` hands that to every write the JSON API and the
  app's server functions make. With sign-in off, a token sent still names its user, and a write
  with none names nobody.
- **Claude says so, for its user.** Claude writes with the user's own token, so reef's MCP
  server sends `reef-writer: claude` on every request, and its writes read "Claude, for ada". The
  header can only add that to the token's user, never name another. Hooks and the terminal's own
  commands are the user's.
- **An author keeps when its user was added,** beside the name, so a name reused after its user
  was removed can be told from the one it named before, though nothing shows it yet.
- **Records with JSON keep it there; the rest keep a history.** Comments, thread and review
  marks, resolutions, work starts, captures, revisions and decisions name their author in their
  records, so no row changed. A node's, a page's and a queue's rows have no record, so each of
  their edits is a row of a new table ([History](store.md#history)), and their histories are
  read at `…/nodes/{id}/history`, `…/page-history/{*path}` and `…/queue/history`.
- **A history is best-effort.** An entry is kept once its write lands, and one that fails to keep
  leaves the write standing with a gap, rather than refusing a write that happened.
- **Shown where each thing is.** A comment's head names its writer, a thread's foot who marked
  it, the Changes tab who captured a revision, a decision who asked it, a queue entry who placed
  it, and a node's History tab every edit of it. The CLI prints the same with `reef node log`,
  `reef page log`, `reef queue log`, `reef comment ls` and `reef queue ls`.

### What C50 found

- **No fresh data directory.** `EditRow` joined the schema last, and every other table's
  fingerprint stayed as it was.
- **A store opened again starts its clock over,** so an edit within the millisecond of the last
  one before a restart could take its key. Each store's keys end with a number it draws when it
  opens, so it can't.

## The CLI

The terminal is where Claude and the user already work, and Claude's loop must not wait for the
app.

### Design choices

- **`reef` is stable and small:** an HTTP client of the JSON API, with no shoal and no Dioxus in
  its tree.
- **It takes the same YAML as frontmatter.** `reef node put` reads a `NodeSpec`, and
  `reef node get` prints one, so the skill's rules and the reference carry over unchanged.
- **A got node's version rides in a comment.** `get` prints `# version: N` before the YAML,
  which YAML skips, and `put` reads it back as the version its writer read. So a put made after
  another writer's is refused as stale, and YAML with no version line is a new node, refused if
  its id is taken rather than written over it. A flag would be dropped between the get and the
  put; the comment travels with the text Claude edits.
- **A group goes in with its members, and `rm` removes only what it names.** A spec's inline
  `nodes:` are lifted out and put as one batch with it, each a new node in its group. `rm`
  takes several ids as one checked batch, and never removes a node it wasn't given: a group's
  members are named with it, and a node another needs goes once nothing needs it.
- **A status or a criterion is a put of its node.** `reef status` and `reef accept` read the
  node, change the one key and put it back at the version read, so they are refused as any put
  is, stale or breaking. A group's status is never set, since its members decide it, and nor is
  a performance criterion's with a target, which its metric decides. Criteria are numbered
  from 1, as check's messages number them.
- **`check` and `next` print what `mdbook-reef` prints.** The service builds the plan as check
  does, and `next`'s report is `reef_model::Next`, which both tools print, so the skill reads
  the same words in either mode.
- **The working directory picks the project:** the registered project whose repo holds it. A
  linked worktree picks its main repo's project (`git rev-parse --git-common-dir`).
  `--project` overrides. A node's plan is the project's only plan; with several, `--plan`
  names one, and without it the command lists them and stops. `next` is the exception: it
  spans the project's plans unless `--plan` narrows it.
- **The queue is the project's** ([C24](../milestones/m5.md)). `reef queue add|move|rm` takes a
  bare id in the picked plan or a full `project/plan/id`, and a node to go beside as a bare id
  in that entry's plan or a full name. Adding a queued node moves it, to the bottom unless
  `--top`, `--bottom`, `--before` or `--after` says where; a move needs one. `ls` lists each
  entry ready, waiting with the needs it waits on, in progress, blocked, done or missing.
  `next` follows the queue across the project's plans, naming nodes in full when it reads
  several, and `--plan` narrows it, leaving other plans' entries out.
- **It reads the service's own config, for `[api]`'s `address`, `url` and `ca` alone.**
  `reef_api::config` finds the file for both (`--conf`, else `$REEF_CONFIG`, else the XDG path),
  so they meet at the address one file names; on another host, `url` names the service's root
  over TLS ([Served beyond localhost](#served-beyond-localhost)). The CLI ignores every other key;
  the service checks them.
- **It says so and stops when the service isn't running.** The message names the root, the key
  that set it, and `reef-serve`, for a connection refused alone; a certificate refused, or any
  other failure, is said with every cause. Only one process can open shoal's data directory, and a
  CLI that started a nightly server from another workspace would own a process nobody stops.
- **A library under a thin binary.** `reef::run` takes the command line, the working directory
  and where to print, so the service's tests drive the real commands against a served twin.
- **`reef start` reads the checkout it runs in** ([C30](../milestones/m7.md)). The checkout must be the
  picked project's repo or a linked worktree of it, and the node a piece of work in its plan.
  It sends the service what it read, since the service reads no checkout, and pins the start
  first, putting the pin back if the service refuses the record. A node is started once: a
  second start says where the first was, and `--restart` replaces it. A node to do goes in
  progress, as `reef status` would put it.
- **`reef work` shows a record**, of a node named or of the one the worktree is on: the one
  started there last, in any of the project's plans, as a capture finds it.
- **`reef node capture` reads the node's changes where it is worked** ([C32](../milestones/m7.md)). Without
  an id it captures the node the worktree is on, and a node named must be worked there. It
  takes the diff from the node's start, carried over what its base branch brought in, to the
  worktree as a seal takes it, through gix for every step but the snapshot, and sends every
  file with its hunks for the service to keep as a revision. Only the local checkout is read.
- **Import and export are the CLI's** ([C26, C27](../milestones/m6.md)). `reef import` reads
  and checks the book where it is, moves each link to lead from the repo's root, and sends the
  service one whole-plan write, so the service never reads or writes a book. A plan that has
  nodes is replaced only with `--replace`, since the write drops every node the book doesn't
  have. `reef export` lists the plan and writes a new book whose files sort in its written
  order, so importing it gives the plan back.
