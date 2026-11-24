# Introduction

reef keeps a plan of work as a graph. Each piece of work is written as markdown, Obsidian-style,
and reef draws the whole plan as a directed graph you can zoom, pan, collapse and click through.

It replaces the large mermaid flowcharts that planning docs outgrow. At a few hundred boxes,
mermaid shrinks the whole diagram to the page width and it cannot be zoomed.

- **Markdown is the source of truth.** A node is a file with YAML frontmatter, or a short entry
  in a group's `nodes:` list when it does not deserve a page of its own.
- **One graph, many views.** Every fence draws part of the same plan, so a box is written once
  and every diagram that shows it agrees.
- **Groups collapse.** A feature shows as one box with its progress (`7/9`) until you open it, so
  the whole plan fits on one screen and you drill into the part you care about.
- **Click a box** to open a sidebar that says why the work is needed and what finishing it will
  do, then its rendered markdown, what it needs, what needs it, its docs, and a link to its page.
- **Status is derived.** A group is done when its members are; a box is *ready* when everything it
  and its groups need is done, so "what can start now" is never kept up by hand.
- **Checked on every change.** Unknown ids, cycles, a box that needs its own group, doc links
  to missing files or headings, and work that does not say why it is needed or what it will do
  are all reported with where they are.
- The viewer is Rust compiled to wasm. Layout of a few hundred boxes takes milliseconds, so every
  collapse or expand lays the whole view out again.

## Two ways to keep a plan

- **In a book.** `mdbook-reef` is an [mdbook](https://rust-lang.github.io/mdBook/) preprocessor.
  A plan is node files in the book's `src`, checked when the book builds and drawn inside its
  pages. The book is committed with the code, and nothing else needs to run.
  [Plans in a book](book/setup.md) covers it.
- **On a server.** Central reef keeps a team's projects and plans in one service, and checks
  every write against the whole plan, as a book's build does. The `reef` CLI, Claude Code
  (through reef's MCP tools) and a web app all read and write the same nodes. A node's keys, the
  checks and the viewer are the same as a book's. [Central reef](central/overview.md) covers it.

A repo holds one or the other, and the Claude skill that comes with each says how to keep the
plan in step with the work: `mdbook-reef install` writes the book's, and `reef claude install`
the server's ([Setting a repo up for Claude Code](central/projects/claude.md)).

## This book

- **[Plans in a book](book/setup.md):** setting a book up, writing nodes, their why and what,
  acceptance criteria and performance, drawing views, the sidebar, and the commands.
- **[Central reef](central/overview.md):** what it is, then deploying a server, adding
  projects and plans, using it from the CLI, the app and Claude Code, and administering it.
- **[Internals](internals/stable.md):** the workspaces and how they are built, the layout, and
  central reef's service, JSON API, store and schema.
- **[Moving shoal's planning docs to reef](appendix/migrating-shoal.md):** the migration the
  example book carried out.

Build it with `mdbook build docs`, or read it as you write with `mdbook serve docs --open`.
