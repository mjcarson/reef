# Beside a benchmark

A reef running on the machine a shoal benchmark runs on changes the benchmark's numbers, mostly
through the cores its shards are pinned to. Stop it for the capture if you can. If it has to keep
running, move its shards onto cores the benchmark leaves alone, put it behind everything else on
them, and keep it quiet.

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
  1. `[shoal] exclude_cores` is what moves them.
- **reef's port is a benchmark's.** shoal-bench serves each single-node workload on 12000 plus its
  place in the workload list, which only grows, and reef's default, 12180, is one of them. While
  reef holds it, that workload can't bind. shoal's lab clusters take ports from 12000 to 12850.
- **The rest costs less.** reef's other threads aren't pinned, and sleep. It holds each shard's
  memory budget, 256 MiB by default. It fetches every remote project each `fetch_every`, 5
  minutes by default, which is git on the cpus and the disk. Idle, it wrote nothing to disk in
  15 s.
- **Nothing in a capture says reef was there.** A capture's fingerprint records the host, the cpu,
  the governor, the kernel and the compiler, not what else ran.

## Stop it for the capture

```sh
sudo systemctl stop reef          # systemctl --user for a user unit
# take the capture
sudo systemctl start reef
```

While it's stopped, a commit still lands: git's hooks fail and git ignores them. Claude Code shows
each failed hook without stopping, and reef's tools say nothing answers. Nothing is lost: the next
capture diffs from the node's start, so the first hook after the restart, or `reef node capture`
in the node's worktree, takes what was missed.

Stopping it is all a capture needs. Its data directory is reef's plans, not a benchmark's root:
never wipe it, nor point a shoal config or lab script at it.

## Keep it running

1. **Move reef's shards with `exclude_cores`.** `[shoal] exclude_cores` lists the physical cores
   no shard may take. It names cores, not cpus, so both of a core's threads stay off. Exclude every
   core but the ones reef should have:

   ```toml
   [shoal]
   cores = 1
   exclude_cores = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]   # 16 cores: the shard on 15
   ```

   `cores` is fixed once the data directory is written, since shoal keeps its count there.
   `exclude_cores` can change at any restart. An [external node](external-shoal.md)'s
   `shoal.yml` takes `resources.exclude_cores` the same way.
2. **Pick cores the benchmark doesn't measure on.** shoal's `shoal.yml` gives the benchmark's
   shards cores 0-11 and leaves 12-15 to its client, which isn't pinned. shoal's lab drivers pin
   cpus 12-15 and 28-31. A reef shard on core 15 runs on cpu 15, off every `shoal.yml` shard's
   core, beside the drivers. The benchmark's config stays as it was.
3. **Keep the rest of the process there too.** `CPUAffinity=` and `AllowedCPUs=` in the unit hold
   reef's other threads, and the git it runs, on the shard's cpu. They don't move the shard.
4. **Put it behind everything else.** With the cpu controller on, a nice value or a weight counts
   only against a cgroup's siblings, and a benchmark runs in `user.slice` or `system.slice`. A
   top-level slice of reef's own with `CPUWeight=idle` runs reef only when nothing else wants its
   cpu, and its wakeups never preempt a benchmark. Under a load that fills its cpu, such as a build
   on every core, reef answers slowly until the load ends.
5. **Move its port.** Put `[api] address` off shoal's ports, such as `127.0.0.1:11180`, and link
   `~/.config/reef/config.toml` to the server's config, so the CLI finds the same port.
6. **Or keep the benchmark off reef's cores** by adding them to its own `exclude_cores`. That is a
   change to the benchmark: `shoal.yml` is the record of what a baseline measured, so a capture
   taken that way isn't comparable to one taken without it. A node asked for more shards than the
   cores left starts with fewer, and a staged cluster without enough cores is refused.
7. **Keep reef quiet for the capture.** Set `fetch_every = "0s"` under `[remotes]` and restart
   the unit, and run no Claude Code session, commit or app against it until the capture is done.

## As a system unit

reef run as an account of its own keeps its locked memory apart, since io_uring charges it to the
user that made the ring: a reef under your own login spends the budget your shoal tests share.
That account also owns the data directory, so your own `rm` or `kill` can't reach it.

`/etc/systemd/system/reef.slice`:

```ini
[Unit]
Description=reef's central planner, behind every shoal benchmark and test on this host

[Slice]
CPUWeight=idle
```

`/etc/systemd/system/reef.service`, beside [A systemd unit](systemd.md)'s:

```ini
[Service]
User=reef
Group=reef
Slice=reef.slice
Environment=REEF_CONFIG=/etc/reef/config.toml
ExecStart=/opt/reef/app/server
Restart=on-failure
TimeoutStopSec=10
LimitMEMLOCK=infinity
CPUAffinity=15
AllowedCPUs=15
MemoryHigh=768M
MemoryMax=1G
```

And in `/etc/reef/config.toml`, its data off every root a benchmark wipes:

```toml
[shoal]
data = "/var/lib/reef/shoal"
cores = 1
exclude_cores = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
memory = "128MiB"

[api]
address = "127.0.0.1:11180"
```
