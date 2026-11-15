# F141. A scan's page over a shard's tablets, and its bounds

A walk's page read one tablet. A scan of a table asked each of its 4,096 tablets for a page of its
own, one round trip a tablet, each waiting its turn in the loop's second inbox, whether the tablet
held a thousand rows or none. A page ended at a count of partitions or at its bytes, and nothing else.

Now a scan's page names the tablets of a table still to be read, and the executor hosting the lowest
of them reads its own in ascending order until a bound is met: the rows the page may hand out, the
bytes its answer may reach, the bytes it may read, its deadline, or the end of its tablets. A tablet
another executor hosts is refused by name, and a tablet that holds nothing costs nothing but a count
read on the loop. How far the page got is one key. This is the engine's half of
[M24](../future-work/scans/milestones.md#m24-scans-on-one-node)'s page, proven in process; nothing
sends one over a wire yet.

## Context

[W4](../future-work/scans/cursor-and-pages.md#what-one-page-is) fixes what one page is: one request
and one answer, to one shard, over some of that shard's tablets. M23 rebuilt the walk of one tablet for
load: planned on the loop, encoded in steps, read in waves, paced by its device, drawn from the scan
budget ([F124](scan-page-planned.md) to [F137](encode-steps-together.md)). M24's second step puts a
page of several tablets over that walk and changes none of it.

[X19](../future-work/scans/the-edges.md) measured why the page has to skip empty tablets itself: 4,096
tablets at a page each cost a table never written about 12.5 ms of four shards, and a sparse table's
empty tablets read 14.1 MiB of index pages a scan.

M24's plan cut this step in two while it was built, and the diagram follows
([Finishing M24](../future-work/scans/whats-left-todo.md#finishing-m24)): this feature is the page and
its bounds, and the split of a sorted partition too large for a page is the step after it. Until
then such a partition is handed out whole, as the walk hands it out.

## What it does

### One page

`ScanAsk` is what a page asks: a table, a set of tablets, a cursor for each tablet the scan left part
way, and its bounds (`shoal-core/src/server/shard/scan.rs`). `ScanAsk::of_page` makes one from a
`ScanPage` frame. A page goes to any executor, which hands it to the one hosting the lowest of its
tablets the node holds a copy of, once; that executor reads the tablets it hosts and refuses every
other `HostedElsewhere`. Nothing is forwarded twice, and no tablet is read by an executor that does
not host it.

The page is admitted on its shard's loop and read by one task in the scan queue. The task asks its
own loop for each tablet in turn, as a walk's page of that tablet alone, a scan's: from the tablet's
cursor or its start, bounded by the rows and bytes the page has left. So each tablet is captured from
the loop's second inbox, encoded in steps and read in waves exactly as a walk's page of it is.

### What it answers

`ScanRead` holds the page's partitions in ascending key order across its tablets, why it ended
(`ScanEnded`), the tablets it refused, what it read and where from, and `reached`, the last key it
covered. A tablet read to its end is covered to its last key, and a tablet that holds nothing with
it, so `reached` alone says which tablets are finished: every tablet below its tablet that the page
did not refuse, and its own when `reached` is its last key. `ScanRead::answer` is the page as a
`ScanAnswer`'s head, and `ScanRead::partition_body` its partitions as a body
([F132](whole-partitions-in-answers.md)).

### Bounds

| Bound | Default | The page ends | `ScanEnded` |
| --- | --- | --- | --- |
| Rows | 1,000 | Once it has handed out that many | `Rows` |
| Answer bytes | 1 MiB | Once its partitions' bytes reach them | `AnswerBytes` |
| Work | 4 MiB | Once the bytes it read, handed out or not, reach them | `Work` |
| Deadline | 5 s, 30 s at most | Before the next tablet, once its time on the shard is up | `Deadline` |
| Its tablets | | Once every tablet it hosts is read | `Tablets` |
| The scan budget | | Where it stands, when a tablet's draw finds no room | `Budget` |

A bound is met, never cut to: a page ends once it has passed one, by a partition at most. A page a
tablet's page left part way for no bound of its own, because its device held it back a second
([F128](device-time-pace.md)), ends `Deadline` too.

### Empty tablets

A table says whether it holds anything of a tablet from counts it already keeps: its map of the
tablet's resident partitions, and its archive map's count of the tablet's partitions
(`tablet_holds`, through `ShoalDatabase::tablet_holds`). A page asks that of each tablet it hosts
once, on the loop at its admission, and reads a page only of those that hold something. A scan of an
empty table is one page an executor.

### Admission and the budget

A page takes one of `scans.pages_in_flight` at its admission and gives it back when it is answered.
Its tablets' pages are marked as its own (`WalkCall::paged`), admitted with it and counted in its
place, so a page reading its hundredth tablet is still one page of its class.

Each tablet's page is admitted with a draw of the bytes the page has left, as a walk's is. Once the
tablet is read, what its draws hold past what it came to hold goes back (`Permit::shrink_to`), so a
page over hundreds of small tablets holds about what its partitions do. A page's first draw may come
with it, drawn in its turn before it reached a loop; every later one waits a quarter second for room
in the scan queue, never on the loop.

## Design choices

**The page is a task over the walk, not a second reader.** Everything M23 built for a scan's page of
one tablet holds for each tablet of a page of several, because it is the same code asked the same
way. What a page of several tablets adds is the choice of tablets, the bounds between them and the
bookkeeping of one admission.

**Admitted once.** Counted as its tablets' pages, a page would give its place up between two tablets,
and another page would take it: both would then be shed half read. One count for the page, held from
its admission to its answer, is what `scans.pages_in_flight` means.

**Empty tablets are decided on the loop, once.** The counts are the table's own, read with nothing
awaited, for all of a page's tablets in one step at its admission. A partition written into a tablet
after that is one made while the scan runs, which a scan may or may not hand out
([P23](../future-work/scans/contract.md#the-contract)).

**The count errs toward holding.** A tablet whose every partition is dead still counts them until a
merge rewrites them, and is read. Reading past dead partitions is slow and right; skipping a tablet
that held a row would lose it.

**A page that covered a key always answers.** A tablet's page shed, failed or given no room after
that ends the page where it stands, and the next page meets the failure first. Progress already made
is never thrown away for a refusal that names nothing of it. Only a page that covered nothing is
refused, by the refusal it met.

**A tablet whose copy cannot be read is refused alone.** `StaleTopology`, `Unavailable` and
`Quarantined` name a tablet in the answer, and the page goes on to the next, as W4 has it.

**Bounds are soft by a partition.** The walk hands a partition out whole, so a page passes its rows or
its bytes by the last one. The step after this one cuts a sorted partition too large for any page.

## Alternatives rejected

**One capture over a whole range of tablets.** A page's capture on the loop is bounded by its
tablet's resident keys; a capture over many tablets would hold the loop for all of them at once, which
is what [item 353](../appendix/resolved/capture-allocations.md) spent three rounds removing.

**Forwarding a tablet to its executor.** A page that read tablets on two executors would have two
positions, and its answer no single `reached`. A refusal the client routes by keeps a page one copy's
and one executor's.

**A second budget for a page's answer.** The scan budget already bounds what a page holds while it
reads. Giving the excess back after each tablet keeps one budget honest instead of adding another.

## Limitations

- ~~**A sorted partition larger than a page is handed out whole**, past the page's answer bytes, until
  M24's next step splits it across pages.~~ Split since [F142](split-partitions.md), which also ends a
  page before a partition that does not fit what it has left.
- **Rows are a soft bound.** A page ends once it has handed out its rows, having passed them by its
  last partition's.
- **The deadline is judged between tablets.** A tablet's page under way is finished first, and a
  paced one ends itself after a second of waiting for its device.
- **A page reads one tablet at a time.** Its reads within a tablet are in flight together
  ([F125](page-reads-in-runs.md)); tablets are not.
- **Each tablet read is a capture step of the loop's second inbox**, as a walk's page of it is, so a
  table of many small tablets takes a step a tablet.
- **A tablet emptied by deletes is read until a merge rewrites it**, since its counts still hold its
  dead partitions.
- **No page reads a cluster's tablets yet.** A cluster node's tablets are refused or read as its map
  says, with no floors: floors and groups are M25's.
- **Nothing a client sends reaches this.** The frames' server half is M24's later step.

## Invariants to uphold

- **A page is one page of its class from its admission to its answer**, and its tablets' pages are
  counted in its place (`paged`), never beside it.
- **`Walks::pages` goes down exactly once a page**, when its task's `Scanned` is handled.
- **Tablets are read in ascending order, and `reached` is the last key covered.** A tablet read to
  its end, or holding nothing, is covered to its last key.
- **No executor reads a tablet it does not host.**
- **`tablet_holds` never says a tablet holds nothing while it holds a live row.** It reads counts
  kept under every change, never the index.
- **A page that covered a key answers with it**, whatever stops it after.
- **Nothing is awaited on the loop for a page**: its draws and its tablets' answers are awaited in
  its task.
- **A tablet's draws are shrunk to what its page holds before the next tablet draws.**

## Performance

Nothing on a query's path changed: the loop's dispatch gained two arms, and a class's count of pages
one addition. A page's cost a tablet that holds something is a walk's page of it. What a scan saves is
between the tablets: a scan of an empty table of 4,096 tablets on a node of two executors is two pages
that read nothing, where it was 4,096 pages, each a turn of the second inbox. The gate's lab exit
measured a scan's rate and its cost beside the foreground
([M24's exit](../future-work/scans/m24-exit.md), 2026-11-12).

## Tests

| Test | Where | Breaks if |
| --- | --- | --- |
| `a_page_reads_its_shards_tablets_in_order_until_a_bound` | `shoal`, `scan_pages.rs` | A page reads one tablet, or another executor's, or passes its rows or bytes by more than a row, or a scan of pages hands a row out twice or not at all |
| `a_page_covers_its_tablets_as_one_ascending_position` | the same | A key at or below `reached` in a tablet the page read is left behind, or a tablet is finished that the page did not cover |
| `a_scan_hands_out_each_tablets_partitions_in_ascending_key` | the same | A tablet's partitions come out of order or twice, resident or archived, unsorted or sorted |
| `an_empty_table_is_scanned_in_one_page_a_shard` | the same | An empty tablet is read, draws from the scan budget, or takes a page of its own |
| `a_page_ends_at_its_deadline_with_what_it_covered` | the same | A page past its deadline reads on, or throws away the tablet it had read |
| `a_scan_page_is_one_page_of_its_class` | the same | A page is counted by its tablets, or takes a driver's admission, or keeps its place once answered |
| `a_page_keeps_no_draw_past_what_it_holds` | the same | A tablet's draw is kept past what its page holds, so a page of many tablets is shed by its own draws |
| `a_permit_shrunk_gives_back_what_it_no_longer_holds` | `shoal-core`, `budget.rs` | A shrunk permit keeps its bytes, or gives them back twice |

## Related

- [W4, the cursor and the pages](../future-work/scans/cursor-and-pages.md#what-one-page-is)
- [F140](scan-frames.md), the frames a page is asked and answered by
- [F87](tablet-walk.md) and [F129](drivers-on-the-rebuilt-walk.md), the walk a page reads each tablet by
- [F116](scan-admission.md), [F117](scan-class.md) and [F118](scan-budget.md), the class, its second
  inbox and its budget
- [F132](whole-partitions-in-answers.md), the body a page's partitions are written as
