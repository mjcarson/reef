# Commands

```sh
mdbook-reef check [book]     # the build's checks without building, for CI; exit 1 on problems
mdbook-reef next [book]      # the work ready to start, in plan order, and what the rest waits on
mdbook-reef skill [book]     # write the Claude skill to the repo's .claude/skills/reef (--print to show it)
mdbook-reef import plan.md --out src/plans/scans [--fence N] [--root id]
```

`import` turns a mermaid flowchart (by default the largest one in the file) into reef nodes:

- each subgraph becomes a group file, and the boxes outside any subgraph become a root group;
- `-->` becomes `needs` and `-.->` becomes `helps`;
- `:::done` or ✅ means done, and `:::optional` or "(optional)" means optional;
- a box whose label starts with an id the book has a heading for (`F125`, `O147`, `X17`,
  `Item 346` and so on) gets that heading as its doc;
- an id defined twice with different labels becomes two nodes, and the import reports it.
