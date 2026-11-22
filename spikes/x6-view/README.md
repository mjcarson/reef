# X6: reef-view inside Dioxus 0.7

A scratch Dioxus 0.7.10 fullstack app that draws a plan with reef-view linked into its own wasm.

- The server reads a book's plan into the viewer's payload the way mdbook-reef does.
- It serves the book's rendered pages under `/book/`, so *Read here* has pages to fetch.
- The client mounts the viewer into an element Dioxus made, through `reef_view::mount`. It swaps
  each later payload in through the handle `mount` returned.
- Buttons pick the payload: the example book, a newer revision of it with one more node done
  (O171), or this plan's own book. Another button hides and shows the view.

It needs no shoal, so it builds on stable or nightly. It is a workspace of its own, which reef's
root workspace excludes.

## Running it

**Unset an exported `RUSTFLAGS` first:** dx passes it to the wasm client too, where it breaks
wasm-bindgen ([X1](../x1-shoal/README.md)).

```sh
unset RUSTFLAGS
# from reef's root: the rendered pages the payload's doc links point at
mdbook build example-book && mdbook build plan
cd spikes/x6-view
dx serve
# with Playwright: drive it in headless Chromium and print what each check saw
python check.py http://127.0.0.1:8080/ /tmp
```

`check.py` draws the example book and opens three nodes' sidebars. It opens every group and
reloads. It swaps in the newer revision and then the other plan, and hides and shows the view.
It double-clicks a collapsed group and a group's toggle. It prints anything the page logged.

## Sizes

`dx build --release` (wasm-opt `-Oz`, binaryen 129), one build a side. *Without* is the same app
with reef-view taken out and `draw` stubbed.

| Client wasm | Raw | Gzipped |
|---|---|---|
| Without reef-view | 2446.5 KiB | 657.5 KiB |
| With reef-view linked in | 2925.5 KiB (+479.0) | 831.5 KiB (+173.9) |
| `reef.js` loaded beside it instead | 815.9 KiB | 305.6 KiB |

Linked in, the viewer shares std, serde and wasm-bindgen's runtime with the app. Loaded beside it,
`reef.js` carries its own copies, and its wasm is base64 inside the script.

## What it found

The plan records these in `plan/src/design/app.md`, under *What X6 found*.

- **One wasm-bindgen.** Dioxus 0.7.10 resolves wasm-bindgen 0.2.130, the version reef-view pins.
  dx takes its CLI's version from the lockfile and downloads its own 0.2.130 to
  `~/.local/share/.dx/tools`. `cargo xtask wasm` runs the 0.2.130 on the PATH. No pin moves.
- **The page-load start has to be off.** With reef-view's `#[wasm_bindgen(start)]` linked in,
  wasm-bindgen makes it the module's `__wbindgen_start`. dx's loader calls the app's `main` only
  when there is no `__wbindgen_start`, so the Dioxus app would never start (read from the bound
  wasm). It sits behind reef-view's default `autostart` feature, which the app turns off.
- **`mount` takes the element, the payload and the view, and returns a handle.** It is a plain
  Rust call, `reef_view::mount(el, payload, view) -> Handle`, with no JavaScript in between.
  - Dioxus makes an empty `div.reef.reef-page`, and its `onmounted` data's `try_as_web_event()`
    is the element.
  - A `use_effect` on the payload's resource mounts the viewer the first time and calls
    `Handle::swap` after that.
  - Dioxus never touches the div's children, since the rsx gives it none, and the classes
    reef-view adds survive re-renders.
- **A payload swaps live, but the view starts again.** O171 shows *done* after the swap, and the
  page is never reloaded.
  - The swap resets the view from its spec, so open groups and the selection are lost.
  - The spec's `root` is the example book's. In the other plan there is no such group, so the
    view draws that whole plan.
- **Saved view state works, keyed by the page's path.** Opened groups survive a reload, under
  `reef:/:0`, the path and the view's index. Every plan drawn at one path shares that key.
- ***Read here* works when links are absolute.** `docs.rs` resolves a doc's link against the
  window's URL, so the server replaces reef-render's `ROOT` with `/book/example/` rather than a
  relative prefix. Docs were fetched from `/book/example/…` and `/book/plan/…`.
- **The sidebar works**: why and what, acceptance, perf charts, and doc sections read in it.
- **reef.css needs three of mdbook's theme variables.** It reads `--fg`, `--bg` and `--links`
  with no fallback. Without `--bg` the sidebar is transparent over the diagram.
  - The app can't link the CSS from the rendered book, since mdbook hashes the copy's name
    (`reef-c90942ff.css`).
  - `asset!` refuses a file outside the crate, even through a symlink.
  - So the server serves mdbook-reef's `assets/reef.css` at `/reef.css`.
- **An unmounted viewer keeps listening.** After the view was hidden and shown, the old viewer
  drew its detached element again on a filter change. It holds forgotten closures, a
  `ResizeObserver`, a `MutationObserver` and two `document` listeners, so every remount leaks one.
- **`cargo xtask wasm`'s output depends on the caller's `RUSTFLAGS`.** The committed `reef.js` was
  built with `-Ctarget-cpu=native` exported. wasm32 then has no reference types, and the glue
  keeps a heap of objects.
  - Built without the flag, the glue uses an externref table and the bundle changes throughout,
    which is how dx builds the app's copy.
  - With the flag, reef-view's change here only renumbers a few closure shims.
- **Head styles in a component with no props.** `document::Style` warns when its props change. A
  plain `style` element in the rsx gets a hydration marker in its text, which breaks its first
  rule. dx's hot-reload replays such warnings to pages that connect later, so they are checked
  from a fresh `dx serve`.
