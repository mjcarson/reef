---
id: "after23"
title: "After M23"
group: "scans-plan"
needs: ["M23"]
nodes:
  - id: "O149"
    title: "O149 Larger WAL segments: 40 MiB the default"
    status: done
    docs: ["../../../appendix/optimizations.md#o149-larger-wal-segments-keep-a-tablets-records-together-four-times-longer"]
  - id: "Rate"
    title: "W12's rate read again at O149's segments"
    status: done
    needs: ["O149"]
  - id: "O147"
    title: "O147 A shard's io_uring workers off its cpu, own kept"
    status: done
    docs: ["../../../appendix/optimizations.md#o147-each-shards-io_uring-workers-run-on-the-shards-own-cpu"]
  - id: "O170"
    title: "O170 Every shard's blocking thread on one shard's cpu"
    status: done
    needs: ["O147"]
    docs: ["../../../appendix/optimizations.md#o170-every-shards-blocking-thread-runs-on-the-first-shards-cpu"]
  - id: "O171"
    title: "O171 Syncs from fewer cpus, fewer queues"
    optional: true
    helps: ["O147"]
    docs: ["../../../appendix/optimizations.md#o171-a-nodes-syncs-sent-from-fewer-cpus-go-through-fewer-of-its-drives-queues"]
  - id: "Host"
    title: "Polled NVMe queues, the WAL apart"
---
# After M23

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
