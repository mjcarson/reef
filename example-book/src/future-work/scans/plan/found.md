---
id: "found"
title: "Found on the way, any time"
group: "scans-plan"
nodes:
  - id: "I338"
    title: "Item 338: a node below its resident floor evicts each check"
    status: done
    docs: ["../../../appendix/resolved/resident-floor-reserve.md"]
  - id: "I340"
    title: "Item 340: an archive the map forgot is never deleted"
    status: done
    docs: ["../../../appendix/resolved/orphan-archives-swept.md"]
  - id: "I349"
    title: "Item 349: a driver's shed page logged at ERROR"
    status: done
    docs: ["../../../appendix/resolved/shed-pages-logged-quietly.md"]
  - id: "I352"
    title: "Item 352: every core probed after each applied batch"
    status: done
    docs: ["../../../appendix/resolved/core-probes-off-the-loop.md"]
  - id: "I355"
    title: "Item 355: a scan's tablet shed for a minute, given up"
    status: done
    docs: ["../../../appendix/resolved/scan-pages-served-in-turn.md"]
  - id: "I357"
    title: "Item 357: a preload's inserts shed past every retry"
    status: done
    docs: ["../../../appendix/resolved/preload-sheds-waited-out.md"]
  - id: "I358"
    title: "Item 358: no control leader for 15 s under load"
    status: done
    docs: ["../../../appendix/resolved/control-answers-waited-for.md"]
  - id: "I359"
    title: "Item 359: the cell's own climb, not the scan's"
    status: done
    docs: ["../../../appendix/resolved/the-cells-own-climb.md"]
  - id: "I360"
    title: "Item 360: M23's exit row names no function"
    status: done
    docs: ["../../../appendix/resolved/lab-exit-rows.md"]
  - id: "I361"
    title: "Item 361: a missing archive fails every archive pass"
    status: done
    docs: ["../../../appendix/resolved/lost-archive-skipped.md"]
  - id: "I363"
    title: "Item 363: a copy that lost an archive is not repaired"
    docs: ["../../../appendix/known-issues.md#363-a-copy-that-lost-an-archive-is-not-repaired"]
  - id: "I365"
    title: "Item 365: a projection still needs its partition key"
    docs: ["../../../appendix/known-issues.md#365-a-projection-must-still-carry-its-tables-partition-key-which-two-pages-say-it-need-not"]
  - id: "I366"
    title: "Item 366: three wire comments wrong, a head unpinned"
    docs: ["../../../appendix/known-issues.md#366-three-comments-in-the-wires-code-say-what-it-does-not-and-one-head-is-pinned-by-no-test"]
  - id: "I367"
    title: "Item 367: the partitions micro bench does not compile"
    docs: ["../../../appendix/known-issues.md#367-the-partitions-micro-bench-does-not-compile"]
  - id: "I368"
    title: "Item 368: a B-tree's count unchecked by bytecheck"
    docs: ["../../../appendix/known-issues.md#368-an-archived-b-tree-whose-stored-count-is-below-its-entries-passes-bytecheck-and-overflows-rkyvs-iterator"]
  - id: "I364"
    title: "Item 364: a dropped client's last connect logs an ERROR"
    status: done
    docs: ["../../../appendix/resolved/dropped-client-connects-quietly.md"]
  - id: "I362"
    title: "Item 362: a bundle its access check refuses ended its node"
    status: done
    docs: ["../../../appendix/resolved/unreadable-bundle-ends-its-connection.md"]
  - id: "I370"
    title: "Item 370: a partition past half the scan budget shed for ever"
    status: done
    docs: ["../../../appendix/resolved/scan-partition-past-its-budget.md"]
  - id: "I371"
    title: "Item 371: a page over dead partitions never moved"
    status: done
    docs: ["../../../appendix/resolved/scan-page-over-dead-partitions.md"]
  - id: "I372"
    title: "Item 372: a table's encode steps held at a row or two"
    status: done
    docs: ["../../../appendix/resolved/encode-cost-relearned.md"]
  - id: "I374"
    title: "Item 374: pages held a read and waited for its copy"
    status: done
    docs: ["../../../appendix/resolved/scan-draws-held-and-waited.md"]
  - id: "I204"
    title: "Item 204: the stream budget a shard's, not a node's"
    status: done
    docs: ["../../../appendix/resolved/stream-budget-a-nodes.md"]
  - id: "O140"
    title: "O140 A resident read's tail beside inserts"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o140-a-resident-reads-p99-beside-the-cells-inserts-is-six-times-scylladbs-on-the-same-hosts"]
  - id: "O156"
    title: "O156 An integer sort key beside a padded string"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o156-an-integer-sort-key-priced-beside-a-padded-string"]
  - id: "O157"
    title: "O157 A node's heap on transparent huge pages"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o157-a-nodes-heap-on-transparent-huge-pages"]
  - id: "O158"
    title: "O158 What openraft holds of every node's floor"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o158-what-openraft-holds-of-every-nodes-floor"]
  - id: "O159"
    title: "O159 A segment's checkpoint holds a shard's loop"
    status: done
    docs: ["../../../appendix/optimizations.md#o159-a-compacted-segments-checkpoint-and-its-evictable-marking-hold-a-shards-loop-for-milliseconds"]
  - id: "O160"
    title: "O160 A shard's tick and a sealed segment"
    optional: true
    docs: ["../../../appendix/optimizations.md#o160-a-shards-tick-and-a-sealed-segment-hold-a-zen1-shards-loop-for-up-to-two-milliseconds"]
  - id: "O161"
    title: "O161 A WAL batch synced by a flush, not a FUA write"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o161-a-wal-batch-is-made-durable-by-a-flush-where-a-fua-write-would-do"]
  - id: "O163"
    title: "O163 A partition refused only once it is read"
    optional: true
    docs: ["../../../appendix/optimizations.md#o163-a-partition-the-scan-budget-cannot-hold-is-read-or-encoded-before-it-is-refused"]
  - id: "I375"
    title: "Item 375: a page of 64 MiB delays a get past 1 ms once a page"
    docs: ["../../../appendix/known-issues.md#375-a-scans-page-of-a-64-mib-partition-still-delays-a-get-on-its-shard-past-a-millisecond-about-once-a-page"]
  - id: "O164"
    title: "O164 A partition past 128 records grows them"
    optional: true
    docs: ["../../../appendix/optimizations.md#o164-a-partition-past-128-bounded-records-is-cut-into-128-larger-ones"]
  - id: "O165"
    title: "O165 A get validates its pieces in one call"
    optional: true
    docs: ["../../../appendix/optimizations.md#o165-a-gets-load-of-a-partition-in-pieces-validates-every-piece-in-one-handler-call"]
  - id: "O166"
    title: "O166 A snapshot cut folds a partition in pieces"
    optional: true
    docs: ["../../../appendix/optimizations.md#o166-a-snapshot-cut-and-an-export-fold-a-partition-in-pieces-into-one-record"]
  - id: "O167"
    title: "O167 A merge rewrites every piece"
    optional: true
    docs: ["../../../appendix/optimizations.md#o167-a-merge-rewrites-every-piece-of-a-large-partition-it-merges-whole"]
  - id: "I376"
    title: "Item 376: dead archived bytes counted toward no bound"
    status: done
    docs: ["../../../appendix/resolved/dead-archived-bytes-counted.md"]
  - id: "O168"
    title: "O168 A fold over fragments holds every record"
    optional: true
    docs: ["../../../appendix/optimizations.md#o168-a-fold-over-fragments-holds-every-record-of-its-partition-until-its-last-piece-is-made"]
  - id: "O169"
    title: "O169 A paced walk's reads take longer in flight"
    optional: true
    docs: ["../../../appendix/optimizations.md#o169-a-paced-walks-reads-into-fresh-heap-buffers-take-longer-in-flight"]
  - id: "O172"
    title: "O172 A split partition under an overlay reads more than its page"
    optional: true
    docs: ["../../../appendix/optimizations.md#o172-a-partition-handed-out-part-way-under-an-overlay-or-fragments-reads-more-than-its-page"]
  - id: "I377"
    title: "Item 377: a resident row past half the scan budget refused"
    docs: ["../../../appendix/known-issues.md#377-a-resident-or-overlaid-row-past-half-the-scan-budget-is-refused-by-name"]
  - id: "I378"
    title: "Item 378: a row past its node's body bound refusable"
    docs: ["../../../appendix/known-issues.md#378-a-row-past-its-nodes-request-body-bound-can-still-be-refused-by-a-scan"]
  - id: "I379"
    title: "Item 379: a bench's restart timed on a survivor"
    docs: ["../../../appendix/known-issues.md#379-a-benchs-kill-or-stop-event-times-its-nodes-catch-up-on-a-member-that-stayed-up"]
  - id: "I381"
    title: "Item 381: a get that named no key was never answered"
    status: done
    docs: ["../../../appendix/resolved/empty-get-refused.md"]
  - id: "I382"
    title: "Item 382: a scan in rows held its shard for the rows it dropped"
    status: done
    docs: ["../../../appendix/resolved/dropped-rows-stepped.md"]
  - id: "I383"
    title: "Item 383: a page given up for no progress was never answered"
    status: done
    docs: ["../../../appendix/resolved/stalled-page-answered.md"]
  - id: "I384"
    title: "Item 384: a scan's patience counted its caller's own pauses"
    status: done
    docs: ["../../../appendix/resolved/scan-patience-counts-its-own-wait.md"]
  - id: "I385"
    title: "Item 385: a page looking for its position read past its work"
    status: done
    docs: ["../../../appendix/resolved/position-search-bounded.md"]
  - id: "O173"
    title: "O173 A record validated up to three times on a page that cuts it"
    optional: true
    docs: ["../../../appendix/optimizations.md#o173-a-record-is-validated-up-to-three-times-on-a-page-that-cuts-it"]
  - id: "O174"
    title: "O174 A loop round trip for every tablet of a page"
    optional: true
    docs: ["../../../appendix/optimizations.md#o174-a-page-takes-one-round-trip-of-its-shards-loop-for-every-tablet-that-holds-anything"]
  - id: "O175"
    title: "O175 A body in partitions allocates twice a partition"
    optional: true
    docs: ["../../../appendix/optimizations.md#o175-a-body-in-partitions-allocates-twice-a-partition-and-its-frames-are-cut-by-a-search-from-its-first-part"]
  - id: "O176"
    title: "O176 A scan's spec copied a page and read again a tablet"
    optional: true
    docs: ["../../../appendix/optimizations.md#o176-a-scans-spec-is-copied-for-every-page-and-read-again-for-every-tablet"]
  - id: "O177"
    title: "O177 Small copies a chunk and a row, shoalctl's rows formatted again"
    optional: true
    docs: ["../../../appendix/optimizations.md#o177-small-copies-a-chunk-and-a-row-in-a-scans-answer-and-shoalctl-formats-every-row-again-at-each-page"]
  - id: "O178"
    title: "O178 A client's connection probes small answers for no loss"
    optional: true
    docs: ["../../../appendix/optimizations.md#o178-a-clients-connection-probes-three-in-a-hundred-of-its-small-answers-for-a-loss-that-did-not-happen"]
  - id: "O179"
    title: "O179 A shard holds its cpu through its slice while its fsync waits, not taken"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o179-a-shard-holds-its-cpu-through-its-eevdf-slice-while-the-io-wq-worker-running-its-fdatasync-waits"]
  - id: "O180"
    title: "O180 A paced resident scan's encode steps past their bound"
    optional: true
    docs: ["../../../appendix/optimizations.md#o180-a-paced-resident-scans-encode-steps-run-past-their-bound-on-their-own-cpu-about-once-in-ninety"]
  - id: "O181"
    title: "O181 A walk of one tablet in no key order reads its map ahead"
    optional: true
    docs: ["../../../appendix/optimizations.md#o181-a-walk-of-one-tablet-in-no-key-order-reads-its-maps-sixteen-pages-ahead-for-its-next-walk-to-miss"]
  - id: "O182"
    title: "O182 A client's default of one scan stream a shard"
    optional: true
    docs: ["../../../appendix/optimizations.md#o182-a-clients-default-of-one-scan-stream-a-shard"]
  - id: "O183"
    title: "O183 A rows page shed at a chunk cuts back further than it needs"
    optional: true
    docs: ["../../../appendix/optimizations.md#o183-a-page-in-rows-shed-at-a-chunk-cuts-back-further-than-it-needs"]
---
# Found on the way, any time

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
