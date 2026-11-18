---
id: "after24"
title: "After M24, its steps and its exit"
group: "scans-plan"
needs: ["M24"]
nodes:
  - id: "I388"
    title: "Item 388: a client allocated a body before its answer was held to its page"
    status: done
    docs: ["../../../appendix/resolved/scan-body-held-to-its-page.md"]
  - id: "I386"
    title: "Item 386: a scan asked more pages of a connection than its node serves"
    status: done
    needs: ["I388"]
    docs: ["../../../appendix/resolved/scan-pages-held-to-the-bound.md"]
  - id: "I387"
    title: "Item 387: a page that refused a tablet answered a key its client refuses"
    status: done
    needs: ["I386"]
    docs: ["../../../appendix/resolved/reached-never-in-a-refused-tablet.md"]
  - id: "I389"
    title: "Item 389: a page in rows shed at a chunk was refused whole"
    status: done
    needs: ["I387"]
    docs: ["../../../appendix/resolved/rows-page-shed-answers-what-it-covered.md"]
  - id: "Principal"
    title: "F148 A scan's share a principal"
    status: done
    needs: ["I389"]
    docs: ["../../../features/scan-shares.md"]
  - id: "I380"
    title: "Item 380: a client that never reads holds its scans' draws"
    status: done
    needs: ["Principal"]
    docs: ["../../../appendix/resolved/unread-scan-answers-bounded.md"]
  - id: "Helper"
    title: "F149 A helper stepping through keys a caller names"
    status: done
    needs: ["I380"]
    docs: ["../../../features/key-steps.md"]
  - id: "X39"
    title: "X39 The next cursor sent ahead, priced"
    status: done
    needs: ["Helper"]
    docs: ["../cursor-sent-ahead.md"]
    description:
      why: "W4 left the next cursor sent ahead to be priced on M24's streams; the plan can't keep it, or drop the client's default to one stream a shard, until it is measured."
  - id: "Cursor"
    title: "The next cursor sent ahead of a page's rows, not taken"
    status: done
    needs: ["X39"]
    description:
      why: "W4 named sending a stream's next cursor ahead of a page's rows, so its next request leaves sooner, as a later gain to be priced on M24's streams."
      what: "The cursor sent ahead is measured and not taken: it cost the cell's reads 1.06 times at p99 at one stream a shard and bought nothing at two, so its client half is reverted."
  - id: "X40"
    title: "X40 Saved positions between pages, priced"
    status: done
    needs: ["Cursor"]
    docs: ["../saved-positions-priced.md"]
    description:
      why: "X17 found nothing to save on the lab's one-page tablets, so the plan can't build or drop W5's saved positions until a page's cost many pages deep in a tablet is priced."
  - id: "Saved"
    title: "F150 Saved positions between pages"
    status: done
    needs: ["X40"]
    docs: ["../../../features/saved-positions.md"]
  - id: "X41"
    title: "X41 A scan answer's hop between shards, priced"
    status: done
    needs: ["Saved"]
    docs: ["../answer-hop-priced.md"]
    description:
      why: "A scan's answer is written by whichever shard accepted its connection, not always the one that read it; nothing had priced that hop, so W6's hand-off couldn't be decided."
  - id: "Handoff"
    title: "A scan connection handed to its slot, not taken"
    status: done
    needs: ["X41"]
    description:
      why: "A scan's connection lands on whichever shard accepted it, so a page's answer may hop from the shard that read it; W6 left handing the connection over as a gain to measure first."
      what: "The hand-off is measured and not taken: X41 priced the hop at 1.07 times an answer's cpu, 0.07% of a Zen1 core at the default link share."
  - id: "Close24"
    title: "The workspace run, a review, the cell before and after"
    status: done
    needs: ["review24"]
    description:
      why: "*After M24* changes a connection's write relay (items 380 and 386), so its box waits on a workspace run, a review, and the reference cell read before and after."
      what: "The *After M24* box is done: the review's seven defects, items 394 to 400, are fixed, and the reference cell on titan is within on every figure."
---
# After M24, its steps and its exit

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
