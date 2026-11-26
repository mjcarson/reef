# The app

## Context

mdbook turns a directory of markdown into static pages. Centralized reef has no such directory,
and needs screens with live state: picking a project and a plan, a queue the user drags into
order, a node's diff, comment threads, and a node editor. That is an app, not a book. It is a
Dioxus 0.7 fullstack app, `reef-app`, on axum: the same server and runtime as the shoal client,
which is tokio.

## The viewer inside the app

`reef-view` already lays out, draws and searches the plan, and its sidebar already renders the
why and what, acceptance criteria and performance charts. It is Rust compiled to wasm with
wasm-bindgen, so it can live in a Dioxus app, with four changes:

1. **Auto-start behind a default feature.** Today it mounts every `div.reef` on page load, from
   `script.reef-data` (`crates/reef-view/src/lib.rs`). Linked into the app's wasm without the
   gate, it would start a second viewer.
2. **A `mount(element, payload, view)` entry point** that returns a handle.
3. **A handle that swaps payloads**, keeping open groups, the selection and the scroll, so the
   app can refresh live.
4. **A selection callback, and storage keyed per plan.** The app opens the selected node in its
   own panel, and each plan remembers its own open groups.

X6 made the first two, and a swap that starts the view again from its spec. [C12](../milestones/m4.md)
made the rest: `Handle::swap` draws a newer revision where the old one was, `Handle::swap_plan`
draws another plan from its own view, `Handle::on_select` reports the selection, and `mount`
takes the plan's name, which its saved groups are kept under.

### Design choices

- **Linked into the app's wasm.** reef-app depends on reef-view with `default-features = false`
  and calls `reef_view::mount` from Rust. Linked in, the viewer adds 174 KiB gzipped to the
  app's wasm. `reef.js` beside it would add 306 KiB and a second runtime.
