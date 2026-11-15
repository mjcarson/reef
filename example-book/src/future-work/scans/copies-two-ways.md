# X38. Which copies a scan reads, two ways

**Reported** (run 2026-11-14). This is the record of spike [X38](spikes.md#x38-which-copies-a-scan-reads-two-ways):
which of a tablet's voter copies a scan across a cluster prefers, the half of
[Q38](contract.md#questions-to-answer) the user asked to have measured. The other half, which copies may be read
at all and how a level is served, is settled by design on [W13](contract.md#q38-which-copies-a-scan-reads-and-its-levels-2026-11-14).

**The verdict: option 1, followers then leaders.** The two options read as fast and cost the foreground the same on
every figure; only the share of pages a leading copy answered tells them apart, 0.36 against 0.48 beside the cell.
The rule reads a tie, and a tie is option 1's. What made both fast is what they share: every tablet dealt once to a
lane a member, and a member out of tablets taking from the others' lanes. [The control](#the-control-the-rotated-driver-at-the-same-commit)
measured that against the rotated driver that every scan before X38 used, at the same commit. Dealing and taking
read `Big` 2.66 times as fast alone and 5.1 times as fast beside the cell, and `Keyword` 17.8 times as fast, at no
cost the cell or the paced stream showed.

## The question

Two options, side against side:

- **Option 1, followers then leaders** (`--scan-copies followers`): every tablet dealt to one of its followers,
  so each member reads an even share, its leader only when no follower can be asked; a member that has read
  its own share takes a tablet it follows from another member's lane, and one it leads only once none it
  follows waits anywhere.
- **Option 3, every voter alike** (`--scan-copies voters`): every tablet dealt to any of its copies, its leader
  as much as a follower; a member that has read its own share takes any tablet it holds a copy of.

Both deal every tablet once to a lane a member, evenly, give each member its shards' share of the streams and
let a member take from the others' lanes what it holds a copy of
([F115](../../features/bench-scans.md#the-scan)'s driver). So they differ in the copies they prefer and in
nothing else: a deal with no taking would finish at the pace of the slowest device whichever copies it chose,
which is what the rotated driver every scan before X38 used does today. On M23's rounds again its europa read
its share of `Big` inside the arm's warmup and its device was idle after.

**What would change the design**, fixed with the user before the run:

- Option 3 is taken only if its scan is faster by a disjoint range, beside the cell or alone, its cell
  figures hold [W7](isolation.md#the-budget)'s budget wherever option 1's do, and no cell or paced figure is
  worse by a disjoint range.
- Otherwise option 1, a tie included, since it spares leaders at no measured cost.
- A result the rule cannot read goes to the user.

[The predictions](../../../shoal-spike-scan/results/x38/predictions.md) were written and committed before any
round ran.

## How it is run

On M23's side cluster `m23` through its harness ([M23's exit](m23-exit.md)): europa on the Optane at
`lead_weight: 2`, so it leads most groups, and titan and hyperion on their 970 EVOs, every scan setting the
product's default, under the `performance` governor with the tmdb node and the maintenance timers held and put
back. One worktree at one commit builds the node and the driver for both sides; a side is its driver's flag
(`AB_A_BENCH="--scan-copies followers"`, `AB_B_BENCH="--scan-copies voters"`), written beside each round's
captures.

- **The Big leg** (`BIG_ONLY=1 AB_SIDES=1`, eight side-rounds, ABBA twice): a fresh cluster a round, its preload
  and a restart, then the cell alone, `Big` and `Keyword` each scanned alone back to back, and the cell beside
  one whole scan of `Big`: a scan read from its archives, bound by its devices' shares.
- **The Keyword leg** (`KEYWORD_ONLY=1 AB_SIDES=1`, eight side-rounds): the cell beside a whole scan of a
  resident `Keyword`, while Keyword's own paced stream of 200 inserts a second writes into the groups the scan
  reads, so a leader's copy read under option 3 is the copy its group's writes commit through.

`ab-report.py big` and `keyword` read every round by its side: the whole scan's time beside the cell, the scan of
`Big` alone's rate, each member's share of the pages and the share a copy that led its group answered, the
tablets each took from another's lane, the cell's figures beside the scan and alone, and the paced stream's
insert p99 before the scan and while it ran. The cell's figures beside a scan are its event's `during` window,
cut at the scan's own marks.

## What was run

- **The build**: `df39dddd`, one worktree (`target/lab/x38/wt`), the node built for `znver1` into its
  `target/deploy` (`x16-node`, sha256 `221e4475…`), the driver the same commit's.
- **The cluster**: `m23`, 6 GiB a node (`NODE_MEMORY`). europa: the Optane at `/optane/shoal-m23`, six shards,
  `lead_weight: 2`. titan and hyperion: their 970 EVOs at `/xfs/shoal-m23`, five shards each. Every scan setting
  the product's default, the cell M23's rounds again's of 1,100,000 rows.
- **The run**: a system unit, `x38-chain`, from copies of the harness: the Big leg 20:19 to 22:14 UTC, the
  Keyword leg to 23:04, each leg snapshotted before it and verified after, every host identical to its snapshot
  both times. The rounds are in [`results/x38`](../../../shoal-spike-scan/results/x38/), the predictions beside
  them.
- **The QUICK pass first**, one round a side of each leg at a tenth of `Big`, proved every arm ran and the new
  figures landed. It is not kept: its arms are too short to read (below, on leads).

## The figures

### Big

Medians over four rounds a side, the range in brackets, read by `ab-report.py big`. The guide is the rotated
driver's on the same cluster four days before ([O147's](../../../shoal-spike-scan/results/o147/) side A,
2026-11-10, the product's defaults then), never a side: M24's work and item 392 landed between.

| Figure | A: followers | B: voters | Disjoint | Guide: rotated, 2026-11-10 |
| --- | --- | --- | --- | --- |
| Whole scan beside the cell, s | 28.2 [28.1, 28.4] | 28.4 [28.3, 28.9] | no | 82.7 [73.8, 88.5] |
| Big alone, partitions a second | 131,000 [130,700, 131,500] | 131,100 [130,800, 131,200] | no | 65,900 [63,000, 76,100] |
| Big alone, MiB a second | 130.0 [129.6, 130.5] | 130.0 [129.7, 130.2] | no | 65.4 |
| europa's share of the pages, beside the cell | 0.84 [0.840, 0.845] | 0.85 [0.843, 0.853] | no | 0.24 |
| europa's share, Big alone | 0.69 [0.692, 0.693] | 0.69 [0.691, 0.693] | no | 0.21 |
| Pages a leading copy answered, beside the cell | 0.36 [0.308, 0.397] | 0.48 [0.483, 0.542] | **yes** | |
| Pages a leading copy answered, Big alone | 0.26 [0.223, 0.307] | 0.44 [0.433, 0.482] | **yes** | |
| Tablets taken from another's lane, beside the cell | 2,085 [2,080, 2,099] | 2,098 [2,088, 2,133] | no | |
| The cell's read p99 beside the scan, ms | 3.17 [2.86, 3.23] | 3.14 [3.07, 3.47] | no | 3.25 |
| The cell's insert p99 beside the scan, ms | 32.0 [31.7, 32.4] | 32.2 [31.7, 33.8] | no | 32.7 |
| The cell's inserts a second beside the scan | 1,727 [1,689, 1,738] | 1,725 [1,685, 1,727] | no | |
| The cell alone: read p99, ms | 3.83 [3.46, 4.66] | 3.92 [3.65, 4.87] | no | |
| The cell alone: insert p99, ms | 34.0 [33.1, 34.9] | 33.6 [32.8, 35.0] | no | |
| The paced stream's insert p99 while the scan ran, ms | 28.4 [28.2, 29.2] | 28.7 [28.2, 29.1] | no | |
| Zen1 partitions a device read, Big alone | 37.2 [37.2, 37.6] | 37.1 [36.9, 37.3] | no | 37.5 |
| Node cpu a 1,000 partitions, Big alone, ms | 43.0 [42.8, 43.2] | 43.3 [43.1, 43.5] | no | |

Every other figure the report prints overlaps too: no long capture or view-held capture on either side, steps past
100 µs by the wall alone a handful a side, and the restart before each capture 11.8 and 10.4 s.

### Keyword

Medians over four rounds a side, read by `ab-report.py keyword`. The guide is the rotated driver's
([O152's](../../../shoal-spike-scan/results/o152/) side A, 2026-11-09).

| Figure | A: followers | B: voters | Disjoint | Guide: rotated, 2026-11-09 |
| --- | --- | --- | --- | --- |
| Whole scan beside the cell, s | 4.95 [4.7, 5.1] | 4.8 [4.7, 5.0] | no | 141 [138, 185] |
| europa's share of the pages | 0.905 [0.882, 0.907] | 0.899 [0.887, 0.909] | no | 0.25 |
| Pages a leading copy answered | 0.36 [0.296, 0.527] | 0.50 [0.464, 0.529] | no | |
| Tablets taken from another's lane | 2,342 [2,247, 2,356] | 2,314 [2,264, 2,358] | no | |
| The cell's read p99 beside the scan, ms | 3.51 [3.43, 4.38] | 3.62 [3.54, 5.06] | no | 5.02 |
| The cell's insert p99 beside the scan, ms | 33.0 [32.0, 33.4] | 33.1 [32.2, 35.2] | no | 36.1 |
| The cell's inserts a second beside the scan | 1,700 [1,669, 1,723] | 1,689 [1,626, 1,707] | no | |
| The paced stream's insert p99, median second before, ms | 29.0 [27.9, 29.6] | 28.6 [28.3, 28.8] | no | 31.0 |
| The paced stream's insert p99, median second during, ms | 29.1 [28.6, 31.0] | 28.2 [27.4, 31.8] | no | 31.9 |
| Encode wall a resident partition, µs | 11.2 [8.9, 12.2] | 12.6 [12.1, 12.8] | no | |

A scan of `Keyword` now lasts five seconds, so its paced stream's median second during it is the median of about
five; the report's partitions a second and node cpu a 1,000 partitions are taken over seconds the scan does not
fill, and are left out here.

## Against the predictions

1. **Taking ends the slowest device's tail, on both sides: held, past its band beside the cell.** The whole
   scan of `Big` beside the cell took 28.2 and 28.4 s, under 150; `Big` alone read 131,000 partitions a second on
   both sides, over 45,000; europa answered 0.84 and 0.85 of the pages beside the cell and 0.69 alone, over 0.45.
   Alone that is where the prediction said it would stop, 60 to 70%. Beside the cell it went further: the cell's
   syncs and reads on the 970 EVOs hold their scans back (W42's factor A), and europa's Optane takes up what
   they leave.
2. **A leading copy's share: held at the median, two rounds below its band.** Under `followers` a leading copy
   answered 0.36 of the pages beside the cell at the median, and 0.31 and 0.32 in two of four rounds, under the
   band's 0.35; under `voters` 0.48, 12.8 points more, the ranges disjoint as the amendment said. Under
   `followers` every page a leading copy answered was europa's, its taking: it read some 3,450 of the 4,096
   tablets, about 2,080 of them taken from titan's and hyperion's lanes, and 1,260 to 1,630 of them it led.
   Titan and hyperion answered no page from a leading copy.
3. **The two sides read as fast: held.** Every figure of the scan's speed overlaps, on both legs.
4. **The cell sees the scan the same way on both sides: held, and not above the rotated driver's.** The cell's
   read and insert p99 beside `Big` overlap between the sides and sit beside the rotated driver's of 2026-11-10,
   not above them. Both sides' figures beside the scan are under the cell's alone, inside
   [W7's budget](isolation.md#the-budget).
5. **Keyword's writes do not tell the sides apart: held.** The paced stream's insert p99 during the scan and the
   cell's figures beside it overlap.

The rule: option 3's scan is not faster by any disjoint range, so it is not taken; a tie is option 1's.

## Leads after a restart

The driver's `followers` means the copies that are not the route table's preferred leader, the leader a
group returns to once leads settle. `led` in the figures is the copy's real leadership when it answered. After a
restart a shard hands a lead back to its preferred member one group every 5 s at most (O63's handback,
`BALANCE_INTERVAL`, `shoal-core/src/server/shard/groups.rs:89-99`), so for some minutes after a restart a group's
leader may be a copy the driver takes for a follower. The QUICK pass's short Keyword arm ran inside that time:
under `followers` a leading copy answered 0.38 of its pages, against 0.15 under `voters`, the other way round. In
the full rounds `Big`'s scan beside the cell runs after the cell alone and both scans alone, and the leads had
settled; `Keyword`'s capture follows its preload more closely, and its fourth `followers` round read 0.53 from
leading copies, as much as `voters`. M25's driver chooses by the same map, so **option 1 spares the leaders the
map prefers**; a lead that has not returned is read as a follower until it does.

## The control: the rotated driver at the same commit

The guides above are from other builds, so they cannot say how much of the change is the dealing and taking and
how much the build. The control measures it at one commit. It was run after the rounds, from 23:10 UTC on
2026-11-14 to 00:38 on the 15th, and [its predictions](../../../shoal-spike-scan/results/x38/predictions.md#the-control-at-the-commit-written-after-the-rounds-and-before-it-ran-2026-11-14)
were committed before it ran.

How it was run:

- **The same setup as the rounds**: the worktree, the node, the cluster and the harness.
- **The sides**: side A `--scan-copies rotated` (the default: one shared queue, a follower rotated by the tablet) and
  side B `--scan-copies followers`.
- **The legs**: `BIG_ONLY=1`, then `KEYWORD_ONLY=1`, with `AB_SIDES=1` and `AB_ORDER="A B B A"`. That is two rounds a
  side on each leg.
- **The unit** was `x38-control`, and every host was identical to its snapshot after both legs.
- **The results** are in [`results/x38/control-big`](../../../shoal-spike-scan/results/x38/control-big/) and
  [`control-keyword`](../../../shoal-spike-scan/results/x38/control-keyword/).

### Big

| Figure | A: rotated | B: followers | Disjoint | B against A |
| --- | --- | --- | --- | --- |
| Whole scan beside the cell, s | 145 [142.3, 147.7] | 28.6 [28.4, 28.7] | **yes** | 5.1 times as fast |
| Big alone, partitions a second | 49,200 [48,300, 50,200] | 130,900 [130,600, 131,200] | **yes** | 2.66 times |
| europa's share of the pages, beside the cell | 0.22 [0.200, 0.233] | 0.84 [0.841, 0.848] | **yes** | |
| europa's share, Big alone | 0.22 [0.207, 0.233] | 0.69 [0.692, 0.693] | **yes** | |
| Pages a leading copy answered, beside the cell | 0 | 0.33 [0.288, 0.380] | **yes** | |
| Tablets taken from another's lane, beside the cell | 0 | 2,100 [2,082, 2,118] | **yes** | |
| The cell's read p99 beside the scan, ms | 3.22 [3.19, 3.24] | 3.23 [3.14, 3.33] | no | |
| The cell's insert p99 beside the scan, ms | 32.8 [32.8, 32.8] | 32.5 [31.8, 33.3] | no | |
| The cell's inserts a second beside the scan | 1,706 [1,704, 1,708] | 1,707 [1,685, 1,728] | no | |
| The paced stream's insert p99 while the scan ran, ms | 28.9 [28.7, 29.0] | 29.0 [28.6, 29.3] | no | |
| Zen1 partitions a device read, Big alone | 37.2 [37.1, 37.3] | 37.3 [37.2, 37.3] | no | |
| Node cpu a 1,000 partitions, Big alone, ms | 104.8 [103.2, 106.4] | 43.3 [43.1, 43.4] | **yes** | |

Side B repeats the rounds' side A within its ranges: 28.6 s beside the cell, 130,900 partitions a second alone, and
europa answering 0.84 of the pages.

Some disjoint figures come from arms that no scan drives. The cell alone did 1,701 inserts a second against 1,660,
at an insert p99 of 33.2 ms against 34.7, and the restart before each capture took 12.6 s against 10.6. With two
rounds a side that is their spread: the rounds' eight `followers` rounds of the cell alone ran from 1,658 to 1,711
inserts a second and from 33.1 to 34.9 ms, which holds both sides.

The nodes' cpu is not a read's cost. They spent about as much cpu a second on both sides, 5.2 seconds against
5.7. So a partition costs 2.4 times as much under the rotated driver because the same second's cpu is spread over
fewer partitions.

### Keyword

| Figure | A: rotated | B: followers | Disjoint | B against A |
| --- | --- | --- | --- | --- |
| Whole scan beside the cell, s | 91.6 [79.8, 103.4] | 5.15 [4.7, 5.6] | **yes** | 17.8 times as fast |
| europa's share of the pages | 0.26 [0.243, 0.286] | 0.91 [0.906, 0.909] | **yes** | |
| Pages a leading copy answered | 0.005 [0, 0.011] | 0.48 [0.460, 0.496] | **yes** | |
| The cell's read p99 beside the scan, ms | 4.30 [4.23, 4.38] | 4.75 [4.02, 5.48] | no | |
| The cell's insert p99 beside the scan, ms | 34.7 [34.4, 35.0] | 35.7 [34.3, 37.1] | no | |
| The cell's inserts a second beside the scan | 1,662 [1,658, 1,667] | 1,625 [1,595, 1,655] | **yes** | 0.98 |
| The paced stream's insert p99 while the scan ran, ms | 30.4 [29.9, 30.9] | 30.6 [29.7, 31.5] | no | |
| Encode wall a resident partition, µs | 15.8 [15.7, 15.8] | 12.8 [12.2, 13.4] | **yes** | |

**The cell's inserts beside the scan** were 2.2% fewer under `followers`. The ranges are disjoint, at two rounds a
side, but the windows differ: five seconds against ninety. Both sides sat under the rounds' own `followers` range,
1,669 to 1,723, so this leg ran lower as a whole. The cell's read and insert p99 and the paced stream overlap. One
`followers` round's five seconds held the cell at a read p99 of 5.48 ms and an insert p99 of 37.1. This is not a
figure the rule reads, since the rule compares option 1 with option 3, but M25's exit reads the cell beside a
written group's scan again.

**A resident partition encodes faster under `followers`** because europa encodes 0.91 of the pages, on faster
cores.

### Against the predictions

6. **The rotated driver finishes at the slowest device's pace: held on its shares, failed on its paces.** These
   parts held:
   - europa answered 0.22 of the pages beside the cell (one round 0.200) and 0.22 alone, inside the predicted 0.2
     to 0.3;
   - the sides are disjoint;
   - side B repeats the rounds' side A.

   Side A was slower than predicted. Beside the cell it took 145 s, against a predicted 65 to 100. Alone, `Big`
   read 49,200 partitions a second, against 50,000 to 75,000. Each 970 EVO read its three eighths of `Big` at
   about 19 MiB a second alone and about 8 beside the cell, not the 20 to 26 predicted. The prediction took O147's
   rotated driver as this commit's. At this commit the same driver is 1.75 times slower beside the cell and 1.34
   times slower alone. [Why](#why-the-rotated-driver-is-slower-at-this-commit) is below.
7. **The cell sees both alike: held.** The cell's read and insert p99 beside `Big` overlap between the sides, and
   so do its inserts.
8. **Keyword says whether O152's 141 s was the driver: held, and most of it was.** Side A took 91.6 s beside the
   cell, 17.8 times side B's 5.15 s, with europa answering 0.26 of the pages. Between O152's 141 s and the rounds'
   5 s, the build accounts for 1.54 times (141 s to 91.6) and the driver for 17.8 times (91.6 to 5.15). On a log
   scale the driver is 87% of the 27.5 times between them.

### What share is the dealing and taking

At one commit, dealing and taking against the rotated driver reads `Big` 2.66 times as fast alone and 5.1 times as
fast beside the cell, and `Keyword` 17.8 times as fast beside the cell. The cell and the paced stream saw both
drivers alike.

The guides understated the gain. O147's rotated driver, four days earlier, was faster than this commit's, so the
rounds' 2.9 times against O147 beside the cell is 5.1 times at one commit. Beside the cell the gain is largest,
because a fixed share a member is read at its slowest device's pace, and beside the cell that device is also the
one the cell's flushes slow ([W42](scan-speed.md#the-gap-factor-by-factor)'s factor A). Dealing and taking moves
that work to the device the flushes leave alone.

### Why the rotated driver is slower at this commit

The same rotated driver on the same cluster read `Big` alone at 65,900 partitions a second at O147 (2026-11-10) and
49,200 at this commit. In the arm's window, titan and hyperion read 6.1 million partitions together, against 7.8 to
8.8 million in O147's side A rounds: about a quarter fewer. Their device-read sizes did not change: 37.2 partitions
a device read against 37.5. Two things changed, and their product is about the loss:

- **A sixth of a 970 EVO's scan bytes are now its map's.** Since [item 392](../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md)
  a scan's map reads are read ahead sixteen pages (64 KiB) a read, and are paced from the device's share. The map's
  bytes were 15.7% of the data bytes on titan and hyperion in every round, under both drivers. O147's build read
  the map unpaced and counted none of it.
- **The device's learned cost a byte is about a quarter higher.** On titan and hyperion it was 3,045 to 3,662 ps
  at the end of each round's `Big` alone, against 2,414 to 3,227 at O147. Why it rose was not measured.

The device's time share is fixed, so these two leave the data about 0.69 of the bytes a second it had,
`1 ÷ (1.157 × 1.25)`.

The read-ahead mostly goes unused under the bench's driver:

- In round 1, hyperion made 7,975 map reads of about 65 KB each, some 127,600 pages, for 4,339 walk pages, and
  took 1,765 pages from the scans' cache. That is about 29 pages read for each walk page of one tablet, where a
  tablet holds about 1.7 a run.
- The read-ahead past a tablet serves the walk of its executor's next tablet in key order. The bench's walks ask one
  tablet at a time in the driver's seeded order, so the next tablet a member walks is seldom the one read ahead.

`followers` pays the same 15.7% on the Zen1 hosts, but reads most of the table from the Optane. There the map is
13.4% of the bytes and a byte costs about a sixth as much.

This is filed as [O181](../../appendix/optimizations.md#o181-a-walk-of-one-tablet-in-no-key-order-reads-its-maps-sixteen-pages-ahead-for-its-next-walk-to-miss).
It does not change the verdict: X38's two options ran at this commit, so they paid the same.

## What it says for the design

- **M25's driver prefers followers, then leaders** ([W6](distribution.md#choosing-copies)): every tablet dealt once
  to the follower dealt the fewest so far, a lane a member, and a member out of tablets taking one waiting in
  another's lane that it follows, then one it leads only once none it follows waits anywhere.
- **The dealing and taking are not optional for either option.** A scan that reads a fixed copy a tablet finishes
  at its slowest device's pace whichever copy it prefers: at one commit it read `Big` 5.1 times slower beside the
  cell and `Keyword` 17.8 times slower ([the control](#the-control-the-rotated-driver-at-the-same-commit)).
- **The bench keeps the rotated driver as its default until M25** moves it onto client pages, whose driver deals and
  takes. A spec that names no copies keeps its digest, so every capture before X38 stays comparable. Until then a
  bench scan of a cluster with unlike devices reads at its slowest device's pace unless it names `followers`.
- **Option 1 costs nothing measured, and leaders answer a quarter fewer of its pages** (0.36 of them against 0.48
  beside the cell). On the lab that changed nothing a leader's writes showed, since europa's Optane carries both.
  Where a leader's device is the slow one, that share is what would show.

## Limitations

- **One shape of cluster.** Three nodes, a leader-heavy fast device (europa at `lead_weight: 2` on the Optane) and
  two slow ones. Equal lead weights, a slow device that leads, disks, and more than three nodes were not measured.
  Where every device reads alike, dealing evenly is most of what taking buys.
- **The admin walk stood in for a client's pages.** The bench reads a cluster through every member's admin walk,
  since a cluster node answers no client scan before M25. M25's driver reads by `ScanPage`, a page over a shard's
  tablets, and its exit reads this again.
- **The level was `One`.** No barrier was taken: a `Quorum` scan's barrier goes to a group's leader whichever copy
  is read, and is M25's to measure ([W12](performance.md)).
- **Leads after a restart** are read as followers until they return, above.
- **The control ran two rounds a side.** Its scan figures are far apart, but a figure that differs by a few percent is
  read against the rounds' eight, as its cell alone's and its Keyword cell's inserts are above.
- **Why a 970 EVO's learned cost a byte rose** between O147 and this commit was not measured. O181 names the map's
  share, which the figures show, and leaves the cost open.

## Found on the way

- **[O181](../../appendix/optimizations.md#o181-a-walk-of-one-tablet-in-no-key-order-reads-its-maps-sixteen-pages-ahead-for-its-next-walk-to-miss)**
  (optional): a walk of one tablet at a time, asked in no key order, reads its map sixteen pages ahead, and its next
  walk seldom uses them. Since item 392 the 970 EVOs pay for those pages from their scan share. The control found it
  ([above](#why-the-rotated-driver-is-slower-at-this-commit)).
- **Not a defect**: the QUICK pass's leads before they settled are the caveat above. O63's handback is paced so a
  restart does not move every lead at once.

## Related

[W6](distribution.md#choosing-copies) for how a scan chooses copies; [W13](contract.md#q38-which-copies-a-scan-reads-and-its-levels-2026-11-14)
for Q38; [F115](../../features/bench-scans.md) for the driver; [M23's exit](m23-exit.md) for the harness and the
cluster; [W42](scan-speed.md#the-gap-factor-by-factor) for what member shares cost a scan.
