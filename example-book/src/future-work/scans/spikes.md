# Exploratory spikes

~~**No spike here has run.**~~ **X16 and X20 have run** (2026-10-29), **and X37, X17 and X18**
(2026-10-30), **and X19** (2026-11-07). This page is the list of what has to be measured before the
[milestones](milestones.md) of scans can be more than a guess, and how each would be measured.

A milestone plan is a set of claims: that the walk rebuilt is fast enough and gentle enough, that a
pace exists that holds the foreground, that an order index is worth its cost. The pages before this
one make those claims as preferences and hang each on a question ([W13](contract.md#questions-to-answer)).
A spike is what settles a question, in the sense the object storage part gave the word
([its spikes](../../object-storage/spikes.md#what-a-spike-is)): it answers a question and says in
advance what result would change the design, its code is thrown away, it is never a prototype of the
feature, and it reports what it did not measure.

## How each is written

| Field | Holds |
| --- | --- |
| **Question** | The question, and the `Q` it serves |
| **What would change the design** | The result, stated before the run, that would move a preference |
| **Method** | What is built and what is run |
| **Where** | Host, device, filesystem, governor |
| **Records** | The table it prints |
| **Depends on** | Other spikes, a prerequisite, a gate |
| **Cost** | An afternoon, days, or a week. A guess, and labelled as one |

**Where a spike runs** is the object storage part's lab and its rules
([Where a spike runs](../../object-storage/spikes.md#where-a-spike-runs)): titan and hyperion (Zen1,
the 970 EVO, a disk), europa (the Optane, a disk); built for `znver1` in a target directory of its own;
a scratch directory on the device measured, never the tmdb cluster's; the tmdb node stopped and the
governor at `performance`, both put back; sides alternated by round, four rounds or more, a difference
counted only when the intervals do not overlap; every table labelled by host, cpu, governor, device and
filesystem.

## The spikes at a glance

| # | Spike | Settles | Needs | Cost |
| --- | --- | --- | --- | --- |
| ✅ X16 | The walk beside the foreground, [reported](walk-beside-the-foreground.md) | Q33, Q34, Q37 in part | The lab | ~~Days~~ Done 2026-10-29 |
| ✅ X17 | A page rebuilt, [reported](page-rebuilt.md), both halves | Q33, Q35 | X16 | ~~A week~~ Done 2026-10-30 |
| ✅ X18 | Table reads paced by idle time, [reported](table-reads-paced.md) | Q34, Q37 | X16 | ~~Days~~ Done 2026-10-30 |
| ✅ X19 | The edges, [reported](the-edges.md) | ~~Q36, and~~ What M24 and M26 must bound, and whether M24 splits a sorted partition across pages (Q36 settled without it on 2026-11-06, with the user) | X17 | ~~Days~~ Done 2026-11-07 |
| ✅ X20 | ScyllaDB's scan path, at the source *(optional)*, [read and run](scylladb-scan-sources.md) | W3's readings | Nothing | ~~Days~~ Done 2026-10-29 |
| X21 | Declared order, priced | Q40 | M25 | A week |
| X22 | The ordered listing, two ways | Q41 | M27 | A week |
| ✅ X37 | A tablet's records together, priced, [reported](tablet-records-together.md) | Q62 | X16 | ~~Days~~ Done 2026-10-30 |
| ✅ X38 | Which copies a scan reads, two ways, [reported](copies-two-ways.md) | Q38's copy choice, by the user's decision of 2026-11-14 | M24 | ~~An afternoon to build, a day on the lab~~ Done 2026-11-14 |
| ✅ X39 | The next cursor sent ahead, priced, [reported](cursor-sent-ahead.md): measured, not taken | Whether W4's later gain is kept, by the user's decision of 2026-11-15 | Items 386 and 388 | ~~A day to build, half a day on the lab~~ Done 2026-11-15 |
| ✅ X40 | Saved positions between pages, priced, [reported](saved-positions-priced.md): built as F150 | Whether W5's optional row is built | M24 | ~~An afternoon~~ Done 2026-11-15 |
| ✅ X41 | A scan answer's hop between shards, priced, [reported](answer-hop-priced.md): measured, not taken | Whether W6's hand-off is built | M24 | ~~An afternoon to build, half a day on the lab~~ Done 2026-11-15, in process |

## The spikes

### X16. The walk beside the foreground

**Reported 2026-10-29** on [its own page](walk-beside-the-foreground.md), with W1's four rows' before
and after beside it, and recorded on W13 as [evidence for Q33, Q34 and Q37](contract.md#x16-measured-evidence-for-q33-q34-and-q37-none-settled-2026-10-29).
The walk as it stands does not hold W7's budget beside a scan of a table read from its archives
(the cell's read p99 1.13 times, its insert p99 1.60 times), and its pages hold their loop for
microseconds there: what hurts is the reads beside the WAL's flushes on one device, so X18 comes
first, as the second case below says. A resident sorted table's captures held the loop up to 6.5 ms
and moved the read tail, so the capture's rebuild matters for it. W1's rows at their provisional
defaults changed neither, cost nothing on the query path, and scanned a resident table 16% slower.

| Field | |
| --- | --- |
| **Question** | What does the walk as it stands cost the reference cell when driven as a client's scan would drive it, every shard's two pages at once, back to back; and where does a page's time go? Q33 and Q34, and Q37 in part. No one has measured a walk's effect on a foreground's tail ([F87](../../features/tablet-walk.md#limitations)) |
| **What would change the design** | If the walk as it stands already holds [W7](isolation.md#the-budget)'s budget at full speed, W5's rebuild shrinks to its pacing. If no pace of it holds the budget even on a table larger than memory, where the loop holds are microseconds, then what hurts is the reads' queue and device, not the capture, and X18 comes first. If the cell's tail moves with the loop holds alone, the capture's rebuild is the first change |
| **Method** | A side cluster of the bench's own with the reference cell's table and a scanned table. The scan driven in process through `ShoalPool::walk`, or by the loader's `walk` at every shard's concurrency, beside `rw50` on the cell through `shoaladm bench`. Two scanned tables: one larger than a node's memory, after a restart, so most partitions are archived; and `MovieByKeyword`, resident and overlaid. Each page's `held`, `read` and counts kept; the cell's p99s before, during and after; eviction passes by cause; device bytes |
| **Where** | titan, hyperion and europa; `performance` governor |
| **Records** | By host and table: partitions a second, a page's held and read time at p50, p99 and max, the cell's read and write p99 ratio, `Resident` passes, device bytes a partition |
| **Depends on** | The lab. The bench's `scan` event helps and is not needed: a paced cell beside a hand-driven walk serves |
| **Cost** | Days |

### X17. A page rebuilt

| Field | |
| --- | --- |
| **Question** | What does each change of [W5](tablet-reader.md) buy, on its own: keys planned rather than bytes captured; resident partitions encoded in steps from a second inbox; archive reads in runs in disk order rather than one at a time in key order; the reads in a queue of their own rather than the medium one; rows encoded on the shard rather than walked partitions sent; and what a saved position between pages would save? Since [W42](scan-speed.md) (2026-10-29) also: where a walked partition's up to 73 µs of a Zen1 host's cpu goes, and what one operation a record and several reads in flight buy before runs do. Q33 and Q35 |
| **What would change the design** | If runs buy little because the lab's archives lie mostly in key order after archive passes, reads stay in key order with several in flight. W42 found the lab's archives in no such order: a table loaded once is never copied by a pass and keeps its merges' hash order, so runs wait on merges written in key order ([Q62](contract.md#questions-to-answer)), and reads in flight come first either way. If the profile finds most of a partition's cpu outside its read's operations, W12's cost budget is met there, not at the ring. If stepped encoding costs more than the holds it removes, encoding returns to the plan with a byte bound a step. If rows cost the shard much more than partitions cost the client, Q35's opt-in for unfiltered scans moves forward. If a saved position saves a third of a page, it becomes a gate's work rather than optional |
| **Method** | A spike build of the walk with each change behind a switch, thrown away after. Pages timed step by step on `MovieByKeyword` resident and on a table larger than memory after a restart; encode and decode cpu a GiB, rows against partitions, on server and client; a page asked twice with and without its sorted keys and index position kept. Since W42: a `perf` profile of a walked partition on titan and europa (user, kernel and interrupts, ring operations a partition); an arm with only the background running; reads in flight at 1, 4 and 16; runs at gaps of 4 KiB and 128 KiB over archives merged in hash order and in key order; one operation a record against three |
| **Where** | titan and europa |
| **Records** | Each change's page time, loop hold, device operations and bytes a partition, beside the walk as X16 measured it, and since W42 its cpu and device reads a partition beside W12's cost budget |
| **Depends on** | X16; [O139](../../appendix/optimizations.md#o139-a-scan-driver-asks-a-shed-tablet-again-where-another-tablet-would-be-answered) helps, since a shed stream's asks again are part of a Zen1 node's cpu |
| **Cost** | A week |

### X18. Table reads paced by idle time

| Field | |
| --- | --- |
| **Question** | Which ceiling and trickle let a scan read a table's archives without moving the cell's read or write p99 past the budget, on each kind of device the lab has? Since [W42](scan-speed.md) (2026-10-29), against a walk fast enough to reach its ceiling, and with the ceiling stated in device time or reads as well as bytes: at one record a read 100 MiB a second is about 100,000 reads, half a 970 EVO's. Q34 and Q37 |
| **What would change the design** | If no ceiling holds the write p99 on the 970 EVO, where a table's archives share the device with its WAL, the idle check has to see flushes too, or the default ceiling drops. If no pace holds the Optane's budget, as X12 found for recovery there, the Optane's bound is set the way M16 set one ([S13](../../object-storage/isolation.md#io-on-a-slice)). If idle gating alone, without a ceiling, holds the budget, the ceiling is a safety rather than the pace |
| **Method** | X12's method on a table's archives: ceilings of 32, 64, 128 and 256 MiB/s a device, each with and without idle gating, trickles of 1, 2 and 4 MiB/s, beside the cell; on the 970 EVO with the WAL beside, the Optane, and a disk holding the archives' root |
| **Where** | titan and hyperion (the 970 EVO, a disk), europa (the Optane, a disk) |
| **Records** | By device and pace: the cell's read and write p99 ratio, the scan's MiB/s, the trickle's share |
| **Depends on** | X16, and ~~a spike build with the pacing~~ [F119](../../features/scan-pacing.md)'s pacing, built into the product with `scans.ceiling` and `scans.trickle` as its settings; a trickle equal to the ceiling is the pace without idle gating; and since X17's reads half (2026-10-30) its walk in runs, which reads 3.6 to 6.6 times a device's default ceiling unpaced ([its record](page-rebuilt.md#what-it-says-for-the-design)), past every ceiling X18 tries |
| **Cost** | Days |

### X19. The edges

**Reported 2026-11-07** on [its own page](the-edges.md), and recorded on W13 as
[the decisions it closed](contract.md#x19-measured-the-edges-decided-with-the-user-2026-11-07). The user's
rule for every one of them: performance first, and no partition may fail or go missing from a scan. So M24
splits a sorted partition too large for a page across pages, from records of bounded size, which also cures
the whole-record hold X19 found on a shard's thread ([item 373](../../appendix/resolved/whole-record-holds.md)).
A row that cannot be split is handed out from its read buffer under a budget floor of the largest row, never
refused for size (built as [F134](../../features/walked-records-uncopied.md) and
[F135](../../features/scan-budget-floor.md)). A listing's fields kept apart from inline bytes is required before M26: at the default
threshold a listing read 58 times its entries' fields. A page's work bound is at most 4 MiB. An empty table
costs a counter's read a tablet, except a sorted table emptied by deletes, whose dead chains are read until
merged. It found and fixed items 370, 371, 372 and 374, and filed item 373, O162 and O163.

| Field | |
| --- | --- |
| **Question** | What does a scan cost at its edges: a table of 4096 empty tablets; a table of one sorted partition larger than a page; a listing of a bucket of inline objects, bytes read a listed entry against an inline threshold from 1 KiB to 16 MiB; a filter that keeps nothing, its pages ended by work? ~~Q36, and~~ What M24 and M26 must bound. Q36 was settled by design on 2026-11-06, with the user, leaving X19 the one thing it bears on: whether M24 fills the cursor's reserved sort-key position ([the record](contract.md#q36-the-cursor-and-the-frames-2026-11-06)) |
| **What would change the design** | If a listing of inline objects reads too much against the budget, keeping a listing's fields apart from an object's bytes moves from optional to required before M26. If one huge sorted partition's page breaks the budget, splitting a sorted partition across pages moves before M24 and fills the cursor's reserved ~~suffix~~ position, with [P20](contract.md#the-contract) restated for a partition handed out in pieces; otherwise M24 keeps refusing one whose answer passes a page's whole draw of 4 MiB by name, which ends its scan. If an empty table costs more than a page a shard, the per-tablet counters are not what W4 assumes |
| **Method** | A standalone node in process for the empty table, the huge partition and the filter; a side cluster for the listing |
| **Where** | europa in process; titan for the listing |
| **Records** | Pages and time for each edge; bytes read a listed entry by threshold |
| **Depends on** | X17 |
| **Cost** | Days |

### X20. ScyllaDB's scan path, at the source

*(Optional: no gate waits on it.)*

**Read and run 2026-10-29** on [its own page](scylladb-scan-sources.md), at `scylla-2026.3.4`
(`daef9dad`): of W3's twenty claims fourteen held as written, four held with something W3 left out,
and two were wrong in part, both in the semaphore (a read is admitted while up to two others need
only the cpu; a saved reader also goes when its time to live runs out), and neither changes the
design. The 2025 post's scanner, `fee-mendes/tablet-fullscans`, turned out to simulate its
scheduling rather than scan, so the run drove a scanner of the same shape written for it, beside
X16's cell on X16's hosts: ScyllaDB's scan read the table 40 to 52 times as fast as Shoal's walk
beside the same cell and doubled the cell's p99 while it ran (2.34 times on reads, 2.15 on
inserts, for 2.15 s), so it holds W7's budget no better; carrying every row to the client it filled
the client's link, and the cell's tail on that client went to the kernel's minimum retransmission
timeout. Recorded on W13 as [evidence for Q33, Q34 and Q37](contract.md#x20-run-evidence-for-q33-q34-and-q37-none-settled-2026-10-29).

| Field | |
| --- | --- |
| **Question** | Are [W3](prior-art.md)'s readings right at a pinned release, and what does ScyllaDB's own tablet-era full scan cost its foreground on the lab? |
| **What would change the design** | A borrowed mechanism read differently at the source than in the documents, saved readers' memory bound or the semaphore's admission most likely, changes W5's or W7's version of it. A ScyllaDB foreground far better beside a scan than W7's budget would argue for tightening the budget |
| **Method** | [X14](../../object-storage/ceph-and-s3-sources.md)'s method: a release pinned by its commit, the files W3 names read and cited by line, predictions written before. Optionally a three-node ScyllaDB on the lab, the 2025 post's tablet scanner beside a stress load, every host change recorded and put back |
| **Where** | Reading anywhere; the lab for the run |
| **Records** | Each W3 claim confirmed, corrected or still recalled; the foreground's p99 ratio beside a scan |
| **Depends on** | Nothing |
| **Cost** | Days |

### X21. Declared order, priced

| Field | |
| --- | --- |
| **Question** | What does an order index cost a write at apply and at compaction, an ordered page covering and not, and how evenly do skewed placement values fall over 4096 tablets with and without spread? Q40 |
| **What would change the design** | If index maintenance moves an insert's p99 past what [W12](performance.md#the-budgets) allows a declared-order table, the index's delta or its runs are redesigned before M27. If pages that are not covering are slow on disks, covering is the default for an order's fields. If realistic skew puts a group past its commit rate at spread one, a default spread, or a refusal of a declaration without one, follows |
| **Method** | A pure, seeded simulation, as [X2](../../object-storage/placement-simulation.md) was, of placement values drawn uniformly and by Zipf at 0.9, 1.1 and 1.3 over 4096 tablets and the lab's and larger node counts, spread 1 to 64. And a spike build of an index in one table's apply and compaction, measured with inserts, gets and ordered pages |
| **Where** | Anywhere for the simulation; titan and europa for the build |
| **Records** | The simulation's imbalance by spread; an insert's and a get's p99 with and without the index; ordered pages a second, covering and not, on an SSD and a disk |
| **Depends on** | [M25](milestones.md#m25-scans-across-a-cluster) |
| **Cost** | A week |

### X22. The ordered listing, two ways

| Field | |
| --- | --- |
| **Question** | Which ordered listing does M28 build: an ordered `ObjectMeta` on W8's index for new buckets, or Q32's pending-then-complete index beside the existing layout? Q41 |
| **What would change the design** | If the pending commit before every put and delete costs a put little and orphans are rare under failures, Q32's index serves every bucket and the layout stays. If it doubles a small put's latency, the ordered layout for new buckets is the way, and existing buckets list unordered |
| **Method** | Both built as spikes on a side cluster: put and delete rates and latencies, listing pages a second by prefix, list-after-write and list-after-delete checked under a kill, and orphans counted after crashes between the pending entry and the commit |
| **Where** | The lab |
| **Records** | Each way's put, delete and listing figures, and its correctness under faults |
| **Depends on** | [M27](milestones.md#m27-ordered-scans-by-declared-order) |
| **Cost** | A week |

### X37. A tablet's records together, priced

| Field | |
| --- | --- |
| **Question** | What does writing a merge's partitions in key order cost a merge, and how many stretches does it leave a tablet on its shard, under each load the lab runs: a table loaded once, a table overwritten, a table inserted into for an hour? Is that few enough for [W12](performance.md#the-budgets)'s cost budget, or does a table need its blocks consolidated? Q62 |
| **What would change the design** | If the sort costs a merge more than a tenth of its time or memory, it moves off the compactor's critical path, or merges grow. If a tablet keeps so many stretches that a page reads fewer than ten partitions a device read, consolidated blocks become a gate's work rather than optional. If archive passes reorder an overwritten table well enough on their own, consolidation stays optional for good |
| **Method** | A spike build of the compactor with the sort behind a switch, thrown away after. `Big` loaded on X16's side cluster each way; the merge's time and memory by segment; from the archive map, each tablet's stretches on a shard and their lengths; a walk of every tablet with runs at gaps of 4 KiB, its reads a page and device bytes a partition; a get's p99 and a snapshot cut timed beside, each way. Then an hour of overwrites and inserts, and the stretches counted again |
| **Where** | titan and europa |
| **Records** | By layout: a merge's time and memory, stretches a tablet and their records, reads a page, device bytes a partition, a get's p99, a cut's time |
| **Depends on** | X16 |
| **Cost** | Days |

### X38. Which copies a scan reads, two ways

Filed 2026-11-14, when Q38 was taken up before M25. The rest of Q38, which copies may be read at all and how
a level is served, follows from the code and the contract and is settled by design; which of a tablet's
voter copies a scan prefers is the user's, who asked for it measured: two options, side against side
([its record](copies-two-ways.md)).

**Reported 2026-11-14: option 1, followers then leaders.** The two read `Big` beside the cell in 28.2 and
28.4 s and cost the cell and a paced stream the same on every figure. Leading copies answered 0.36 of the
pages against 0.48. The rule gives a tie to option 1. Both are fast because both deal and take: europa
answered 0.84 of the pages under either. Its control, the rotated driver against followers at the same commit,
put that at 5.1 times beside the cell and 2.66 alone on `Big`, and 17.8 times on `Keyword`. It also found
[O181](../../appendix/optimizations.md#o181-a-walk-of-one-tablet-in-no-key-order-reads-its-maps-sixteen-pages-ahead-for-its-next-walk-to-miss).

| Field | |
| --- | --- |
| **Question** | Does a scan that reads followers and takes a leader's copy only of what no follower waits for (option 1) cost the foreground less, or read slower, than one that reads every voter copy alike, its leader as much as a follower (option 3)? Q38 |
| **What would change the design** | Option 3 is taken only if its scan is faster by a disjoint range, beside the cell or alone, its cell figures hold [W7](isolation.md#the-budget)'s budget wherever option 1's do, and no cell or paced figure is worse by a disjoint range; otherwise option 1, a tie included, since it spares leaders at no measured cost. A result the rule cannot read goes to the user. W6's preferred direction, followers first, stands under option 1 |
| **Method** | The bench's scan driver reads either (`--scan-copies followers` and `voters`, F115's driver), both dealing every tablet once to a lane a member, evenly, each member's streams reading its own lane and then taking from the others' what they hold a copy of, so the two differ only in the copies they prefer. M23's harness's `BIG_ONLY` and `KEYWORD_ONLY` rounds, one worktree for both sides (`AB_A_BENCH`, `AB_B_BENCH`), eight side-rounds each, ABBA |
| **Where** | The lab's three hosts as M23's side cluster `m23`: europa on the Optane at `lead_weight: 2`, so it leads most groups, titan and hyperion on their 970 EVOs; `performance` governor |
| **Records** | By side: the whole scan's time beside the cell and the scan of `Big` alone's rate; each member's share of the pages and the share a copy that led its group answered; the cell's inserts a second and read and insert p99 beside the scan and alone; the paced stream's insert p99, which for `Keyword` writes into the groups the scan reads |
| **Depends on** | M24, done; nothing of M25, since the bench's driver reads a cluster through the admin walk |
| **Cost** | An afternoon to build, a day on the lab. A guess |

### X39. The next cursor sent ahead, priced

**Reported 2026-11-15** ([the record](cursor-sent-ahead.md)): **measured, not taken.** With the ahead one stream a shard
read as fast as two without it, but the cell beside it read 1.06 times slower at the p99 second by second, the ranges
disjoint, and at two streams it bought nothing. Its client half is reverted; O182 is filed.

Filed 2026-11-15, when the *After M24* box was taken up ([Finishing After M24](whats-left-todo.md#finishing-after-m24)).
W4 left sending a stream's next page before its rows are read as a later gain "priced on M24's streams"
([W4](cursor-and-pages.md#alternatives-rejected)); the user decided it is measured before it is kept.

| Field | |
| --- | --- |
| **Question** | Does a stream that asks its next page as soon as its last page's head arrives, before that page's body, read a scan faster, or let one stream a shard read as fast as two do today? |
| **What would change the design** | It is kept, on by default, if a scan of resident rows or of archived rows at the client's default streams is faster by a disjoint range, or one stream a shard with it reads as fast as two without (the ranges overlapping), and the cell's read and insert p99 beside it are no worse by a disjoint range; in the second case the client's default may drop to one stream a shard, which asks half its node's scan class. Otherwise it is reverted and recorded as measured, not taken |
| **Method** | **The pre-check first** (added 2026-11-15, before any run). The ahead keeps at most two pages of a stream in flight, so a stream with it asks no more of its node at once than two streams without it, and what it buys is bounded by what a second stream a shard buys. So the scans alone, resident and archived after a restart, are first run at one and two streams a shard on the build as it is, six rounds, the order alternating (`STREAMS_ONLY=1`). If one stream a shard reads as fast as two on both, the ranges overlapping, the ahead cannot meet either clause: at the default streams it would ask four pages a shard of a class of two, and one stream a shard already reads as fast as two without it. It is then recorded measured, not taken, without being built, and what one stream a shard buys is filed apart. Otherwise: the client's driver built with the ahead behind a builder setting, off and on in one build, so a side is a flag. M24's exit harness: titan's standalone node of five shards, europa's client. A scan alone of resident rows and of archived rows after a restart, each at one and two streams a shard, the ahead off and on; and the cell beside a resident scan at the default streams, both ways. Four rounds, the sides alternating |
| **Where** | titan (the node, its 970 EVO) and europa (the client), over 1 GbE, `performance` governor |
| **Records** | By side: the scan's time and rate, its pages and those asked again, the node's scan figures and sheds, and beside the cell its read and insert p99 |
| **Depends on** | Items 386 and 388, which hold a stream's second page to its node's bound and its body to its page's |
| **Cost** | A day to build, half a day on the lab. A guess |

### X40. Saved positions between pages, priced

**Reported 2026-11-15** ([the record](saved-positions-priced.md)): **built**, as [F150](../../features/saved-positions.md).
At K_max, 62,914 partitions of a key alone a tablet, a page's keys and plan held its loop up to 840 µs and its pass
over the tablet's keys was 55% of its time on the shard; a cut partition's later pages cost no more than its first.

Filed 2026-11-15 with X39. [X17](page-rebuilt.md#a-page-captured-against-planned) found nothing to save on the lab's
tables, every tablet one page, and left W5's saved positions optional, to be priced on a table whose tablets take
many pages ([W5](tablet-reader.md#saved-positions-optional)). Since [Resolved #392](../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md)
a page's reads of its map are served by a read-ahead cache, the user's choice over cursors kept between pages, so
what a saved position could still save is a page's pass over its tablet's resident keys and its search inside a
partition cut across pages.

| Field | |
| --- | --- |
| **Question** | What does a page cost to find its place again in a tablet many pages deep: its pass over the tablet's resident keys and its selection, its view of the map, and its search inside a cut partition? |
| **What would change the design** | ~~Saved positions are built if, on a tablet a node of the lab's size (6 GiB) holds resident, a page's capture holds its loop past W7's 100 µs, or a tablet's last pages cost its shard 1.5 times what its first did.~~ Amended 2026-11-15 by the *After M24* plan's review, before any run: a planned page passes over every resident key of its tablet on every page, so its first page is as dear as its last and the old rule's 1.5 times could never be met by the key pass. The deepest tablet that matters is **K_max**, the resident partitions of a table of the smallest rows a schema can declare, a `u64` key alone, that a 6 GiB node holds, over its 4,096 tablets, measured in the run itself. Saved positions are built if, at K_max or at four times it, a page's keys and plan together hold its loop past W7's 100 µs, or a page's pass over its tablet's keys is 10% or more of its time on the shard, or a partition cut across pages costs its k-th page 1.5 times what its first did. Otherwise measured, not taken, and the row stays optional |
| **Method** | An ignored timing in release on a standalone node in process, pinned to one core, as `the_readers_cost_a_row` is: ~~one tablet filled to 10,000 to 200,000 resident partitions~~ K_max measured first, a node of 6 GiB filled with `u64` keys until it evicts, then one tablet filled to K_max and to four times it, every page small enough that the tablet takes many pages; the same tablet read from its archives after a restart; and one sorted partition of many records cut across pages; every page's capture parts (its keys, its plan and its view), its longest step and its time on the shard, by the page's place in its tablet |
| **Where** | europa, one pinned core; a Zen1 host's run of the same binary if the result lies near the rule's line |
| **Records** | By a tablet's resident partitions and a page's place: the capture's parts, the longest step, the page's time on its shard |
| **Depends on** | M24, done |
| **Cost** | An afternoon. A guess |

### X41. A scan answer's hop between shards, priced

**Reported 2026-11-15** ([the record](answer-hop-priced.md)): **measured, not taken.** In process, a 4 MiB answer
written after a hop cost 1.063 times the cpu a gibibyte on europa and 1.072 times on titan, 0.07% of a Zen1 core at
31.25 MB a second; the lab's spike build was not made.

Filed 2026-11-15 with X39. A connection lands on whichever shard accepted it, so a page's answer is written by a
shard that did not read it, unless the connection landed on the one that did; W6 left handing a scan's connection to
the executor it reads as a later gain, measured first ([W6](distribution.md#streams)). The hop moves buffers and
copies nothing ([F134](../../features/walked-records-uncopied.md)); what it can cost is a wake across cores and a core
writing bytes another core read. X11 priced a hop of an inbound stream at 1.30 to 1.72 times the cpu a gibibyte in
plaintext, and in its noise under kTLS ([X11](../../object-storage/streamed-bodies.md#4-a-connection-handed-over-and-bytes-that-hop));
nothing has priced a scan's.

| Field | |
| --- | --- |
| **Question** | What does a page's answer cost its node when it is written by a shard that did not read it, against the same bytes written by the shard that did? |
| **What would change the design** | The hand-off is built only if hopped answers cost at least 1.25 times the cpu a gibibyte of answers written where they were read, and the difference is at least 2% of a Zen1 core at the default link share, or a scan's rate on europa's loopback, unpaced, is bound by the shard writing its answers. Otherwise measured, not taken |
| **Method** | **In process first** (amended 2026-11-15 by the plan's review, before any run): two glommio executors pinned to two cores of one host, a scan answer's body of 4 MiB as a shard encodes it, written to a loopback socket's sink by the executor that built it and by the other after a hop that moves its buffers as a page's answer does; each side's own cpu a gibibyte on the writing thread and the hop's on both, on europa and, built for znver1, on titan. If the hopped write costs less than 1.25 times the other's, or the difference is less than 2% of a Zen1 core at the default link share's rate, it is measured, not taken, and the lab's spike build is not made. Otherwise: a spike build, never merged, that counts each shard's own cpu writing a scan's frames and their bytes, by whether the page was read on that shard or on another. A scan of resident rows alone on titan's standalone node from europa at the default share and with `link_share: null`, and the same on europa's loopback, unpaced. Four rounds |
| **Where** | titan (the node) and europa (the client, and the loopback leg), `performance` governor |
| **Records** | By kind of write: the cpu a gibibyte, its share of a core at the scan's rate, and the scan's rate |
| **Depends on** | M24, done |
| **Cost** | An afternoon to build, half a day on the lab. A guess |

## What has to be on the record before each gate

| Gate | Decided first |
| --- | --- |
| M23 | Q33, Q34, ~~and~~ Q37's default and Q62, by X16, X17, X18 and X37 |
| M24 | ~~Q35 and Q36, by X17, X19 and design~~ Q35 settled 2026-11-06 by X17 and design, with the user ([the record](contract.md#q35-what-a-scans-answer-carries-2026-11-06)), and Q36 the same day by design, with the user's leave to settle it before X19 ([the record](contract.md#q36-the-cursor-and-the-frames-2026-11-06)); ~~X19's bounds, and whether a sorted partition is split across pages~~ X19's bounds and the split, decided 2026-11-07 with the user ([the record](contract.md#x19-measured-the-edges-decided-with-the-user-2026-11-07)) |
| M25 | ~~Q38 by design, settled in part on 2026-11-14~~ Q38 settled 2026-11-14 ([the record](contract.md#q38-which-copies-a-scan-reads-and-its-levels-2026-11-14)), by design and its copy choice by X38, the user's decision the same day: followers then leaders; and ~~Q39 with the user~~ Q39 settled with the user on 2026-11-10, ahead of M24's grant: `all` includes `scan` |
| M26 | ~~What a listing reads, by X19~~ Decided 2026-11-07 by X19 and the user: a listing's fields kept apart from inline bytes, required before M26 ([the record](contract.md#x19-measured-the-edges-decided-with-the-user-2026-11-07)) |
| M27 | Q40, by X21 |
| M28 | Q41, by X22 |

## Related

[W13](contract.md#questions-to-answer) for the questions; [the milestones](milestones.md) for the
gates; [the object storage part's spikes](../../object-storage/spikes.md), whose shape and rules this
keeps; [What's left to do](whats-left-todo.md) for the order drawn.
