# The stable workspace

reef's root workspace builds on stable Rust. It holds every crate but central reef's service,
which needs shoal and is a workspace of its own ([The central workspace](central.md)).

```
docs                 this book: `mdbook build docs`, or `mdbook serve docs --open`
crates/reef-model    nodes, the checked graph, views (shared by both sides)
crates/reef-layout   the layout, plain Rust, tested natively
crates/reef-render   markdown rendering, doc links and why/what extraction over any DocSource
crates/reef-view     the wasm viewer
crates/mdbook-reef   the preprocessor and CLI; embeds assets/reef.js and assets/reef.css
crates/reef-api      centralized reef's JSON API: the service's types, routes and config location
crates/reef          centralized reef's terminal client, an HTTP client of the JSON API
xtask                cargo xtask wasm: builds reef-view and bundles it into assets/reef.js
example-book         shoal's scans plan, migrated: a proof of concept on a real 190-box plan
central              centralized reef: the store and service over shoal, a workspace of its own
```

```sh
cargo install wasm-bindgen-cli --version 0.2.130   # once; must match the wasm-bindgen crate
cargo xtask wasm                                   # after changing reef-view, reef-model or reef-layout
cargo test --workspace
cd example-book && mdbook serve
```

`assets/reef.js` is generated, but it is committed so that `mdbook-reef` builds without the wasm
toolchain.
