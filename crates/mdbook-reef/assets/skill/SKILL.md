---
name: reef
description: Keep a reef planning graph (an mdbook with `[preprocessor.reef]`) in step with the work. Use when planning work, before starting or after finishing a feature, milestone, spike, fix or optimization, when running benchmarks whose results belong in the plan, when a decision adds, moves or drops work, or when editing markdown with an `id:` in its frontmatter or a ```reef fence.
---

# reef: keeping the plan in step with the work

reef draws a book's plan as a directed graph. **The markdown is the source of truth**: every
box is a node written in YAML frontmatter, and every ```reef fence draws part of the one graph.
If the frontmatter is wrong, every diagram is wrong. Update it in the **same change** as the
work it describes, never as a follow-up.

The full key reference is in [reference.md](reference.md). Read it before writing
`description`, `acceptance` or `performance`, or a fence.

## 1. Find the plan

- The book is the directory whose `book.toml` has `[preprocessor.reef]`. Nodes live under its
  `src/` (only in `roots` if `[preprocessor.reef] roots = [...]` is set).
- A node is any markdown file whose frontmatter has an `id`, or an entry in some node's
  `nodes:` list. To find them: `grep -rn '^id:\|^  - id:' <book>/src`.
- `mdbook-reef check <book>` prints the totals, including how many items are ready to start.
- `mdbook-reef next <book>` lists the work ready to start, in plan order, and for each item
  that waits, the needs (its own and its groups') that are not done yet. Take the next piece of
  work from it rather than from grepping frontmatter.
- Before starting work, find its node by id or title. Read its `needs`, its group, and its
  `docs` page: they say what the work is gated on and how it was meant to be done.

## 2. Before starting: draw the work

If the work has no node, add one.

- **Where:** follow how the group's siblings are written. Usually that is an inline entry in the
  group's `nodes:` list, with `docs` pointing at the work's own page if it has one. Use a node
  file of its own (with `group:`) only where the siblings do. Don't make up a design page just
  to have something to link: with no page, leave out `docs`.
- **Which group:** adding work to a group whose members are all done reopens it, and everything
  that needs that group waits again. Do that only when the work really gates what comes after;
  otherwise put it in the open group that comes next, or a new group.
- **`id`:** unique across the whole book. Follow the book's existing scheme (look at sibling
  ids, e.g. `F125`, `O146`, `Item 346` → `I346`).
- **`title`:** short. Keep any id prefix the book uses (`F125 Archive reads in runs`).
- **`needs`:** only what directly gates it. Skip an arrow a longer path already implies.
  Naming a group means all of it. Never name the node's own group.
- **`helps` and `optional: true`:** for work that helps but does not gate.
- **`docs`:** link the design or feature page, relative to this file. Add a `#heading` anchor
  only if the page holds more than this work, and only one that exists. The sidebar shows that
  section, and takes the design choices from its "Design choices" section, or write `decisions`
  in frontmatter.
- **`description`:** always write `why` and `what`, even when the node links a doc. The build
  fails on work that has neither written nor found in its doc, and a doc's opening paragraph
  rarely says either plainly. One or two plain sentences each, about 30 words at most:
  - `why`: the problem or gap. What is slow, wrong, missing or unknown without this work, or for
    a spike, what the plan can't decide until it is answered.
  - `what`: what will be true or possible once it is done. The outcome, not the steps.
  - Name the specific thing (the shard's loop, a scan's page), never "improves performance".
  - `details` holds anything longer. Write only what is known: no invented figures.
- **`acceptance`:** write the criteria now, before the code: how will it be known to be done?
  One per line, each `unit`, `integration`, `performance` or `feature`. Add `test:` with the
  test's path once the test exists.
- **Performance targets before there are figures:** the check requires a criterion's `metric` to
  be a metric in `performance` that has figures. So until the work is measured, write the target
  as text only, naming the metric's key: `perf: "A repeated scan answers in at most 2 ms (repeat-scan-ms)"`.
  Add `metric:` and `at-most:`/`at-least:` in the change that records the figures (step 4).
- **`status`:** leave it `todo` (the default) while the work is only planned; set
  `status: in-progress` in the change that starts the code.

## 3. While working

- A decision that adds, moves or drops work changes the graph in the same change that records
  the decision: add the node, change its `needs`, or delete it.
- Split a node when only part of it waits on something.
- New work found along the way (a bug, a follow-up) gets its own node, not a note in a body.
- Use `status: blocked` when it is stuck, and say on what in the body.

## 4. When benchmarks are run

Record the figures in the node's `performance` key: `before` and `after` for each metric, as a
number or `[median, lo, hi]`, with `better: higher|lower`. Give the same `key` to the same
metric on other nodes so a group can draw its trend. A sweep uses `points: [[x, before, after], ...]`.
Say how they were measured in `note` (what, how many rounds, on what), and name the two sides
with `labels` when "Before"/"After" says too little. Never invent figures: only write what was
measured. Then turn any text-only performance criterion into one with `metric` (the metric's
`key`, or its `name`) and its `at-most`/`at-least` target, so whether it is met is derived.

## 5. When finishing

In the same change that finishes the work:

- `status: done` on the node. Never set a status on a group: a group is done when its members
  are, and "ready" is derived from `needs`.
- Each acceptance criterion gets `status: met` or `status: unmet`. Leave a performance criterion
  with a `metric` and `at-least`/`at-most` alone: its status is derived from the `after` figure.
- If the result moved later work (it unblocked, dropped or added something), update those nodes.
- If what was built differs from what was planned, update `description.what` to say what it does.

## 6. Always check

After every edit to the plan, run:

```sh
mdbook-reef check <book>
```

It exits non-zero and prints `file:line` for unknown ids, cycles, a node that needs its own
group, duplicate ids, doc links to missing files or headings, work with no `why` or `what`, and
malformed `description`, `acceptance` or `performance`. Fix every problem before you finish. If `mdbook-reef` is not on the PATH, the
`command` in `book.toml`'s `[preprocessor.reef]` says how the book runs it.

## Don't

- Don't edit `reef.js` or `reef.css` beside `book.toml`: they are generated by
  `mdbook-reef install`.
- Don't draw the plan as a mermaid flowchart; add nodes and a ```reef fence.
- Don't strike or delete a finished node: mark it done, so the plan shows what was done.
