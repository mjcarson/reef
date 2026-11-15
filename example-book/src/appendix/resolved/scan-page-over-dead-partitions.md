# 371. A scan's page over partitions with no live row drew for each until it was shed, and never moved

## Symptom

Once every row of a tablet's partitions had been deleted, a scan of the tablet never finished: its
page was shed, "the scan budget is spent", at every ask, from the same cursor, for as long as the scan
asked. A sorted table keeps such a partition in its archive as its record and a fragment of tombstones
over it ([F61](../../features/fragmented-partitions.md)), so it is read like any other. Six hundred
partitions of eight kibibyte rows, every row deleted, about 5 MiB of records, were enough under a
budget of 2 MiB.

The same page, under a budget large enough to hold it, read every dead record of the tablet in one
page however many there were. The page's pace never ended it, and its waiter saw no progress, so it
could give the page up after eleven seconds and ask it again from the same place
([item 355](scan-pages-served-in-turn.md)).

## Cause

A partition with no live row hands out nothing, and every bound of a page was counted from what it
handed out:

- **No bound counted it.** The page ends at its partitions' count or their bytes, both counted only
  as partitions are pushed (`PendingWalk::finish`, `shoal-core/src/server/tables/walk.rs`). A wave of
  the page's reads plans from the same counts (`plan_reads`, `walk/reads.rs`, from `page.partitions`
  and `page.bytes`). So a page over dead partitions planned wave after wave, each up to its bound.
- **Its draws were never given back.** Each partition was drawn at its wave and again at the merge,
  for the copy it would hand out ([F125](../../features/page-reads-in-runs.md)). A dead partition hands
  out no copy, but its merge's draw stayed counted as held until the page ended. The page's holding
  grew by twice every dead record it read until a draw found no room.
- **Nothing it covered moved it.** A page that is shed ends short at the last partition it handed out
  (`short`), its device's pace ends it there too (`paced_out`), and its waiter's progress moves as a
  partition is handed out. Holding none, the page was refused `Shedding` and asked again from the cursor
  it began at, which read the same dead records first.

## Evidence

**Established by reproducing it.** `a_page_over_dead_partitions_moves_its_cursor_within_its_draw`
(`shoal/tests/scan_edges.rs`) writes six hundred sorted partitions into one tablet of a standalone node
with a scan budget of 2 MiB, starts it again so they are archived, deletes every row, and starts it
twice more so the tombstones are archived over them and nothing of them is resident. A driver's walk
then reads 1,200 records, a base and a fragment each, and hands out nothing. On the tree before the fix
(`f631775c`, 2026-11-07), the scan of the tablet:

```text
test a_page_over_dead_partitions_moves_its_cursor_within_its_draw ... FAILED
Error: Io(Custom { kind: Other, error: "the scan of the dead tablet ended WalkRefusal { code:
Shedding, msg: \"the scan budget is spent: 9136 bytes found no room in their wait\", applied: None,
shed: Some(Budget) } after 31 sheds" })
```

The 9,136 bytes are one dead partition's record and fragment: the budget was full of the page's own
draws for the ones before it. Thirty-one sheds is the test's ten seconds of patience.

## The fix

- **A key the merge covers moves the page.** Each one is recorded (`WalkRows::covered`), handed out or
  not, and marks the waiter's progress. A page that is shed or paced out ends short at the last key it
  covered, so its next page starts past the dead partitions it read.
- **What a dead partition read counts toward the page's byte bound** (`WalkRows::passed`), at the
  merge and when a wave is planned. A page over dead partitions ends at its bound, as a page of live
  ones does, its cursor at the last key it read.
- **The merge's draw for a partition that comes to nothing is given back at once.** Its wave's read
  stays held until the page ends, as a live one's does.

A page may now end holding no partition, with its cursor past keys it read and did not hand out. That
is [W4](../../future-work/scans/cursor-and-pages.md#bounds-and-short-pages)'s rule for a short page,
and every driver already moves its cursor to the page's `next`. The TMDB loader's walk check
(`examples/tmdb_dataset/src/walk.rs`) and `tablet_walk.rs`'s page check held a page's cursor to be its
last handed-out key, and now hold it to be in the tablet, at or past it, and past the walk's last
cursor.

## Alternatives rejected

- **Give the draws back and nothing else.** That bounds a page's memory and not its work: a page read
  every dead record of its tablet whatever there were. It still could not be paced out or ended short,
  having handed nothing out, and its waiter could still give it up and ask the same reads again.
- **Skip a dead partition by the archive map's counters.** `TabletUsage` counts a partition whose chain
  folds to nothing, since the chain is in the index: only reading the chain says it is dead.
- **Build M24's work bound now.** [W4](../../future-work/scans/cursor-and-pages.md#bounds-and-short-pages)'s
  bound counts every byte a page reads, filtered or not, against a bound of its own. The fix counts only
  what a page reads and cannot hand out, against the bound it already has, which is the walk's own rule
  that a page's work is its bytes. M24's bound, with a filter, is M24's.

## Invariants to uphold

- **Every key a page's merge covers moves the page**: its cursor goes no less far, and its waiter sees
  progress.
- **A page that covered a key is never refused for its budget or its pace**: it ends short at the last
  key it covered.
- **A page's byte bound counts what it read and could not hand out** beside what it handed out, so no
  page reads an unbounded run of dead records. F133 lost this for an archived partition, whose fold
  steps on its own, and [Resolved #376](dead-archived-bytes-counted.md) restored it: the bytes a
  partition's records hold are counted apart from the work an arm steps.
- **A draw for a copy that is never made is given back as the merge learns it**, so a page never holds
  for nothing.
- **A page's `next` may be a key it did not hand out**, at or past its last partition's. Nothing reads
  the cursor as the last partition's key.

## Still open

- **What a scan of an empty or emptied table costs** is [X19](../../future-work/scans/spikes.md#x19-the-edges)'s
  to measure: a tablet the counters call held for its dead chains is read, and one that holds nothing
  costs a page.
- **A start leaves the dead partitions its compaction of the last log rewrote in memory** until a log
  after it rotates (`mark_evictable`). That holds no live row and is bounded by one log, so it is not a
  defect, and the test starts twice to step past it.

## Tests

| Test | Where | Breaks if |
| --- | --- | --- |
| `a_page_over_dead_partitions_moves_its_cursor_within_its_draw` | `shoal/tests/scan_edges.rs` | A dead partition counts toward no bound again, its draw is kept, or a page that covered only dead keys is refused rather than ended short |
| `a_page_check_names_every_broken_rule` | `examples/tmdb_dataset/src/walk.rs` | The loader's check refuses a page whose cursor moved past dead partitions, or accepts one whose cursor did not move |
| `resident_changes_shadow_their_archives` and the other walks of `tablet_walk.rs` | `shoal/tests/tablet_walk.rs` | A page's cursor leaves its tablet, goes behind its last partition, or stops moving |

## Related

[F118](../../features/scan-budget.md), the scan budget; [F125](../../features/page-reads-in-runs.md),
the waves; [F128](../../features/device-time-pace.md) and [item 355](scan-pages-served-in-turn.md),
the pace and the waiter; [Resolved #370](scan-partition-past-its-budget.md), the other edge of the same
draws; [F61](../../features/fragmented-partitions.md), the chains a sorted table keeps its deletes in;
[Resolved #376](dead-archived-bytes-counted.md), which restored this page's byte bound for an archived
partition after F133.
