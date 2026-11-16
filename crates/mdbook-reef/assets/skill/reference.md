# reef reference

## A node's keys

```yaml
---
id: F125                       # unique across the book; the only required key
title: F125 Archive reads in runs   # default: the file's first heading, then the id
status: in-progress            # todo | in-progress | done | blocked   (default: todo)
optional: true                 # drawn dashed
group: m23steps                # the group this belongs to (alias: parent)
needs: [F124, w1]              # solid arrows from each; a group means all of it (aliases: after, requires)
helps: [O171]                  # dashed arrows: helps, does not gate
docs: [../features/page-reads-in-runs.md#design-choices]   # relative to this file (aliases: doc, links)
collapsed: false               # on a group: start open (groups start collapsed)
description: Reads in runs.    # the sidebar's description (default: taken from the docs)
decisions:                     # the sidebar's design choices (alias: design; default: from the docs)
  - Runs cut in disk order, issued in the merge's order.
acceptance: [...]              # see below (alias: criteria)
performance: {...}             # see below (alias: perf)
nodes:                         # members with no file of their own; same keys, plus body
  - id: I346
    title: "Item 346: paced reads go to their device together"
    status: done
    needs: [F125]
    body: Markdown for the sidebar.
---
# F125 Archive reads in runs

The body is what the sidebar shows.
```

- A node with members (a file with `nodes:`, or nodes naming it in `group:`) is a group. Its
  status, progress (`7/9`) and readiness are derived; do not set `status` on it.
- Quote YAML strings that contain `:`, `#`, or start with a symbol.
- Unknown keys (e.g. `tags:`) are ignored.

## acceptance

```yaml
acceptance:
  - unit: An empty key list answers at once          # <kind>: <text>, the short form
  - integration: { text: A dropped client's scan ends, test: "tests/scan.rs::dropped_client" }
  - feature: Saved positions survive a restart
    status: met                                       # todo | met | unmet (default: todo)
  - kind: performance                                 # the long form
    text: Shared syncs carry at least 6,000 inserts a second
    metric: inserts                                   # a metric's key or name: its chart is drawn here
    node: O151                                        # whose metric (default: this node's)
    at-least: 6000                                    # or at-most; checked against the metric's after median
  - perf: { text: Every metric's change, chart: change }   # a whole chart: change | pair
```

- Kinds: `unit` (`unit-test`), `integration` (`integration-test`, `e2e`), `performance`
  (`perf`, `bench`), `feature` (`feature-support`, `support`). Text is markdown.
- `metric` matches a metric's `key`, or its `name`, in that node's `performance`, and that
  metric must have figures; until it does, write the criterion as text only.
- `metric`, `chart`, `node`, `at-least`, `at-most` are only for `performance`. A target needs a
  `metric`; `metric` and `chart` don't go together.
- Met or not: `status` if given; otherwise a performance criterion with a target is met when
  its metric's `after` median (every `after` point, for a sweep) is within it.

## performance

```yaml
performance:
  labels: [Buffered, Shared]          # names for the two sides (default: Before, After)
  note: Medians of eight rounds.      # markdown under the table
  metrics:
    - name: rw50, inserts a second
      key: inserts                    # the same metric on other nodes, for a group's trend (default: name)
      better: higher                  # higher | lower (default: lower)
      before: [3523, 3476, 3547]      # a number, or [median, lo, hi] in that order
      after: [6837, 6783, 6915]
    - name: insert p99, ms
      unit: ms
      before: 17.2
      after: 10.2
    - name: decode by row size        # a sweep: [x, before, after]; either value may be null
      unit: µs
      x: row size (KiB)
      points: [[1, 1.2, 0.5], [64, 31, 2.4], [1024, 480, 19]]
```

Checked: every metric has figures, ranges are `[median, lo, hi]` in order, no `before` of zero.

## A view

````markdown
```reef
root = "scans"          # this group's members (default: the whole plan)
focus = "m23"           # or: this group open, its outside neighbours faded around it
direction = "LR"        # TB (default) or LR
expand = ["m23steps"]   # groups open at first; or "all" / "none"
height = "80vh"
page = "../plan.md"     # a "Full page" button linking to that chapter
layout = "page"         # fill the page; at most one per page, no height
```
````

## Commands

```sh
mdbook-reef check [book]     # the build's checks, exit 1 on problems
mdbook-reef import plan.md --out src/plans/x [--fence N] [--root id]   # mermaid flowchart -> nodes
mdbook-reef install [book]   # reef.js, reef.css, book.toml, and this skill
mdbook-reef skill [book]     # rewrite this skill from the installed mdbook-reef
```
