---
id: "m25steps"
title: "M25's steps and its exit"
group: "scans-plan"
needs: ["Q38", "Q39", "I387"]
nodes:
  - id: "Amend"
    title: "The plan's amendments to Q38, drawn"
    description:
      why: "Q38 had a copy behind a floor refuse with its applied index, which no frame has a field for, and a review of M25's plan found six more of its rules that couldn't be built as written."
      what: "Q38's record carries seven amendments M25 is built to, among them a refusal without its applied index and a page dropped against the floor held when it was asked."
  - id: "I403"
    title: "Item 403: a write dropped part way leaves a frame's rest to the next"
    status: done
    needs: ["Amend"]
    docs: ["../../../appendix/resolved/torn-client-write.md"]
  - id: "F151"
    title: "F151 The scans' wire version and grant"
    needs: ["I403"]
    description:
      why: "A member built before scans can't read a committed grant naming `scan`, so a cluster can neither answer scans nor grant `scan` until a wire version of its own is activated."
      what: "A cluster activates the scans' wire version as every addition since F48 is, and only then answers scans and commits `scan` in its grants."
  - id: "I404"
    title: "Item 404: a bootstrap's grants commit scan before activation"
    needs: ["F151"]
    docs: ["../../../appendix/known-issues.md#404-a-bootstraps-seeded-grants-commit-scan-before-the-scans-wire-version-is-activated"]
    description:
      why: "A bootstrap's seeded grants are committed outside the path that holds `scan` back, so a configuration granting `[read, scan]` leaves a member built before scans unable to read its policy."
      what: "A bootstrap commits no `scan` below the scans' wire version, warning by name, and shoaladm grants its inventory's `scan` again once activated, so a deployment loses nothing it asked for."
  - id: "F152"
    title: "F152 A cluster node's page: voters, floors, barriers"
    needs: ["I404"]
    description:
      why: "Since M24 a cluster node grants no scans: a page there must first read voter copies alone, at or above each group's floor, with `Quorum`'s barriers, as Q38 decided."
      what: "A cluster node serves a scan's page by Q38's rules: a copy that is not a voter refused `Unavailable`, a floor waited for at most 250 ms, and a page's groups' barriers taken together."
  - id: "I393"
    title: "Item 393: a get at One on a learner's shard is served from its copy"
    needs: ["F152"]
    docs: ["../../../appendix/known-issues.md#393-a-get-at-one-that-reaches-a-learners-shard-is-served-from-the-learners-copy"]
    description:
      why: "A get at `One` that reaches a move's destination before the move is published is answered from the learner's copy, which may be far behind or rebuilt empty."
      what: "A get at `One` that reaches a learner's copy is refused, `Unavailable` to a client and `StaleTopology` to a peer, by the rule M25 gives a scan's page."
  - id: "Deal"
    title: "The lanes dealt and taken, shared with the bench"
    needs: ["I393"]
    description:
      why: "A scan that reads a fixed copy a tablet finishes at its slowest device's pace; X38's control found dealing and taking 5.1 times faster beside the cell."
      what: "Every tablet is dealt to a lane, followers then leaders, and taken by whichever member is free, by one dealing that the client's driver and the bench share."
  - id: "F153a"
    title: "F153 The client knows its cluster"
    needs: ["Deal"]
    description:
      why: "M24's driver scans one standalone node, learning its shards from refusals; a driver over a cluster needs every member and each tablet's voter copies known to the client."
      what: "A scanning client knows its cluster: its members, the connections to each, and each tablet's voter copies by node and slot, which the cluster driver deals from."
  - id: "F153b"
    title: "F153 The driver over a cluster"
    needs: ["F153a"]
    description:
      why: "M24's driver reads one standalone node; a scan of a cluster must choose voter copies, keep floors and barriers, and carry through a move, a failover and a restart."
      what: "`client.scan` reads a cluster's table from its voter copies, followers then leaders, every refusal routed around, and a fresh client can resume a scan from its token."
  - id: "I401"
    title: "Item 401: with no authentication, one slow reader sheds every client's scans"
    needs: ["F153b"]
    docs: ["../../../appendix/known-issues.md#401-on-a-node-that-authenticates-nobody-one-slow-reader-sheds-every-clients-scans"]
    description:
      why: "On a node that authenticates nobody every connection is one principal, so one slow reader's unread pages hold half the scan budget and every other client's scans are shed."
      what: "On a node that authenticates nobody, each connection is held to the budget's share rather than one anonymous principal, so one slow reader no longer sheds every other client's scans."
  - id: "I402"
    title: "Item 402: a client sending no cancels gives a page's place back early"
    needs: ["I401"]
    docs: ["../../../appendix/known-issues.md#402-a-client-that-does-not-cancel-abandoned-pages-gives-a-pages-place-back-before-its-node-does"]
    description:
      why: "A client built with `cancel_abandoned(false)` gives an abandoned page's place back while its node still holds it, so its next page on that connection may be shed."
      what: "Without cancels, a client keeps a page's place until its reader delivers or drains the answer, so its count matches its node's and its next page on that connection isn't shed for it."
  - id: "Fail"
    title: "A kill, a move, a restart, held to the oracle"
    needs: ["I402"]
    description:
      why: "P20 promises each live row once through a failover, a move or a node's restart; M25 can't claim it until a scan through each is checked by the oracle."
      what: "A scan carried through a node's kill, a move and a restart in the fixture hands out every live row once, as the scan oracle judges P20."
  - id: "F154"
    title: "F154 The bench on client pages"
    needs: ["Fail"]
    description:
      why: "The bench reads a cluster through every member's admin walk, its rotated driver the default, since a cluster node answered no client scan before M25."
      what: "On a cluster that activated the scans' wire version `shoaladm bench` scans through client pages by default, copies named by `--scan-copies`, and older specs read back as the admin walk."
  - id: "Close25"
    title: "The workspace run, a review, the cell before and after"
    needs: ["F154"]
    description:
      why: "M25 can't go to its lab exit until its changes pass a workspace run and a review by fresh readers, and the reference cell shows them costing a query nothing."
      what: "M25's changes have passed a workspace run and a review, its findings fixed, and the reference cell has been read before and after them."
  - id: "Exit25"
    title: "The exit's rounds on the lab"
    needs: ["Close25"]
    description:
      why: "M25 is delivered only once its lab exit is run: the budget beside a cluster scan at its defaults on the lab's three nodes, and followers set beside leaders."
      what: "The budget holds on the lab's three nodes beside a scan at its defaults, and a scan's figures on followers are on record beside the same scan forced onto leaders."
---
# M25's steps and its exit

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
