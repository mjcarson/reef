# 373. A scan's page held its shard's thread for a whole archived partition's verify and fold

## Symptom

A scan of a table with large sorted partitions held its shard's thread for milliseconds at a time, and
no figure of the node counted it. [X19](../../future-work/scans/the-edges.md#e2-one-sorted-partition-past-a-page)
measured it on europa, four shards in process, medians of four rounds: the longest single call a page
made on its shard's thread, by that thread's own cpu.

| Partition | Kibibyte rows | 32-byte rows |
| --- | --- | --- |
| 2 MiB | 0.14 ms | 0.32 ms |
| 8 MiB | 0.59 ms | 1.26 ms |
| 15 MiB | 1.25 ms | 2.47 ms |
| 32 MiB, a 256 MiB budget | 3.56 ms | |
| 64 MiB, a 256 MiB budget | 7.12 ms | |

[W7](../../future-work/scans/isolation.md#the-budget) allows a scan 100 µs of its shard at a time. The
reference cell's mixture beside back-to-back pages of one archived partition had its insert p99 on that
shard at 1.80 times its p99 alone for 8 MiB, and 1.42 times for 15 MiB.

## Cause

A sorted partition was one archived record, however large. A scan's page read it, and then did two things
with it, each in one synchronous call on the shard's thread inside the scan queue's task:

- `ArchiveMap::verify_record` checked the record's checksum over its whole payload (`record_checksum`);
- the table's walk fold (`walk_fold`), or a chain's `fold_chain`, validated it with rkyv and copied it
  into an aligned buffer, or deserialized it and took its tombstones out.

The task yielded only between them. Neither was a loop step, so `ScanFigures`' step counters, which W7's
step clause and M23's exit were judged by, never saw them
([F130](../../features/loop-steps-by-kind.md)).

A checksum or an rkyv validation of a record cannot be cut into steps: the first would change every
archive's format to feed it in pieces, and the second is over the whole archive by construction. So the
records themselves had to be bounded.

## Evidence

**Established by measuring it** at X19 (2026-11-07), with the spike's clock around the two calls, on the
tree before the fix; reading the code said the same, since neither call has a yield inside it. X19's
record holds the table above, and item 374's fix changed none of its figures beyond their ranges.

The fix was measured in the same build both ways, since `max_record_bytes` set to a gibibyte writes
every partition as one whole record, as before F133:
`bounded_records::the_longest_hold_a_scan_page_makes_on_its_shard` (ignored). One shard on europa, in
release, the governor at powersave as the host was. A resident partition of 50 lines was got one get
after another for 3 s alone, then for 8 s beside back-to-back scan pages of one archived partition of
kibibyte rows. A hold shows as a get that waited for it.

The third of three runs; the gets' p99 was 35 to 45 µs in every arm, and 3 s of gets alone made none
past 1 ms:

| Partition, kibibyte rows | Records | Pages in 8 s | Gets past 1 ms | Gets past 5 ms | Longest get |
| --- | --- | --- | --- | --- | --- |
| 2 MiB, one whole record | 1 | 385 | 0 | 0 | 0.57 ms |
| 2 MiB, records of at most 1 MiB | 3 | 385 | 0 | 0 | 0.66 ms |
| 15 MiB, one whole record | 1 | 52 | 53 | 1 | 7.7 ms |
| 15 MiB, records of at most 1 MiB | 20 | 52 | 3 | 0 | 1.4 ms |
| 15 MiB, records of at most 256 KiB | 79 | 52 | 1 | 0 | 1.0 ms |
| 64 MiB, one whole record | 1 | 12 | 49 | 11 | 24.8 ms |
| 64 MiB, records of at most 1 MiB | 85 | 12 | 11 | 0 | 2.0 ms |
| 64 MiB, records of at most 256 KiB | 128 | 12 | 15 | 0 | 1.2 ms |

Beside one whole record a 64 MiB page made a get wait about 25 ms, once a page: X19's hold, its
checksum and its fold, with the copy's fresh memory. Beside records of at most 1 MiB no get waited 5 ms.
About one get a page still passed 1 ms beside 64 MiB in records, and a quarter of the bound made it no
shorter, so it is no record's: it is filed as
[item 375](../known-issues.md#375-a-scans-page-of-a-64-mib-partition-still-delays-a-get-on-its-shard-past-a-millisecond-about-once-a-page).
The test could not be run against the tree before the fix, which had no bound to set; the gibibyte bound
writes what that tree wrote.

## The fix

A merge writes a sorted partition whose archive passes `max_record_bytes` (1 MiB) as a base and the
pieces its rows continue in, each a record of at most the bound ([F133](../../features/bounded-records.md)).
A page then works on one record at a time:

- its reader reads a chain's records one by one, each verified apart, none folded
  (`ArchiveMap::read_records_from`);
- its merge folds them a record a step (`PendingWalk::fold_apart`) and hands each out as a piece of the
  walked partition, the page's steps taking each record's bytes before the next;
- a partition in pieces with fragments over it has each record validated in a step of its own, and the
  fragments folded over the pieces 64 KiB at a time (`DiskRows`, [F127](../../features/folds-in-steps.md)).

No call a page makes on its shard's thread holds more than one bounded record. The user chose, beside
the fix, that a get reads a partition in pieces in place, and that the bound is 1 MiB.

## Alternatives rejected

- **Feed the checksum in pieces.** Every archive's format would change, for a check that is fast when
  its record is small. Rejected with the user at X19.
- **Move the checksum and the validation to a thread that holds no loop.** A page would still hold the
  whole record, and twice for its copy. Rejected with the user at X19.
- **Split a partition across pages over whole records**, M24's split alone. Each page would read and
  verify the whole record again: about 4 GiB read for a 64 MiB partition in pages of 1 MiB. The split is
  M24's, on these records.
- **Write the pieces as fragments of rows.** No format would move, but every reader of a large
  partition would fold it, a get included, where it read the record in place.

## Invariants to uphold

- **No record a merge, an install or an archive pass writes of a sorted partition is past
  `max_record_bytes`**, unless it holds one row or its partition passes 128 records (O164). A writer that
  wrote a large partition whole would bring the hold back for that partition.
- **A page verifies and folds a chain's records one at a time, with a yield between them.** A reader
  that joined them first (`read_chain`, `join`) would hold the loop for the whole partition again.
- **Every reader of a chain reads its pieces**; F133's invariants say how.

## Still open

- **A page of a 64 MiB partition still delays a get past 1 ms about once a page**, by something per
  page and not per record ([item 375](../known-issues.md#375-a-scans-page-of-a-64-mib-partition-still-delays-a-get-on-its-shard-past-a-millisecond-about-once-a-page)).
- **A whole record a build before F133 wrote** is verified and folded whole until a merge or an archive
  pass rewrites it.
- **A get's load validates every piece in one handler call**, the foreground's twin of this hold, as it
  validated the one record before (O165).
- **A partition past 128 records** has records past the bound (O164).
- **A captured page's overlay**, a backup's alone, folds its archive whole.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `bounded_records::a_scan_page_folds_a_partition_in_pieces_a_record_at_a_time` | A scan's page of an archived partition past the bound hands out a piece a record, none past the bound, each record read alone; a fragment and an overlay over its pieces read back as the model holds them |
| `bounded_records::a_partition_past_the_bound_is_written_in_bounded_records` | A merge writes a partition past the bound as records of at most it, and a fragment, an update and a restart keep it so |
| `bounded_records::an_archive_pass_moves_pieces_and_cuts_a_whole_record` | A pass cuts a whole record written before the fix into bounded records |
| `partitions::tests::a_partition_is_archived_in_bounded_records` | A partition past the bound is archived as records of at most the bound |
| `bounded_records::a_corrupt_piece_is_refused_by_name` | A page that reads a record alone still refuses a corrupt one by name, and hands out none of its partition |
| `bounded_records::the_longest_hold_a_scan_page_makes_on_its_shard` (ignored) | The timing above |

## Related

- [F133](../../features/bounded-records.md), which built the fix, and its other invariants.
- [X19](../../future-work/scans/the-edges.md), which found this, and
  [its decisions](../../future-work/scans/contract.md#x19-measured-the-edges-decided-with-the-user-2026-11-07).
- [F127](../../features/folds-in-steps.md), the fold in steps a chain's fragments run in.
- [Item 374](scan-draws-held-and-waited.md), fixed the same day, on the same pages.
