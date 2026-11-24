# Projects and plans

- **A project is one git repo.** Its plans' doc links lead to the repo's files, from its root,
  and reef finds the project from any checkout of it.
- **A plan is one graph of work in a project.** A project can have several plans, such as
  `backend` and `app`, and `reef next` looks across them all. It also has one queue, which
  orders work across its plans.
- **A node is one piece of work.** It holds a why, a what, its needs and its acceptance
  criteria, the same keys a book's frontmatter takes (see [Writing nodes](../../book/nodes.md)).
- **A plan's pages lie over its repo.** A plan can keep markdown pages of its own, such as
  design notes, and a page hides the repo's file at the same path.

Projects and plans are added with the CLI. The app lists them and edits their nodes, but it
can't add them. Run every command below from a checkout of the repo, or name the project with
`--project`.
