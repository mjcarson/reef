# Beside a benchmark

A reef running on the machine a shoal benchmark runs on changes the benchmark's numbers, mostly
through the cores its shards are pinned to. Stop it for the capture if you can. If it has to keep
running, move its shards onto cores the benchmark leaves alone, and keep it quiet.

## Why it shows

- **reef's shards sit on the cores a benchmark takes first.** reef's in-process store is a shoal
  node, and shoal picks its cpus the same way every time: it skips cpu 0, walks the rest in core
  order, and takes one cpu from each physical core before any core's second thread. A benchmark's
  node walks the same order, so its first shards land on exactly reef's. On a 16-core machine
  where cpu n and cpu n+16 share core n, reef's two shards are cpus 16 and 1, the same two cpus
  as the benchmark's first two shards.
- **Each shard is pinned there.** Idle, each of reef's shards woke about 13 times a second and ran
  about 1 ms of each second (measured 2026-11-24). Each wakeup takes the cpu from a benchmark
  shard, and tail latencies show it first. In use, the shards work on those cpus: on an MCP call,
  on the hooks after every commit and every Claude Code Stop, on the app's reads.
- **An affinity mask doesn't move them.** shoal reads the machine's cpus from sysfs, not from the
  process's mask, so `taskset` or the unit's `CPUAffinity=` still leaves the shards on cpus 16 and
  1. reef's own `[shoal]` has no key that picks cores.
- **The rest costs less.** reef's other threads aren't pinned, and sleep. It holds each shard's
  memory budget, 256 MiB by default. It fetches every remote project each `fetch_every`, 5
  minutes by default, which is git on the cpus and the disk. Idle, it wrote nothing to disk in
  15 s.
- **Nothing in a capture says reef was there.** A capture's fingerprint records the host, the cpu,
  the governor, the kernel and the compiler, not what else ran.

## Stop it for the capture

```sh
systemctl --user stop reef
# take the capture
systemctl --user start reef
```

While it's stopped, a commit still lands: git's hooks fail and git ignores them. Claude Code shows
each failed hook without stopping, and reef's tools say nothing answers. Nothing is lost: the next
capture diffs from the node's start, so the first hook after the restart, or `reef node capture`
in the node's worktree, takes what was missed.

## Keep it running

1. **Move reef's shards with an external node.** Run reef's store as [an external shoal
   node](external-shoal.md), whose `shoal.yml` can list `resources.exclude_cores`: the physical
   cores no shard may take. It names cores, not cpus, so both of a core's threads stay off.
   Exclude every core but the last two, and the two shards land on those:

   ```yaml
   resources:
     cores: 2
     memory: "256MiB"
     exclude_cores: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]   # 16 cores: shards on 14 and 15
   ```

   To keep the plan reef already has, stop the unit and point both of the node's paths at reef's
   data directory (`~/.local/share/reef/shoal` on a user unit), where the in-process pool kept both
   of its writers. Keep `cores: 2`, the count that directory was written with.
2. **Pick cores the benchmark doesn't measure on.** shoal's `shoal.yml` gives the benchmark's shards
   cores 0-11 and leaves 12-15 to the client, which runs on cpus 28-31 alone. reef's shards on
   cores 14 and 15 then run on cpus 14 and 15, off every shard's core, and share a core with the
   client's last two cpus. The benchmark's config stays as it was.
3. **Or keep the benchmark off reef's cores** by adding them to its own `exclude_cores`. That is a
   change to the benchmark: `shoal.yml` is the record of what a baseline measured, so a capture
   taken that way isn't comparable to one taken without it. A node asked for more shards than the
   cores left starts with fewer, and a staged cluster without enough cores is refused.
4. **Keep reef quiet for the capture.** Set `fetch_every = "0s"` under `[remotes]` and restart
   the unit, and run no Claude Code session, commit or app against it until the capture is done.
