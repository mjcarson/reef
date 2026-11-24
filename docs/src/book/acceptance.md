# Acceptance criteria

A node's `acceptance` key lists how the work will be known to be done. Each criterion is one of
four kinds, each with its own colour: **unit test**, **integration test**, **performance** and
**feature**.

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
    at-least: 6000                                    # or at-most: checked against the metric's after
  - perf: { text: Every metric's change, chart: change }   # a whole chart: change | pair
```

- **Kinds:** `unit` (`unit-test`), `integration` (`integration-test`, `e2e`), `performance`
  (`perf`, `bench`), `feature` (`feature-support`, `support`). The text is markdown.
- **Met or not:** what `status` says. Failing that, a performance criterion with a target is
  met when its metric's `after` median is within it (for a sweep, every point measured after);
  otherwise it is still to do.
- **Charts:** a performance criterion with a `metric` draws that metric under it: its sweep
  when it has `points`, else its before and after bars. The target is a dashed line. The chart
  answers the pointer like those in the Performance section.

The sidebar's Acceptance section lists each criterion with its kind's badge and ✓ met, ✗ not
met or ○ to do. On the diagram, a box with criteria shows a pill for each kind with how many are
met (`U 1/2`), filled once all are; a collapsed group counts its members'.

The chips in the toolbar and in the Acceptance section show or hide each kind. The choice
holds for every view in the book and is remembered in the browser. The build checks, with
`file:line`:

- a criterion with no text;
- a `metric` that is not in the node's `performance`, or a `node` that is not a node;
- a target with no `metric`, or both `metric` and `chart`, or a `chart` other than `change`
  or `pair`;
- a `metric`, `chart`, `node` or target on a criterion that is not `performance`.
