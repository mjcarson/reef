---
id: "after25"
title: "After M25"
group: "scans-plan"
needs: ["M25"]
nodes:
  - id: "Agg"
    title: "Aggregates pushed to the shards"
    description:
      why: "Every count or sum reads the whole table through a scan; nothing answers one where the rows are, as ScyllaDB's parallelized aggregation does."
      what: "A count or a sum is answered by each scan page as a partial, on the shard that holds its rows, as ScyllaDB's parallelized aggregation does."
  - id: "Coord"
    title: "A scan coordinated by a node, for thin clients"
    description:
      why: "The client library drives every scan, so a client that is not Rust, or a gateway that should not know the topology, cannot scan (Q42)."
      what: "A node can drive a scan for a thin client, so a client that is not Rust, or a gateway, scans a table without knowing its cluster's topology."
  - id: "Exec"
    title: "Scan work on executors of its own"
    description:
      why: "Scan work runs on the table shards under a ceiling, so a scan cannot run at a device's speed; Q37 is asked again after M25."
      what: "Scan work runs on executors of its own, apart from the table shards, so a scan can run at its devices' speed if Q37 is answered that way."
---
# After M25

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
