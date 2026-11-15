# M24's exit on the lab

**Run 2026-11-12.** This is the record of [M24](milestones.md#m24-scans-on-one-node)'s exit: a
client's scan over the wire to a standalone node, the reference cell on the scan's own client and
link, judged by [W7's foreground budget](isolation.md#the-budget), with both hosts' TCP counters and
the node's own scan figures read beside it. Eight rounds, 88 arms, every scan handing out every row
once, no cell operation failed, both hosts identical to their snapshots after.

**The answers.**

- **Beside a scan of archived rows the budget holds**, in rows, in partitions, filtered and
  projected: the cell's read p99 0.71 to 0.82 times its own before and its insert p99 1.07 to 1.08
  times, every range overlapping the control's. No eviction pass of any kind, and at the median no
  scan step past 100 µs. The scan read 3.7 MiB a second beside the cell and 16.7 alone.
- **Beside a scan of resident rows the budget misses.** The scan filled the link, 110.6 MiB a second
  of rows over 1 GbE, and the cell's read p99 went from 0.46 ms to 1.6 ms, 3.50 [2.41, 4.82] times,
  and its insert p99 1.63 [1.31, 1.77] times, both disjoint from the control in rows. The node's
  shards were each under a quarter of a core meanwhile. It is filed as
  [item 390](../../appendix/resolved/resident-scan-fills-the-link.md),
  and **whether M24 is delivered with it is [the user's to decide](#for-the-user-to-decide)**: M24
  is not marked delivered by this record. Decided since: the node paces a scan's answers by a share
  of its link ([F147](../../features/scan-link-pace.md)). M24 was delivered on 2026-11-14, after
  [the confirmation](link-pace.md#the-confirmation) ran this resident half again on the cure.
- **The step clause misses in the resident arms alone**: 34 and 77 of 10,000 scan steps passed
  100 µs there, nine in ten of them encode steps on their own cpu, the longest 0.8 ms.
- **What a body costs**, a scan alone: a row costs the client 0.69 µs in rows and 0.77 in
  partitions, and 1,085 and 1,133 bytes on the wire. A filter that keeps a row in a hundred puts 1%
  of the bytes on the wire and a projection of two fields 3.4%, each reading every record at the
  unfiltered scan's rate.
- **[Item 386](../../appendix/known-issues.md) is measured**: at the client's default of two streams
  a shard a page was shed for nearly every page answered, which cost the node 22.4 µs of cpu a row
  against 17.3 at one stream a shard, and the scan no time.

## The question

Does the reference cell keep W7's budget beside a client's scan when the scan's rows and the cell's
answers share one client and one link, as [X20](scylladb-scan-sources.md#what-the-run-says)'s rounds
carrying rows had it for another store? And what does a scan cost: its rate as its node reports it,
its node's and its client's cpu a row and its bytes on the wire, in rows and in partitions, filtered
and projected ([W12](performance.md#what-is-compared-with-what), [Q35](contract.md#questions-to-answer))?

## How it was judged

By W7 as written: the cell's read and insert p99 over a scan's own seconds within
`max(1.10 × before, before + 0.1 ms)` of the window before it, at the median over the rounds; no
eviction pass caused by memory; no scan step on a shard's loop past 100 µs; the scan's rate
reported. Beside it a **control every round**, the cell alone over the same seconds of an arm,
since M23's exit showed the cell moves by itself over an arm: a scan arm's rise is the scan's only
where its range is disjoint from the control's. Every scan is checked against the load: its rows
counted and their ids summed.

The predictions were written before the rounds
([predictions.md](../../../../shoal-spike-scan/results/m24-exit/predictions.md)), and amended after
the quick pass, before any round, where that pass had already shown one wrong.

## What was run

### The node, the driver and the build

- **The node**: `m24-node` on titan (Ryzen Embedded V1756B, Zen1, a 970 EVO, 1 GbE), the product's
  standalone node serving the spike's schema, five shards on the tmdb node's cores, 6 GiB, every scan
  setting its default, its allocator's purge delay 100 ms as every deployed unit's. It requires a
  principal: a user granted `all` drove it, and a user granted `read` alone was refused a scan by
  name before round one. Each second it writes its own scan, step and eviction figures to a file,
  since a standalone node answers no statistics over the wire.
- **The driver**: `m24-exit` on europa (Ryzen 9 7945HX), pinned to cpus 12 to 15 and their siblings,
  one process and one client. The cell is the reference cell's mixture, half reads and half inserts
  of a kibibyte row, one query a bundle, sixteen in flight, over 100,000 rows. The scan is a typed
  `client.scan(..)` on the same client.
- **The table scanned**: a million rows of a kibibyte with a tag of a hundred values, so a filter
  keeps one row in a hundred and a projection carries an id and a tag.
- **The build**: the product at `53c6a904`, one znver1 build for both programs
  (`m24-node` sha256 `eec74925…0245`, `m24-exit` `f96fc269…5396`).
- **The hosts**: the tmdb node stopped on both, the maintenance timers held, the governors at
  `performance`, all put back by the run
  ([m24-exit-host-changes.txt](../../../../shoal-spike-scan/results/m24-exit/m24-exit-host-changes.txt));
  `verify` found both identical to their snapshots.

The script is [m24-exit-lab.sh](../../../../shoal-spike-scan/results/m24-exit-lab.sh), its sampler
[m24-exit-sample.sh](../../../../shoal-spike-scan/results/m24-exit-sample.sh), and the tables come
from [m24-exit-report.py](../../../../shoal-spike-scan/results/m24-exit-report.py), whose whole
output is [report.md](../../../../shoal-spike-scan/results/m24-exit/report.md).

### The rounds

Eight rounds, each on a node made fresh and loaded, five and three quarter hours in all. In a round:

| Arms | The node | What they answer |
| --- | --- | --- |
| The cell beside a scan of resident rows, in rows and in partitions | As loaded, nothing evicted | The link: a resident row is read from memory and nothing paces it |
| The cell beside a scan of archived rows, in rows and in partitions; filtered and projected in rounds 1 to 4 | Restarted before each | The budget beside a paced scan |
| The cell alone over an arm's seconds | Restarted | The control |
| A scan alone in rows, in partitions, filtered, projected, and in rows at one stream a shard | Restarted before each | A scan's own rate and cost; item 386's price |

Every cell arm warms 100 s before it measures ([Resolved #351](../../appendix/resolved/the-cells-fall-is-the-drives.md)),
measures 60 s before its scan, the scan's own seconds, and 30 s after. The archived arms' order
reverses by round.

## The tables

Every cell is the median over the rounds and its range.

### The cell beside a scan

| Arm | Read p99 before, µs | During, µs | Ratio | Insert p99 before, µs | During, µs | Ratio | Budget | Against the control |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| The cell alone (the control) | 5,681 | 4,168 | 0.83 [0.58, 0.93] | 40,991 | 41,167 | 1.00 [0.95, 1.07] | within | - |
| Archived, rows | 6,005 | 4,373 | 0.73 [0.62, 0.92] | 40,575 | 44,111 | 1.08 [1.05, 1.16] | within | overlap, overlap |
| Archived, partitions | 5,789 | 4,355 | 0.73 [0.65, 0.89] | 40,687 | 44,271 | 1.08 [1.01, 1.18] | within | overlap, overlap |
| Archived, filtered | 6,601 | 4,687 | 0.71 [0.69, 0.84] | 42,175 | 44,927 | 1.07 [1.06, 1.10] | within | overlap, overlap |
| Archived, projected | 5,929 | 4,909 | 0.82 [0.71, 1.03] | 41,423 | 45,151 | 1.07 [1.01, 1.17] | within | overlap, overlap |
| Resident, rows | 462 | 1,616 | 3.50 [2.41, 4.82] | 40,079 | 65,983 | 1.63 [1.31, 1.77] | **over** | disjoint, disjoint |
| Resident, partitions | 459 | 1,629 | 3.53 [3.45, 4.78] | 40,239 | 66,719 | 1.65 [0.63, 1.76] | **over** | disjoint, overlap |

The cell ran at about 1,390 operations a second before each scan. Beside an archived scan it kept
0.97 to 1.02 of that, and beside a resident one 0.84 and 0.81.

Two things about the cell itself, so the ratios are read rightly:

- **Its reads are slower after a restart, and quicken through an arm.** A restarted node reads the
  cell's rows from their archives, a read p99 near 6 ms where the freshly loaded node's is 0.46 ms,
  and the control's own read ratio is 0.83. The archived arms' read ratios under one are the cell's,
  not a scan's gift.
- **Its insert p99 is 41 ms**, at sixteen in flight on one standalone node past the drive's change
  of flush. The budget is a ratio of the cell to itself, so it is judged as it stands; the figure is
  not a product claim.

### The scans

| Arm | Scan, s | Rows/s handed out | Rows/s read | MiB/s of bodies | The node's own: MiB/s | Partitions/s | Wire, MiB/s | Pages | Asked again |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Archived, rows, beside the cell | 270.0 [253.6, 294.7] | 3,705 | 3,705 | 3.7 | 3.7 [3.4, 3.9] | 3,699 | 4.8 | 1,004 | 2,292 |
| Archived, partitions, beside the cell | 265.6 [251.7, 299.9] | 3,768 | 3,768 | 3.9 | 3.8 [3.3, 4.0] | 3,766 | 5.1 | 1,004 | 2,232 |
| Archived, filtered, beside the cell | 235.0 [229.6, 249.0] | 43 | 4,256 | 0.0 | 4.2 [4.0, 4.3] | 4,252 | 1.0 | 334 | 1,292 |
| Archived, projected, beside the cell | 288.5 [271.2, 294.9] | 3,467 | 3,467 | 0.1 | 3.5 [3.4, 3.7] | 3,462 | 1.1 | 1,004 | 1,828 |
| Resident, rows, beside the cell | 9.0 [9.0, 9.0] | 111,408 | 111,408 | 110.6 | 107.6 [105.6, 108.8] | 107,646 | 116.3 | 1,004 | 0 |
| Resident, partitions, beside the cell | 9.4 [9.4, 9.5] | 106,627 | 106,627 | 110.6 | 105.0 [97.2, 107.3] | 105,012 | 113.8 | 1,004 | 0 |
| Alone, rows | 59.4 [59.1, 60.0] | 16,823 | 16,823 | 16.7 | 16.7 [16.6, 16.9] | 16,758 | 17.4 | 1,004 | 946 |
| Alone, partitions | 61.2 [61.0, 61.6] | 16,332 | 16,332 | 16.9 | 16.3 [16.2, 16.3] | 16,275 | 17.6 | 1,004 | 958 |
| Alone, filtered | 53.4 [53.3, 53.9] | 187 | 18,734 | 0.2 | 18.7 [18.4, 18.7] | 18,672 | 0.2 | 255 | 314 |
| Alone, projected | 58.3 [57.8, 58.7] | 17,160 | 17,160 | 0.5 | 17.1 [16.9, 17.2] | 17,112 | 0.6 | 1,004 | 412 |
| Alone, rows, one stream a shard | 59.5 [59.2, 59.8] | 16,805 | 16,805 | 16.7 | 16.7 [16.5, 16.8] | 16,757 | 17.3 | 1,003 | 0 |

The wire's figures beside the cell include the cell's own megabyte a second. The node's own rate is
what its shards counted, read from its figures; it agrees with what the client received.

- **An archived scan alone is bound by its device's share**: 16.7 MiB a second, every arm within a
  percent of every other round's. Beside the cell it reads 3.7, since the cell's reads are in flight
  on the device and the scan keeps to its trickle and what idle time is left
  ([F119](../../features/scan-pacing.md), [F128](../../features/device-time-pace.md)).
- **A resident scan is bound by the link.** 110.6 MiB a second of rows is 116 on the wire, a
  gigabit.
- **In partitions a scan alone is 3% slower by the wall than in rows**, 61.2 s against 59.4, the
  ranges disjoint, and 4% slower on the full link, where it carries 4.5% more bytes a row. Beside
  the cell the two overlap.

### What a row costs, a scan alone

| Way | Bytes of body a row | Wire bytes a row handed out | Client cpu a row handed out, µs | Client cpu a row read, µs | The node's threads' cpu a row read, µs | Scan steps' own cpu a row read, µs |
| --- | --- | --- | --- | --- | --- | --- |
| Rows | 1,041 | 1,085 [1,078, 1,090] | 0.69 [0.65, 0.73] | 0.69 | 22.40 [22.05, 22.89] | 0.02 |
| Partitions | 1,088 | 1,133 [1,128, 1,139] | 0.77 [0.75, 0.82] | 0.77 | 23.20 [22.95, 23.52] | 0.02 |
| Filtered | 1,058 | 1,204 [1,189, 1,209] | 11.50 [10.00, 13.00] | 0.11 | 17.52 [17.23, 17.96] | 0.02 |
| Projected | 33 | 36 [35, 36] | 0.23 [0.20, 0.26] | 0.23 | 20.02 [19.14, 21.08] | 0.02 |
| Rows, one stream a shard | 1,041 | 1,082 [1,071, 1,090] | 0.62 [0.59, 0.65] | 0.62 | 17.28 [17.03, 17.40] | 0.02 |

The node's cpu a row is every thread of the node over the scan's seconds, on a scan its device
holds to 17,000 rows a second: most of it is the node being awake for a minute, not the rows, and
it is a figure to compare arms by, never a cost a row. The steps' own cpu is what the loop's scan
steps spent, 0.02 µs a row.

- **Partitions cost a client 1.12 times rows** a row, the ranges disjoint, and the node 1.04 times.
- **A filter saves the wire and the client, not the device**: 1% of the unfiltered scan's bytes on
  the wire and takes a tenth less time, every record read. A projection puts 3.4% of the bytes
  on the wire and takes the unfiltered scan's time.
- **A scan's shed pages cost its node 5 µs a row**: 22.4 µs at two streams a shard against 17.3 at
  one, disjoint, with 946 pages asked again of 1,004 answered and none at one stream; the scan
  takes 59.4 s either way.

### The node's own figures over each scan

| Arm | Scan steps | Past 100 µs by the wall, a 10,000 | On their own cpu, a 10,000 | Past 1 ms, the rounds summed | Eviction passes: resident, budget, floor | Pages shed, the rounds summed | Given up |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Archived, rows | 5,083 | 0.0 [0.0, 0.0] | 0.0 | 0 | 0, 0, 0 | 18,702 | 0 |
| Archived, partitions | 5,082 | 0.0 [0.0, 12.6] | 0.0 [0.0, 11.7] | 0 | 0, 0, 0 | 18,318 | 0 |
| Archived, filtered | 4,154 | 0.0 [0.0, 0.0] | 0.0 | 0 | 0, 0, 0 | 5,212 | 0 |
| Archived, projected | 7,368 | 1.5 [0.0, 9.2] | 0.5 [0.0, 7.1] | 0 | 0, 0, 0 | 7,230 | 0 |
| Resident, rows | 50,548 | 33.8 [31.4, 40.6] | 29.8 [25.5, 36.7] | 2 | 0, 0, 0 | 0 | 0 |
| Resident, partitions | 49,764 | 77.3 [71.3, 85.2] | 71.8 [65.4, 81.8] | 1 | 0, 0, 0 | 6 | 0 |
| Every scan alone | 5,046 to 5,057 | 0.0 [0.0, 0.0] | 0.0 | 0 | 0, 0, 0 | 2,542 to 7,477; none at one stream | 0 |

No eviction pass ran in any figures file, the loads' included, so the resident arms' rows were all
resident. In a round's resident arms together (round 2), 518 of 93,639 encode steps passed 100 µs,
487 of them on their own cpu, 175 µs by the wall on average and the longest 0.78 ms; 53 of 10,180
captures did, 23 on their own cpu.

### The link

Both hosts' TCP counters over each scan's seconds, a second.

| Arm | titan sent, MiB/s | titan: segments resent | titan: loss probes | titan: timeouts | europa: segments resent |
| --- | --- | --- | --- | --- | --- |
| The cell alone (the control) | 1.0 | 40.41 [35.67, 45.34] | 40.29 | 4.63 | 0.00 |
| Archived, rows | 4.8 | 49.19 [42.79, 52.43] | 49.15 | 4.80 | 0.00 |
| Archived, partitions | 5.1 | 49.22 [41.86, 55.19] | 49.17 | 4.68 | 0.00 |
| Resident, rows | 116.3 | 97.97 [47.24, 212.79] | 97.97 | 5.27 | 0.00 |
| Resident, partitions | 113.8 | 124.81 [13.26, 197.79] | 124.81 | 5.02 | 0.00 |
| A scan alone, any way | 0.2 to 17.6 | 0.13 to 0.22 | 0.00 to 0.17 | 4.31 to 5.19 | 0.00 |

- **Every segment titan resent beside the cell was a loss probe europa had no need of.** In round
  2's control titan sent 4,008 probes in 100 s and europa answered 4,005 of them as duplicates
  (`TCPDSACKRecv` 4,008 on titan, `TCPDSACKOldSent` 4,005 on europa); so in the archived and the
  resident arms read. A scan alone resends nothing. It is the cell's small answers probed after the
  5 ms minimum a client's connection holds
  ([Resolved #304](../../appendix/resolved/lone-frame-loss-probe.md)), about three answers in a
  hundred, and is filed as
  [O178](../../appendix/optimizations.md#o178-a-clients-connection-probes-three-in-a-hundred-of-its-small-answers-for-a-loss-that-did-not-happen).
- **A resident scan doubles them and adds no timeout**: 98 and 125 a second against the control's
  40, rows disjoint from it. Nothing here waited the kernel's 200 ms, which is what X20's foreground
  waited.
- **titan's timeouts, four to five a second in every arm**, are there with no cell and no scan
  bytes to speak of; they are the host's, counted host wide, and were not followed.

## Predictions against results

| Prediction | Result |
| --- | --- |
| 1. The budget holds beside an archived scan, ratios 0.95 to 1.12, overlapping the control | **Held** on the verdict. The read ratios were 0.71 to 0.82, under the range, as the control's own was |
| 2. An archived scan does not fill the link; 18 to 33 MiB a second alone | **The claim held, the rate did not**: 16.7 alone and 3.7 beside the cell (amended after the quick pass to 12 and 4). "A few tens" of resent segments was wrong in number, thousands an arm, and right against the control |
| 3. The control's ratio is not 1.00 | **Half right**: the insert ratio was 1.00 [0.95, 1.07], the read ratio 0.83 |
| 4. The resident arm misses if any does: read p99 2 to 10 times, in milliseconds | **Held**: 3.5 times, 1.6 ms. Its rate alone, 60 to 112 MiB a second, was not measured: no resident scan ran alone |
| 5. Retransmissions rise on titan in the resident arms by hundreds | **Held**, about 500 more over a scan's nine seconds |
| 6. Partitions cost the node at most 0.7 times rows and the client 1.2 to 2 times; bytes within 10% | **Wrong on the node**, 1.04 times; the client 1.12 times, under the range; bytes 4.5% apart |
| 7. A filter and a projection take within 15% of the unfiltered scan's time, at about 1% and 2 to 4% of its bytes | **Held**: 0.90 and 0.98 of its time, 1% and 3.4% of its bytes, a million rows read by each |
| 8. No `Resident` eviction pass | **Held**, and none during a load |
| 9. Under 30 scan steps in 10,000 past 100 µs | **Held archived** (none at the median), **missed resident**: 34 and 77 |
| 10. `asked_again` zero alone | **Wrong**, said before the rounds: 946 a scan at the default streams |
| 11. Every scan hands out every row once | **Held**, 80 of 80 |

Of what would have surprised: no archived arm left the budget and no cell operation failed, but **a
scan in partitions was slower than a scan in rows by the wall**, 3% alone.

## What was found on the way

- **A scan of resident rows fills its client's link**
  ([item 390](../../appendix/resolved/resident-scan-fills-the-link.md)),
  the miss above.
- **A client's connection probes its small answers for a loss that did not happen**
  ([O178](../../appendix/optimizations.md#o178-a-clients-connection-probes-three-in-a-hundred-of-its-small-answers-for-a-loss-that-did-not-happen)),
  optional.
- **Item 386 has a price**, 5 µs of the node's cpu a row and a shed page for each answered, on a
  node of five shards at the client's defaults. Its page says so now; its cure is still the user's.
- **A standalone node's figures cannot be read from outside it**, which
  [the todos page](../../appendix/todos.md) already had; the exit's node writes them to a file
  instead, and the bench cannot drive a client's scan beside a cell at all, which is why the exit
  has a driver of its own.

## What it says for the design

- **The pace does what it was built for, over the wire as on the walk.** A scan of archived rows
  beside the cell is held to a quarter of its rate alone and moves neither p99 past the control.
- **Nothing paces a resident row, and W7 said so**: "nothing here shares out the link itself"
  ([the wire](isolation.md#network)). The scan's connection keeping the kernel's minimum and
  the foreground's keeping 5 ms kept the cell's wait to milliseconds, where X20's other store went
  to 200 ms; they did not keep it to the budget.
- **The link held the cell, not the shards**: each shard thread ran at 14 to 21% of a core through
  a resident scan (round 2), and the cell's rate fell by a sixth.
- **Rows as the default body stands** ([Q35](contract.md#q35-what-a-scans-answer-carries-2026-11-06)):
  partitions cost a client and a node a little more a row on this table of one row a partition, and
  4.5% more bytes.

## For the user to decide

1. **The resident miss, and whether M24 is delivered.** The budget's sentence is "beside one full
   scan a node at its default ceiling", and the exit asks for X20's case; a resident table scanned
   by the cell's own client over 1 GbE is that case, and it is over on both p99s. The ways out I
   see, in the order I would take them:
   - **a ceiling on a scan's answer bytes**, a node's or a connection's, as the device's ceiling is
     for archived bytes: the default would be the question, since 100 MiB a second is what the
     device's is and is a gigabit;
   - **the client holding its scan's connections to fewer bytes in flight** when it shares its link
     with its own foreground, which only the client knows;
   - **reading the clause as the device's**, the link left to the deployment, and saying so on W7.
   Until one is chosen M24 stays open: its box and its gate's row are not marked.

   **Decided with the user on 2026-11-13: the first**, as a share of the link's speed
   ([F147](../../features/scan-link-pace.md)). Its sweep ([the record](link-pace.md)) cured the link's
   part, item 390, and found ~~the node's own cost~~ what is left beneath it,
   [item 391](../../appendix/resolved/paced-scan-reads-and-inserts.md),
   which M24 ~~now waits~~ waited on. **Resolved by [the confirmation](link-pace.md#the-confirmation)**
   (2026-11-14): the exit's resident half was run again over eight rounds at the default share, on
   [item 392](../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md)'s fix, with the scan's twin in every
   round.
   - The cell's inserts held the budget at the median, at 1.00 to 1.04 times.
   - Its reads overlapped the twin's, a paced bulk flow of the scan's rate, against which the user decided the
     read clause is judged.
   - M24 is delivered.
2. **The step clause in the resident arms**: 34 and 77 steps in 10,000 past 100 µs, encode steps on
   their own cpu. M23 read this clause with its remainders apart
   ([Resolved #350](../../appendix/resolved/encode-copy-deadline.md)'s Still open is the same kind);
   whether these are that remainder or a cost of the link being full was not separated here.
   **Decided with the user on 2026-11-13**: the clause is read as M23 read it, with its remainder kept apart. At the
   confirmation 100 to 139 steps in 10,000 passed, 92 to 96% of them encode steps on their own cpu, and the remainder
   is filed as
   [O180](../../appendix/optimizations.md#o180-a-paced-resident-scans-encode-steps-run-past-their-bound-on-their-own-cpu-about-once-in-ninety),
   after M24.
3. **Item 386's cure**, priced above. **Decided with the user on 2026-11-13**: it stays open after M24.
   **Resolved on 2026-11-15** ([Resolved #386](../../appendix/resolved/scan-pages-held-to-the-bound.md)), the user's
   choice: the client holds its pages on each connection to its node's bound.

## Limitations

- **One node, one pair of hosts, one link.** hyperion and europa as a node were not run, though the
  budget names all three hosts.
- **The driver is the exit's own**, not loadgen's `Driver`, which reads rows from a file and cannot
  scan through a client beside a cell; its cell is the reference cell's mixture and bundle, not its
  code. The approved plan had it on loadgen's.
- **One build for both programs**, znver1, where the plan had the driver native.
- **A million rows, not the two the predictions were written for**, changed before the rounds so an
  archived arm fits its cell's feed.
- **The filtered and projected scans ran beside the cell in four rounds**, alone in all eight.
- **No resident scan ran alone**, so what the link costs a resident scan's own rate is not known,
  and no arm separates the link from the node in the resident miss: the shards' idle cpu is the
  evidence, from one round's samples.
- **The client's port is plaintext**, with a principal by SCRAM; nothing was measured under TLS.
- **"No change in how long a small answer waits in a connection's outbox"**, W7's fourth clause, is
  not measured apart from the cell's p99.
- **The node's figures are polled each second**, so a scan's are its seconds' to within one.
- **TCP counters are a host's**, not a connection's.

## Related

- [W7](isolation.md), the budget and the rules on the wire; [M24](milestones.md#m24-scans-on-one-node).
- [M23's exit](m23-exit.md), the same budget on the walk, on a cluster.
- [X20](scylladb-scan-sources.md), another store's scan carrying its rows to one client.
- [F143](../../features/scan-rows.md), [F144](../../features/scan-pages-on-the-wire.md),
  [F145](../../features/scan-driver.md), what was measured.
- The records: [the captures](../../../../shoal-spike-scan/results/m24-exit/captures),
  [arms.json](../../../../shoal-spike-scan/results/m24-exit/arms.json),
  [report.md](../../../../shoal-spike-scan/results/m24-exit/report.md).
