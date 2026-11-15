# A scan paced by its link, on the lab

**Run 2026-11-13 and 2026-11-14.** This is the record of [item 390](../../appendix/resolved/resident-scan-fills-the-link.md)'s
cure on the lab, [F147](../../features/scan-link-pace.md): a node paces a scan's answers to a share
of the link they leave by. It has two runs:

- **the share's sweep**, which chooses the default;
- **the confirmation**, which judges [M24](milestones.md#m24-scans-on-one-node)'s exit again at that
  default.

Between them it records item 391's chase, and after them [O179's slice measured again](#the-slice-again-after-item-392).

## The question

[M24's exit](m24-exit.md) found a scan of resident rows filling the client's 1 GbE link. The
client's own reference cell went over [W7's budget](isolation.md#the-budget) beside it: its read
p99 rose 3.5 times and its insert p99 1.63 times.

The user decided on 2026-11-13 that the node paces a scan's answer bytes. The default is a share
of the node's link speed, and M24 is delivered once the cure holds the budget. So the run answers
three questions:

- **What share of titan's gigabit can a scan of resident rows take** with the cell's p99s still
  inside the budget?
- **Is the link the cause of the exit's miss at all**, and not a cpu? The exit could not separate
  the two: no arm ran a resident scan held under the link's rate.
- **At the default, does the exit's resident half hold**, for the scan's own client and for another
  client of the node?

## How it is judged

By W7 as the exit judged it: the cell's read and insert p99 over a scan's own seconds within
`max(1.10 × before, before + 0.1 ms)` of the window before, at the median over the rounds. A
ratio is read against a control of its own, **the cell alone on a node freshly loaded**, its rows
resident as the scanned rows are. (The exit's control was restarted, with its rows archived, and a
read p99 near 6 ms where a resident one is 0.46.) A rise is the scan's only where its range is
disjoint from that control's.

The default is **the largest share that holds both p99s**, since W7's goal is a scan's ceiling,
taken cheaply ([the scan speed decisions](scan-speed.md)).

The predictions were written before any round:
[predictions.md](../../../../shoal-spike-scan/results/link-pace/predictions.md).

## What is run

The exit's node and driver, harness and hosts, at the commit that adds F147:

- **the node:** `m24-node` on titan, five shards, 6 GiB, a principal required;
- **the driver:** `m24-exit` on europa;
- **the data:** a million tagged rows of a kibibyte, the cell at sixteen in flight, every arm
  warmed 100 s.

[m24-exit-lab.sh](../../../../shoal-spike-scan/results/m24-exit-lab.sh) gains two kinds of round:

- **The sweep (`PACE_ONLY=1`).** For each share (none, 12.5%, 25%, 50% and 75%), a node made fresh
  with `scans.link_share` at it, loaded, and the cell beside a scan of its resident rows. Then the
  cell alone on a node made fresh and loaded, the round's control. The shares' order reverses by
  round.
- **The confirmation (`CONFIRM=1`).** A node made fresh at the default and loaded. Then, their order
  alternating by round:
  - the cell beside a resident scan in rows, and in partitions;
  - the control;
  - a resident scan alone;
  - **another client's cell**, driven from hyperion beside a resident scan from europa, both
    started by the wall's clock, since the node's pace is what protects a client that is not the
    scanner.

  In the first four rounds the archived control and the cell beside an archived scan run too.

The tables come from [m24-exit-report.py](../../../../shoal-spike-scan/results/m24-exit-report.py),
whose output lands beside the predictions in
[shoal-spike-scan/results/link-pace/](../../../../shoal-spike-scan/results/link-pace).

## The sweep

**Run 2026-11-13, 02:59 to 06:03 UTC**, four rounds and 40 arms at `d24c0777`:

- every scan handed out every row once (32 of 32);
- no cell operation failed;
- all three hosts matched their snapshots after.

Four `QUICK` passes ran first. They found the cell's read p99 rising at every share beside a flat
control, so three arms joined the sweep before its rounds, to place that rise: another client's
cell, and a projected and a filtered scan that send almost nothing. The predictions were amended
the same way before any round.

Every cell is the median over the four rounds and its range. The whole output is
[report.md](../../../../shoal-spike-scan/results/link-pace/sweep/report.md).

| Arm | Wire, titan sent MiB/s | Cell's read p99, µs: before, during | Read ratio | Insert ratio | Cell's read p50 during, µs | Shards busy | cpu5's softirq |
| --- | --- | --- | --- | --- | --- | --- | --- |
| The cell alone (the control) | 1.0 | 460, 464 | 1.00 [0.98, 1.03] | 0.98 [0.93, 1.01] | 242 | 8% | 2% |
| No pace | 113.0 | 478, 1,602 | 3.33 [2.41, 3.40] | 1.59 [0.84, 1.75] | 552 | 18% | 38% |
| 12.5% | 16.6 | 464, 873 | 1.88 [1.81, 1.94] | 1.16 [1.07, 1.23] | 244 | 10% | 6% |
| 25% | 32.1 | 466, 962 | 2.07 [1.99, 2.15] | 1.16 [1.13, 1.28] | 253 | 11% | 10% |
| 50% | 62.5 | 458, 1,038 | 2.27 [2.20, 2.41] | 1.42 [1.37, 1.46] | 314 | 15% | 17% |
| 75% | 91.7 | 454, 1,366 | 3.02 [2.85, 3.14] | 1.66 [1.49, 1.81] | 664 | 17% | 25% |
| 25%, another client's cell (hyperion) | 30.6 | 476, 949 | 1.99 [1.97, 2.07] | 1.30 [1.20, 1.39] | 268 | 11% | 10% |
| A filtered scan, a row in a hundred sent | 5.6 | 450, 1,437 | 3.19 [2.91, 3.40] | 1.37 [1.19, 1.65] | 285 | 37% | 3% |
| A projected scan, 33 bytes a row sent | 15.5 | 457, 1,360 | 2.98 [2.83, 3.28] | 1.42 [1.23, 1.47] | 296 | 40% | 7% |

**Every scan arm's rise is disjoint from the control's, on both p99s, and none holds the budget.**
At the cell's 0.46 ms the budget's larger arm is `before + 0.1 ms`, 0.56 ms.

Notes on the columns:

- The shards are titan's cpus 1 to 4 and 6. cpu5, which titan's NIC interrupts, hosts none.
- The projected and filtered scans repeat for as long as a 25% scan in rows takes, each pass whole,
  and read about 500,000 rows a second.

### What it says

- **The pace binds where it is set.** The scan's bodies move at 14.9, 29.8, 59.5 and 89.1 MiB a
  second at 12.5% to 75%: 95 to 100% of the share of 125 MB a second, less the wire's framing.
  The scan alone takes 66.7, 33.3, 16.7 and 11.1 s, against 9.0 unpaced, every round within a
  tenth of a second.
- **It takes off what the link added.** At 25% and under, the cell's read median is its own
  (244 and 253 µs against 242), where at 75% and unpaced it doubles (664 and 552). The read p99
  falls from 3.33 times to 1.88 and 2.07. The insert p99 falls from 1.59 times to 1.16. cpu5's
  softirq, the link's own work, falls from 38% to 6 and 10%.
- **What is left is the node's, not the link's or the client's.**
  - Another client's cell, on its own host and link, rises as the scanner's own does: 1.99 times
    against 2.07.
  - A filtered scan that sends a fifth of what the 12.5% scan does costs the cell more than any
    paced scan in rows: 3.19 times on the read p99, every shard 37% busy.
  - So a resident scan costs the cell's reads on the node itself, through its reading and
    encoding on the shards the cell's reads run on. It does so even at 12.5%, where the shards
    are 10% busy against the control's 8%.
- **The read rise beside a paced scan is steady, not spikes.** Second by second the cell's read p99
  is about 860 µs through the whole 12.5% scan and its median does not move: about one read in a
  hundred waits about 0.4 ms longer, every second. A cause that steady at a shard load that small
  is not the steps' share of the cpu. ~~The candidates are listed in item 391.~~ Item 391 placed it in titan's
  network path, [below](#item-391-placed).
- **Long steps rise with the pace.** A paced scan in rows passes 100 µs in 88 to 115 of its steps
  in 10,000, where the unpaced and the repeated scans pass it in 32 to 35. A slower scan's steps
  find the shard idler and colder. It is the step clause's to read, beside item 391: the confirmation read it with its
  remainder apart, as [O180](../../appendix/optimizations.md#o180-a-paced-resident-scans-encode-steps-run-past-their-bound-on-their-own-cpu-about-once-in-ninety).

### Against the predictions

| Prediction | Result |
| --- | --- |
| 1. No pace reproduces the exit's miss | **Held**: read p99 3.33 times, insert 1.59, the scan at 110.4 MiB a second |
| 2. The pace binds where it is set | **Held**, 95 to 100% of each share |
| 3. The budget holds at 25% and under | **Wrong**: 1.88 and 2.07 times on the read p99, 1.16 on the insert p99 |
| 4, 5. 50% near the edge, 75% over | 50% over on both, 75% over on both |
| 6. Resent segments follow the link's use | **Held**: 57 and 63 a second at 12.5% and 25%, 77 at 50%, 138 at 75%, against the control's 43 |
| 7. The step clause does not change with the pace | **Wrong**: paced scans' long steps are three times the unpaced scan's |
| 8. The default is 25% | Not judged by the budget, which no share holds; **chosen with the user**, [above](#decided-with-the-user-2026-11-13) |
| 9. The pace takes off what the link added | **Held**, above |
| 10. A floor remains that no share removes | **Held**: about 0.4 ms at 12.5% and up, beside another client's cell too, and larger beside scans that send almost nothing. Its rule for the default would pick 75%, since the repeated scans read rows faster than any paced scan does; that makes them a measure of the node's floor, not a yardstick for a share, so the rule is not used |
| 11. The insert p99 holds at every share | **Wrong**: 1.16 times at 12.5% and 25%, disjoint from the control |

## Decided with the user (2026-11-13)

Three questions went to the user after the sweep, and were settled the same day.

1. **Item 390 is resolved as the link's part**, which the pace cures
   ([its page](../../appendix/resolved/resident-scan-fills-the-link.md)). **~~The node's own cost~~ What is left is
   [item 391](../../appendix/resolved/paced-scan-reads-and-inserts.md)**,
   chased next: a lab diagnostic places it before anything is built for it.
2. **M24 stays open behind item 391**: the budget does not hold beside a resident scan at any share
   while it stands. Item 391 was resolved by [the confirmation](#the-confirmation) on 2026-11-14, and M24 was delivered.
3. **The default share is 25%.** 12.5% and 25% cost the cell's inserts the same, 1.16 times. 25%
   costs its reads 0.09 ms more at p99, the ranges disjoint, for a scan twice as fast, 29.8 MiB a
   second against 14.9.

## Item 391 placed

**Run 2026-11-13, 07:22 to 10:26 UTC**, the same node, driver and harness, every arm a 25% scan of the resident
rows on a node made fresh and loaded, moved by one thing. The predictions were written before each run
([predictions.md](../../../../shoal-spike-scan/results/item-391/predictions.md)) and amended only before a run's
rounds. The records are under
[shoal-spike-scan/results/item-391/](../../../../shoal-spike-scan/results/item-391).

### Not the node's threads

Four rounds with one candidate moved each. Rounds 1 and 2 ran clean. Rounds 3 and 4 are set apart: a sampler of
mine ran on titan unpinned, forking processes on the shards' cpus, and every arm of them had the cell's read p99
near 2 ms, the control's too ([why](../../../../shoal-spike-scan/results/item-391/diag/disturbed/README.md)).

| Arm, rounds 1 and 2 | Cell's read p99 ratio | Node's own get p99, ms: before, during | Ping p99 from hyperion, ms (round 2) |
| --- | --- | --- | --- |
| The cell alone | 0.99 | 0.26, 0.26 | 0.15, 0.15 |
| 25% | 1.97 | 0.27, 0.31 | 0.15, 0.71 |
| 25%, the allocator never purging | 2.08 | 0.26, 0.31 | 0.14, 0.71 |
| 25%, io_uring's workers on spare cpus | 2.53 (0.36 to 0.91 ms) | 0.17, 0.25 | 0.15, 0.70 |
| 25%, pages of 4 MiB | 2.07 | 0.26, 0.31 | 0.15, 0.70 |

- **None of the three moved the cell's read p99.** io_uring's workers on spare cpus cut the shards' time waiting to
  run from 51 ms a second to 2.7, and the cell's read tail did not notice.
- **The node's own wait for a get barely moved**: from its frame's arrival to its answer's write, by the node's own
  query counters (`ReadVerb::Queries`, added for this), 0.04 ms at p99, where the client saw 0.47.
- **A ping answered in titan's kernel rose as the cell did**, beside every scan arm and not the control.
- **titan's NIC queue took bursts of up to 65 KB** beside the paced scan, about half a millisecond of the gigabit:
  its BQL limit rose from 4 to 35 KB and its bytes in flight peaked at 65 KB.

A quick pass with titan's root qdisc `fq` in place of `fq_codel` left the ping's p99 at 0.68 ms beside the scan.
Ethernet flow control is off at both ends; both NICs are Realtek r8169s without scatter-gather or segmentation
offload.

### Any paced flow, for reads; the scan's own, for inserts

Three rounds, the 25% scan beside the cell alone with two TCP connections from titan to a sink on europa, paced
together by `SO_MAX_PACING_RATE` as a scan's are, with no Shoal in them
([item-391-bulk.py](../../../../shoal-spike-scan/results/item-391-bulk.py)), in its middle window:

| Arm, three rounds | Cell's read p99 ratio | Cell's read median during, µs | Insert p99 ratio | Ping p99 during, ms |
| --- | --- | --- | --- | --- |
| The cell alone | 0.96 [0.96, 1.00] | 243 | 1.01 [0.97, 1.07] | 0.13 |
| The 25% scan | 2.03 [2.00, 2.17] | 253 | **1.27 [1.19, 1.29]** | 0.69 |
| A bulk flow at 25%, no Shoal | 1.93 [1.85, 2.64] | 265 | 1.00 [0.99, 1.02] | 0.74 |
| A bulk flow at 12.5%, no Shoal | 1.50 [1.27, 2.40] | 253 | 1.02 [0.95, 1.10] | 0.52 |

- **The cell's reads wait in titan's network path**: a paced flow with no Shoal in it costs them as much as the scan,
  the ranges overlapping, and the ping with them. Any steady flow out of the lab's NICs makes what follows it wait
  about half a millisecond at the tail.
- **The cell's inserts are the scan's to answer for**: the same flow leaves them where they were, and the scan
  raises them 1.27 times, the ranges disjoint.

### The insert part: the scan's work, and the intent log's syncs

Three rounds with the same flow sent from a shard's own cpu (cpu 2) left the cell's inserts at 1.02 [0.99, 1.02] times
and the node's own insert wait at 1.00, where the 25% scan raised them 1.23 [1.13, 1.61] and 1.20: the kernel's
transmit work on a shard's cpu is not what costs the inserts.

Then three rounds of a `stage-profile` node, each sampled write's time on the node split by stage, p99 in milliseconds:

| Stage | The cell alone: before, during | Beside the 25% scan: before, during |
| --- | --- | --- |
| Written | 9.2, 8.8 | 9.1, 7.0 |
| Waited for its sync | 18.4, 19.1 | 19.4, **23.4** |
| Synced, as the shard sees it | 22.8, 23.3 | 23.6, **27.8** |
| On the node | 38.9, 39.4 | 39.9, **44.2** |

| titan's NVMe | The cell alone: before, during | Beside the 25% scan: before, during |
| --- | --- | --- |
| Flushes a second | 375, 375 | 366, **478** |
| Milliseconds a flush | 3.30, 3.27 | 3.38, 2.30 |

~~The drive is no slower beside the scan, each flush faster; the shard syncs its intent log about 30% more often for the
same inserts, in smaller pieces, and a write waits on a busier chain of syncs. A shard's loop writes its staged
intents after any message that leaves its inbox empty, and a loop a scan keeps awake meets an empty inbox after more
of its inserts.~~ The flush counts of this run did not hold up against the share's sweep's four rounds, where every scan
arm flushed **fewer** times beside its scan than before it, 0.80 to 0.93 as often, each flush 3.4 to 3.7 ms against the
cell alone's 3.2, the more scan work the fewer:

| The sweep's arm, four rounds | Flushes during over before | Milliseconds a flush during | Insert p99 ratio |
| --- | --- | --- | --- |
| The cell alone | 1.03 | 3.18 | 0.98 |
| 25% | 0.93 | 3.40 | 1.16 |
| 75% | 0.80 | 3.71 | 1.66 |
| No pace | 0.83 | 3.49 | 1.59 |
| A projected scan, the node's work alone | 0.83 | 3.51 | 1.42 |

So the drive is not what slows, and the shard's syncs come later, not smaller. A sync is stamped issued before its
submission reaches the ring, and synced once its task runs again after the completion is reaped; a shard's executor
submits and reaps its main ring's I/O only when it polls, which runnable scan work can put off. ~~That is the reading the
counts fit, not yet shown.~~ The polling test below disproved it.

### Not the executor's polling

**Run 2026-11-13, 19:40 to 20:46 UTC**, three rounds of the stage-profile node, the 25% scan and the cell alone, each
with the glommio fork's `GLOMMIO_PREEMPT_ON_DEADLINE=1` off and on. The fork's switch (a diagnostic, `4fb93d2`, never
on by default) makes a preemption due once the shortest latency of the task queues active now has passed since the
executor last polled its rings, where the timer armed at that poll, for the queues active then, decides by itself up to
100 ms. With it on, a shard submits and reaps its ring at least every queue's latency while scan work runs. The switch
was read back from the running node's environment. Round 1's untraced control is set apart: a compile check of mine
ran bpftrace on titan inside its middle window
([why](../../../../shoal-spike-scan/results/item-391/poll/disturbed/README.md)).

| Arm, three rounds | Cell's insert p99 ratio | Node's own insert p99, ms: before, during | Waited for its sync, p99 ms | Synced, p99 ms | Cell's read p99 ratio |
| --- | --- | --- | --- | --- | --- |
| The cell alone | 0.97 [0.96, 0.98] | 40.5, 40.0 | 19.1, 18.5 | 23.1, 22.8 | 0.98 |
| The cell alone, the switch on | 0.96 [0.96, 0.97] | 40.9, 40.5 | 19.7, 19.0 | 23.7, 23.1 | 1.03 |
| The 25% scan | 1.40 [1.18, 1.41] | 40.5, **55.7** | 19.3, **29.0** | 23.4, **34.2** | 2.05 |
| The 25% scan, the switch on | 1.27 [1.15, 1.35] | 40.7, **52.7** | 19.7, **27.7** | 23.8, **32.0** | 1.84 |

- **The switch leaves the inserts' rise**, the ranges overlapping: a write still waits about 10 ms more for its sync
  at p99 and the sync about 9 ms more, where the cell alone's hold flat with it on or off. A shard's polling is not
  where the scan's time goes, or not most of it.
- **The cell alone is unchanged by the switch**, and its reads rise beside the scan about twice either way, the
  network path's part.
- **The insert tail has a second population this profile missed.** A write whose intent log rotated under it carries
  no durability stages and `m24-node` leaves it out, and a few arms' node insert wait had seconds of 130 to 300 ms at
  p99 that no write in the stage file held. They are rare: no arm of the stages run had one, and of this run's only
  the disturbed control more than two, all ten of its from four seconds after my check began on. They do not make
  the scan's rise: no scan window of either run held more than one.

So the sync's extra time is spent between its claim and its return as the shard sees it, the executor's polling ruled
out. The next test times each fdatasync in the kernel and from its ring's submission to its completion, on titan, to
say whether the kernel's sync itself is what lengthens.

### The kernel's fdatasync, its worker waiting for a shard's cpu

**Run 2026-11-13, 20:54 to 21:47 UTC**, three rounds of the stage-profile node: the 25% scan and the cell alone, each
traced on titan by [item-391-fsync.bt](../../../../shoal-spike-scan/results/item-391-fsync.bt) (bpftrace, pinned to the
sampler's cpu), and the round's untraced control. The trace times every fdatasync of the node twice, in the kernel
(`vfs_fsync_range` in the io-wq worker that runs it: XFS's log force and the device's flush) and from its ring's
submission to its completion, and counts how long the node's io-wq workers and the kworkers waited to run on the shards'
cpus. Medians of three rounds:

| Arm | Fdatasync in the kernel, p99 ms: before, during | Through the ring, p99 ms, during | Fsyncs a second | Workers' waits past 1 ms a second | Kworkers' run delay, ms a second |
| --- | --- | --- | --- | --- | --- |
| The cell alone, traced | 23.6, 23.1 | 23.3 | 362, 366 | 0.8, 0.7 | 4.2, 4.3 |
| The 25% scan, traced | 23.4, **34.7** | 34.8 | 364, 329 | 0.8, **15.6** | 4.1, 7.1 |

- **The kernel's own fdatasync is what lengthens**, about 11 ms at p99, and a write's `synced` stage with it (23.8 to
  34.9). The ring adds 0.1 ms to the kernel's time, so a shard's executor adds nothing to a sync: what the polling
  test ruled out, shown.
- **Fewer syncs, each longer**: a shard has one in flight at a time, so a longer one leaves room for fewer.
- **The io-wq workers that run them wait for their shard's cpu**: their waits past a millisecond rise twentyfold beside
  the scan, where their total wait rises only 1.17 times. A worker runs on its shard's cpu (`io_worker_cpus: own`, F139),
  and an fdatasync sleeps and is woken more than once (the log's write, the device's flush). Linux 7.0's EEVDF lets the
  task running on a cpu finish its slice before a woken one of the same slice preempts it (`RUN_TO_PARITY`), and the
  base slice on titan is 2.8 ms; a shard kept busy by a scan's steps holds its cpu through more of them.
- **The trace costs the cell nothing that shows**: the traced cell alone's insert ratio is 0.99 [0.93, 1.01] against the
  untraced control's 0.98 [0.94, 1.01]. It adds about 40 µs to the cell's read p99 throughout, before and during alike.

~~That is the reading the long waits fit, not yet shown.~~ The slice test below found the long waits removable and not
the whole of it, and the drive's counters found the scan reading it. The next test gives every thread of the node a
custom EEVDF slice of 100 µs (`chrt -o -T`, which Linux 7.0 takes for `SCHED_OTHER` and a forked thread inherits), as
each shard could ask for at its start, so a woken worker preempts its shard within a slice's worth; and the trace names
the thread that ran just before each long wait.

### A shorter slice for the node's threads

**Run 2026-11-13, 21:54 to 23:03 UTC**, three rounds of the stage-profile node: the 25% scan traced at the default slice,
the 25% scan and the cell alone traced with every thread of the node at a custom slice of 100 µs (an item's
`nodeslice=100000`: `chrt -a -o -T`, set once the node was made and loaded, each thread's slice read back), and the
untraced control. The trace names the thread that ran on a cpu just before each wait past a millisecond.

| Arm, three rounds | Cell's insert p99 ratio | Fdatasync in the kernel, p99 ms: before, during | Fsyncs a second | Workers' waits past 1 ms a second | Those waits after a shard's executor |
| --- | --- | --- | --- | --- | --- |
| The cell alone | 1.01 [1.00, 1.03] | - | - | - | - |
| The cell alone, 100 µs | 0.99 [0.97, 1.00] | 23.4, 22.9 | 364, 364 | 0.1, 0.3 | 42% of 106 |
| The 25% scan | 1.34 [1.31, 1.39] | 23.5, **33.2** | 358, 322 | 1.0, **10.2** | **87%** of 1,170 |
| The 25% scan, 100 µs | **1.04 [1.03, 1.25]** | 23.9, 26.3 | 363, 469 | 0.2, 0.2 | 44% of 39 |

- **A busy shard did hold its workers off**: 87% of the long waits beside the scan came straight after a shard's
  executor ran on the cpu, and at 100 µs they fell to the cell alone's.
- **The inserts rose less, but not in every round.** The short slice's range is wholly below the default's. Round 2's
  rose 1.25 times all the same, its fdatasync p99 still 9 ms longer and its fsyncs fewer, with the workers' long waits
  gone. In rounds 1 and 3 the drive flushed more often and faster beside the scan than before it (469 and 498 fsyncs a
  second, round 1's flushes 2.2 ms each against 3.4 before), the cell running 1,760 to 1,860 operations a second against 1,360, which
  no arm at the default slice did in this run; whether the slice brings that on, or the drive changed its pace on its
  own as it has in a few windows of earlier runs, three rounds do not settle.
- **The cell alone is the same at 100 µs**, its reads too (1.03), and the scan's read rise is unchanged (1.85).

Filed as [O179](../../appendix/optimizations.md#o179-a-shard-holds-its-cpu-through-its-eevdf-slice-while-the-io-wq-worker-running-its-fdatasync-waits),
optional and ~~unsettled: what the slice is worth is measured again once the scan's reads of its map below are fixed~~
not taken: measured again with them fixed, it bought the inserts nothing ([the slice again](#the-slice-again-after-item-392)).

### A scan's walk reads the map from the device

Through every run above titan's NVMe counters said what the scan's figures did not: beside a scan of resident rows the
node read its drive hundreds of times a second, in reads of 7 to 14 KB, where its scan figures counted no archive read
(`reads` and `archived` 0 on every shard).

| Arm | titan's NVMe reads a second: before, during | MB/s during | Flushes a second: before, during |
| --- | --- | --- | --- |
| The cell alone (the sweep, four rounds) | 24, 23 | 1.3 | 368, 385 |
| 12.5% | 22, 131 | 1.8 | 369, 396 |
| 25% | 30, 194 | 1.9 | 390, 356 |
| 50% | 20, 444 | 4.1 | 371, 331 |
| 75% | 16, 679 | 7.4 | 369, 292 |
| No pace | 29, 803 | 6.3 | 400, 314 |
| Projected, almost nothing sent | 20, **3,858** | **28.7** | 368, 314 |
| Filtered | 16, 3,110 | 25.2 | 367, 328 |
| The 25% scan, traced (three rounds) | 17, 246 | 3.6 | 365, 331 |
| The 25% scan at 100 µs (the slice test) | 22, 278 | 2.7 | 367, 470 |

A reading of the code (an Explore agent's, 2026-11-13) found what they are: every page of a scan reads its table's
archive map from the device. A page's merge pulls the map's view of its range to compare keys with its resident
partitions, and each of the map's runs reads its index pages for the range with direct I/O, never through the map's
page cache, and the page after it reads the boundary page again. Every partition a table ever archived is in its runs,
so a page over resident rows reads them too; none of it draws on the scan's device pace or budget, and no figure counts
it. The node's own figures predict the rate: 5,090 pages in 33 s at 1.4 runs a page is about 214 run cursors started a
second, against about 230 reads more than the cell alone's. Filed as
[item 392](../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md),
the cause of item 391's insert part: the faster a scan walks, the more it reads, the fewer fdatasyncs the intent logs
get through, and the higher the cell's inserts.

### The map in memory: the cause shown

**Run 2026-11-13, 23:35, to 2026-11-14, 00:43 UTC**, three rounds of a spike build (branch `i392-spike`, `05d92122`,
never merged) whose runs read every page into memory at open when the node's environment names
`SHOAL_SPIKE_RUNS_IN_MEMORY=1`, and whose cursors read from there: a scan does all its work and reads none of its map
from the device. Each arm traced; the switch decided with the user to be built only as a test of the cause, before item
392's cure ([the predictions](../../../../shoal-spike-scan/results/item-391/predictions.md)).

| Arm, three rounds | Cell's insert p99 ratio | titan's NVMe reads a second, during | Fdatasync in the kernel, p99 ms: before, during | `synced`, p99 ms: before, during | Workers' waits past 1 ms a second | Cell's read p99 ratio |
| --- | --- | --- | --- | --- | --- | --- |
| The cell alone | 0.97 [0.96, 0.98] | 29 | - | 23.6, 23.0 | - | 1.01 |
| The cell alone, the map in memory | 1.00 [0.98, 1.03] | 18 | 23.7, 23.4 | 24.0, 23.6 | 0.7 | 0.97 |
| The 25% scan | 1.30 [1.12, 1.42] | 246 | 23.5, **34.1** | 23.7, **34.8** | 12.8 | 1.92 |
| The 25% scan, the map in memory | **0.90 [0.87, 0.95]** | **28** | 23.5, **22.8** | 23.9, **23.2** | 8.7 | **2.38** |

- **Without its map's reads the scan costs the inserts nothing**: their p99 beside it 0.90 times its before, the ranges
  disjoint from the scan that reads, the kernel's fdatasync and a write's `synced` stage as short as before the scan,
  and the fsyncs a second 1.03 times as many. Item 392 is item 391's insert part.
- **The workers' long waits are not it**: beside the scan with its map in memory the io-wq workers still waited past a
  millisecond 8.7 times a second, 81% of them straight after a shard's executor, and the fdatasyncs did not lengthen.
  O179's slice takes away waits that do not cost the inserts.
- **The cell's reads rose more beside the scan that read nothing**: 2.38 [2.09, 2.67] times against 1.92 [1.89, 1.98],
  the ranges disjoint, where the node's own get p99 rose the same in both (317 to 373 µs, and 316 to 377). The difference
  is outside the node's threads, item 391's read part; the scan sent the same bytes a second in both (32.2 and 32.0 MiB
  of titan's), and why its answers cost the reads more once nothing on the node waits for a device is not known.
- **The drive's faster pattern is the drive's**: windows where the cell ran 1,600 to 2,000 operations a second, its
  fdatasyncs shorter and more frequent, came in a control and in a scan reading its map as well, so no arm here is
  judged on one window alone.

### The fix

Decided with the user (2026-11-13 and 2026-11-14): item 392 is cured by pacing and counting a scan's reads of its map
and by reading each of the map's pages once a walk, through a scans' cache of the map's own rather than cursors kept
between pages ([Resolved #392](../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md)). A walk's cursors read
sixteen pages at once past their range, and past their tablet, and keep them for the walk's next pages; every read a
scan's page makes of its map is drawn from its budget and paced by its device, and counted. Through a node in process a
walk of 8,000 partitions in 4,096 pages read its map 4 times, where every page had read it. ~~Whether item 391's insert
part holds W7's budget with it is the next rounds', on the lab.~~

### The fix on the lab

**Run 2026-11-14, 01:31 to 02:24 UTC**, three rounds of the stage-profile node built at the fix (`fad49b35`): the 25% scan
and the cell alone, each traced, and the untraced control
([the predictions](../../../../shoal-spike-scan/results/item-391/predictions.md)).

| The 25% scan, three rounds | Before the fix (the trace's run) | With the fix |
| --- | --- | --- |
| Cell's insert p99 ratio | 1.38 [1.28, 1.39] | **1.06 [0.95, 1.18]** |
| Its p99 a second, the median's ratio | 1.22, 1.28, 1.25 | **1.04, 1.02, 1.01** |
| Node's own insert p99, ms: before, during | 40.6, 55.7 | 40.7, 42.4 |
| `synced`, p99 ms: before, during | 23.8, 34.9 | 23.6, 24.7 |
| Fdatasync in the kernel, p99 ms: before, during | 23.4, 34.7 | 23.4, 24.2 |
| titan's NVMe reads a second, during | 246 | 42 |
| Fsyncs a second: before, during | 364, 329 | 363, 364 |
| Cell's read p99 ratio | 1.97 [1.91, 1.98] | **2.78 [2.26, 2.79]** |
| Its p99 a second, the median's ratio | 1.98, 1.92, 1.90 | **2.16, 2.15, 2.21** |

- **Item 391's insert part is cured**: second by second the cell's inserts beside the scan are within 1.04 times their
  own, and the window's p99 within W7's 1.10 at its median. Round 2's 1.18 is six seconds of 130 to 210 ms on the node's
  own insert wait, the population a few arms of every run have had, scans and controls alike, which lines up with titan's
  crash-looping kubelet and etcd more often than not and is not the scan's; every other second of that arm held 34 to 56 ms.
- **The scan's figures count its map**: each shard's about 1,018 pages read the map 83 to 85 times, 64 KiB each, and took
  2,200 to 3,300 pages from the scans' cache.
- **The read part is larger than before the fix**: 2.16 to 2.21 times second by second, against 1.90 to 1.98, the node's
  own get p99 rising alike in both (312 to 373 µs with the fix). The difference is outside the node's threads, as the
  spike's was. It is item 391's read part, ~~and now more than its twin, the paced bulk flow with no Shoal in it (1.93)~~
  which the next run set beside its twin in the same rounds.

### The read part beside its twin

**Run 2026-11-14, 03:14 to 04:23 UTC**, three rounds on the fix's node, none traced: the 25% scan beside the cell on the
scanning client (europa), the cell driven from hyperion beside the same scan from europa, the cell beside two TCP
connections from titan to a sink on europa paced together to 31.25 MB a second with no Shoal in them, and the control
([the predictions](../../../../shoal-spike-scan/results/item-391/predictions.md)).

| Arm, three rounds | Cell's read p99 ratio | Its p99 a second, the median's ratio | Insert p99 a second |
| --- | --- | --- | --- |
| The cell alone | 1.00 [1.00, 1.02] | 1.00 [1.00, 1.01] | 0.99 |
| The 25% scan, the cell on its client | 2.22 [2.16, 2.71] | 2.07 [2.03, 2.25] | 1.06 |
| The 25% scan, the cell on another client | 2.22 [2.00, 2.27] | 2.09 [2.06, 2.18] | 1.03 |
| A bulk flow at 25%, no Shoal | 1.89 [1.83, 2.51] | 1.88 [1.83, 2.58] | 1.02 |

- **Another client's cell rises as much as the scanning client's**: the scan's cost to the reads is not its own client's
  work beside the cell, and the earlier run's other client said the same before the fix.
- **The scan costs the reads what a paced bulk flow of its rate costs, within its spread**: the scan's medians are about
  a tenth above the twin's, 2.07 and 2.09 against 1.88 second by second, and the ranges overlap, the twin's own from 1.83
  to 2.58 a round. The fix's 2.2 against the trace's 1.9 before it is inside that spread too.
- **The inserts hold beside each**, 1.02 to 1.06 second by second.

So item 391's read part is titan's network path under any steady quarter of its link: a scan, or two connections with
nothing of Shoal in them, raises the cell's read p99 about twice. W7's read clause is judged against that twin, as decided
with the user.

## The confirmation

~~Not run: it judges M24's exit at the default, and no share holds the budget while item 391 stands. It runs once 391 is
cured, as M24's exit again.~~

**Run 2026-11-14, 04:36 to 08:38 UTC**: eight rounds and 64 arms (`CONFIRM=1`).

- **What ran:** M24's exit's resident half again, at the default share, a quarter of titan's gigabit.
- **The node:** item 392's fix (`fad49b35`), built as the product is, with no profiling feature.
- **Every round**, its arms' order alternating by round:
  - the cell beside a resident scan in rows, and in partitions;
  - the control;
  - a resident scan alone;
  - another client's cell, driven from hyperion beside a scan from europa;
  - **the twin**: the cell alone beside a bulk flow of 31.25 MB a second from titan, with no Shoal in it, paced as a
    scan's answers are (`CONFIRM_BULK`).
- **Rounds 1 to 4** ran the archived control and the cell beside an archived scan in rows too.

The predictions were written before any round
([predictions.md](../../../../shoal-spike-scan/results/link-pace/confirm/predictions.md), where each is marked). The
whole output is [report.md](../../../../shoal-spike-scan/results/link-pace/confirm/report.md). Every scan handed out every
row once (36 of 36), no cell operation failed, and all three hosts matched their snapshots after.

| Arm, eight rounds | Read p99 ratio | Its p99 a second | Against the twin | Insert p99 ratio | Against the control |
| --- | --- | --- | --- | --- | --- |
| The cell alone (the control) | 1.02 [0.96, 1.04] | 1.00 [0.98, 1.01] | - | 0.99 [0.90, 1.11] | - |
| Beside a resident scan in rows | 2.20 [2.02, 2.69] | 2.19 [2.12, 2.26] | overlap | **1.00 [0.91, 1.07]** | overlap |
| In partitions | 2.22 [1.85, 3.05] | 2.13 [2.05, 2.28] | overlap | **1.01 [0.88, 1.10]** | overlap |
| Another client's cell (hyperion) | 2.27 [2.00, 2.56] | 2.15 [2.05, 2.50] | overlap | **1.04 [0.92, 1.07]** | overlap |
| The twin: a paced bulk flow, no Shoal | 2.35 [1.83, 2.77] | 2.07 [1.85, 2.81] | - | 0.99 [0.95, 1.03] | overlap |

| Archived, four rounds | Read p99 ratio | Insert p99 ratio | The scan |
| --- | --- | --- | --- |
| The cell alone (the archived control) | 0.85 [0.78, 0.98] | 1.00 [0.95, 1.02] | - |
| Beside a scan of archived rows | 0.69 [0.66, 1.03] | 1.05 [0.97, 1.11] | 300.4 [294.5, 307.7] s |

- **The insert clause holds beside every resident scan.**
  - At the median the inserts rose 1.00, 1.01 and 1.04 times, each range overlapping the control's. Before item 392's
    fix the 25% scan raised them 1.16 times in the sweep and 1.27 in the diagnostic.
  - titan's NVMe read 14 to 43 times a second beside the scans, against the control's 22.
- **The read clause holds against the twin.**
  - Every resident arm's range overlaps the twin's, over the window and second by second.
  - The medians' quotients are 0.94 to 0.96 over the window and 1.03 to 1.06 second by second.
  - Against the bare budget every arm is over, the twin as well: any steady quarter of titan's link costs the reads
    about twice, with Shoal in it or not.
- **The archived half holds as at the exit**, with a scan's map reads paced as its records are: reads 0.69 and inserts
  1.05 against the archived control.
- **A resident scan alone** takes 33.3 s every round, at 29.8 MiB a second of bodies, 95% of its share.
- **The step clause**, read as M23 read it with its remainder apart:
  - 100 to 139 of a resident scan's steps in 10,000 pass 100 µs by the wall;
  - 92 to 96% of those that pass on their own cpu are encode steps;
  - none passes 10 ms, and the longest is 4.3 ms.

  That is more than at the exit (34 and 77) or in the sweep (88 to 115). The remainder is filed as
  [O180](../../appendix/optimizations.md#o180-a-paced-resident-scans-encode-steps-run-past-their-bound-on-their-own-cpu-about-once-in-ninety),
  after M24.

So the conditions decided with the user hold: the inserts within the budget at the median, the reads overlapping the
twin, and the archived arms within. Item 391 is resolved
([its page](../../appendix/resolved/paced-scan-reads-and-inserts.md)), and
[M24](milestones.md#m24-scans-on-one-node) is delivered.

## The slice again, after item 392

**Run 2026-11-14, 08:50 to 10:21 UTC**: four rounds and 16 arms (`PACE_ONLY=1`), on the confirmation's build (`f01ab412`).
The user decided that [O179](../../appendix/optimizations.md#o179-a-shard-holds-its-cpu-through-its-eevdf-slice-while-the-io-wq-worker-running-its-fdatasync-waits)'s
slice is measured again once item 392 is fixed, since the first test ran with the scan still reading its map from the
device. Each arm was a node made fresh and loaded, every arm but the control traced as the slice test's were:

- the cell beside a 25% scan of resident rows, at the default slice;
- the same with every thread of the node at a slice of 100 µs;
- the cell alone at 100 µs;
- the round's control, the cell alone at the default slice, untraced.

The predictions were written before any round
([predictions.md](../../../../shoal-spike-scan/results/o179/predictions.md), where each is marked), and the whole output
is [report.md](../../../../shoal-spike-scan/results/o179/slice/report.md). Every scan handed out every row once (8 of 8),
no cell operation failed, and both hosts matched their snapshots after.

| Arm, four rounds | Cell's insert p99 ratio | Read p99 ratio | Fdatasync in the kernel, p99 ms: before, during | Workers' waits past 1 ms a second: before, during | titan's NVMe flushes a second, during | Steps past 100 µs a 10,000 |
| --- | --- | --- | --- | --- | --- | --- |
| The cell alone | 1.01 [0.94, 1.05] | 0.99 [0.96, 1.02] | - | - | 374 | - |
| The cell alone, 100 µs | 0.99 [0.94, 1.03] | 0.99 [0.98, 1.00] | 23.2, 22.9 | 0.2, 0.2 | 376 | - |
| The 25% scan | 0.99 [0.93, 1.05] | 1.97 [1.94, 2.56] | 23.2, 23.3 | 0.9, **9.8** | 378 | 133.4 [124.7, 141.3] |
| The 25% scan, 100 µs | 0.95 [0.94, 1.10] | 2.00 [1.99, 2.29] | 23.9, 22.8 | 0.2, **0.2** | 364 | 135.4 [125.9, 150.1] |

- **The slice still takes the workers' long waits away**: beside the scan at the default slice they waited past a
  millisecond 9.8 times a second, 84% of those waits straight after a shard's executor ran, and at 100 µs 0.2 times.
- **They cost the inserts nothing**: with the scan's map reads paced and read once, the inserts held at either slice, the
  ranges overlapping, and the kernel's fdatasync p99 and the drive's flushes were what they were before the scan. This
  is what the spike with the map in memory showed, now on the product's build.
- **Nor does the slice cost a scan's steps**: as many passed 100 µs by the wall at 100 µs as at the default, and on their
  own cpu 1,914 over the rounds against 1,888, 91% of them encodes at either. A worker's preemption of its shard is short.
- **The reads rose about twice at either slice**, as any steady quarter of titan's link raises them.

So O179 stays optional and is not taken: it removes waits a run has not shown to cost anything. The tracing raised every
traced arm's read p99 before its scan to 506 to 519 µs, against the untraced control's 463, so a traced arm is judged
against its own before window and never against an untraced arm's reads.

## Related

- [F147](../../features/scan-link-pace.md), what is measured.
- [M24's exit](m24-exit.md), the miss this cures.
- [Resolved #391](../../appendix/resolved/paced-scan-reads-and-inserts.md) and
  [Resolved #392](../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md), what the rounds after the sweep found and cured.
- [W7](isolation.md#network), the rules on the wire.
