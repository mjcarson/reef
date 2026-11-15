---
id: "scans-plan"
title: "What's left to do"
collapsed: false
nodes:
  - id: "Contract"
    title: "Agree the contract, P20 to P26"
    status: done
  - id: "Bench"
    title: "The bench's scan kind and scan event"
    status: done
  - id: "X16"
    title: "X16 The walk beside the foreground"
    status: done
    docs: ["../walk-beside-the-foreground.md"]
  - id: "X20"
    title: "X20 ScyllaDB's scan path at the source"
    status: done
    optional: true
    docs: ["../scylladb-scan-sources.md"]
  - id: "X37"
    title: "X37 A tablet's records together, priced"
    status: done
    docs: ["../tablet-records-together.md"]
  - id: "Shed"
    title: "O139 A shed stream moves on"
    status: done
    optional: true
    docs: ["../../../appendix/optimizations.md#o139-a-scan-driver-asks-a-shed-tablet-again-where-another-tablet-would-be-answered"]
  - id: "Q62"
    title: "Q62 decided"
    status: done
    needs: ["X37"]
  - id: "X17"
    title: "X17 A page rebuilt: its reads"
    status: done
    needs: ["X16"]
    helps: ["Shed"]
    docs: ["../page-rebuilt.md"]
  - id: "X17b"
    title: "X17 A page rebuilt: plan, encodes, positions"
    status: done
    needs: ["X16"]
    docs: ["../page-rebuilt.md"]
  - id: "X18"
    title: "X18 Table reads paced by idle time"
    status: done
    needs: ["X16"]
    docs: ["../table-reads-paced.md"]
  - id: "M22"
    title: "M22 The harness and the facts"
    status: done
    needs: ["Contract", "Bench", "X16"]
    docs: ["../milestones.md#m22-the-harness-and-the-facts"]
  - id: "SortKeys"
    title: "Sort keys of any Ord type, Resolved 207, F131"
    status: done
    optional: true
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
  - id: "Q35"
    title: "Q35 What a scan's answer carries"
    status: done
  - id: "Q36"
    title: "Q36 The cursor and the frames"
    status: done
  - id: "Whole"
    title: "Whole partitions in answers"
    status: done
  - id: "X19"
    title: "X19 The edges"
    status: done
    needs: ["X17", "X17b"]
    docs: ["../the-edges.md"]
  - id: "Bounded"
    title: "Large sorted partitions as bounded records"
    status: done
    needs: ["X19"]
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
  - id: "M24"
    title: "M24 Scans on one node"
    status: done
    needs: ["m24steps"]
    docs: ["../milestones.md#m24-scans-on-one-node"]
  - id: "X38"
    title: "X38 Which copies a scan reads, two ways"
    status: done
    needs: ["M24"]
    docs: ["../copies-two-ways.md"]
  - id: "Q38"
    title: "Q38 Which copies a scan reads, and its levels"
    status: done
    needs: ["X38"]
  - id: "Q39"
    title: "Q39 all includes scan, decided with the user"
    status: done
  - id: "M25"
    title: "M25 Scans across a cluster"
    needs: ["m25steps"]
    docs: ["../milestones.md#m25-scans-across-a-cluster"]
  - id: "Apart"
    title: "A listing's fields kept apart from inline bytes"
    needs: ["X19"]
  - id: "M26"
    title: "M26 Listing a bucket's objects"
    needs: ["M25", "Apart"]
    docs: ["../milestones.md#m26-listing-a-buckets-objects"]
  - id: "X21"
    title: "X21 Declared order, priced"
    needs: ["M25"]
    docs: ["../spikes.md#x21-declared-order-priced"]
  - id: "M27"
    title: "M27 Ordered scans by declared order"
    needs: ["X21"]
    docs: ["../milestones.md#m27-ordered-scans-by-declared-order"]
  - id: "X22"
    title: "X22 The ordered listing, two ways"
    needs: ["M27"]
    docs: ["../spikes.md#x22-the-ordered-listing-two-ways"]
  - id: "M28"
    title: "M28 The ordered listing"
    needs: ["M26", "X22"]
    docs: ["../milestones.md#m28-the-ordered-listing"]
---
# What's left to do

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.

```reef
root = "scans-plan"
```
