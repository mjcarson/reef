# 374. Scan pages each read a large partition and waited for room for its copy, shed with their reads thrown away

## Symptom

A listing of a bucket of 16 MiB inline objects through one member, eight streams, hardly moved. On X19's side
cluster (2026-11-07, titan, a node of five shards and so a scan budget of 40 MiB), it listed one object of 32 in
120 s. Its pages were shed `Shedding` 73,210 times while titan's NVMe read 7.46 GB, about 440 records of
16 MiB, for the one object handed out. Every bucket of objects up to 4 MiB listed whole, at the device's pace.

## Cause

A scan's page draws an archived partition's bytes from the budget twice, in two steps:

1. its wave draws the record's read, before the read is issued ([F125](../../features/page-reads-in-runs.md));
2. its merge draws the copy it hands out, as it folds the read ([Resolved #370](scan-partition-past-its-budget.md)).

Between the two, the page holds its read and waits up to 250 ms for room for the copy. With several pages
over large partitions at once, each drew and read its record, and each then waited for room the others' reads
held. A page whose wait ran out was shed, its read given back unused, and its stream asked again at once, so a
new page took the room and read again. The budget was never short of one partition's whole need, two records'
worth: it was spread over several pages' first halves.

Two pages in lockstep get out of it, since the first wait to run out frees its room for the other's. Eight
streams asking again on their own did not.

## Evidence

**Established by reproducing it**, twice:

- **On the lab**, as the symptom says: `x19 list` of the 16 MiB bucket on titan, paced, eight streams, each of
  X19's four rounds at e3c7d639 and 512f2ece, the listing cut at 120 s. Europa's Optane listed the same bucket
  in 34.6 s, reading 113 MB an object of 16.8 MB.
- **In process**, by `a_page_holds_no_read_while_it_waits_for_its_copy` (`shoal/tests/scan_edges.rs`). It holds
  the first of two pages' reads once its wave is drawn (`faults::hold_page_reads`), asks the second, and counts
  the reads held. On the tree before the fix:

```text
test a_page_holds_no_read_while_it_waits_for_its_copy ... FAILED
assertion `left == right` failed: the second page read its partition while the first held its own
  left: 2
 right: 1
```

A first test that asked two pages at once in rounds passed both before and after the change, since the rounds
kept the pages in step, so it was not kept.

## The fix

**A wave draws its partitions' merge copies with their reads, in one draw** (`PageDraws::cover_ahead`, from
`plan_reads`). The copies are left drawn and not yet held, so each merge's cover finds them there. A page then
holds the whole of its partitions' need or nothing past its admission: one that finds no room waits, and is
shed, before it reads anything.

**On the lab after the fix** (2026-11-07): X19's whole listing leg again on the fix, at `fc156277`, on a side
cluster bootstrapped and filled for it, four rounds, the bucket of 32 inline objects of 16 MiB listed through
one member by eight streams. Medians of the four rounds, the range in brackets:

| Member and pace | Before: listed | Before: read an object | After: listed | After: read an object |
| --- | --- | --- | --- | --- |
| titan, paced | 1 of 32 in 120 s | 7,459 MB | 32 in 8.6 s [8.4, 8.7] | 16.8 MB |
| titan, unpaced | 32 in 44.5 s [39.9, 45.5] | 172 MB | 32 in 0.7 s [0.7, 0.8] | 16.8 MB |
| europa, paced | 32 in 34.7 s [33.0, 38.8] | 114 MB | 32 in 5.0 s [5.0, 5.1] | 16.8 MB |
| europa, unpaced | 32 in 41.1 s [37.7, 53.2] | 168 MB | 32 in 0.3 s | 16.8 MB |

Every object is read once, its own size. Every other bucket, 1 KiB to 4 MiB, listed at a rate within its rounds'
range of the run before the fix, none disjoint, so drawing the copies ahead costs a page of small partitions
nothing measurable.

## Alternatives rejected

- **A budget whose waiters are served in turn**, as a device's scan takes are since item 355. That orders the
  draws but keeps the two halves, so the first page in line could still hold its read while the next took the
  rest.
- **Back off a shed stream at random**, in the drivers. That spreads the collisions out and keeps them, and every
  driver would have to know it.
- **Fewer streams for large partitions.** No driver knows a partition's size before it asks.

## Invariants to uphold

- **A page never holds part of a partition's need while it waits for the rest**: a wave draws its reads and
  ~~their copies~~ the pieces its folds will make together. Since [F134](../../features/walked-records-uncopied.md)
  a record is handed out where its read put it, so a partition's need is its read alone unless fragments lie
  over it, whose fold makes pieces of its own: only those are drawn ahead. A read that followed a moved
  partition onto fragments its view did not name draws its fold's pieces at the merge, the one place a page may
  wait while it holds a read.
- **A merge's cover of a partition its wave drew finds the room already drawn**, and draws nothing.
- **A partition past the whole budget is still refused by name at once** (item 370): the wave's single draw is
  judged whole.

## Still open

- ~~**A page that waits for room is still shed, and its stream asks again.**~~ It reads nothing before it is
  shed now, but on the lab a listing of 32 objects of 16 MiB was shed 1,870 to 5,270 times, titan's budget of
  40 MiB holding one such partition's read and copy at a time. A budget whose waiters are served in turn, as a
  device's scan takes are since item 355, would cut the round trips; M24's floor of the largest row and O162's
  single copy let two such rows in at once, ~~O162~~ built as [F134](../../features/walked-records-uncopied.md).
  Both the rest are built since [F135](../../features/scan-budget-floor.md) (2026-11-08): the scan budget serves
  its waiting draws in turn, its head waiting past 250 ms for its room, and every node keeps a budget of at
  least the largest row it admits. Only a page behind the head is still shed at its 250 ms, and asks again at
  the back. The listing's sheds were not counted again on the lab.
- A page's admission still draws its byte bound first, so with many pages admitted at once a partition needing
  nearly the whole budget may wait until most of them are done.
- **A wave draws a copy for every partition it reads, whether or not its merge copies it.** A page whose filter
  keeps nothing copies nothing. X19's E4, run again on this fix, drew twice its work bound: 8.1 MiB at a 4 MiB
  bound, at no cost in time. At 16 MiB the eight pages in flight outgrew a four-shard node's 32 MiB and waited
  for room, a page's p99 rising from 80 to 299 ms unpaced
  ([the record](../../future-work/scans/the-edges.md#e4-a-filter-that-keeps-nothing)). The same draw refuses an
  archived partition whose read and copy pass the budget before it is read, where it was read first (a 17 MiB
  one, 178 ms). [O162](../optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it),
  required before M24, removes the copy, and with it this half of the draw. ~~Open~~ Done (2026-11-07,
  [F134](../../features/walked-records-uncopied.md)): a wave draws the heap buffers its reads fill and the
  pieces a fold over fragments makes, and nothing for a record handed out as it lies; E4 run again on it is on
  F134's page.

## Tests

| Test | Where | Breaks if |
| --- | --- | --- |
| `a_page_holds_no_read_while_it_waits_for_its_copy` | `shoal/tests/scan_edges.rs` | A page reads its partition while another holds the room its copy needs, or is refused where it should wait; since F134 its partitions have a fragment over each, whose fold makes the pieces it waits for room for |
| `a_scan_page_keeps_its_reads_in_flight` | `shoal-core/src/server/tables/walk/reads.rs` | ~~A wave's reads are drawn without their partitions' copies, or the copies are drawn twice~~ A wave's reads are not drawn as the buffers they fill before they are issued, or a copy is drawn for a record handed out as it lies (F134) |

## Related

[Resolved #370](scan-partition-past-its-budget.md), the draws' whole need judged; [F118](../../features/scan-budget.md),
the scan budget; [X19](../../future-work/scans/spikes.md#x19-the-edges), whose listing found it.
