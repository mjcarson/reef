---
id: "after23opt"
title: "After M23, optional"
group: "scans-plan"
helps: ["M23"]
acceptance:
  - perf: { text: "The opt-in shared WAL mode at least doubles rw50's inserts a second", node: O151, metric: inserts, at-least: 7046 }
nodes:
  - id: "O144"
    title: "O144 A page's reads let go as they fold, moot"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o144-a-scan-page-holds-its-reads-beside-the-partitions-folded-from-them"]
  - id: "O145"
    title: "O145 Partitions together in a step"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o145-a-planned-partitions-encode-step-is-a-message-through-its-loop-each"]
  - id: "O150"
    title: "O150 No merge read of a row an insert replaces"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o150-a-merge-reads-the-archived-record-of-every-partition-an-insert-replaces-whole"]
  - id: "O151"
    title: "O151 WAL syncs bound by a loaded 970 EVO, opt-in"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o151-a-zen1-nodes-wal-syncs-are-bound-by-its-970-evos-flushes-once-the-drive-is-loaded"]
    performance:
      labels: [Buffered, Shared]
      note: "M23's cell alone (2026-11-09): medians and ranges of eight side-rounds, F138's build `3f87e3ff` on both sides. The mixed bench went the other way (shared 0.67 times the operations a second at 2.15 times the write p99), so `buffered` stays the default."
      metrics:
        - { name: "rw50, inserts a second", key: inserts, better: higher, before: [3523, 3476, 3547], after: [6837, 6783, 6915] }
        - { name: "rw50, insert p99", key: insert-p99, unit: ms, before: [17.2, 17.1, 17.3], after: [10.2, 10.1, 10.6] }
        - { name: "rw50, read p99", key: read-p99, unit: ms, before: [0.956, 0.921, 0.983], after: [1.58, 1.47, 1.63] }
        - { name: "insert100, inserts a second", better: higher, before: [3567, 3532, 3589], after: [6873, 6061, 6959] }
        - { name: "insert100, insert p99", unit: ms, before: [17.6, 17.5, 17.7], after: [11.1, 10.8, 14.2] }
    acceptance:
      - perf: { text: "Shared syncs carry at least 6,000 rw50 inserts a second", metric: inserts, at-least: 6000 }
      - perf: { text: "Reads keep their p99 under a millisecond", metric: read-p99, at-most: 1 }
      - perf: { text: "Every metric's change, side by side", chart: change }
      - unit: "`wal-sync = shared` parses, and `buffered` stays the default"
        status: met
      - integration: { text: "The rw50 cell runs to its end under both modes", test: "lab/cells/rw50", status: met }
      - feature: "The mode is opt-in, per node, from its config"
        status: met
  - id: "O152"
    title: "O152 A step's own cpu read only past its bound, not taken"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o152-a-steps-own-cpu-read-only-past-its-bound"]
  - id: "O153"
    title: "O153 Measuring logic behind a feature, not taken"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o153-always-on-measuring-logic-behind-a-feature-flag-measured"]
  - id: "O154"
    title: "O154 A capture's view allocates nothing on the loop"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o154-a-captures-view-allocates-nothing-on-the-loop"]
  - id: "O155"
    title: "O155 Two scan reads a device collapse a scan, a floor"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o155-two-scan-reads-a-device-collapse-a-scans-rate-ninefold"]
---
# After M23, optional

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
