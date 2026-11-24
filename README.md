# reef

Planning graphs you can zoom, pan, collapse and click through. Write each piece of work as
markdown, Obsidian-style, and reef draws the whole plan as a directed graph. It replaces the
large mermaid flowcharts that planning docs outgrow.

A plan lives in one of two places:

- **In an mdbook**, through the `mdbook-reef` preprocessor, checked when the book builds and
  drawn inside its pages.
- **On a server**, central reef, which keeps a team's projects and plans, checks every write
  against the whole plan, and serves them to the `reef` CLI, Claude Code and a web app.

## The docs

reef's docs are an mdbook in [`docs`](docs/src/SUMMARY.md):

```sh
mdbook serve docs --open
```

- [Introduction](docs/src/introduction.md)
- [Plans in a book](docs/src/book/setup.md): setting a book up, writing nodes, views, the
  sidebar and the commands
- [Central reef](docs/src/central/overview.md): deploying a server, adding projects and plans,
  using it, administering it
- [Internals](docs/src/internals/stable.md): the workspaces, the layout, and central reef's
  service, JSON API, store and schema

## Quick start

```sh
cargo install --path crates/mdbook-reef      # the preprocessor and CLI
mdbook-reef install path/to/book             # writes reef.js + reef.css, edits book.toml
```

Then write a node in the book's `src` ([Writing nodes](docs/src/book/nodes.md)) and draw it
with a ```` ```reef ```` fence ([Drawing a view](docs/src/book/views.md)).
