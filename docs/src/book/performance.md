# Performance

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
