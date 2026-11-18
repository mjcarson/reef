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
    description:
      what: "A WAL segment is 40 MiB by default, so a scan of `Big` reads 37.5 partitions a device read against 9.8, with no foreground figure the user's rule named disjointly worse."
  - id: "Rate"
    title: "W12's rate read again at O149's segments"
    status: done
    needs: ["O149"]
    description:
      why: "At M23's exit the share's 75 ms bought a 970 EVO about 20 MiB a second, not the 32 expected, and the user had W12's rate read again at O149's segments."
      what: "W12's rate is the share's at 40 MiB segments: 26.7 MiB a second of a Zen1 host's 970 EVO, against 13.9 at 10 MiB in the same rounds."
  - id: "O147"
    title: "O147 A shard's io_uring workers off its cpu, own kept"
    status: done
    docs: ["../../../appendix/optimizations.md#o147-each-shards-io_uring-workers-run-on-the-shards-own-cpu"]
    description:
      what: "A node can move its shards' io_uring workers off their cpus with F139's `resources.io_worker_cpus`, an opt-in; `own` stays the default, since moving them cost the cell a quarter of its inserts."
  - id: "O170"
    title: "O170 Every shard's blocking thread on one shard's cpu"
    status: done
    needs: ["O147"]
    docs: ["../../../appendix/optimizations.md#o170-every-shards-blocking-thread-runs-on-the-first-shards-cpu"]
    description:
      what: "Under F139's opt-in every shard's blocking thread runs on the cpus named for its io_uring workers, not on the first shard's cpu; under the default `own` nothing moves."
  - id: "O171"
    title: "O171 Syncs from fewer cpus, fewer queues"
    optional: true
    helps: ["O147"]
    docs: ["../../../appendix/optimizations.md#o171-a-nodes-syncs-sent-from-fewer-cpus-go-through-fewer-of-its-drives-queues"]
    description:
      what: "Each shard's syncs keep a drive queue of their own with its io_uring workers off its cpu, so moving the workers no longer costs the cell a quarter of its inserts."
  - id: "Host"
    title: "Polled NVMe queues, the WAL apart"
    description:
      why: "No lab SSD has polled queues, so every scan read completes by an interrupt, and the WAL's synced flushes share the device a scan reads (W42's factor A)."
      what: "Polled NVMe queues on a host and the WAL on a device of its own are priced on the rebuilt walk, so a deployment knows what each buys a scan."
---
# After M23

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
