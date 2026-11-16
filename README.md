# reef

Planning graphs for [mdbook](https://rust-lang.github.io/mdBook/). Write each piece of work as
markdown, Obsidian-style, and reef draws the whole plan as a directed graph you can zoom, pan,
collapse and click through, inside the book.

It replaces the large mermaid flowcharts that planning docs outgrow. At a few hundred boxes,
mermaid shrinks the whole diagram to the page width and it cannot be zoomed.

- **Markdown is the source of truth.** A node is a file with YAML frontmatter, or a short entry
  in a group's `nodes:` list when it does not deserve a page of its own.
- **One graph, many views.** Every fence draws part of the same plan, so a box is written once
  and every diagram that shows it agrees.
- **Groups collapse.** A feature shows as one box with its progress (`7/9`) until you open it, so
  the whole plan fits on one screen and you drill into the part you care about.
- **Click a box** to open a sidebar with its rendered markdown, what it needs, what needs it, its
  docs, and a link to its page.
- **Status is derived.** A group is done when its members are; a box is *ready* when everything it
  and its groups need is done, so "what can start now" is never kept up by hand.
- **Checked at build time.** Unknown ids, cycles, a box that needs its own group, and doc links
  to missing files or headings all fail the build with `file:line`.
- The viewer is Rust compiled to wasm. Layout of a few hundred boxes takes milliseconds, so every
  collapse or expand lays the whole view out again.

## Setup

```sh
cargo install --path crates/mdbook-reef      # the preprocessor and CLI
mdbook-reef install path/to/book             # writes reef.js + reef.css, edits book.toml
```

`install` adds `[preprocessor.reef]` and adds `reef.js` and `reef.css` to
`output.html.additional-js` and `additional-css`. `reef.js` is a single classic script with the
wasm inlined, so the book also works when opened straight from `file://`.

`install` also writes a [Claude skill](crates/mdbook-reef/assets/skill/SKILL.md) to
`.claude/skills/reef/` at the root of the git repository that holds the book (skip it with
`--no-skill`). It teaches coding agents to draw new work as nodes before starting it, to mark
it done and its acceptance criteria met in the change that finishes it, to record benchmark
figures in `performance`, and to run `mdbook-reef check` after every edit. After upgrading
reef, `mdbook-reef skill [book]` refreshes it.

## Writing nodes

```markdown
---
id: m23                        # unique across the book; the only required key
title: M23 The walk rebuilt    # default: the file's first heading, then the id
status: done                   # todo | in-progress | done | blocked   (default: todo)
optional: true                 # drawn dashed
group: scans                   # the group this belongs to (alias: parent)
needs: [m22, w1]               # solid arrows from each; naming a group needs all of it
helps: [o171]                  # dashed arrows: helps, does not gate
docs: [milestones.md#m23-the-walk-rebuilt-for-load, ../../features/f125.md]
collapsed: false               # a group that starts open (groups start collapsed)
description: Reads in runs.    # the sidebar's description (default: taken from the docs)
decisions:                     # the sidebar's design choices (alias: design; default: from the docs)
  - Runs cut in disk order, issued in the merge's order.
performance:                   # how the work changed what was measured (alias: perf); see below
  metrics:
    - { name: "inserts a second", better: higher, before: 3523, after: [6837, 6783, 6915] }
nodes:                         # members with no file of their own
  - id: i346
    title: "Item 346: paced reads go to their device together"
    status: done
    needs: [f125]
    body: Markdown for the sidebar.
---
# M23 The walk rebuilt

The body is what the sidebar shows. If this file is in SUMMARY.md, the sidebar also links to it.
```

- Any markdown file under `src` whose frontmatter has an `id` is a node. To look in only some
  directories, set `[preprocessor.reef] roots = ["plans"]`.
- Keys reef does not know, such as an Obsidian `tags:` list, are ignored.
- Links in a body and in `docs` are relative to the file, just like ordinary markdown links. A doc
  link's label is the heading it points to.
- reef strips YAML frontmatter from every chapter, since mdbook would otherwise render it as text.

## The sidebar

Clicking a box opens a sidebar. Each of its sections folds from its header (▾), and the
header stays pinned to the sidebar's top while you scroll through the section, so a long one
can be folded from anywhere in it. The sections, all open to start:

- **Description:** a brief form of the work.
- **Performance:** it shows how the work changed what was measured. See
  [Performance](#performance).
- **Documentation:** each doc link, with *Open page ↗*, and below it that section of the doc,
  which its header (▾) folds. It is fetched only when the sidebar shows it, so docs add
  nothing to the page. A book opened from `file://` cannot fetch, and says so.
- **Design choices.**
- **The rest:** members, needs, what needs it, and the node's own body.

The description and design choices are written in frontmatter (`description`, `decisions`) or
taken at build time from the node's first doc that has them:

- **Description:** the first paragraph of a subsection with one of these names, or the second
  cell of a table row (`| **Impact** | ... |`) with one of them. Failing both, the first
  paragraph of the linked section.
- **Design choices:** the subsection with one of these names. A list whose items open with a
  bold lead shows each choice folded, with its lead as the summary.

Struck-through text is left out of both, since they say what is true now. The default names:

```toml
[preprocessor.reef]
description-sections = ["What it does", "Symptom", "Summary", "Overview", "Impact"]
design-sections = ["Design choices", "Design", "Decisions", "The fix"]
performance-sections = ["Performance", "Results", "Benchmark", "Benchmarks", "Measurements"]
```

### Performance

A node's `performance` key holds figures measured before and after the work:

```yaml
performance:
  labels: [Buffered, Shared]          # names for the two sides (default: Before, After)
  note: Medians of eight side-rounds. # markdown under the table
  metrics:
    - name: rw50, inserts a second
      key: inserts                    # the same metric on other nodes, for a group's trend (default: name)
      better: higher                  # higher | lower (default: lower)
      before: [3523, 3476, 3547]      # a number, or [median, lo, hi]
      after: [6837, 6783, 6915]
    - name: insert p99, ms
      unit: ms
      before: 17.2
      after: 10.2
    - name: decode by row size        # a sweep: [x, before, after], either value may be null
      unit: µs
      x: row size (KiB)
      points: [[1, 1.2, 0.5], [64, 31, 2.4], [1024, 480, 19]]
```

The sidebar's Performance section starts folded, with the first two changes in its summary. Open,
it shows:

- **A table:** each metric before and after, its change (`−41%`, or `1.94×` past double or
  half) and whether that is better or worse. When both sides have ranges, a *Disjoint* column
  says whether they overlap.
- **Change bars:** each metric's change on one log scale, better to the right whichever way the
  metric improves. A change whose ranges overlap is drawn faint.
- **Before and after bars:** each metric on its own scale, with its range as a whisker.
- **Sweep lines:** a metric with `points`, both sides across its x. An axis that spans more than
  fifty times is drawn on a log scale.
- **On a group,** each metric that two or more of its members measure (matched by `key`),
  followed across them in plan order. Each point opens its member.

The charts answer the pointer the way a dashboard's do:

| On a chart | |
|---|---|
| Hover | a tooltip with each series' figure; on a line chart, a crosshair snapped to the nearest point |
| Hover a metric, in its table row or a bar | that metric lights up in the table and in both bar charts |
| Click a legend entry | show that series alone; click again to show them all |
| Ctrl- or cmd-click a legend entry | hide or show just that series |
| Drag across a line chart | zoom to that x range; the y axis refits |
| Double-click, or *Reset zoom* | zoom back out |
| Click a group's trend | open the member under the crosshair |

A node with no `performance` key still gets the section when one of its docs has measurements:

- **A subsection named in `performance-sections`:** that subsection.
- **Failing that, for a doc link to a heading:** that section's tables that have a header row,
  each captioned by the bold text that opens the paragraph before it. This skips the headerless
  `| | |` table an optimization entry opens with.

Like a doc read in the sidebar, it is fetched only when the sidebar shows it, and struck-through text is left out.
The build checks `performance` and reports, with `file:line`:

- a metric with no figures;
- a range that is not `[median, lo, hi]` in order;
- a `before` of zero.

## Drawing a view

````markdown
```reef
root = "scans"          # this group's members (default: the whole plan)
focus = "m23"           # or: this group open, with its outside neighbours faded around it
direction = "LR"        # TB (default) or LR
expand = ["m23steps"]   # groups open at first; or "all" / "none"
height = "80vh"
page = "../plan.md"     # a "Full page" button linking to that chapter
```
````

A fence with `layout = "page"` fills the page instead of the text column. mdbook keeps its
pages 750px wide, and a large plan needs more than that. Such a view takes everything to the
right of the book's sidebar and is as tall as the window. The sidebar stays, so readers leave
the page the way they leave any other. A plain scroll zooms there, and the prev/next arrows are
hidden. Give the view a chapter of its own and link to it from the embedded views with `page`:

````markdown
# The scans plan

```reef
root = "scans-plan"
layout = "page"
```
````

A page holds at most one `layout = "page"` view, and it takes no `height`.

Opening and closing groups is remembered for each page in the browser.

| In the diagram | |
|---|---|
| Click a box | open its sidebar |
| ▸ / ▾, or double-click a collapsed group | open or close a group |
| Drag | pan |
| Ctrl + scroll, or pinch | zoom (plain scroll zooms in full screen and in `layout = "page"`) |
| `/` | search by id or title; a match inside a closed group opens it |
| Hover | lights up everything upstream and downstream |
| `+` `-` `0` `f` `Esc` | zoom in, zoom out, fit, full screen, close the sidebar |

## Commands

```sh
mdbook-reef check [book]     # the build's checks without building, for CI; exit 1 on problems
mdbook-reef import plan.md --out src/plans/scans [--fence N] [--root id]
```

`import` turns a mermaid flowchart (by default the largest one in the file) into reef nodes:

- each subgraph becomes a group file, and the boxes outside any subgraph become a root group;
- `-->` becomes `needs` and `-.->` becomes `helps`;
- `:::done` or ✅ means done, and `:::optional` or "(optional)" means optional;
- a box whose label starts with an id the book has a heading for (`F125`, `O147`, `X17`,
  `Item 346` and so on) gets that heading as its doc;
- an id defined twice with different labels becomes two nodes, and the import reports it.

## Layout

The layout is layered (Sugiyama-style), done group by group from the inside out. Each open group
is laid out on its own and then becomes one box in its parent's layout. An arrow between groups
is laid out at the innermost level that holds both ends. On each level:

- back edges are reversed to break cycles;
- boxes are layered by longest path, with sources pulled down to just above what they feed;
- long edges get dummy boxes;
- barycenter sweeps order each layer, keeping the order with the fewest crossings;
- weighted isotonic regression places the boxes along each layer, which keeps their order and
  gaps and straightens long edges.

A group with no arrows among its members is packed in a grid.

## Building

```
crates/reef-model    nodes, the checked graph, views (shared by both sides)
crates/reef-layout   the layout, plain Rust, tested natively
crates/reef-view     the wasm viewer
crates/mdbook-reef   the preprocessor and CLI; embeds assets/reef.js and assets/reef.css
xtask                cargo xtask wasm: builds reef-view and bundles it into assets/reef.js
example-book         shoal's scans plan, migrated: a proof of concept on a real 190-box plan
```

```sh
cargo install wasm-bindgen-cli --version 0.2.130   # once; must match the wasm-bindgen crate
cargo xtask wasm                                   # after changing reef-view, reef-model or reef-layout
cargo test --workspace
cd example-book && mdbook serve
```

`assets/reef.js` is generated, but it is committed so that `mdbook-reef` builds without the wasm
mdbook-reef skill [book]     # write the Claude skill to the repo's .claude/skills/reef (--print to show it)
toolchain.
