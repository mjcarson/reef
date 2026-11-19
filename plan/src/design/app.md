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

### Design choices

- **Mount the viewer first; a native port is optional.** [C55](../milestones/m4.md) rebuilds
  the canvas and sidebar as Dioxus components over `reef-model` and `reef-layout`, but only once
  the app works with the mounted viewer.
- **Two sidebars until then.** The viewer's sidebar keeps the why, the what, acceptance and
  performance. The Dioxus node panel beside it holds the editor, the Changes tab and the Comments
  tab.
- **mdbook-reef keeps the viewer as it is.** The static mode builds with the auto-start feature,
  and its committed `reef.js` is unchanged in behaviour.

### Open questions (X6)

- **The wasm-bindgen pin.** `reef-view` pins `wasm-bindgen = "=0.2.130"`, and `cargo xtask
  wasm` runs wasm-bindgen-cli 0.2.130. If Dioxus 0.7 needs a newer one, the pin and xtask move
  together and `reef.js` is regenerated. The fallback is loading `reef.js` as a separate wasm
  module beside the app, which needs no unifying but is a second runtime to download.
- Whether saved view state, *Read here* doc fetches and the sidebar behave the same inside the
  app.

## The shell

### Design choices

- **A project and plan picker** as the app's front page, then each plan full-window.
- **Server functions call `reef-service`**, and the same axum router mounts the JSON API's
  routes ([C53](../milestones/m3.md)), so one port serves both.
- **Live refresh from the change stream.** When Claude puts a node, the app swaps the new
  revision in through the mount handle.

## Docs in central mode

The viewer's *Read here* fetches a linked doc's rendered mdbook page and cuts out the section
under the anchor's heading, inside the page's `<main>`. Centralized reef has no rendered pages.
A node's doc is a markdown file in its project's repo, or a page stored with the plan
([C28](../milestones/m6.md)).

### Design choices

- **The service renders docs as pages.** It renders a linked file or plan page as HTML with a
  `<main>` and mdbook's heading ids, so the viewer's `docs.rs` works unchanged.
- **The same section for Claude.** next_task returns the doc section as markdown, so Claude reads
  the same design the user sees.
- **Links that leave the repo are refused.**
- **Docs come from the working tree.** The docs shown are those of the checked-out branch, which
  can differ from the commit a node was done on.
