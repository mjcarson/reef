# F142. A sorted partition too large for a page, split across pages

A walk handed a partition out whole. A sorted partition larger than a page went out in one page all
the same, past the page's bytes by everything it held: read whole, drawn whole from the scan budget,
and refused by its key once the budget could not hold it beside its read. A scan of a table with one
partition of a gibibyte had no page that could carry it.

Now a scan's page hands such a partition out part way. The page stops after a record of it, or after
a piece of it in memory, once it holds what it has left; its answer says how far into the partition it
got; and the next page goes on from that row, reading nothing before it. No partition is refused for
its size, and each of its rows is handed out once.

## Context

[X19](../future-work/scans/the-edges.md#what-it-says-for-the-design) decided this with the user on
2026-11-07: performance first, and no partition may fail or go missing from a scan, so M24 splits a
sorted partition too large for a page and refuses none for its size. It landed three things first,
each of which this stands on: records of bounded size ([F133](bounded-records.md)), so a large
partition's archive is a base and the pieces its rows continue in, each at most a mebibyte; a walked
record handed out where its read put it ([F134](walked-records-uncopied.md)); and a scan budget that
holds any row ([F135](scan-budget-floor.md)).

X19 also priced the way not to do it: a split that read the whole partition for every page would read
a 64 MiB partition about 64 times in pages of a mebibyte, some 4 GiB.

[Q36](../future-work/scans/contract.md#q36-the-cursor-and-the-frames-2026-11-06) had reserved the place:
a sort-key position beside each cursor and in each answer, which [F140](scan-frames.md) built filled.
[P20](../future-work/scans/contract.md#the-contract) is restated for it: each row of a split partition
live at its group's floor and still live when the scan reaches it is handed out exactly once.

This is the second half of the step M24's plan first drew as one
([Finishing M24](../future-work/scans/whats-left-todo.md#finishing-m24)); the first half is the page
over a shard's tablets ([F141](scan-pages-over-tablets.md)).

## What it does

### A page judges a partition before it reads it

A scan's page over several tablets may cut (`WalkBounds::cut`), and it judges each partition by what
is known of it before anything of it is read or encoded: an archived one by its records' bytes, a
resident one by its table's count of it (`PendingWalk::judge`).

- **It fits what the page has left**: handed out whole, as every partition was.
- **It does not, and the page has covered a key**: the page ends before it (`WalkRows::full`), and the
  next page starts with it. A page that may cut reads no such partition ahead in a wave, so nothing is
  read to be thrown away.
- **It fits no page at all**, a sorted partition in memory or archived in more than one record: it is
  handed out part way, from its first row.
- **The page goes on inside it** (`WalkBounds::resume`): handed out part way, from the position.

An unsorted partition is one row and is never cut. Nor is a sorted partition archived as one record: a
page passes its bytes by a record at most.

### Three ways a partition is handed out part way

**From its archive** (`split_archived`). The page reads the partition's records one at a time, in sort
order, until they hold what it has left, and hands each out where its read put it. Each record is
drawn from the scan budget before it is read and paced by its device as a read alone is. The page then
stops after the last record it took, and its position names that record's last sort key.

**From memory, held as its archive's records** (`split_shared`). The same, with no read: a window of
the records, each copied out as it lies.

**From memory, loaded** (`split_resident`). The partition is encoded in steps of the loop's second
inbox as it always was ([F126](encodes-in-steps.md)), from the position's sort key, and the steps stop
once their pieces hold what the page has left. The page is cut after the last piece's last row.

Where fragments lie over the records, or the position lies inside a record, the window is folded a
step at a time into pieces of the page's own, bounded on both sides: after the position, and no
further than the window's last record, so a fragment's rows past it stay the next page's.

### The position

`Position` is the last sort key handed out, in its table's archived form, with a hint beside it: how
many of the partition's records lie wholly at or before that key, and a stamp of the archive the hint
was taken from (its base, its last piece and how many pieces it has). It is at most 1,024 bytes, and a
page never stops after a row whose sort key would not fit one.

A page going on inside a partition trusts the hint only where it can: from an archive, while the
stamp is the archive's; from memory, where the record before the hint ends exactly at the position.
Then it starts at the record the hint names and reads nothing before it. Otherwise the position is
found from its sort key alone: the records are passed over from the first, each wholly at or before
it let go, and the rows before it folded out of the record it lies in. That happens once after a
merge rewrites the partition, and the page it ends with carries a hint for the new archive.
~~The search was counted nowhere, so one page could read a partition of any size to find its place~~
Since [Resolved #385](../appendix/resolved/position-search-bounded.md) the search is the page's work:
once the records passed over hold what the page has left to read, the page ends, hands out nothing,
and answers with the same sort key and a hint of where the search got to, marked as a search
(`Position::SEARCHING`, the hint's top bit), under the archive's stamp. The next page looks on from
that record, each still held to the sort key.

### What a page answers

`WalkPage::position` and `ScanRead::position` carry the position; `reached` is the partition's key,
and the page's last partition is the part handed out. `ScanRead::answer` puts the position in the
answer's head, where a client reads it into the tablet's cursor. In a body in partitions the part is
written as pieces every one of which has more after it (`PartitionBody::of_cut`), and the client's
reader accepts a body that ends inside a partition only when its answer carries a position and the
partition is the one at `reached` (`PageExpect::cut`).

## Design choices

**Cut at a record's end, or a piece's.** An archived record handed out where its read put it is the
cheapest thing a scan hands out. Cutting inside one would copy it to save at most a record's bytes of
a page.

**A position is a sort key, and the hint is advice.** A row's place in its partition's sort order is
the one thing a merge, a move and a restart all leave alone. The hint only saves reads, and a wrong or
stale one costs a search, never a row.

**The stamp is the archive's own facts.** A record is never rewritten in place, so a chain whose base
and last piece lie where they lay, in as many pieces, holds the same records in the same order. No
generation has to be kept for it.

**The page ends before a partition that does not fit.** A page that read on past its bytes by a whole
partition each time made its bytes a lower bound. Ending before it makes them an upper one, by a
record at most, which is what a client sizing a buffer wants. It is so only for a page that may cut:
a driver's and an operator's walk hand out as they did.

**One fold for every case that needs one.** [F127](folds-in-steps.md)'s fold already laid a part's
rows over an archive within bounds on both sides. A window of records under their fragments is one
such part, so the split added no second fold.

**Every way out keeps the draw honest.** A record let go is released as it is let go; a fold's pieces
are drawn before they are made and take its records' place; a draw that finds no room cuts the
partition at what was read, and only a page that read nothing of it is shed.

## Alternatives rejected

**Reading the whole partition and trimming it.** Simple, and X19's 4 GiB.

**A position that is a record's number, or a byte offset.** Either is a place in an archive. A merge
between two pages would leave it naming rows already handed out, or skipping some.

**Cutting inside a record by a count of rows.** A page's rows would then be exact for a sorted table.
It costs a copy of every record cut, and a page's rows are met, not cut to, for every other partition.

**Refusing a partition past some size.** What the walk did, and what the user ruled out.

## Limitations

- **A sorted partition archived as one record is never cut.** A page passes its bytes by one record,
  a mebibyte at most by default.
- **Rows stay a soft bound.** A page ends once it has handed out its rows; a partition is cut by
  bytes alone.
- **A partition under a resident overlay is read whole for every page of it.** New rows written to a
  large archived partition lie over it in memory until a merge, and the fold of them reads every
  record each page ([O172](../appendix/optimizations.md#o172-a-partition-handed-out-part-way-under-an-overlay-or-fragments-reads-more-than-its-page)).
  Its fragments are read every page too.
- **A position found from its sort key alone reads the records before it once**, after a merge
  rewrote the partition under its scan.
- **A row larger than a page is handed out alone and whole.** It cannot be split; the scan budget's
  floor holds it ([F135](scan-budget-floor.md)), and a resident one past half the budget is still
  refused ([item 377](../appendix/known-issues.md)).
- **Only a scan's page over several tablets cuts.** A walk's page of one tablet, a driver's or an
  operator's, hands a partition out whole as it did.

## Invariants to uphold

- **A position's sort key is the last row handed out, and the next page hands out only rows after
  it.** Every path takes its rows by that bound, whatever the hint says.
- **A hint is trusted only when it is shown to hold**: an archive's by its stamp, memory's by the
  record before it ending at the position. Never otherwise.
- **A hint marked as a search is where to look on from, never the record after the position**, and
  a page reads no more than its work and a record past it looking
  ([Resolved #385](../appendix/resolved/position-search-bounded.md)).
- **A page cut inside a partition ends there**: `next` is the partition's key and `position` is set,
  and nothing after that partition is on the page.
- **A window's fold never passes the window's last record's last key** unless the window reaches the
  partition's end, or a fragment's rows would be handed out before the records they sort among.
- **A page that may cut reads no partition ahead that it will not hand out whole.**
- **Each record is drawn before it is read, and released when it is let go.**
- **A body ends inside a partition only when its answer carries a position for the key it reached.**
- **Only a planned page cuts.** A captured page, a backup's, holds partitions whole.

## Performance

Nothing on a query's path changed. A scan's page of a table whose partitions fit a page does what it
did, less the read of a partition that would not have fit, which it no longer plans. For a partition
past a page, what is held is a count and not a time, by
`an_archived_partition_past_a_page_is_read_a_record_at_a_time`: the tests' big group, 600 rows of a
kibibyte archived as about ten records, scanned in pages of 128 KiB, reads no more than its archive's
bytes and one record, where reading the partition again for every page would read it about five
times. Nothing was timed here; the gate's lab exit measured a scan's rate
([M24's exit](../future-work/scans/m24-exit.md)), on a table of one row a partition, so no partition
was split in it.

## Tests

| Test | Where | Breaks if |
| --- | --- | --- |
| `a_resident_partition_past_a_page_is_split_across_pages` | `shoal`, `scan_pages.rs` | A resident partition past a page comes whole, or a row of it twice, out of order or not at all, or a page passes its bytes by more than a piece |
| `an_archived_partition_past_a_page_is_read_a_record_at_a_time` | the same | A page going on inside a partition reads a record before where the last stopped, or a record is read twice, or copied where it could be handed out as it lies |
| `a_search_for_a_position_is_held_to_its_pages_work` | the same | A page whose position's hint is another archive's passes over every record before it in one page, outside its work ([Resolved #385](../appendix/resolved/position-search-bounded.md)) |
| `a_split_partition_holds_through_writes_and_a_restart` | the same | Rows written or deleted between pages, or a merge that rewrites the archive, make a scan hand a row out twice, miss one, or hand out one deleted ahead of it |
| `a_partition_deleted_under_its_scan_hands_out_nothing_more` | the same | A page going on inside a partition that is gone fails, loops or hands out a deleted row |
| `a_cut_partitions_body_says_more_of_it_is_to_come` | the same | A cut partition is written as whole, or a client reads a body that ends inside a partition its answer did not say was cut |
| `a_position_reads_back_and_refuses_what_it_did_not_write` | `shoal-core`, `tables/walk.rs` | A position's bytes move, or bytes that are no position are compared by |
| `a_page_reads_its_shards_tablets_in_order_until_a_bound` | `shoal`, `scan_pages.rs` | A page that may cut passes its bytes by a partition that did not fit |

## Related

- [X19, the edges](../future-work/scans/the-edges.md), which decided it
- [W4, the cursor and the resume token](../future-work/scans/cursor-and-pages.md#the-cursor-and-the-resume-token)
- [F133](bounded-records.md), [F134](walked-records-uncopied.md) and [F135](scan-budget-floor.md), what
  it stands on
- [F141](scan-pages-over-tablets.md), the page that cuts, and [F140](scan-frames.md), the position's
  place in the frames
- [F126](encodes-in-steps.md) and [F127](folds-in-steps.md), the steps and the fold it reuses
