# X40. Saved positions between pages, priced

**Reported** (run 2026-11-15). This is the record of spike [X40](spikes.md#x40-saved-positions-between-pages-priced):
what a page costs to find its place again in a tablet many pages deep.
[W5](tablet-reader.md#saved-positions-optional) left saved positions between pages optional, to be priced on a table
whose tablets take many pages, and [X17](page-rebuilt.md#a-page-captured-against-planned) found nothing to save on the
lab's tables, every tablet one page. Since [Resolved #392](../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md)
a page's reads of its map are served by a read-ahead cache, so what a saved position could save is a page's pass over
its tablet's resident keys and its search inside a partition cut across pages.

**The verdict: built**, as [F150](../../features/saved-positions.md). A planned page looks at every resident key of
its tablet on every page. At the depth a lab-sized node can hold of the smallest rows, that pass held a page's loop
up to 840 µs and was 55% of its time on its shard, past both of the rule's bounds. A partition cut across pages cost
its later pages no more than its first.

## The question

**What would change the design**, amended 2026-11-15 by the *After M24* plan's review before any run
([the spike's row](spikes.md#x40-saved-positions-between-pages-priced)): the deepest tablet that matters is **K_max**,
the resident partitions of a table of the smallest rows a schema can declare, a `u64` key alone, that a 6 GiB node
holds over its 4,096 tablets, measured in the run itself. Saved positions are built if, at K_max or at four times it,
a page's keys and plan together hold its loop past W7's 100 µs, or its pass over its tablet's keys is 10% or more of
its time on the shard, or a partition cut across pages costs its k-th page 1.5 times what its first did. Otherwise
measured, not taken. The predictions were written first (`shoal-spike-scan/results/x40/predictions.md`).

## How it is run

An ignored timing in release on europa, against a standalone node of one shard in process
(`shoal/tests/scan_depth.rs`, `a_deep_tablets_pages_cost`):

```text
cargo test --release -p shoal --test scan_depth -- --ignored --nocapture
```

- **K_max**: 200,000 partitions of a key alone written, and the shard's own count of its rows read before and after
  (`memory_bytes` in its report), which is what its eviction is held to; then a lab node's rows' budget, a 6 GiB node
  of five shards less what it carves out (`Resources::shard_budget`), over that charge and 4,096 tablets.
- **One tablet at K_max and at four times it**, written straight into tablet zero, read a page at a time at a client's
  default bounds (1,000 rows) by one stream, so each page is asked only once the one before it is handed out; each
  page's capture parts, keys looked at and steps read from the shard's figures before and after it
  (`ReadVerb::Walks`). Then the node restarted and the tablet read again.
- **One sorted partition of 100,000 rows of 100 bytes**, cut across pages, the same way, resident and after a restart.

## K_max

A resident partition of a key alone is charged 25 bytes by the shard's count of its rows. A 6 GiB node of five shards
keeps 6,442,450,940 bytes for rows, so **K_max is 62,914 partitions a tablet**, four times it 251,656.

The shard's count is of rows alone; the process holds more for each partition, its tablet's map's bucket and its
eviction list's entry among them, so a node's resident memory would reach its bound before its rows did, and the true
depth is smaller. The rule's second bound is met down to about 3,400 keys a tablet, so the verdict stands at any depth
a node could hold.

## Before: every page passes over its tablet

| Tablet | Page | Keys looked at | Keys, µs | Plan, µs | Capture, µs | Encodes, µs | Client's wait, µs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| K_max, resident | 1 | 62,914 | 756.2 | 83.2 | 844.2 | 162.9 | 1,677 |
| | 32 | 62,914 | 335.1 | 50.5 | 387.5 | 160.9 | 968 |
| | 63 (last) | 62,914 | 100.0 | 42.0 | 144.0 | 142.4 | 661 |
| Four times K_max, resident | 1 | 251,656 | 2,938.9 | 160.7 | 3,104.8 | 248.3 | 4,220 |
| | 127 | 251,656 | 2,460.5 | 104.7 | 2,567.6 | 203.9 | 3,463 |
| | 252 (last) | 251,656 | 1,451.1 | 72.3 | 1,525.6 | 110.0 | 2,032 |

- **Keys and plan held the loop up to 840 µs at K_max** and 3.1 ms at four times it: past 100 µs on every page.
- **The keys pass was 55.3% of the pages' time on the shard at K_max**, 88.2% at four times it, and 55.9% and 72.7%
  after a restart.
- **A page's pass shrinks along its tablet** without any saving: it looks at every key, about 1.6 ns each, but keeps
  and selects only those past its start, about 10 ns each, so the last page costs a seventh of the first.
- **A restart left the tablet resident**: at K_max every partition was read from memory again after it, and at four
  times it 39% of each page's.

The sorted partition of 100,000 rows:

| Partition | Pages | Keys and plan, µs, the worst | Its later pages against its first |
| --- | --- | --- | --- |
| Resident | 14 | 3.2 | 0.52 times |
| After a restart, in bounded records | 6 | 3.0 | 1.04 times |

A cut partition's later pages cost what its first did: its position is trusted by its archive's stamp and found
by its hint.

## Against the predictions

| Prediction | Held? |
| --- | --- |
| 1. K_max between 5,000 and 10,000 | **Failed**: 62,914. A partition of a key alone is charged 25 bytes by the shard's count, not 150 to 300 |
| 2. 3 to 6 ns a key looked at, under 100 µs at K_max | **Failed**: 1.6 ns a key looked at and about 10 a key kept and selected, 840 µs at K_max |
| 3. The pass more than 10% of a page's time at K_max | **Held**: 55% |
| 4. The pass the same on a tablet's first page as its last | **Failed**: a seventh, since a page keeps only the keys past its start |
| 5. An archived tablet's pages look at no key | **Failed**: a restart left the tablet resident |
| 6. A cut partition's later pages about its first, within 1.2 times | **Held**: 0.52 and 1.04 |

## After: F150

The same timing on the build with [F150](../../features/saved-positions.md):

| Tablet | Page | Keys looked at | Keys, µs | Plan, µs | Capture, µs | Encodes, µs | Client's wait, µs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| K_max, resident | 1 | 62,914 | 790.1 | 133.1 | 929.0 | 205.6 | 1,813 |
| | 2 | 1,000 | 238.8 | 125.6 | 366.4 | 175.8 | 947 |
| | 32 | 1,000 | 6.7 | 99.4 | 107.8 | 165.4 | 657 |
| | 63 (last) | 914 | 0.3 | 77.4 | 79.2 | 153.5 | 585 |
| Four times K_max, resident | 1 | 251,656 | 3,084.5 | 189.1 | 3,281.5 | 250.2 | 4,396 |
| | 2 | 1,000 | 964.3 | 163.6 | 1,130.9 | 220.3 | 1,863 |
| | 127 | 1,000 | 7.0 | 125.5 | 134.2 | 209.6 | 984 |
| | 252 (last) | 656 | 0.5 | 88.1 | 90.3 | 113.0 | 537 |

- **Every page after the second looks at its own keys alone**, its keys found in about 7 µs where they took 100 to
  2,900 µs, and the pass is 7.8% of the pages' time on the shard at K_max and 6.6% at four times it.
- **A page's wait at the client** in the middle of a tablet is 657 µs at K_max, from 968, and 984 at four times it,
  from 3,463.
- **The first page is as dear as it was**, and the second cuts the rest into buckets, 239 µs at K_max and 964 at four
  times it: both still hold the loop past 100 µs, which F150's limitations say.
- A cut partition's pages are as they were.

## What it says for the design

- **A tablet's depth costs every page of it**, since a planned page cannot find its smallest keys without looking at
  all of them. Saving what one page found answers every later page of the tablet.
- **The position is a cache of keys, never the scan's state**: a page that finds none, or one made at an older stamp,
  passes over its tablet as before, and nothing a client holds names it.
- **What is left is the first pass**, O(K) once a tablet, which a step-at-a-time pass or a sorted index a tablet would
  answer; neither is asked for by anything measured on the lab's tables, whose tablets are one page.

## Limitations

- **In process on europa**, one shard, rows of a key alone: the worst row for the pass. A table of wider rows holds
  fewer partitions a tablet at the same memory.
- **K_max from the shard's count of its rows**, which leaves out the process's own memory a partition: an upper bound
  on the depth, not the depth a node reaches.

## Related

- [X40 on the spikes page](spikes.md#x40-saved-positions-between-pages-priced), and
  [W5](tablet-reader.md#saved-positions-optional).
- [F150](../../features/saved-positions.md), what was built.
- [The *After M24* box](whats-left-todo.md#finishing-after-m24).
