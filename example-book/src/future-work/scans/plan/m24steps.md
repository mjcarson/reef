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
  - id: "AB24"
    title: "The reference cell before and after, on a Zen1 host"
    status: done
    needs: ["Run24"]
  - id: "Exit24"
    title: "The exit's rounds on the lab"
    status: done
    needs: ["AB24"]
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
  - id: "Sweep"
    title: "The share's sweep on the lab"
    status: done
    needs: ["F147"]
  - id: "I391"
    title: "Item 391: beside a paced scan, reads wait in the host, inserts on the node"
    status: done
    needs: ["Sweep", "I392"]
    docs: ["../../../appendix/resolved/paced-scan-reads-and-inserts.md"]
  - id: "I392"
    title: "Item 392: a scan's walk reads the map from the device, unpaced, uncounted"
    status: done
    docs: ["../../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md"]
  - id: "Confirm24"
    title: "The confirmation: the exit's resident half again, beside its twin"
    status: done
    needs: ["I391"]
---
# M24's steps and its exit

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
