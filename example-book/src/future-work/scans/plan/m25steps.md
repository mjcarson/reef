---
id: "m25steps"
title: "M25's steps and its exit"
group: "scans-plan"
needs: ["Q38", "Q39", "I387"]
nodes:
  - id: "Amend"
    title: "The plan's amendments to Q38, drawn"
  - id: "I403"
    title: "Item 403: a write dropped part way leaves a frame's rest to the next"
    status: done
    needs: ["Amend"]
    docs: ["../../../appendix/resolved/torn-client-write.md"]
  - id: "F151"
    title: "F151 The scans' wire version and grant"
    needs: ["I403"]
  - id: "I404"
    title: "Item 404: a bootstrap's grants commit scan before activation"
    needs: ["F151"]
    docs: ["../../../appendix/known-issues.md#404-a-bootstraps-seeded-grants-commit-scan-before-the-scans-wire-version-is-activated"]
  - id: "F152"
    title: "F152 A cluster node's page: voters, floors, barriers"
    needs: ["I404"]
  - id: "I393"
    title: "Item 393: a get at One on a learner's shard is served from its copy"
    needs: ["F152"]
    docs: ["../../../appendix/known-issues.md#393-a-get-at-one-that-reaches-a-learners-shard-is-served-from-the-learners-copy"]
  - id: "Deal"
    title: "The lanes dealt and taken, shared with the bench"
    needs: ["I393"]
  - id: "F153a"
    title: "F153 The client knows its cluster"
    needs: ["Deal"]
  - id: "F153b"
    title: "F153 The driver over a cluster"
    needs: ["F153a"]
  - id: "I401"
    title: "Item 401: with no authentication, one slow reader sheds every client's scans"
    needs: ["F153b"]
    docs: ["../../../appendix/known-issues.md#401-on-a-node-that-authenticates-nobody-one-slow-reader-sheds-every-clients-scans"]
  - id: "I402"
    title: "Item 402: a client sending no cancels gives a page's place back early"
    needs: ["I401"]
    docs: ["../../../appendix/known-issues.md#402-a-client-that-does-not-cancel-abandoned-pages-gives-a-pages-place-back-before-its-node-does"]
  - id: "Fail"
    title: "A kill, a move, a restart, held to the oracle"
    needs: ["I402"]
  - id: "F154"
    title: "F154 The bench on client pages"
    needs: ["Fail"]
  - id: "Close25"
    title: "The workspace run, a review, the cell before and after"
    needs: ["F154"]
  - id: "Exit25"
    title: "The exit's rounds on the lab"
    needs: ["Close25"]
---
# M25's steps and its exit

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
