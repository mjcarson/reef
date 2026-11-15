# 392. A scan's walk read its table's archive map from the device on every page, unpaced and uncounted, even over resident rows

## Symptom

Chasing [item 391](paced-scan-reads-and-inserts.md)'s
insert part on the lab (2026-11-13), titan's NVMe counters said what the scan's own figures did not. Beside a scan of a
million resident rows, which its figures said read no archive (`reads` and `archived` 0 on every shard), the node read
its drive hundreds of times a second, in reads of 7 to 14 KB. The rate followed the scan's own:

| The share's sweep, four rounds | titan's NVMe reads a second: before, during | MB/s during | Cell's insert p99 ratio |
| --- | --- | --- | --- |
| The cell alone | 24, 23 | 1.3 | 0.98 |
| A scan at 25% of the link | 30, 194 | 1.9 | 1.16 |
| At 75% | 16, 679 | 7.4 | 1.66 |
| Unpaced | 29, 803 | 6.3 | 1.59 |
| Projected, almost nothing sent | 20, 3,858 | 28.7 | 1.42 |

They shared the device with the intent logs' fdatasyncs, which beside the scan came later and took longer: the kernel's
fdatasync p99 rose from 23.4 to 34.7 ms beside the 25% scan, the fsyncs fell from 364 to 329 a second, and the
reference cell's inserts rose 1.3 to 1.4 times, over [W7's budget](../../future-work/scans/isolation.md#the-budget) of
1.10.

## Cause

A walk page's capture takes the archive map's view of its key range (`ArchiveMap::scan_range`), which reads nothing on
the loop. Its merge, off the loop, pulls the view's entries to compare their keys with the page's resident
partitions, and each of the map's runs reads its index pages for the range through a cursor of its own
(`RunCursor::next`, `tables/storage/fs/index/run.rs`): 4 KiB pages, direct, never through the map's page cache, so a
scan would not evict what point lookups cached.

- **A page over resident rows read it too.** Every partition a table ever archived is in its runs, so the merge pulled
  their entries before a resident copy shadowed them: one or more index reads a run a page, two or three pages each, and
  the next page read the boundary page again. A table has 4,096 tablets and a walk asks for them a page or so each, so
  a scan of a million rows over five shards made about 5,000 pages and read the map about 7,000 times.
- **Nothing paced them.** F119, F125 and F128 pace and draw an archived record's read. None of the calls between the
  merge's `head` and the cursor's read took from the device's pace, its read slots or the scan budget, so the reads ran
  at whatever rate the scan walked, and the device's learned cost a byte never saw them.
- **Nothing counted them.** `PagedIndex::page_reads` counted the cached point lookups alone, and a scan's `reads`
  counted records.

## Evidence

**Established by measuring it on the lab, reading the source, and a spike that took the reads away**, then
**reproduced by a test**.

- The lab's counters above, and an Explore agent's reading of the walk, the map's index and the paces (2026-11-13).
  The node's own figures predicted the rate: 5,090 pages in 33 s at 1.4 of the map's runs a page is about 214 run
  cursors started a second, against about 230 reads more than the cell alone's.
- **The spike** ([the map in memory](../../future-work/scans/link-pace.md#the-map-in-memory-the-cause-shown), branch
  `i392-spike`, never merged) served the runs' pages from memory, three rounds each. Beside the 25% scan titan's NVMe
  read 28 times a second against 246, the kernel's fdatasync p99 held at 22.8 ms against 34.1, and the cell's inserts
  rose 0.90 [0.87, 0.95] times against 1.30 [1.12, 1.42].
- **The fix on the lab** ([its record](../../future-work/scans/link-pace.md#the-fix-on-the-lab)), three rounds: titan's NVMe
  read 42 times a second beside the 25% scan, the kernel's fdatasync p99 rose 23.4 to 24.2 ms, and the cell's inserts held
  1.01 to 1.04 times their own second by second, 1.06 [0.95, 1.18] over the window, where without the fix they rose 1.22 to
  1.28 and 1.38.
- **The test**, `a_walks_pages_read_each_index_page_once`, written first: a run of 6,000 partitions walked in pages of
  fifty, as a scan's stream asks them. Against the unfixed tree:

  ```text
  120 reads of the run's 42 pages by 120 walk pages, at most 5
  ```

## The fix

- **A walk reads its map ahead, once.** A walk page's view is `ArchiveMap::scan_walk`, whose cursors share a
  `ReadAhead`: on a page's first read the cursor reads up to sixteen pages at once (`SCAN_PAGES`, 64 KiB) past its range,
  and keeps its range's last page and every page past it in a cache of the scans' own (`PagedIndex::scan_cache`, a
  `PageCache` beside the point lookups'). The walk's next pages take theirs from it with no read. The view's key range
  is unchanged; only its reads go further. The read-ahead runs past the page's tablet: a walk goes on through its
  executor's tablets in key order, and a shard's map holds only its own partitions, so the next tablet's entries are the
  run's next pages. A run a merge retires is forgotten by the cache with it.
- **Its reads keep to the page's pace and budget.** Every read of the device a cursor makes is admitted first through a
  `RunReads` its caller hands down through `Scan::next_with`: a scan page's `PageMapReads` covers the read's buffer from
  its draws on the scan budget (F118), takes its bytes from its device's pace (F119, F128) and holds a place among the
  device's scan reads (F128) only while it is in flight. A driver's page, the compactor's merge and a scan of the whole
  map pass `Unpaced`. A read the budget has no room for ends the page short where it stands, as a record's does, or
  sheds it if it covered nothing.
- **They are counted.** Each class's walk figures carry `map_reads`, `map_bytes` and `map_cached`, a node's summed from
  its shards' and zero from a build before this.
- **Its room is a setting.** `storage.*.filesystem.map.scan_cache_bytes`, 1 MiB a table a shard unless set, beside
  `page_cache_bytes`;
  below one page there is no read ahead, and every page reads its own range, still paced and counted.

Through a node in process, a walk of 8,000 partitions in 4,096 pages read the map 4 times, 56 of its pages, and took
4,145 pages from the cache, where every page had read its own.

## Alternatives rejected

- **A stream's run cursors kept from page to page**, the plan's saved positions between pages brought forward: what the
  user chose first. The cache reaches the same reads, each index page once in 64 KiB, with no state tied to a stream's
  next request, nothing to invalidate when a client stops asking, and concurrent scans sharing what any of them read;
  decided with the user on 2026-11-14.
- **Pacing and counting alone.** At the default share a 25% scan's map reads took about 50 ms of the device's 75 a
  second, so the pace would not have bound them, and item 391's insert part would have stayed.
- **The scans reading through the point lookups' cache.** A whole scan of more partitions than its budget holds evicts
  every page the lookups cached, which is why `RunCursor` bypassed it.
- **Skipping the map over tablets whose partitions are all resident.** It needs an exact count kept through evictions,
  restarts and moves of what is archived and not resident; a miscount loses rows.
- **Reading ahead only to the page's tablet's end.** A tablet of the lab's table is about 1.7 index pages a run, walked
  in about one page, so a bound at the tablet would have saved the boundary re-reads alone.

## Invariants to uphold

- **Every read a scan page makes of its device is drawn and paced, its map's as its records'**: a new read in the walk's
  path takes a `RunReads` from its page, never `Unpaced`, unless it is a driver's (F129).
- **A read-ahead page is kept only in the scans' cache, never in the point lookups'**, and only a run's page; a run's
  pages never change, which is what makes a cached page never stale.
- **A retired run's pages leave the scans' cache in the merge that retires it** (`PagedIndex::merge`), and the cache
  holds pages, never a run's handle, so it never keeps a run's file open.
- **A capture's view allocates nothing on the loop** (O154, F136): `scan_walk`'s start moves an `Rc`'s count and copies a
  `u64`; the read ahead is made at the scan's first read, off the loop.
- **`done` follows every `admit`**, whether the read succeeded or not, so a place among the device's scan reads and a
  draw's room are never held past their read.
- **No borrow of a cache is held across an await.**

## Still open

- ~~Whether item 391's insert part holds W7's budget with the fix, which the lab's rounds measure next~~ It does
  ([the rounds](../../future-work/scans/link-pace.md#the-fix-on-the-lab)), and M24's confirmation held it over eight rounds:
  1.00 to 1.04 at the median beside every resident scan, titan's NVMe at 14 to 43 reads a second
  ([the confirmation](../../future-work/scans/link-pace.md#the-confirmation), [Resolved #391](paced-scan-reads-and-inserts.md)).
  ~~O179's slice is measured again after it, as decided with the user.~~ O179's slice was measured again after it
  and not taken: with the map's reads paced and read once, the workers' waits it removes cost the inserts nothing
  ([the record](../../future-work/scans/link-pace.md#the-slice-again-after-item-392)).
- ~~Beside a scan that no longer reads its drive the cell's reads rise more than before, 2.2 times second by second against
  1.9, outside the node's threads: item 391's read part.~~ Set beside its twin in the same rounds, the rise is a paced bulk
  flow's of the scan's rate within its spread, 2.07 [2.03, 2.25] second by second against the twin's 1.88 [1.83, 2.58]
  ([the read part beside its twin](../../future-work/scans/link-pace.md#the-read-part-beside-its-twin)): item 391's read
  part, titan's network path under a steady quarter of its link. The confirmation's eight rounds held it beside the twin,
  and item 391 is resolved.
- The cache fills only when walks run, but holds up to `scan_cache_bytes` a table a shard: on a node of many tables and
  shards that is map memory a node's floor counts ([item 338](resident-floor-reserve.md)).
- A walk of a single tablet reads ahead past it into pages it may never use, one 64 KiB read where it read 8 to 12 KiB;
  a driver's walk of one placement group's tablet is such a walk.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_walks_pages_read_each_index_page_once` (shoal-core, `index`) | A walk of 120 pages reads its 42-page run 120 times, where it reads it in five or fewer |
| `a_walks_read_ahead_reads_no_further_than_its_walk_goes` (shoal-core, `index`) | A read ahead past its bound reads and keeps pages its walk never asks for |
| `a_walks_read_ahead_leaves_the_point_lookups_cache` (shoal-core, `index`) | A walk evicts what point lookups cached, and their lookups read again |
| `a_merged_away_run_leaves_the_scans_cache` (shoal-core, `index`) | A retired run's pages stay in the scans' cache |
| `a_walks_reads_of_the_device_are_admitted_by_its_reader` (shoal-core, `index`) | A cursor reads the device without its reader's leave, or reads one it refused |
| `no_room_for_the_scans_cache_reads_nothing_ahead` (shoal-core, `index`) | A cache with no room still reads ahead, every read thrown away, or its reads go uncounted |
| `a_scan_pages_reads_of_its_map_are_drawn_paced_and_counted` (shoal-core, `walk::reads`) | A scan page's map reads skip its budget, its device's pace or its read slots, or go uncounted |
| `a_scan_page_with_no_room_for_a_read_of_its_map_reads_none` (shoal-core, `walk::reads`) | A page reads its map past its budget, or is refused holding a draw |
| `a_classs_map_reads_are_its_shards_summed` (shoal-proto, `stats`) | A node's map figures are not its shards' summed, or a build before them fails to read |
| `a_walk_reads_its_tables_map_ahead_and_counts_it` (`shoal/tests/tablet_walk.rs`) | A node's walk in small pages reads its map a page at a time, or its figures do not count it |

## Related

- [Item 391](paced-scan-reads-and-inserts.md),
  whose insert part this is, and [the record](../../future-work/scans/link-pace.md#item-391-placed) that placed it.
- [F76](../../features/paged-archive-map.md), the paged map whose runs the walk reads.
- [F118](../../features/scan-budget.md), [F119](../../features/scan-pacing.md) and
  [F128](../../features/device-time-pace.md), the budget and paces its reads keep to now.
- [F129](../../features/drivers-on-the-rebuilt-walk.md), why a driver's page stays unpaced.
- [O179](../optimizations.md#o179-a-shard-holds-its-cpu-through-its-eevdf-slice-while-the-io-wq-worker-running-its-fdatasync-waits),
  the slice the same chase found and the spike set aside.
