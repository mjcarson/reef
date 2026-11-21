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
  the app works with the mounted viewer.
- **Two sidebars until then.** The viewer's sidebar keeps the why, the what, acceptance and
  performance. The Dioxus node panel beside it holds the editor, and gains a Changes tab and a
  Comments tab with M7 ([C33](../milestones/m7.md), [C35](../milestones/m7.md)).
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

## The shell

### Design choices

- **A project and plan picker** as the app's front page, then each plan full-window.
- **Server functions call `reef-service`**, and the same axum router mounts the JSON API's
  routes ([C53](../milestones/m3.md)), so one port serves both.
- **Live refresh from the change stream.** When Claude puts a node, the app swaps the new
  revision in through the mount handle.
- **The app listens where the CLI looks.** Its server is its own `axum::serve` on `[api] address`,
  loopback only, rather than `dioxus::serve`, which listens on `PORT` or 8080. It takes
  reef-serve's place: only one process can open shoal's data directory. Under `dx serve`, dx names
  the address and proxies its own port to it.
- **Plans at `/plans/{project}/{plan}`, server functions under `/app/`,** so neither meets the
  JSON API's `/api/v1`. The server serves reef.css at `/reef.css` from mdbook-reef's assets.

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
- **The same section for Claude.** next_task returns the doc section as markdown, so Claude reads
  the same design the user sees.
- **Links that leave the repo are refused.** Check refuses a link that climbs out with `..`, and
  no doc is read or served through a symlink that leads out of the repo.
- **Only markdown is served.** An image or other file a doc links to doesn't load in its page.
- **Docs come from the working tree.** The docs shown are those of the checked-out branch, which
  can differ from the commit a node was done on.
