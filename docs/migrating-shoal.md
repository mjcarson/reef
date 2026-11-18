# Moving shoal's planning docs to reef

This note is a proposal; nothing in shoal has been changed. It was tried on a scratch copy of
`shoal/docs`, where the scans plan's whole-plan diagram imported cleanly:

- **What came in:** 190 boxes, 11 subgraphs and 125 arrows. 140 boxes were linked to the
  heading they are numbered by.
- **Checks:** `mdbook-reef check` found no problems.
- **Viewer timings:** the diagram drew with its groups collapsed (43 boxes), and laid out in about
  20 ms with every group open (201 boxes).
- **One real defect found:** `Pace` is defined twice in the mermaid source, once as W1's pacing
  row and once as F147. Mermaid silently merges the two into one box.

## The proof of concept in `example-book`

`example-book` is this migration carried out on a copy of the scans plan from shoal's
`ScanSupport` branch. It holds:

- the scans pages, plus the 19 feature pages and 48 appendix pages the plan's boxes link to
  (98 pages in all);
- the import, with `Pace_2` renamed `F147`;
- all eight mermaid diagrams on *What's left to do* replaced by reef views;
- a full-width page (`layout = "page"`) for the whole plan.

`mdbook-reef check` finds no problems: 202 nodes, 125 arrows, 10 fences.

Each of the 190 work items says why it is needed and what finishing it will do. Where shoal's
docs say it, the sidebar takes it from them: feature pages give *Context* and *What it does*,
resolved items *Symptom* and *The fix*, and optimization entries their *Impact* row. A doc's
opening paragraph is never taken, since in shoal's docs it is usually a status line
("Reported 2026-10-29. This is the record of spike X16"). Where the docs don't say a part, it
is written in the node's `description`. In the PoC, 53 work items take both parts from their
docs, and 137 have at least one written: 82 whys and 137 whats in all. That includes every
spike, milestone and box with no doc, such as "The exit's quick passes". 65 boxes also get
design choices from their docs. This adds about 325 KB of data to each page that draws the
plan.

How the eight diagrams became views:

| Section | View |
|---|---|
| Before the first gate | `focus = "w1"` |
| The gates | `root = "scans-plan"`, `expand = "none"`: each milestone's group is one box |
| Finishing M23, M24, M25 | `focus = "m23steps"` / `"m24steps"` / `"m25steps"` |
| Finishing After M24 | `focus = "after24"`, `expand = ["review24"]` |
| The work beside the gates | `focus = "found"` |
| The whole plan | `root = "scans-plan"`, with a link to the full-width page |

Gaps the PoC found:

- **Parts with no group of their own.** *Before the first gate* drew a "can start now"
  subgraph, and *The gates* drew only the milestones and spikes (19 boxes). The whole-plan
  diagram has no such groups, so `focus = "w1"` and the collapsed whole plan (43 boxes) only
  come close. A `now` group, or a `gates` group holding the milestone boxes, would make them
  match.
- **Wide focused views.** A focused group laid out LR (M23's 23 steps, say) starts at a
  readable zoom and needs panning in the 750px column. The *Full page* button is the remedy.

## Steps

1. `mdbook-reef install docs`
2. Import each plan's whole-plan diagram:

   ```sh
   mdbook-reef import docs/src/future-work/scans/whats-left-todo.md --out docs/src/future-work/scans/plan --root scans-plan
   ```

   Review the result. The importer writes one file per subgraph, and the boxes outside any
   subgraph go in `scans-plan.md`. Give `Pace_2` a real id such as `F147`.
3. Replace each mermaid fence on `whats-left-todo.md` with a view of the same graph:
   - the whole plan: ```` ```reef ```` with `root = "scans-plan"`;
   - *Finishing M23*: `focus = "m23steps"`;
   - *Finishing M24*: `focus = "m24steps"`, and so on for the other parts.

   The part diagrams and the whole plan then can no longer disagree.
4. Promote a box to a file of its own when it deserves a page: move its entry out of `nodes:` into
   a file with the same frontmatter, and add the file to SUMMARY.md if it should be in the nav.
5. Add `mdbook-reef check docs` to CI next to `mdbook build`.

## Proposed *Planning Docs* rules for CLAUDE.md

**Every planning doc keeps the order of its work as reef nodes**, in the same change that writes
the plan. A plan's nodes live under its directory: a file per feature or milestone, and its small
items in that file's `nodes:` list. Each part of the plan page draws a view of them
(```` ```reef ```` with `focus = "<group>"`), and its last section draws the whole plan
(`root = "<plan>"`).

1. **Every piece of work is a node**, spikes and lab work included. Split a spike where only part
   of it waits on something.
2. **`needs` lists what a piece of work requires.** Name a group to need all of it. Name only
   direct requirements; reef draws the rest.
3. **Mark what is done.** Set `status: done` in the same change that finishes the work. Groups
   and readiness follow on their own.
4. **Mark what is optional** with `optional: true`, and use `helps` rather than `needs` for what it
   feeds. Optional work that feeds nothing goes in an "Optional, any time" group.
5. **Link the docs.** A node's `docs` points at its feature page, its milestone, or its
   known-issues or optimizations entry. `mdbook-reef check` fails on a link that does not
   resolve.
6. **A larger feature is drawn down to its steps** as a group with a member for each step, its
   exit and what is left. A decision that adds, moves or drops work changes the nodes in the same
   change that records it.
7. **Every bug and optimization found on the way is a node** with its number in its title
   (`Item 350: ...`, `O147 ...`), added in the change that files it, in the group it blocks or
   builds on, or in "Found on the way, any time". It is never removed; when it is fixed, its
   status turns `done`.
8. **Run `mdbook-reef check docs`** before committing a plan change.

The rules about `#` in labels, holding level with link length, and keeping a level under six
boxes no longer apply. Labels are plain text, layering is automatic, and wide levels zoom.
