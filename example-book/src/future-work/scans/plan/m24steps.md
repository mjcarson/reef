---
id: "m24steps"
title: "M24's steps and its exit"
group: "scans-plan"
needs: ["M23", "Q35", "Q36", "Q39", "Whole", "X19", "I373", "O162", "Floor"]
nodes:
  - id: "S1"
    title: "F140 The scan frames, the token, the grant's name"
    status: done
    docs: ["../../../features/scan-frames.md"]
  - id: "S2"
    title: "F141 A page over a shard's tablets, and its bounds"
    status: done
    needs: ["S1"]
    docs: ["../../../features/scan-pages-over-tablets.md"]
  - id: "S2b"
    title: "F142 A sorted partition split across pages"
    status: done
    needs: ["S2"]
    docs: ["../../../features/split-partitions.md"]
  - id: "S3"
    title: "F143 The answer in rows, a scan struct a table"
    status: done
    needs: ["S2b"]
    docs: ["../../../features/scan-rows.md"]
  - id: "S4"
    title: "F144 The page on the wire: refusals, cancel, the draw"
    status: done
    needs: ["S3"]
    docs: ["../../../features/scan-pages-on-the-wire.md"]
  - id: "S5"
    title: "F145 The client's driver: streams, tokens, split"
    status: done
    needs: ["S4"]
    docs: ["../../../features/scan-driver.md"]
  - id: "S6"
    title: "F146 SHQL's SCAN, shoalctl, the bench, the example"
    status: done
    needs: ["S5"]
    docs: ["../../../features/shql-scan.md"]
  - id: "Run24"
    title: "The workspace run, the counts, a security review"
    status: done
    needs: ["S6"]
    description:
      why: "M24's 12,400 lines let a client scan over the wire, so the gate waits on the whole workspace passing and a security review of what a client, a node and a token can do."
      what: "The workspace run counts 3,200 tests, its two failures a stale expectation corrected, and the review finds no way round the `scan` privilege and four defects, each fixed."
  - id: "AB24"
    title: "The reference cell before and after, on a Zen1 host"
    status: done
    needs: ["Run24"]
    description:
      why: "M24 adds a comparison in the read relay and an arm in the loop's dispatch, so every query's path changes and must be shown to cost a query nothing."
      what: "The reference cell on titan is within on every figure, 8,087 operations a second before M24 and 8,064 after, and so are its reads-only arm and the storage-free control."
  - id: "Exit24"
    title: "The exit's rounds on the lab"
    status: done
    needs: ["AB24"]
    description:
      why: "M24 is marked delivered only once the budget holds beside a client's scan over the wire to a standalone node on the lab, the cell on the scan's own client."
      what: "Eight rounds on titan and europa held the budget beside a scan of archived rows, and found it missed beside resident rows, the scan filling the link (item 390)."
  - id: "I390"
    title: "Item 390: a resident scan fills its client's link"
    status: done
    needs: ["Exit24"]
    docs: ["../../../appendix/resolved/resident-scan-fills-the-link.md"]
  - id: "F147"
    title: "F147 A scan's answers paced by its link"
    status: done
    needs: ["I390"]
    docs: ["../../../features/scan-link-pace.md"]
    description:
      what: "A node paces each scan's answers to a share of the link they leave by, 25% unless set, so a resident scan no longer fills its client's link; loopback is never paced."
  - id: "Sweep"
    title: "The share's sweep on the lab"
    status: done
    needs: ["F147"]
    description:
      why: "F147 paces a scan's answers to a share of its link, but which share holds the cell's p99s beside a resident scan, and whether the link was the exit's cause at all, was unknown."
      what: "Four rounds over shares from 12.5% to 75% show the pace takes off what the link added; no share holds the budget, and 25% is the default, chosen with the user."
  - id: "I391"
    title: "Item 391: beside a paced scan, reads wait in the host, inserts on the node"
    status: done
    needs: ["Sweep", "I392"]
    docs: ["../../../appendix/resolved/paced-scan-reads-and-inserts.md"]
    description:
      what: "The cell's inserts beside a paced resident scan hold the budget once item 392 paces the walk's map reads, and its reads are judged against a twin flow with no Shoal in it."
  - id: "I392"
    title: "Item 392: a scan's walk reads the map from the device, unpaced, uncounted"
    status: done
    docs: ["../../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md"]
  - id: "Confirm24"
    title: "The confirmation: the exit's resident half again, beside its twin"
    status: done
    needs: ["I391"]
    description:
      why: "M24 stayed open behind item 391: its exit's resident half had missed, and could be judged again only on item 392's fix, at the default share."
      what: "Eight rounds at the default share on item 392's fix hold the cell's inserts at 1.00 to 1.04 times and its reads overlapping the twin's, so M24 is delivered."
---
# M24's steps and its exit

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
