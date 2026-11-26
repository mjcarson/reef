# The app

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
- **Over TLS it speaks HTTP/2, else HTTP/1.1.** A plan's page holds three change streams open,
  the plan's, its queue's and its runners', and over HTTP/1.1 a browser keeps six connections
  to a host across all its tabs, so two tabs leave every other call waiting. Over TLS, ALPN
  offers `h2` first, and a browser carries every tab's streams and calls on one connection, up
  to hyper's 200 streams at once. Browsers speak HTTP/2 only over TLS, so the plain loopback
  port stays HTTP/1.1, for the CLI, its hooks and one tab.
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
