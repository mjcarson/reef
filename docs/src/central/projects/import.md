# Importing a book

A plan already kept as an mdbook-reef book comes in whole, its nodes and its pages:

```sh
reef import docs --plan launch     # the book's directory, which holds its book.toml
```

- **It must be a reef book:** its `book.toml` has a `[preprocessor.reef]` table, which
  `mdbook-reef install` adds.
- **The book is checked first,** and a book with any problem imports nothing.
- **A plan that doesn't exist yet** is made by the import, so `plan new` isn't needed.
- **A plan that has nodes** is replaced only with `--replace`, which drops every node and page
  the book doesn't have.
- **The book's other markdown becomes the plan's pages,** at their paths from the book's `src`.
  The book can go once it is imported.
- **A mermaid flowchart** becomes a book first: `mdbook-reef import plan.md --out
  docs/src/plan --root launch` writes its boxes as node files in a book
  ([Commands](../../book/commands.md)), and `reef import` takes it from there.
