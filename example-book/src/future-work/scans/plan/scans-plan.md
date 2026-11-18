---
id: "scans-plan"
title: "What's left to do"
collapsed: false
nodes:
  - id: "Contract"
    title: "Agree the contract, P20 to P26"
    status: done
    description:
      why: "A scan reads a table while it is written, from copies that move and fail; what it promises had to be stated as checkable properties before any frame or reader was built."
      what: "P20 to P26 are agreed as written, each beside the violation that breaks it and the tests that own it, for the scan oracle to judge from M22."
  - id: "Bench"
    title: "The bench's scan kind and scan event"
    status: done
    description:
      why: "Every scans gate is judged by a measurement, yet `shoal-loadgen` could neither time a scan's own throughput nor run one beside a workload, so no gate's exit could be taken."
      what: "`shoal-loadgen` times a whole scan page by page as an operation kind, and its `scan` event runs one beside a workload, cutting the run into before, during and after."
  - id: "X16"
    title: "X16 The walk beside the foreground"
    status: done
    docs: ["../walk-beside-the-foreground.md"]
    description:
      why: "Nobody had measured what the walk as it stands costs a foreground's tail beside a scan, so the plan could not tell whether its rebuild or its pacing comes first (Q33, Q34)."
  - id: "X20"
    title: "X20 ScyllaDB's scan path at the source"
    status: done
    optional: true
    docs: ["../scylladb-scan-sources.md"]
    description:
      why: "The plan copies five of ScyllaDB's scan mechanisms from W3's readings of its default branch and posts, some only recalled, and nobody had measured what its own full scan costs a foreground."
  - id: "X37"
    title: "X37 A tablet's records together, priced"
    status: done
    docs: ["../tablet-records-together.md"]
    description:
      why: "Nobody knew what writing a merge in key order costs, or whether a tablet's records stay together enough to read ten partitions a device read, W12's cost budget, so Q62 stayed open."
  - id: "Shed"
    title: "O139 A shed stream moves on"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o139-a-scan-driver-asks-a-shed-tablet-again-where-another-tablet-would-be-answered"]
  - id: "Q62"
    title: "Q62 decided"
    status: done
    needs: ["X37"]
    description:
      why: "M23's cost budget is met only where a scan reads a tablet's records in runs, and whether the archive keeps them together, and at what cost to a merge, was unknown."
      what: "Merges written in key order are the layout and no blocks are consolidated: after an hour's churn a tablet's records still read 10.6 partitions a device read."
  - id: "X17"
    title: "X17 A page rebuilt: its reads"
    status: done
    needs: ["X16"]
    helps: ["Shed"]
    docs: ["../page-rebuilt.md"]
    description:
      why: "X16 found a walked partition costing up to 73 µs of a Zen1 host's cpu, about a device read each, outside W12's cost budget; which change to a page's reads closes that was unknown."
  - id: "X17b"
    title: "X17 A page rebuilt: plan, encodes, positions"
    status: done
    needs: ["X16"]
    docs: ["../page-rebuilt.md"]
    description:
      why: "X16 found a resident sorted table's captures holding a shard's loop up to 6.5 ms, and Q35 waited on what rows encoded on the shard cost against partitions sent whole."
  - id: "X18"
    title: "X18 Table reads paced by idle time"
    status: done
    needs: ["X16"]
    docs: ["../table-reads-paced.md"]
    description:
      why: "Once X17's walk in runs read past a device's default ceiling, the pace alone had to hold W7's budget, and which ceiling and trickle do so on each lab device was unknown (Q34)."
  - id: "M22"
    title: "M22 The harness and the facts"
    status: done
    needs: ["Contract", "Bench", "X16"]
    docs: ["../milestones.md#m22-the-harness-and-the-facts"]
    description:
      why: "No scan could be judged: nothing checked a scan against P20 to P22, the fixture could not drive one, and what the walk costs the reference cell was unmeasured."
  - id: "SortKeys"
    title: "Sort keys of any Ord type, Resolved 207, F131"
    status: done
    optional: true
    description:
      why: "A sort key could only be a `String`, so a schema choosing a sort key over a declared order could not order a partition's rows by an integer or a time without zero-padding."
      what: "Any `SortKey` works, the integers, `Uuid`, tuples, a schema's own type and chrono's times among them, and SHQL can name a composite key."
  - id: "I241"
    title: "Item 241: a sorted partition digested without its archive"
    status: done
    optional: true
    docs: ["../../../appendix/resolved/sorted-overlay-digested-whole.md"]
  - id: "M23"
    title: "M23 The walk rebuilt for load"
    status: done
    needs: ["Clauses"]
    helps: ["SortKeys", "I241"]
    docs: ["../milestones.md#m23-the-walk-rebuilt-for-load"]
    description:
      why: "The walk as it stood held neither W7's budget beside a scan nor W12's cost budget, and isolation had to be proven in process before any client could scan."
  - id: "Q35"
    title: "Q35 What a scan's answer carries"
    status: done
    description:
      why: "M24 fixes what a scan's answer carries on the wire, and X17 found rows encoded on the shard cost it 1.3 to 2.9 times what partitions cost a client."
      what: "An answer carries rows encoded on the shard by default, and partitions for an unfiltered, unprojected scan that asks, each page's rows handed on only once its answer is whole and validated."
  - id: "Q36"
    title: "Q36 The cursor and the frames"
    status: done
    description:
      why: "M24 could not build its frames, client driver or resume token until what a cursor holds and the bytes of `ScanPage` and `ScanAnswer` were fixed."
      what: "A page's progress is one key, cursors are keys with a sort-key position reserved, floors are kept a group, and `ScanPage` and `ScanAnswer` have fixed heads of 96 and 80 bytes."
  - id: "Whole"
    title: "Whole partitions in answers"
    status: done
    description:
      why: "Q35 let an unfiltered, unprojected scan take partitions, since X17 found rows encoded on the shard cost it more than they spare a client; built with M24, it needs no later wire version."
      what: "A scan that asks with `.partitions()` gets its answer's body as archived partitions under `PARTITION_LAYOUT`, written by the shard and decoded by the client."
  - id: "X19"
    title: "X19 The edges"
    status: done
    needs: ["X17", "X17b"]
    docs: ["../the-edges.md"]
    description:
      why: "Nobody knew what a scan costs at its edges, so M24 and M26 could not say what to bound, nor whether M24 must split a sorted partition too large for one page."
  - id: "Bounded"
    title: "Large sorted partitions as bounded records"
    status: done
    needs: ["X19"]
    description:
      why: "X19 found a large sorted partition's whole record held its shard's thread 7.12 ms at 64 MiB with no yield, and M24's split across pages needs bounded records to cut at."
      what: "The merges write a large sorted partition as records of at most 1 MiB, so a page verifies and folds one bounded record a step and a get reads them in place."
  - id: "I373"
    title: "Item 373: a page holds its shard through a whole record's verify"
    status: done
    needs: ["Bounded"]
    docs: ["../../../appendix/resolved/whole-record-holds.md"]
  - id: "O162"
    title: "O162 A walked record handed out from its read buffer"
    status: done
    needs: ["X19"]
    docs: ["../../../appendix/optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it"]
  - id: "Floor"
    title: "A scan budget of at least the largest row"
    status: done
    needs: ["X19"]
    description:
      why: "The user ruled no partition may fail a scan, yet a row that cannot be split would be refused for its size by a scan budget smaller than the largest row."
      what: "A node's scan budget is never less than its request body bound and a row's slack, about 64 MiB at the defaults, so any row fits alone and waits its turn."
  - id: "M24"
    title: "M24 Scans on one node"
    status: done
    needs: ["m24steps"]
    docs: ["../milestones.md#m24-scans-on-one-node"]
    description:
      why: "Nothing a client could call scanned a table: there were no scan frames on the wire, no client driver, no SHQL `SCAN` and no `scan` privilege."
  - id: "X38"
    title: "X38 Which copies a scan reads, two ways"
    status: done
    needs: ["M24"]
    docs: ["../copies-two-ways.md"]
    description:
      why: "Q38's design left open which of a tablet's voter copies a scan prefers, followers then leaders or every voter alike, and the user asked for it measured before M25."
  - id: "Q38"
    title: "Q38 Which copies a scan reads, and its levels"
    status: done
    needs: ["X38"]
    description:
      why: "M25 reads a scan across a cluster's copies, so which copies a scan may read and prefer, and how `One` and `Quorum` are served, had to be settled first."
      what: "A scan reads voter copies alone, followers then leaders; its level is resolved once at its start, a floor waited for at most 250 ms, and `Quorum` takes one barrier a group."
  - id: "Q39"
    title: "Q39 all includes scan, decided with the user"
    status: done
    description:
      why: "M24 reads `scan` from a standalone node's file and must say what `all` means there; including it widens every earlier grant of `all`, excluding it makes `all` mean less than everything."
      what: "`all` includes `scan`, so `all` is read, write and scan; cluster grants committed before as `read` and `write` stay those two, and `read` never implies `scan`."
  - id: "M25"
    title: "M25 Scans across a cluster"
    needs: ["m25steps"]
    docs: ["../milestones.md#m25-scans-across-a-cluster"]
    description:
      why: "A cluster node grants no scans, so nothing reads a cluster's copies under floors and read levels or carries a scan through a move, a failover or a restart."
  - id: "Apart"
    title: "A listing's fields kept apart from inline bytes"
    needs: ["X19"]
    description:
      why: "X19 found a listing of inline objects reads each object's whole row, 18,139 device bytes an entry at the default 16 KiB threshold, 58 times its fields."
      what: "A listing reads its entries' fields kept apart from their objects' inline bytes, so listing a bucket of small objects no longer costs reading their bytes."
  - id: "M26"
    title: "M26 Listing a bucket's objects"
    needs: ["M25", "Apart"]
    docs: ["../milestones.md#m26-listing-a-buckets-objects"]
    description:
      why: "A bucket's objects cannot be listed: R25's listing of every live object a page at a time, each with what a stat answers and none of its bytes, does not exist."
  - id: "X21"
    title: "X21 Declared order, priced"
    needs: ["M25"]
    docs: ["../spikes.md#x21-declared-order-priced"]
    description:
      what: "An order index's cost to a write, ordered pages covering and not, and how skewed placement values fall over 4096 tablets are measured, settling Q40 before M27."
  - id: "M27"
    title: "M27 Ordered scans by declared order"
    needs: ["X21"]
    docs: ["../milestones.md#m27-ordered-scans-by-declared-order"]
    description:
      why: "A partition key is hashed whole, scattering a placement value's partitions over all 4096 tablets, so no scan can read them in an order or between bounds (R22)."
  - id: "X22"
    title: "X22 The ordered listing, two ways"
    needs: ["M27"]
    docs: ["../spikes.md#x22-the-ordered-listing-two-ways"]
    description:
      what: "Q41 is settled: M28 lists in path order either on an ordered `ObjectMeta` over W8's index for new buckets, or on Q32's pending-then-complete index beside the existing layout."
  - id: "M28"
    title: "M28 The ordered listing"
    needs: ["M26", "X22"]
    docs: ["../milestones.md#m28-the-ordered-listing"]
    description:
      why: "M26's listing is unordered, with no prefix or delimiter, so no caller can list a bucket's objects in path order or by prefix."
---
# What's left to do

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.

```reef
root = "scans-plan"
```
