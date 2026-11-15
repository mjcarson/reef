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
  - id: "Cursor"
    title: "The next cursor sent ahead of a page's rows, not taken"
    status: done
    needs: ["X39"]
  - id: "X40"
    title: "X40 Saved positions between pages, priced"
    status: done
    needs: ["Cursor"]
    docs: ["../saved-positions-priced.md"]
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
  - id: "Handoff"
    title: "A scan connection handed to its slot, not taken"
    status: done
    needs: ["X41"]
  - id: "Close24"
    title: "The workspace run, a review, the cell before and after"
    status: done
    needs: ["review24"]
---
# After M24, its steps and its exit

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