- **Mount the viewer first; a native port is optional.** [C55](../milestones/m4.md) rebuilds
  the canvas and sidebar as Dioxus components over `reef-model` and `reef-layout`, but only once
  the app works with the mounted viewer. It has since done so
  ([The canvas in Dioxus](#the-canvas-in-dioxus)), and the app no longer links reef-view.
- **Two sidebars until then.** The viewer's sidebar keeps the why, the what, acceptance and
  performance. The Dioxus node panel beside it holds the editor. C55 made them one sidebar,
  whose tabs M7's Changes and Comments join ([C33](../milestones/m7.md),
  [C35](../milestones/m7.md)).
- **mdbook-reef keeps the viewer as it is.** The static mode builds with the auto-start feature,
  and its committed `reef.js` is unchanged in behaviour.

### What X6 found

Tried in `spikes/x6-view`, a scratch Dioxus 0.7.10 fullstack app. Its server reads a book into
the viewer's payload and serves the book's rendered pages, and its client mounts reef-view and
swaps payloads. `check.py` drives it in headless Chromium.

- **One wasm-bindgen, and no pin moves.** Dioxus 0.7.10 resolves wasm-bindgen 0.2.130, the
  version reef-view pins. dx takes its CLI's version from the lockfile and downloads its own
  0.2.130. `cargo xtask wasm` runs the 0.2.130 on the PATH. A Dioxus that needs a newer one
  won't resolve against the `=0.2.130` pin, so the pin, xtask's CLI and `reef.js` move together.
- **The page-load start has to be off.** Linked in, reef-view's `#[wasm_bindgen(start)]` becomes
  the module's start. dx's loader then never calls the app's `main`. It now sits behind the
  default `autostart` feature.
- **`mount(el: HtmlElement, payload: Payload, view: ViewSpec) -> Handle`**, a plain Rust call.
  - The element is an empty `div.reef.reef-page` from Dioxus, reached through its `onmounted`
    data's `try_as_web_event()`.
  - A `use_effect` on the payload's resource mounts the viewer the first time and calls
    `Handle::swap(payload)` after that.
  - Dioxus leaves the div's children alone, since its rsx gives it none, and the classes the
    viewer adds survive re-renders.
- **A payload swaps live, but the view starts again.** A newer revision redraws with no reload,
  and so does a different plan.
  - The swap resets the view from its spec, so open groups and the selection are lost.
  - The spec's `root` is kept: in a plan without that group, the whole plan is drawn.
- **Saved view state works, keyed by the path.** Opened groups survive a reload, under
  `reef:{pathname}:{index}`.
  - Every plan drawn at one path shares that key.
  - A second `mount_all` call starts its index at 0 again.
- ***Read here* needs absolute links.** `docs.rs` resolves a link against the window's URL, so a
  payload's `ROOT` becomes the path the service serves rendered pages from (`/book/example/`),
  never a prefix relative to the page. The sidebar's why, what, acceptance and perf charts work
  unchanged.
- **reef.css needs `--fg`, `--bg` and `--links`,** which mdbook's themes define and it reads
  with no fallback. Without `--bg` the sidebar is transparent. mdbook hashes its copy's name,
  and `asset!` refuses a file outside the crate, so the spike's server serves mdbook-reef's
  `assets/reef.css` itself.
- **An unmounted viewer keeps listening.** Its element gone, it still drew it again on a filter
  change. Its closures are forgotten, and its two observers and two `document` listeners stay, so
  each remount leaks a viewer ([C58](../milestones/m4.md)).
- **`reef.js` depends on the caller's `RUSTFLAGS`.** The committed bundle was built with
  `-Ctarget-cpu=native` exported, which leaves wasm32 without reference types. Built without it,
  as dx builds the app's copy, the glue uses an externref table instead
  ([C59](../milestones/m4.md)).
- **Head styles in a component with no props.** `document::Style` warns when its props change,
  and a plain `style` element in rsx gets a hydration marker in its text that breaks its first
  rule.

## The canvas in Dioxus

### Context

The mounted viewer wrote its own toolbar, SVG and sidebar as HTML strings, so the app couldn't
style or extend them as components, and the editor sat in a second panel beside the viewer's
sidebar. [C55](../milestones/m4.md) draws all of it with Dioxus.

### Design choices

- **reef-draw holds what both viewers draw.** reef-view's pure half, its box sizes, layout
  input, markup, criteria and charts, and the view's camera and search, moved into `reef-draw`
  ([C65](../milestones/m4.md)). reef-view writes its markup; the canvas draws the same elements
  in rsx, checked against it by `tests/diagram.rs`. Boxes are the same size and in the same
  place in both, given the same font.
- **State by id, and a canvas per plan.** Open groups, the selection and the filter are held by
  id, so a newer revision draws where the old one was. The plan's page keys the canvas by
  plan, so another plan starts afresh. Together they do what `Handle::swap` and
  `Handle::swap_plan` did.
- **One sidebar, with tabs.** The node's header, then **Details**, which is what reef-view's
  sidebar shows, **Changes**, which is the node's captured revisions ([C33](../milestones/m7.md)),
  **Comments**, which is its threads ([C35](../milestones/m7.md)), **Decisions** ([C76](../milestones/m9.md)),
  **History**, which is who made each edit of it ([C50](../milestones/m10.md)), **Timeline**, which is
  everything that happened to it ([C90](../milestones/m10.md)), and **Edit**, which is C20's editor. The
  tabs wrap onto a second line where the sidebar is too narrow for them.
- **The boxes say what was hit.** Each box, group head and toggle writes what a press or a move
  landed on to a signal, as the event bubbles, and the SVG's own handler takes it. Nothing reads
  the DOM to find the target, and nothing stops the event.
- **Markup where it is markup.** A node's rendered markdown, and the charts' SVG from reef-draw,
  go in as inner HTML. A chart's view (hidden series, a zoomed range) is a signal; what the
  pointer shows over it, its crosshair and the metric it lights, is written into its SVG, so a
  hover never draws the chart again.
- **mdbook keeps reef-view.** The static mode still builds reef.js from reef-view, drawing from
  reef-draw too.

### What C55 found

- **A key on a nested component is dropped.** Dioxus 0.7.10 reads a key only on the first node
  of an rsx block, so the canvas is the root of a block of its own inside the stage.
- **dioxus-html's `title` is the document's.** Made in the HTML namespace, it gives no tooltip
  in an SVG. A local `dioxus_elements` module adds `svgtitle`, made in SVG's namespace.
- **The SVG holds the pointer from a press,** so the browser sends the click and double-click
  that follow it to the SVG, not to the box. The canvas opens the box the press landed on.
  reef-view looked for the box under the double-click and found none, so its double-click never
  opened a collapsed group. It now opens the box its last press landed on too
  ([C66](../overview.md)).
- **What the server rendered is what hydration keeps.** The sidebar's width, read from local
  storage as its signal started, never reached the stage's style, and the arrows' marker, named
  from a timer, differed from the server's, so no arrow had a head. Storage is now read in an
  effect once the page is live, and the marker's name is fixed.
- **Chrome sends `pointerover` when the page changes under a still pointer.** Lighting a box's
  lineage on it lit the group a click had just collapsed. The canvas lights on `pointermove`,
  as reef-view does, and a new layout lets the hover go.

## The shell

### Design choices

- **A project and plan picker** as the app's front page, then each plan full-window.
- **Server functions call `reef-service`**, and the same axum router mounts the JSON API's
  routes ([C53](../milestones/m3.md)), so one port serves both.
- **Live refresh from the change stream.** When Claude puts a node, the app reads the plan
  again and draws the new revision where the old one was.
- **The app listens where the CLI looks.** Its server is reef-service's `api::serve_router` on
  `[api] address`, plain HTTP on loopback, and over TLS beside it where `[api.tls]` says
  ([C51](../milestones/m10.md)), rather than `dioxus::serve`, which listens on `PORT` or 8080. It takes
  reef-serve's place: only one process can open shoal's data directory. Under `dx serve`, dx names
  the address and proxies its own port to it.
- **Plans at `/plans/{project}/{plan}`, server functions under `/app/`,** so neither meets the
  JSON API's `/api/v1`. The server serves reef.css at `/reef.css` from mdbook-reef's assets.
- **Signed in as reef-serve is** ([C48](../milestones/m10.md)). The whole router sits under the service's one
  layer ([Users and sign-in](architecture.md#users-and-sign-in)), so a browser that isn't signed in
  lands on the service's sign-in page and never loads the app. Once it is, the bar says whom the
  server takes it for, beside a Sign out, read through `/app/me` as the page renders, and shows
  nothing while sign-in is off.
- **A lapsed sign-in offers the way back.** A browser whose token was revoked or expired finds its
  change stream closed and its calls answered 401: the bar says live updates stopped, and a call
  that failed says it needs signing in, each linking the sign-in page, which comes back to the
  same route.
- **An account page at `/account`** ([C77](../milestones/m10.md)), which the bar's name links to: your
  password, changed with your current one; your tokens, by id, kind, label and dates, this
  browser's marked and each other revoked from there; and, for an admin, every user, whose
  password they set or whom they remove, asked to confirm first. Its server functions,
  `/app/account` and the posts under it, call the service as the JSON API's routes do
  ([Managing users and tokens](architecture.md#managing-users-and-tokens)). No password field sits
  in a `form`, so nothing typed is sent before the page's script runs.

## Docs in central mode

The viewer's *Read here* fetches a linked doc's rendered mdbook page and cuts out the section
under the anchor's heading, inside the page's `<main>`. Centralized reef has no rendered pages.
A node's doc is a markdown file in its project's repo, or a page stored with the plan
([C28](../milestones/m6.md)).

### Design choices

- **The service renders docs as pages.** It renders a linked file or plan page as HTML with a
  `<main>` and mdbook's heading ids, so the viewer's `docs.rs` works unchanged. The payload's
  links are absolute paths to them, since `docs.rs` resolves a link against the window's URL
  ([X6](#what-x6-found)).
- **Pages sit under their plan.** A plan's docs are served at
  `/api/v1/projects/{project}/plans/{plan}/docs/`, each at the page mdbook would write for it
  (`docs/design.html` for `docs/design.md`), so a plan's own pages can sit beside its repo's
  ([C28](../milestones/m6.md)).
- **A plan's pages lie over its repo.** A plan keeps markdown pages of its own, named by paths
  in the same space as the repo's files, and read first: a page hides the repo's file at its
  path. Check, briefs, the payload's links and the served pages all read through the one
  layered source, so a link names a page as it names a file ([C28](../milestones/m6.md)).
- **A book's pages keep their paths from its `src`.** Import makes each markdown file of a book
  that isn't a node's file a page at its path from `src`: `plan/src/design/app.md` is the page
  `design/app.md`, and a node's link to it is `design/app.md#…`. A page's links to other pages
  are left as written, since they keep their places, and its other links lead to the repo's
  files from where the page now is. Export writes each page back at its path under `src`.
- **A page edit is checked as a node write is.** Taking out a page a node links to, or the
  heading it names, or the section its why came from, adds a problem and is refused. An edit
  names the version its writer read, and the app reads an open section again after each change.
- **A link can name a node.** `node:C28`, or `node:M6#rows` at a heading of its body, written in
  a page or a node, opens that node in the sidebar: its anchor carries `data-goto`, which both
  viewers' sidebars follow, so a link to a node needs no page of its own. Import turns a link
  into a node's file into one naming the node that heads the file, so it outlives the book, a
  link naming a node the plan lacks is a problem, and export writes each back to the file its
  node lands in ([C68](../milestones/m6.md)).
- **The same section for Claude.** next_task returns the doc section as markdown, so Claude reads
  the same design the user sees.
- **Links that leave the repo are refused.** Check refuses a link that climbs out with `..`, and
  no doc is read or served through a symlink that leads out of the repo.
- **Only markdown is served.** An image or other file a doc links to doesn't load in its page.
- **Pages come from the plan, other docs from the working tree.** A plan's pages are its own,
  versioned with it. Any other doc shown is the checked-out branch's, which can differ from the
  commit a node was done on.

## The queue panel

The user orders a project's ready work, and the terminal sets that order first
([C24](../milestones/m5.md)). The app shows the queue beside the plan and reorders it by dragging
([C25](../milestones/m5.md)).

### Design choices

- **One queue a project, beside the canvas, outside the plan's key.** The panel is a column in
  the plan's page, so moving between a project's plans keeps it, and the canvas's sidebar
  reaches it through context the page provides.
- **Followed on the project's stream.** Whether an entry is ready moves with any of the
  project's plans, so the queue is read again on any change to the project, not only on its own
  `queue-changed`. The canvas still follows its own plan's stream.
- **A drag is one move**, which writes the one row of the entry moved: after the entry it was
  dropped below, or at the top. The place is worked out by name from the queue as the drag
  began, so a change read mid-drag can't put it elsewhere.
- **Pointer events, as the canvas's drags are**: capture on the entry's grip and a movement
  threshold, so touch works too, rather than HTML drag and drop, which touch never fires. The
  arrow keys move a focused entry a place, Home and End to either end, and Delete takes it off.
- **The sidebar queues the selected node** first or last, moving it if it is queued already, and
  takes it off. A sidebar drawn outside a plan's page has no queue, and no buttons.
- **Each entry says who placed it,** "queued by ada" or "moved by Claude, for ada", from the
  queue's history ([C50](../milestones/m10.md)), and nothing for an entry nobody known placed.

## The Changes tab

A node's changes are kept as revisions of its files, hunks and lines ([C32](../milestones/m7.md)). The
Changes tab shows them beside what the plan says of the node ([C33](../milestones/m7.md)).

### Design choices

- **A revision's files, then a file's lines when it opens.** The tab reads the node's
  revisions, each listing its files without their hunks, and shows the latest unless another
  is picked. A file's hunks are fetched and drawn only once it is opened, so a node with many
  changes opens at once.
- **A table a file, as git shows it.** Each hunk is headed `@@ -a,b +c,d @@`, and each line
  is a row numbered on the sides it is on, with git's sign and its text as written. A missing
  final newline, a mode change, a binary file and one too large to keep its lines are said in
  words.
- **Drawn whole.** X2's commit's `Cargo.lock`, 3,609 lines, opens in 83 ms in a release build,
  so a file is drawn whole, each line a plain function's rows rather than a component's scope.
  Time grows with a file's lines, so a much larger file is where a line budget would go.
- **Followed apart from the plan.** The plan's stream bumps a counter of its own for a
  revision captured, which the tab reads its revisions again on, so a capture neither reads
  the plan's payload again nor forgets the docs read. A file left open stays open in the newer
  revision.
- **Kept across tabs, let go with the node.** The revision picked and the files open stay
  while another tab is shown, and are let go when another node is selected.

## Comment threads

The user reviews a node's work in the app as a merge request is reviewed: on the lines of its
diff, and on the node as a whole ([C35](../milestones/m7.md)), over the threads C34 keeps.

### Design choices

- **The plan's threads, read once for the canvas.** The canvas reads every thread of the plan,
  again after each comment, mark or follow the plan's stream announces and after each write
  the page makes, and gives them to the toolbar and the sidebar through context. The toolbar's list,
  the Comments tab and the diff's threads show the same, from one read a change.
- **A line's numbers start a thread on it.** One handler on the diff's table finds the line a
  click on its numbers landed on, by its side and number, and opens a box under it. A thread
  is drawn under its line in each revision it was followed to, where its line was in that
  revision ([C36](../milestones/m7.md)), so an older revision picked still shows it where it was then.
  Two attributes a row and a lookup a line moved X2's `Cargo.lock` from 83 ms to 92 ms to open.
- **The Comments tab, general threads first.** Its count says how many of the node's threads
  are open. The threads on the node come first, then a box for a new one, then those on its
  lines, open before resolved, each named where its line is now, with a way to it: the Changes
  tab at the latest revision its line was followed to, its file open and the line brought into
  view.
- **Outdated, said and drawn.** A thread whose line a later revision lost is named where its
  line was last, "outdated in r*N*", with an outdated tag before its state and a dashed edge.
  It is drawn in the revisions up to the one that lost its line, and in none after.
- **A plan-wide list in the toolbar.** "Comments N" counts the plan's open threads, and lists
  each by its node, where it is, and the first line of its text. An entry selects its node and
  brings its thread into view in the Comments tab.
- **Shown as the service rendered it.** A comment's body comes rendered, raw HTML escaped, so
  the app shows its HTML as it is. A mark sends the version it read, and a stale one says so
  and shows the thread as it now is.
- **A review, as a merge request has** ([C73](../milestones/m8.md)). Every comment box has a second button,
  *Start a review*, or *Add to review* once the node has one open, beside *Comment now*. A
  comment written in a review is drawn with a *pending* tag and a dotted edge, with no Resolve,
  and a reply to a pending thread is only added to its review. The canvas reads the plan's open
  reviews with its threads, pending ones included.
- **The review bar heads the Comments tab** while the node has a review open: how many comments
  are pending in it, *Submit review*, which publishes them all at once, and *Discard*. Each is
  sent over the version read. The open counts, in the tab and the toolbar, leave pending threads
  out until they are published.
- **Who wrote it, in each comment's head** ([C50](../milestones/m10.md)): "ada", "Claude, for ada" or
  "Claude", and nothing for a comment kept before reef knew its users. A thread's foot says who
  marked it last, "resolved by bob", by hand or by the capture whose trailer resolved it.

## The Decisions tab

Every plan a node's sessions asked approval for and every question they asked is kept with the
node, with what came of it ([C75](../milestones/m9.md)). The node's sidebar shows them beside its changes
and comments ([C76](../milestones/m9.md)).

### Design choices

- **A tab of the node's sidebar,** after Comments, since a decision belongs to the node it
  shaped, however its session ran: in a terminal or under a runner.
- **Read with the node's work.** `decisions-recorded` bumps the plan's work counter, as a
  capture or a comment does, so the tab reads them again without reading the plan, and a
  decision kept while the tab is open shows up there.
- **In the order kept, a card each,** numbered, with its verdict, and where and when it was
  answered: in reef's app, or in the terminal.
- **A plan as the service rendered it,** raw HTML escaped. Its rejection's feedback is quoted
  above it, and only the last approved plan is unfolded, so the plan a node was built from is
  the one open.
- **A question shows every choice it offered,** the ones picked ticked. Several picks are
  joined by `, ` as Claude Code joins them, so an answer whose parts are all labels is read as
  those picks, and any other as the user's own words, shown as written.

## The History tab

Who made each edit of the node, what it changed and when ([C50](../milestones/m10.md)).

### Design choices

- **A tab of the node's sidebar,** after Decisions, since a node's edits are the plan's changes,
  however they were made: in the app, the terminal or Claude's tools.
- **Read again on each change to the plan,** as the editor reads the node, so an edit made
  while the tab is open shows up there.
- **A line an edit, newest first:** when, who if anyone known, the version it left the node at,
  and what it changed, as `reef node log` says it. A whole plan's write is tagged, so an
  import reads apart from an edit of the node alone.
- **200 at a time** ([C88](../milestones/m10.md)). "Older edits" reads 200 more, from the newest each time,
  so an edit while older ones are shown leaves no gap among them.

## The Timeline tab

Everything that happened to the node, newest first, in one list ([C90](../milestones/m10.md)): its edits,
each start, each capture, and each comment, thread mark and resolution, which before took
reading four tabs.

### Design choices

- **A tab of its own, after History,** which stays for the edits alone, as `reef node log` prints
  them.
- **Merged on the server, from four reads.** The node's history gives its edits and starts, its
  revisions its captures, and its threads its comments, marks and resolutions; one server
  function merges them by time, so the client draws one list.
- **Every start is in the history.** A work record keeps only a node's latest start, so each
  start keeps a `started` entry in the node's history as it lands, naming who, where and whether
  it started again. A start recorded before that is added from the record, unless an entry names
  it.
- **Paged as History is.** While older edits are kept, nothing older than the oldest edit read is
  shown, so a capture or a comment never sits above a gap of edits.
- **Read again on any change to the node:** a plan's change, and a start, a capture, a comment or
  a mark, so each shows live.
- **A capture's line opens Changes at its revision, and a comment's opens Comments.** A comment
  pending in a review is left out, as everything but its reviewer's Comments tab leaves it out.

## The sessions panel

A runner the user started, `reef loop run`, runs a loop's sessions headless, and central reef
relays them ([The loop in the app](feed-loop.md#later-the-loop-in-the-app)). The app shows each
session's transcript as it comes, starts, pauses and stops the runner's loop, and answers what
the session waits on ([C45](../milestones/m9.md)). A Claude Code session the user is in shows there too,
once `! reef loop run` typed in it attaches it ([C84](../milestones/m8.md)).

### Design choices

- **A column beside the canvas, opened from the bar.** The panel sits in the plan's page beside
  the canvas, outside the plan's key, so moving between a project's plans keeps it. The bar's
  *Sessions* button counts the live sessions and the prompts waiting on the app, so a question
  is seen with the panel shut.
- **Followed on the project's stream, on a counter of its own.** `session-changed` and
  `loop-changed` read the runners again, and the open transcript after what it holds, without
  reading the queue or the plan.
- **A transcript is read after what it holds.** Each read asks for the entries after the last
  one held, and the list follows its end as they come. A session keeps its last 2,000 entries,
  so a read that finds its entries let go starts again from the oldest kept.
- **Each session a row, a terminal's own to read.** A runner shows its host, its checkout, where
  its session stands and its loop's node, with *Start a loop*, *Resume*, *Pause* and *Stop* as
  its loop allows. A terminal's session attached with `! reef loop run` shows the same, and its
  pane, with no buttons: its picked transcript shows no cards and no reply box, and says it is
  answered in its terminal, since only its terminal drives it ([C84](../milestones/m8.md)). A terminal's
  loop with no session attached on its pane is listed beside, to read.
- **Each prompt a card.** A tool's permission shows its input, to allow or to deny with what
  Claude is told. A reply box shows while Claude's turn has ended mid-node.
- **A question as Claude asked it** ([C46](../milestones/m9.md)). Each of Claude's questions shows with
  its header and choices, radio buttons or checkboxes as it allows, and a box for an answer in
  the user's own words, which wins over a pick. *Answer* waits until every question has one,
  and sends each keyed by its question's text, several picks joined by `, ` in the order
  offered, as Claude Code's own dialog sends them. *Don't answer* tells Claude the user chose
  not to. The transcript keeps each question with its choices, and the answer given.
- **A plan to approve, or to reject with feedback** ([C47](../milestones/m9.md)). The plan shows as the
  service rendered it, raw HTML escaped as a comment's body is. *Approve* lets ExitPlanMode run.
  *Reject with feedback* waits for what to change, and denies ExitPlanMode with it, which Claude
  plans again from and asks again, so the revised plan comes back as a card of its own. The
  transcript keeps each version of the plan, folded, beside the answer it got.
- **Ended runners folded away.** The live runners come first; one that let go or went silent is
  listed under *Ended*, still there to read its transcript.
