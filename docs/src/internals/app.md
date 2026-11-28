# The app

`reef-app` is a Dioxus 0.7.10 fullstack app. Its front page explores every project's plans, each
summed up, and each plan opens at `/plans/{project}/{plan}`, drawn by reef-view linked into the client's
wasm, with the node the viewer selects in a panel beside it that edits its YAML. A plan's page
follows the change stream, so each put shows up live. Its server functions sit under `/app/`,
each a call on the service, and its server mounts `api::router` beside them, so one port serves
the pages and the JSON API alike.

- **The front page is a plan explorer** (C121, `explorer.rs`). `calls::explorer` reads every
  project's plans summed up, `Service::plan_summaries` once a project, on the server as the
  page renders, so it reads right before the script runs; its answer carries the server's clock,
  which the page's relative times count on from, so the server's render and the browser's
  agree. The page then follows one change stream that names no project
  (`live::use_explorer_changes`), since plain HTTP/1.1 opens six connections a host. Each change
  of a kind a summary shows, a put, a start, a loop, a session or a test run, is counted by the
  project its JSON names, and reads that project's plans alone again (`calls::plan_summaries`);
  a project registered, a gap, a reconnect or the stream's first opening, since the server read
  the page before it, reads every project again. Each answer says when its read began, and a
  project shows the read that began last, its own or the page's. Each read again is a plain
  resource, which keeps what it showed until its answer lands, where a server future would
  suspend the page again. A session or run that goes silent reads as gone only once its
  project is next read, as the sessions panel's does.
- **The editor saves as `reef node put` does,** through `reef_api::puts_for`, at the version it
  read: another writer's change since comes back stale with the newer version, and a save that
  would add a problem comes back with check's messages.
- **The Changes tab shows a node's captured revisions,** the latest unless another is picked,
  and a file's diff once it is opened, fetched then and drawn as a table of its hunks. A
  capture the plan's stream announces reads the tab again (`live::WorkChanges`) without
  reading the plan again.
- **Its semantic view groups a file by its outline** (C101). The service keeps hunks with three
  lines of context and never reads git, so `reef node capture` parses both sides of each file a
  grammar reads with tree-sitter (`reef::outline`), matches their items, and sends the ones a
  change touched as the file's `outline`, kept in its row and its summary. `reef_api::outline`
  puts each line in its deepest item, the capture to count and the app to draw, so the two
  agree, and groups items by owner: a type with its impl blocks' members, a module with its
  loose items. `side::outline` lists a file's groups from the revision's listing and fetches
  its hunks once one opens, each item's lines drawn by `diff::hunk_table`, as the classic view
  draws them, so comments work alike. A language is a tree-sitter grammar crate and a `Grammar`
  that names its item nodes; Rust's is the only one yet. `View::diff_mode` keeps the toggle's
  pick in the browser.
- **Comments are threads on a diff's lines or on the node.** A click on a line's numbers opens a
  box under it, and its threads show under it, in each revision their line was followed to.
  The Comments tab holds the node's threads, a box for a general one, and how many are open,
  and the toolbar counts and lists the plan's open threads, each named where its line is now
  and marked if it is outdated. A thread a change resolved names that revision and its commit,
  or what it was reopened after. The canvas reads the plan's threads once a change, for all
  three.

- **A right-click opens a node's menu** (C100). The SVG keeps what each press landed on, a
  right-click's too, and its `contextmenu` opens the menu for that box, group or group head,
  preventing the browser's own; on nothing, the browser's shows. The menu is HTML beside the
  diagram, placed as the charts' tooltip is, so the boxes' markup stays reef-view's. A fixed
  backdrop under it closes it on the next press, wheel or right-click. Its queue items call
  `Queue::edit` as the sidebar's buttons do, and its status is set by `calls::set_status`,
  which puts the node back over the version it read, as `reef status` does, and refuses a group.
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
- **It listens where the CLI looks:** `[api] address`, loopback only, over HTTPS, so it takes
  reef-serve's place, and beside it for other hosts where `[api.tls]` says. Only one process can
  open shoal's data directory, so run one or the other. Under `dx serve`, dx names the server's
  address and proxies its own port to it over plain HTTP, the one plain port reef keeps; point
  `[api] url` at `http://` and dx's port to reach it from the CLI.
- **It speaks HTTP/2, over HTTPS on every port.** A plan's page holds three change streams
  open, the plan's, its queue's and its runners', and over HTTP/1.1 a browser keeps six
  connections to a host across all its tabs, so two tabs leave every other call waiting (C85).
  ALPN offers `h2` first, and a browser carries every tab's streams and calls on one
  connection, up to hyper's 200 streams at once. Browsers speak HTTP/2 only over TLS, which is
  why every port speaks it (C87); a plain-HTTP request is redirected to `https://`.
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
reef --conf reef.toml user add "$USER" && reef --conf reef.toml login --user "$USER"   # once, its first user
cargo +nightly test -p reef-app --features embedded                          # the suite
python check.py                                                              # the browser checks
python check.py --release --only=c33,c35                                     # the diff's timing
```

`tests/app.rs` serves the app's router over shoal in-process and puts plans through the CLI.
`check.py` drives the built server in headless Chromium with Playwright: it starts the server
itself, on a config and data directory of its own under `target/tmp`, reef's own certificate
made under it too, takes that certificate unchecked in every browser context, and stops its
process group at the end. Every server asks its callers to sign in, so the server `dx serve` or
a build starts answers nobody until its first user is added from this host and signed in; the
tests and `check.py` each add a user of their own.
