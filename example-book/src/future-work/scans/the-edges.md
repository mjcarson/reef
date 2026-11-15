# X19. The edges

**Reported 2026-11-07.** This is the record of spike [X19](spikes.md#x19-the-edges): what a scan costs at
its edges, and so what M24 and M26 must bound. The edges are:

- a table of 4096 empty tablets;
- one sorted partition past a page;
- a listing of a bucket of inline objects;
- a filter that keeps nothing.

It ran in two parts:

- the empty tablets, the sorted partition and the filter on a standalone node in process on europa, four
  rounds each;
- the listing on a side cluster of the lab's three hosts, four rounds, as the spike page wrote.

Both ran on ScanSupport with the four defects X19 found fixed, the last of them, item 374, found by the
listing's first run and run again on its fix.

Every host was identical to its snapshot after. The answers:

| Edge | What it costs | So |
| --- | --- | --- |
| **4096 empty tablets** | About 12.5 ms of four shards' pages, 23 µs a page, for a table never written or an unsorted table whose deletes were compacted. 1.9 s for a sorted table emptied by deletes, whose 8,192 dead records the counters call held until a merge rewrites them | W12's empty-table line holds where the counters say a tablet is empty, and M24's page skips those tablets on the loop |
| **One sorted partition past a page** | Served whole on a node of four shards up to 15 MiB archived or overlaid, its page drawing twice its bytes, and refused by name past that; an archived one is refused before it is read. A resident one is refused from 8 MiB. A whole record's checksum and fold hold the shard's thread 1.25 ms at 15 MiB and 7.12 ms at 64 MiB, with no yield ([item 373](../../appendix/resolved/whole-record-holds.md)) | M24 hands such a partition out across pages, from records of bounded size the merges write. It refuses no row for its size: a walked record is handed out uncopied ([O162](../../appendix/optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it)), under a scan budget of at least the largest row |
| **A listing of inline objects** | Every listed object's whole row: 18,139 device bytes an entry at the default 16 KiB, 58 times its fields. Titan listed 901 entries a second at the product's pace | A listing's fields are kept apart from inline bytes, required before M26 |
| **A filter that keeps nothing** | A page draws twice its work bound ~~until O162 ends the copy~~ until [F134](../../features/walked-records-uncopied.md) ended it (2026-11-07): 8.1 MiB at 4 MiB. A larger bound buys no speed: a gibibyte filtered to nothing took 15.2 s paced at 4 MiB and 15.9 s at 16 MiB | A page's work bound is 4 MiB by default and at most |

## The question

The spike page's question: what does a scan cost at its edges, and so what must M24 and M26 bound? The edges are:

- a table of 4096 empty tablets;
- a table of one sorted partition larger than a page;
- a listing of a bucket of inline objects, bytes read a listed entry against an inline threshold from 1 KiB to
  16 MiB;
- a filter that keeps nothing, its pages ended by work.

Since Q36 was settled without it (2026-11-06), X19 bears on one thing that record left: whether M24 fills the
cursor's reserved sort-key position, splitting a sorted partition across pages and restating
[P20](contract.md#the-contract) for a partition handed out in pieces.

**What would have changed the design**, named on the spike page before the run:

- If a listing of inline objects reads too much against the budget, keeping a listing's fields apart from an
  object's bytes moves from optional to required before M26.
- If one huge sorted partition's page breaks the budget, splitting a sorted partition across pages moves before
  M24. Otherwise M24 keeps refusing, by name, one whose answer passes what a page may hold, which ends its scan.
- If an empty table costs more than a page a shard, the per-tablet counters are not what W4 assumes.

The spike page named no line for "too much" or "breaks", so
[the predictions](../../../shoal-spike-scan/results/x19/predictions.md) fixed them before any run:

- **The split** moves if a page of a sorted partition the budget admits breaks
  [W7](isolation.md#the-budget)'s budget on its node.
- **The listing's fields apart** moves if, at the default 16 KiB, a listing reads more than ten times its
  entries' field bytes, or lists fewer than 2,000 entries a second on one member at the product's pace.
- **W12's empty-table line** moves if a scan of a table with no live row costs more than a page a hosting
  shard, in a history a table is commonly in.

## How it was judged

**E1, E2 and E4 on europa.** These ran on a standalone node in the driver's own process: the spike's `EdgeLab`
schema, four shards, every root on a scratch directory of the Optane (and of the disk for E4's second half).

- Each leg writes its tables, then starts its node twice more, the first evicting everything, so nothing it
  wrote is resident and every intent log is compacted into an archive.
- It then asks scan pages of each tablet straight from the shard that hosts it, two in flight a shard, as
  M24's streams would.
- A page's own figures come from its `WalkPage` and a node's from every shard's walk figures before and after.
- What no figure counts, the longest synchronous whole-record step a scan took of its shard's thread, comes
  from the spike's clock around a record's verification and a partition's fold.
- Device bytes come from `/proc/diskstats` of the whole device.

**The foreground beside E2.** The reference cell's mixture ran in process: half gets and half inserts of 100,000
kibibyte rows, four in flight from a client pinned to cpus 12 to 15. It was timed alone and beside back-to-back
pages of one partition, for 20 s each. The keys of the scanned partition's shard are counted apart.

**E3 on a side cluster.** The listing ran on a side cluster of the lab's three hosts, `x19`, X18's hosts and
cores at 6 GiB a node, as the spike page wrote it. The schema is the spike's `ListLab`: eight buckets, each
bound to an inline-only pool of its own threshold.

- Each bucket was filled once with objects of exactly its threshold, at least 256 MiB and 32 objects a bucket,
  then sealed with 512 MiB of filler so every group's log rolled past the fill, and the cluster restarted.
- A listing is M26's. Until then it was read as M26 would read it: a scan's page of every tablet holding the
  bucket's paths, eight streams, through one member's admin walk, its host's NVMe counted around it.
- It ran on titan's 970 EVO and on europa's Optane, at the product's pace and with the pace lifted.

**The lab's rule for a difference**: a difference is a result only where two arms' rounds give ranges that do
not overlap. Four rounds, the arms' order alternating by round.

## What was run

| Host | Cpu | Governor | Node | Roots |
| --- | --- | --- | --- | --- |
| europa, E1, E2, E4 | Ryzen 9 7945HX | `performance` for the run | four shards in the driver's process, 2 GiB a shard's rows, the client on cpus 12 to 15 | the Optane, XFS (`/optane/x19`); E4's second half the disk, XFS (`/hdd/x19`) |
| europa, titan, hyperion, E3 | Ryzen 9 7945HX; Zen1, 4 cores | `performance` for the run | `x19`, X18's hosts and cores at 6 GiB a node, factor 3 | europa the Optane, titan and hyperion `/xfs` on the 970 EVO |

The tmdb node, every host's maintenance timers and its governor were held for each run and put back on every way
out, each change recorded in `shoal-spike-scan/results/x19-host-changes.txt`, and `verify` found every host
identical to its snapshot after.

**The build** is branch `x19-spike` at `7c87ffd7`, spike code never merged, on ScanSupport at `fc156277`
with items 370 to 372 and 374 fixed; every table on this page is from it, its driver and its node built there.
Two earlier sets are kept as evidence, each build on a branch of its own:
- **before item 374's fix**, `results/x19/before374/`: E1 and E4 at `105cce6f`, E2 at `882eee56` and the
  listing's driver at `20eb615c` with its node at `882eee56` (branches `x19-spike-before372` and
  `x19-spike-before374`);
- **before item 372's fix**, `results/x19/before372/`: E2's four first rounds, at `105cce6f`, item 372's
  evidence.

A check round is kept too, `results/x19/check374/`: one round of E2 at a 256 MiB budget on the fix's build, run
straight after the main set ([below](#e2-one-sorted-partition-past-a-page)).

Its switches, process-wide atomics each scan page reads as it begins (`shoal-core/src/server/spike.rs`):
- `EMPTY=skip`: a page of a tablet whose archive counters and resident map hold nothing reads nothing;
- `DROP=all`: a filter that keeps nothing, every live partition read and handed out none of;
- `WORK`: a bound on the bytes a page reads past, apart from its answer bytes.

Beside them, a thread-cpu clock around a record's verification and a partition's fold.

**The records** are in `shoal-spike-scan/results/x19/`, one JSON line a measurement; `x19-report.py` makes the
tables below from them, and `x19-report.md` is its output. The earlier sets and the check round are slimmed by its `slim`, each
record's node figures cut to the scan steps' counters and the budget's peak, which leaves their tables
unchanged. `x19-lab.sh` is the run, and the
[predictions](../../../shoal-spike-scan/results/x19/predictions.md) were written before it.

## What was found on the way

Five defects, four of them reproduced by a test and fixed, and two optimizations:

- **[Item 370](../../appendix/resolved/scan-partition-past-its-budget.md)**: a partition past half the scan
  budget was shed on every ask, never refused by name. It was read from the code while X19 was planned, and
  fixed before the runs.
- **[Item 371](../../appendix/resolved/scan-page-over-dead-partitions.md)**: a page over partitions with no live
  row drew for each until it was shed, and never moved its cursor. Also read from the code and fixed before
  the runs. E1's dead chains are the history it would have hung on.
- **[Item 372](../../appendix/resolved/encode-cost-relearned.md)**, found by E2's first rounds: one slow first
  encode step could hold a table's steps at a row or two for its shard's life. A resident partition of 17 MiB
  of small rows took 247,580 steps and drew nine times its bytes. Fixed, and E2 run again on the fix.
- **[Item 373](../../appendix/resolved/whole-record-holds.md)**,
  found by E2: a scan's page holds its shard's thread for a whole archived partition's checksum and fold, past
  W7's 100 µs from about 2 MiB, and no step counter sees it. Filed, and cured before M24 by large sorted
  partitions written as records of bounded size ([below](#what-it-says-for-the-design)).
- **[Item 374](../../appendix/resolved/scan-draws-held-and-waited.md)**, found by E3's first run: pages each
  read a large partition and waited for room for its copy, and were shed with their reads thrown away. On
  titan the bucket of 16 MiB objects listed one of 32 in 120 s while its NVMe read 7.46 GB. Fixed, a wave
  drawing its partitions' copies with their reads, and the whole listing leg run again on the fix.
- **[O162](../../appendix/optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it)**:
  a walked archived partition is copied while its run's buffer holds it, so a page holds it twice. Found by
  item 370's fix, and required before M24 since the decisions below. Built as
  [F134](../../features/walked-records-uncopied.md) (2026-11-07).
- **[O163](../../appendix/optimizations.md#o163-a-partition-the-scan-budget-cannot-hold-is-read-or-encoded-before-it-is-refused)**:
  a partition the budget cannot hold is read, or encoded up to the budget, before it is refused. Found by E2,
  where a resident 15 MiB partition refused at every ask still raised the cell's insert p99 2.8 times.

Two of the spike's own mistakes cost a cluster run each, and are recorded so nobody repeats them:

- **A listing must key its tablets by the object row's own partition key**, the row derive's hash of
  `path_key`, never `path_key` itself. The first listing walked the wrong tablets for every bucket sparser
  than the 4096 tablets, and listed 20 of 256 objects at 1 MiB.
- **A shed listing must be bounded by time, not by a count of tries**: a shed waits a quarter second for room.

## The tables

Medians over four rounds, the range beside each in brackets where the rounds differ; the whole tables are
`shoal-spike-scan/results/x19-report.md`.

### E1, 4096 empty tablets

A whole table, every tablet a scan's page, two in flight a shard, four shards in process on the Optane:

| History | Tablets the counters call held | Archive reads | Device read | Whole scan, product walk | With the counters skip |
| --- | --- | --- | --- | --- | --- |
| Never written | 0 | 0 | 0 | 12.6 ms [11.7, 15.3] | 12.2 ms [11.6, 14.5] |
| A hundred sorted rows | 98 | 98 | 14.1 MiB | 25.6 ms [24.7, 28.3] | 13.6 ms [12.0, 31.1], 0.46 MiB read |
| 200,000 unsorted rows deleted, before a restart | 4,096 | 0 | 21.4 MiB | 41.1 ms [39.7, 41.9] | 41.3 ms: nothing to skip |
| The same, after a restart | 0 | 0 | 0 | 12.5 ms [11.8, 15.7] | 12.9 ms [11.6, 14.3] |
| 4,096 sorted partitions deleted, before a restart | 2,579 | 0 | 16.1 MiB | 30.2 ms [28.6, 30.5] | 23.6 ms [20.9, 24.1], 1,517 skipped |
| The same, after a restart: dead chains | 2,579 | 8,192 | 55.9 MiB | 1,915 ms [1,912, 1,966] | 1,983 ms [1,909, 2,054], 1,517 skipped |

A page of an empty tablet costs 22 to 23 µs at its median, its round trip from the shard hosting it: 4096 of
them are about 12.5 ms of four shards. The device bytes of the histories with nothing to read are index pages:
an empty tablet of a table whose map holds runs reads at least one of each run's 4 KiB pages its range lies in,
about 3.6 KiB an empty tablet. The skip spares the tablets the counters call empty and no others, so it saves
nothing on dead chains, whose 2,579 tablets are called held. One round of the hundred rows' skip took 31.1 ms,
a page of it 2.2 ms; its other three took 12.0 to 15.2 ms.

### E2, one sorted partition past a page

Four shards, so a scan budget of 32 MiB; a partition alone in its tablet, a scan's page of it asked five times a
round:

| Partition, kibibyte rows | Archived | Overlaid | Resident |
| --- | --- | --- | --- |
| 0.5 MiB | served, 1.9 ms | served, 2.3 ms | served, 0.21 ms |
| 2 MiB | served, 9.8 ms | served, 18.5 ms | served, 0.70 ms |
| 8 MiB | served, 78 ms, 16.1 MiB drawn | served, 83 ms | `ResponseTooLarge` after 117 ms |
| 15 MiB | served, 149 ms, 30.1 MiB drawn | served, 149 ms | `ResponseTooLarge` after 226 ms |
| 17 MiB | `ResponseTooLarge` at once | `ResponseTooLarge` after 259 ms | `ResponseTooLarge` at once |
| 32 and 64 MiB | `ResponseTooLarge` at once | `ResponseTooLarge` at once | `ResponseTooLarge` at once |

Partitions of 32-byte rows meet the same boundaries. On item 374's fix an archived partition whose read and
copy pass the budget is refused before it is read, since its wave draws both at once: before it, a 17 MiB one
was refused after 178 ms of reading. Under a budget of 256 MiB an archived 64 MiB partition was served in 756 ms
[645, 863], 128 MiB drawn, and a resident 32 MiB one in 411 ms [342, 482], 129 MiB drawn. A resident partition
written after the node's start is an overlay over its own archived rows once its log rotates. Its page holds its
encode steps' parts, its archive and the pieces folded from both: about four times its bytes.

The longest synchronous whole-record call a page made on its shard's thread, its own cpu, with no yield
inside ([item 373](../../appendix/resolved/whole-record-holds.md)):

| Partition | Archived, kibibyte rows | Archived, 32-byte rows | Overlaid | Resident |
| --- | --- | --- | --- | --- |
| 2 MiB | 0.14 ms | 0.32 ms | 0.04 ms | 0 |
| 8 MiB | 0.59 ms | 1.26 ms | 0.24 ms | refused |
| 15 MiB | 1.25 ms | 2.47 ms | 0.40 ms | refused |
| 32 MiB, a 256 MiB budget | 3.56 ms | | 1.18 ms (0.69) | 1.06 ms (0.61) |
| 64 MiB, a 256 MiB budget | 7.12 ms | | 1.86 ms (1.57) | |

No step past 100 µs was counted in any case: the shard's step counters do not see these calls.

**The overlaid and resident pages' holds were the host's that hour, not the fix's.** In the main set they ran
up to 1.8 times those of the set before item 374's fix, the overlaid pages' ranges disjoint at 15 to 64 MiB under
the larger budget. The check round, the same build run straight after the main set, gave figures within the
earlier set's ranges or at their edge, in parentheses above: 0.69 ms at an overlaid 32 MiB, against
0.69 [0.68, 0.70] before the fix and 1.18 [1.08, 1.20] in the main set. Every archived page's hold, item 373's
figures, has ranges in the two sets that overlap, and the check round's are within 0.1 ms of both. The spike's
clock keeps a page's longest call, not which call it was, so the records cannot say what the host was doing;
that hour followed the listing's cluster on europa.

**The cell beside back-to-back pages of one partition** on its shard, against the cell alone, its own shard's
keys apart, 20 s a side:

| Scanned | Pages in 20 s | Shard's get p99 | Shard's insert p99 | Every get's p99 | Every insert's p99 |
| --- | --- | --- | --- | --- | --- |
| An archived 8 MiB partition | 251 | 1.20× [1.10, 1.22] | 1.80× [1.69, 1.88] | 1.11× | 1.49× |
| An archived 15 MiB partition | 102 | 1.16× [1.07, 1.29] | 1.42× [1.06, 1.70] | 1.03× | 1.26× |
| An overlaid 15 MiB partition | 134 | 1.22× [1.18, 1.29] | 2.13× [1.58, 2.36] | 1.21× | 1.40× |
| A resident 15 MiB partition, refused | 0 | 1.18× [1.13, 1.22] | 2.81× [2.67, 3.21] | 1.12× | 1.52× |
| **The control**: the same 15 MiB in kibibyte partitions, in the same tablet | 1,183 | **3.07× [2.96, 3.18]** | **4.14× [3.97, 4.17]** | 2.13× | 2.63× |

The cell's get p99 alone was about 0.21 ms and its insert p99 0.64 ms, so W7's line, `max(1.10×, +0.1 ms)`, is
0.31 and 0.74 ms. Each of the shard's ratios has a range that overlaps its range before item 374's fix.

### E4, a filter that keeps nothing

Sixteen hot tablets of 64 MiB of kibibyte rows, 1 GiB, every partition read and none handed out under `DROP`,
against the product's walk handing every one out:

| Device and pace | Walk | Pages | A page, p50 and p99 | Most a page drew | Whole walk |
| --- | --- | --- | --- | --- | --- |
| Optane, paced | the product's, 1 MiB answers | 1,072 | 31 / 34 ms | 2.0 MiB | 16.4 s |
| | nothing kept, 1 MiB of work | 1,072 | 30 / 34 ms | 2.0 MiB | 16.3 s |
| | nothing kept, 4 MiB | 272 | 114 / 122 ms | 8.1 MiB | 15.2 s |
| | nothing kept, 16 MiB | 80 | 456 / 590 ms | 18.7 MiB | 15.9 s |
| Optane, unpaced | nothing kept, 4 MiB | 272 | 19 / 20 ms | 8.1 MiB | 2.5 s |
| | nothing kept, 16 MiB | 80 | 74 / 299 ms | 18.7 MiB | 2.7 s |
| Resident, the same rows | nothing kept, 4 MiB | 272 | 22 / 27 ms | 4.0 MiB | 3.0 s |
| Disk, paced, 4 × 16 MiB | the product's | 68 | 167 / 332 ms | 2.0 MiB | 6.1 s |
| | nothing kept, 4 MiB | 20 | 771 / 959 ms | 8.1 MiB | 6.6 s |
| | nothing kept, 16 MiB | 12.5 | 1,138 / 1,726 ms | 14.7 MiB | 6.3 s |

The longest synchronous call was 0.8 ms on the Optane, in the reads of a run's records.

**On item 374's fix a page that keeps nothing draws twice its work bound.** A wave draws each partition's copy
with its read, and a page that drops everything copies nothing. Before the fix such a page drew about its bound:
1.0, 4.1 and 16.4 MiB. At 4 MiB that costs no time: its pages' times and its walk's are within their ranges of
before. At 16 MiB the eight pages in flight outgrew the node's 32 MiB budget and waited for room. Unpaced, a
page's p99 rose from 80 [77, 82] to 299 [298, 299] ms, and the walk from 2.49 to 2.73 s. A resident page reads
nothing, so it draws no copies.
[O162](../../appendix/optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it),
required before M24, hands a record out from its read with no copy, and so draws its read alone again. It is
built as [F134](../../features/walked-records-uncopied.md) (2026-11-07), whose page has E2 and E4 run again on it
(2026-11-08). A page that keeps nothing drew 1.03, 4.10 and 16.4 MiB, and no page waited for room, so the unpaced
16 MiB pages' p99 was 78 ms. An archived 15 MiB partition drew 15.1 MiB.

### E3, a listing of inline objects

The side cluster on item 374's fix, each bucket filled with objects of exactly its threshold, every tablet
holding its paths walked as scan pages through one member's admin walk, eight streams. Device bytes are the
member's whole NVMe; the rates are entries a second. Every listed entry was archived, none resident.

| Bucket | Objects | Walked bytes an entry | Device bytes an entry | Device bytes a field byte | titan, paced | titan, unpaced | europa, paced | europa, unpaced |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| L1k | 262,144 | 1,344 | 1,514 | 5.1 | 9,823 [9,772, 9,865] | 175,875 | 35,333 | 352,735 |
| L4k | 65,536 | 4,416 | 4,900 | 16.6 | 3,098 [3,077, 3,152] | 53,729 | 14,052 | 125,538 |
| **L16k, the default** | 16,384 | 16,720 | 18,139 | **58.1** | **901 [876, 907]** | 15,205 | 5,059 | 47,348 |
| L64k | 4,096 | 65,872 | 68,988 | 221 | 333 [331, 340] | 6,786 | 1,593 | 16,708 |
| L256k | 1,024 | 262,480 | 266,614 | 855 | 163 [162, 165] | 2,704 | 404 | 6,589 |
| L1m | 256 | 1,048,896 | 1,053,634 | 3,560 | 50.2 [49.1, 50.7] | 824 | 100 | 2,367 |
| L4m | 64 | 4,194,624 | 4,200,448 | 14,191 | 14.2 [13.9, 14.4] | 199 | 25.0 | 619 |
| L16m | 32 | 16,777,552 | 16,799,184 | 53,844 | 3.73 [3.66, 3.79] | 42.8 | 6.30 | 120 |

An entry's fields archive as 296 to 312 bytes, its path's length deciding which (`x19 fields`). Europa's device
bytes an entry are within 1% of titan's.

- **A listing reads every listed object's whole row**: its inline bytes, its fields, and the index pages and run
  gaps its reads span, 8% more at the default threshold.
- **At the product's pace a page costs its device about 0.3 ms and 1.2 ms a megabyte, so a listing's rate is
  its entries a page times what the pace allows, until its pages are large.** The pace gives a device's scans
  75 ms of its time a second ([F128](../../features/device-time-pace.md)). Titan's paced listings took 0.34 to
  0.52 ms of it a page for pages of 74 to 304 KB, 150 to 220 pages a second, then 1.5 ms for 1 MB, 5.3 for
  4.2 MB and 20.1 for 16.8 MB, about 60 MiB a second; that fit is within 27% at every size. **One page here is
  one tablet**, so the buckets' rates follow how many objects a tablet holds as much as their size: 64 a tablet
  at 1 KiB, sixteen at 4 KiB, four at 16 KiB. Unpaced, titan listed 11 to 18 times as fast.
- **Kept apart, a listing's fields would make a dense bucket's pages small, and a sparse bucket's no cheaper.**
  By that fit, a page of 64 entries of 16 KiB read whole is 1.16 MB and about 1.7 ms, some 2,800 entries a
  second at the pace on this drive; the same page of fields alone, about 22 KB, is about 0.33 ms, some 14,000.
  A bucket of four objects a tablet lists at about 1,000 a second either way, its page costing its read, not
  its bytes: what M24's pages over several of a shard's tablets change. These are estimates from the fit, not
  measurements.
- **An empty tablet over the wire costs about a tenth of a millisecond of a listing's wall.** The 16 MiB
  bucket walked over every tablet, 4,096 pages for its 32 objects, listed 28.8 a second unpaced against 42.8
  over the 32 tablets holding them, 0.4 s more for 4,064 pages through eight streams. Paced, 3.12 [2.56, 3.68]
  against 3.73.
- **Before item 374's fix the 16 MiB bucket listed one object in two minutes** on titan, its pages shed 73,210
  times while the NVMe read 7.46 GB. On the fix it lists 32 in 8.6 s, each read once, its own size
  ([the run before and after](../../appendix/resolved/scan-draws-held-and-waited.md#the-fix)). Every other
  bucket listed at the same rate either side of the fix, its rounds' ranges overlapping.


## What it says for the design

Every decision below was taken with the user on 2026-11-07, from the tables above. The user's rule for all of
them: **performance first, and no partition may fail or go missing from a scan.** That rules out refusing a
partition by size, which ends its scan by name, as a design: what M24 builds has to hand out every partition a
table holds.

| Decision | Evidence |
| --- | --- |
| **M24 hands out a sorted partition too large for one page across pages, from records of bounded size.** Merges write a large sorted partition as several records of at most a bounded size, as fragments already are ([F61](../../features/fragmented-partitions.md)). A page reads, verifies and hands out the partition a record at a time, and ends where the budget or its bounds stop it. Its cursor's reserved sort-key position, Q36's, says how far into the partition it got, so the next page goes on from that row. [P20](contract.md#the-contract) is restated for a partition handed out in pieces: each of its live rows once. This also cures [item 373](../../appendix/resolved/whole-record-holds.md), since no single verification or fold holds a shard's thread for more than one bounded record | A whole record's verification and fold held a shard's thread about 0.1 ms a mebibyte of kibibyte rows and 0.16 of 32-byte ones, 1.25 ms at 15 MiB and 7.12 ms at 64, with no yield inside. A record bounded near a mebibyte keeps that near W7's 100 µs step. A split across pages over whole records would re-read and re-verify the record at every page: a 64 MiB partition in pages of 1 MiB would read about 4 GiB. Beside the cell, a huge partition's pages were gentler than the same bytes in kibibyte partitions (the shard's get p99 1.16 times against 3.07), so handing one out costs a scan nothing per byte that a table of small partitions does not |
| **A row that cannot be split is never refused for size.** A walked archived record is handed out from the buffer its read put it in, never copied ([O162](../../appendix/optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it), required before M24 by this decision, built as [F134](../../features/walked-records-uncopied.md) on 2026-11-07). Every node keeps a scan budget of at least the largest row it admits, so any row fits alone, and a page waits its turn for room (built as [F135](../../features/scan-budget-floor.md) on 2026-11-08: the floor is the node's request body bound and a row's slack, and the scan budget serves its waiting draws in turn) | A page held an archived partition twice, its read and its copy: 30.1 MiB drawn for 15 MiB. A 16 MiB inline object needed about 32 MiB, refused outright on a node of one shard. Without the copy, a node of four shards scans two such rows at once rather than one, and a floor of one largest row costs memory only on a node of one or two shards |
| **A listing's fields are kept apart from an object's inline bytes, required before M26** | A listing read every listed object's whole row: at the default threshold of 16 KiB, 18,139 device bytes an entry, 58 times the entry's 312 bytes of fields, and titan listed 901 entries a second at the product's pace. Both lines the predictions fixed before the run were crossed: more than ten times the fields' bytes, and fewer than 2,000 entries a second. The bytes line is the row's size alone; the rate line is as much the bucket's four objects a tablet, at a page a tablet. Kept apart, by the pages' fit, a dense bucket of 16 KiB objects would list about five times as fast at the pace |
| **A page's work bound is 4 MiB by default and at most 4 MiB**, its rows 1,000 and its answer bytes 1 MiB, as [W4](cursor-and-pages.md#bounds-and-short-pages) has them | A larger bound buys no speed: a gibibyte filtered to nothing took 15.2 s paced at 4 MiB and 15.9 s at 16 MiB, 2.5 and 2.7 s unpaced, and on a disk a 16 MiB page took up to 1.8 s. A page that keeps nothing draws its work bound from the budget, and on item 374's fix twice that ~~until O162 ends the copy~~ until F134 ended it (2026-11-07): 8.1 MiB at 4 MiB, 18.7 at 16, where its pages outgrew a 32 MiB budget and a page's p99 rose to 299 ms unpaced. At 4 MiB that draw cost no time |
| **W12's empty-table line holds where the counters say a tablet is empty, and M24's page skips those tablets on the loop.** [W4](cursor-and-pages.md#the-request-scanpage)'s claim, an empty tablet a counter's read, is right for a table never written and for an unsorted table whose deletes were compacted | A whole table of 4096 empty tablets was 12 ms of four shards' pages, 22 µs a page, all of it round trips. The counters called the tablets of a sorted table emptied by deletes held, 2,579 of them, until a merge rewrites their dead chains, and a scan read them all: 8,192 dead records, 1.9 s. An unsorted table's deletes were held until a restart compacted them. A sparse table's empty tablets read 14.1 MiB of index pages without the skip, 0.46 MiB with it |

**What it leaves.**

- **M24's work grows**: bounded records for large sorted partitions (✅ built before M24 as
  [F133](../../features/bounded-records.md), 2026-11-07), a walked record handed out from its read
  buffer (✅ [F134](../../features/walked-records-uncopied.md), 2026-11-07), a scan budget's floor (✅
  [F135](../../features/scan-budget-floor.md), 2026-11-08), the split across pages in the reserved position, P20
  restated, and the work bound's maximum.
- **The sizes are M24's to settle**: a record's bound (1 MiB, settled with F133), ~~the floor's exact size~~ the
  floor's (settled with F135 on 2026-11-08, with the user: the node's request body bound and a row's slack of
  64 KiB, about 64 MiB at the defaults, the row held once), and how a partition's pieces lie in a page.
- **M26's listing reads fields kept apart**, and its rate is M26's exit to measure.
- **[Item 374](../../appendix/resolved/scan-draws-held-and-waited.md), found by the listing, is fixed**: pages
  each read a large partition and waited for room for its copy, and were shed with their reads thrown away.


## Limitations

- **The foreground beside E2 is in process, not W7's.** The cell's client shared europa with its node, its
  inserts went to the same Optane as the scan's reads, and the scan was asked page after page with no pause. Its
  ratios compare one scan with another on one setup, the huge partition with the control. They are not W7's
  budget, which is judged on the lab beside the bench.
- **A small answer's wait in a connection's outbox was not measured.** In process, pages pass through no
  connection.
- **The listing walked the tablets the bucket's rows lie in, the way an M26 listing that skips empty tablets
  would.** One walk of every tablet a round, of the 16 MiB bucket, shows what an empty tablet adds over the wire.
- **A listing's speed with fields kept apart is an estimate**, from what an entry's fields archive as (296 to
  312 bytes) and the fit of a paced page's device time to its bytes on one drive, titan's 970 EVO. M26's exit
  measures it.
- **A page covered one tablet**, as X19's walk does, so a sparse bucket's rate was its tablets'. M24's pages read
  a shard's tablets in order until a bound, which reads fewer and larger runs.
- **Every table is the run on item 374's fix.** The runs before it are kept in `results/x19/before374/`. The
  listing's rates for every bucket but 16 MiB fall within the fix's ranges. E1's and E2's page times and draws
  fall within them too, but an archived 17 MiB partition is now refused before its read, and E4 draws twice
  its work bound.
- **The overlaid and resident pages' longest holds in the main set are high for that hour**, as E2 says. One
  check round is all that shows it, and the records cannot name the host's cause.
- **E1, E2 and E4 ran on europa alone**, the Optane and its disk; no 970 EVO ran them in process.
- **Item 372's evidence is E2's first four rounds**, set apart: the steps' lock-in depended on which shard's first
  step was slow, so it struck some partitions and some runs, and not others.

## Related

- [X19's entry on the spike page](spikes.md#x19-the-edges), and [its predictions](../../../shoal-spike-scan/results/x19/predictions.md),
  written before the runs.
- [W4](cursor-and-pages.md), the cursor and the page's bounds; [W9](bucket-listing.md), the listing;
  [W7](isolation.md#the-budget) and [W12](performance.md#the-budgets), the budgets.
- [Resolved #370](../../appendix/resolved/scan-partition-past-its-budget.md),
  [#371](../../appendix/resolved/scan-page-over-dead-partitions.md) and
  [#372](../../appendix/resolved/encode-cost-relearned.md), fixed on the way;
  [item 373](../../appendix/resolved/whole-record-holds.md),
  [O162](../../appendix/optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it)
  and [O163](../../appendix/optimizations.md#o163-a-partition-the-scan-budget-cannot-hold-is-read-or-encoded-before-it-is-refused),
  filed.
- [X18](table-reads-paced.md) and [X17](page-rebuilt.md), whose walk and pace this measured at its edges.
