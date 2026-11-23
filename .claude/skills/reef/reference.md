# reef reference

## A node's keys

A node is the YAML `get` prints: a `# version:` line, then the keys a book's frontmatter takes.

```yaml
# version: 3                   # the version it was read at; leave it out for a new node
id: F125                       # unique across the plan; the only required key
title: F125 Archive reads in runs   # default: the body's first heading, then the id
status: in-progress            # todo | in-progress | done | blocked   (default: todo)
optional: true                 # drawn dashed
group: m23steps                # the group this belongs to (alias: parent)
needs: [F124, w1]              # solid arrows from each; a group means all of it (aliases: after, requires)
helps: [O171]                  # dashed arrows: helps, does not gate
docs: [design/page-reads.md#design-choices]   # from the repo's root, where the plan's pages lie (aliases: doc, links)
collapsed: false               # on a group: start open (groups start collapsed)
description:                   # why it is needed and what finishing it will do; see below
  why: A scan's archive reads wait on each other, one page at a time.
  what: A page's archive reads go out together, in disk order.
decisions:                     # the sidebar's design choices (alias: design; default: from the docs)
  - Runs cut in disk order, issued in the merge's order.
acceptance: [...]              # see below (alias: criteria)
performance: {...}             # see below (alias: perf)
nodes:                         # a new group's members, each put as a node of its own; same keys, plus body
  - id: I346
    title: "Item 346: paced reads go to their device together"
    status: done
    needs: [F125]
    body: Markdown for the sidebar.
```

- A node with members (nodes naming it in `group:`, or a group put with `nodes:`) is a group.
  Its status, progress (`7/9`) and readiness are derived; do not set `status` on it.
- A node's body is the markdown `reef node get <ID> --md` prints after its frontmatter. A put
  of YAML keeps it, and a put of that file writes it. Its links lead from the repo's root, and
  `node:<ID>` links a node.
- Quote YAML strings that contain `:`, `#`, or start with a symbol.
- Unknown keys (e.g. `tags:`) are ignored.

## description

```yaml
description:
  why: A caller with a long key list waits for every shard before it sees any row.   # the problem or gap
  what: Callers step through named keys as pipelined gets, each key back in order.   # the outcome once done
  details: |                     # optional, markdown, shown below them
    Longer background, scope, what is left out.
description: Some text.          # a plain string is `details` alone
```

- Every node that is not a group needs a `why` and a `what`, written or taken from its first doc
  that has them: `why` from a section (or `| **Impact** |`-style table row) named Why, Context,
  Motivation, Problem, Symptom, Question, The question or Impact; `what` from What it does, What, Summary,
  Overview or The fix. A section's first paragraph or list is taken, a list as its items' bold
  leads. Never from a doc's opening paragraph. Write both anyway.
- One or two plain sentences each. `why`: what is slow, wrong, missing or unknown without the
  work. `what`: what is true or possible once it is done, not how.
- The only keys are `why`, `what` and `details`. Inline: `description: { why: ..., what: ... }`.

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
- `accept` sets one by its number, from 1 in this order, with its `test`.

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

A ```reef fence in a node's body or a plan page draws part of the plan.

````markdown
```reef
root = "scans"          # this group's members (default: the whole plan)
focus = "m23"           # or: this group open, its outside neighbours faded around it
direction = "LR"        # TB (default) or LR
expand = ["m23steps"]   # groups open at first; or "all" / "none"
height = "80vh"
page = "plan.md"        # a "Full page" button linking to that page
layout = "page"         # fill the page; at most one per page, no height
```
````

## Tools

Claude Code names reef's MCP tools `mcp__reef__<name>`. Each takes a `plan` where the project
has several, and says what its command prints. A write that is stale or adds a problem is a
tool error carrying the message, and writes nothing.

| Tool | Reads or writes | Takes | Its command |
|------|-----------------|-------|-------------|
| `mcp__reef__next` | reads | `plan` | `reef next` |
| `mcp__reef__get` | reads | `id` | `reef node get <ID>` |
| `mcp__reef__check` | reads | `plan` | `reef check` |
| `mcp__reef__put` | writes | `yaml` | `reef node put <FILE>` |
| `mcp__reef__status` | writes | `id`, `status` | `reef status <ID> <STATUS>` |
| `mcp__reef__accept` | writes | `id`, `criterion`, `status`, `test` | `reef accept <ID> <N> met --test <TEST>` |

## Commands

Each picks the working directory's project, else `--project`, and its only plan, else `--plan`.

```sh
reef project show                    # the project the working directory belongs to
reef plan ls                         # its plans, each at its revision
reef next                            # ready work, the queue's first, then what the rest waits on
reef node get C38                    # a node's YAML, after its `# version:` line
reef node get C38 --md               # as a node file: frontmatter, version inside, then its body
reef node put c38.yml                # put YAML, or a node file to write its body too
reef status C38 in-progress          # todo | in-progress | done | blocked
reef accept C38                      # its criteria, numbered from 1
reef accept C38 2 met --test tests/skill.rs::writes_twice   # met | unmet | todo
reef check                           # the plan's problems, exit 1 on any
reef start C38                       # record the node's start in this checkout, and mark it in progress
reef work                            # the node this worktree is on, and what it started from
reef page ls                         # the plan's own pages, each at its version
reef page get design/feed-loop.md    # a page's markdown, after its `<!-- version: N -->` line
reef page put design/feed-loop.md feed-loop.md   # put it back, keeping that line
reef queue ls                        # the user's order, and what next makes of each entry
reef export plan --replace           # write the plan back over the book it was exported to
reef skill                           # rewrite this skill from the installed reef
```
