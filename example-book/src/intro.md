# About this book

This is reef's example, and its proof of concept: the scans plan from
shoal's `ScanSupport` branch, migrated to reef as
`docs/migrating-shoal.md` proposes. Shoal's pages were copied in unchanged, with three exceptions:

- **The plan's nodes** in `future-work/scans/plan/` were written by
  `mdbook-reef import` from the *whole plan* mermaid diagram on
  [What's left to do](future-work/scans/whats-left-todo.md): 190 boxes, 11 groups and 125
  arrows. The second box named `Pace` was renamed `F147`.
- **The eight mermaid diagrams on *What's left to do*** are now reef views of that one graph.
  Each part of the plan focuses on its own group, so the parts and the whole can no longer
  disagree.
- **[The scans plan, full width](plan.md)** is a page that holds only a view with
  `layout = "page"`. It fills the window beside this sidebar, and the sidebar stays, so you can
  leave the page from it. Every embedded view links to that page with its *Full page ↗* button.

Only the pages the plan's boxes link to are copied: the scans pages, the feature pages, and the
appendix entries. Links from those pages to the rest of shoal's book go nowhere. Shoal's other
mermaid diagrams render as code here, because this book has no mdbook-mermaid.
