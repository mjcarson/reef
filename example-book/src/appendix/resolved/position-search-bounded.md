# 385. A page looking for its position read a whole partition outside its work

## Symptom

A page that goes on inside a sorted partition carries a position: the last sort key handed out, and
a hint of which archived record comes next. When the hint could not be trusted, the page read the
partition's records from the first, passing over each that lay wholly before the position, and none
of that counted. A page asked to read 64 KiB could read a partition of any size end to end before it
handed out a row, with no bound of its work and no check of its deadline, holding a place in its
shard's scan class the while.

The hint is untrusted after a merge rewrote the partition, which is the honest case, and whenever a
client sends a position whose stamp is not the archive's, which any principal holding `scan` may do
on every page.

## Cause

`split_archived` (`shoal-core/src/server/tables/walk.rs`) trusts a hint only when its stamp is the
chain's ([F142](../../features/split-partitions.md)). Otherwise it starts at the first record and
passes over those at or before the position. The bytes of the records passed over went into no
count: not the window, not `page.passed`, not the answer's work. The loop's only bound was the
window's bytes, which a search does not add to.

## Evidence

**Found by reading the source**, by both independent readers of M24's exit review (2026-11-12), **and
reproduced.** `a_search_for_a_position_is_held_to_its_pages_work` in `shoal/tests/scan_pages.rs`
takes a scan some six records into a partition of ten, changes its position's stamp, and goes on in
pages of 64 KiB, a record each. Against the tree before the fix the first page passed over every
record before the position and handed out rows at once:

```text
thread 'a_search_for_a_position_is_held_to_its_pages_work' panicked at shoal/tests/scan_pages.rs:931:5:
a search past some six records in pages of one took 0 pages
```

## The fix

The search is the page's work. Records passed over are summed, and once they reach what the page has
left to read, with records still ahead, the page ends there: it hands out nothing, counts what it
passed over as work, and answers with a position that keeps the sort key and names where the search
got to, under the archive's own stamp. The next page looks on from that record.

A position's hint has a bit for this, `Position::SEARCHING`. Without it a trusted hint is the record
after the position, read without a look, as before. With it the hint is only where to look on from,
and each record is still held to the sort key. A partition is a few hundred records at most, so no
count reaches the bit; a build that does not know it reads the hint as past every record and trusts
nothing, which is the search from the first record it always made.

## Alternatives rejected

**A binary search of the records by their edges.** Fewer reads, but each probe is a record read and
validated out of order, against the pace's grain, and a page's bound would still be a count of
probes; a linear search bounded by the page's work reads each record once and in order.

**Checking every resumed page's first record against the sort key.** It would let any hint be where
to look on from, at one more validation of a record on every page of every split partition.

**Refusing a position whose stamp is not the archive's.** A merge between two pages would end the
scan.

## Invariants to uphold

- **A page reads no more than its work and a record past it, whatever it is looking for.**
- **A hint with `SEARCHING` set is never the record after the position**: every record from it is
  held to the sort key until one lies past it.
- **A position's sort key is the last row handed out**, and a search never moves it.
- **A page that searched and handed out nothing still answers a position that moved**, so its scan
  moves.

## Still open

A partition in memory in shared pieces (`split_shared`) is searched the same way with no bound. It
reads no device and its pieces are already resident; it is left as it is.
[O173](../optimizations.md#o173-a-record-is-validated-up-to-three-times-on-a-page-that-cuts-it): a
record passed over is validated to find its last key.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_search_for_a_position_is_held_to_its_pages_work` (`shoal/tests/scan_pages.rs`) | The first page after the stamp moved passes over six records and hands out rows, where it may read one |
| `a_split_partition_holds_through_writes_and_a_restart` (`shoal/tests/scan_pages.rs`) | Were a searching hint taken as the record after the position, rows before it would be handed out twice |

## Related

- [F142](../../features/split-partitions.md), whose position this is.
- [O172](../optimizations.md#o172-a-partition-handed-out-part-way-under-an-overlay-or-fragments-reads-more-than-its-page):
  the other way a split partition reads more than its page.
