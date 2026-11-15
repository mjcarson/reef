# 389. A page in rows shed at a chunk's draw was refused whole, with every key it had covered

## Symptom

A scan's page answered in rows whose chunk found no room in the scan budget was refused whole, `Shedding`.
Every chunk it had written for the tablets before, and every key it had covered, went with it, and its client
asked the page again from where it began. Nothing was handed out twice or lost: the work was done again, at
the moment the budget was shortest. A page in partitions shed at a later tablet answered with what it covered,
as [F141](../../features/scan-pages-over-tablets.md) says a page that covered a key does.

## Cause

A page answered in rows writes each tablet's rows into its body in chunks of about a step's bytes, each drawn
from the scan budget before it is made ([F143](../../features/scan-rows.md),
`shoal-core/src/server/tables/walk/rows.rs`). A chunk whose draw shed returned the shed to the page's task
(`PageTask::take`, `shoal-core/src/server/shard/scan.rs`), which refused the page whatever it had covered. It
could not have ended the page where it stood: a chunk gathers rows across partitions, and may end inside one,
so the body at a shed could hold part of a partition, and nothing said how far it held each one whole.

## Evidence

**Found by reading the source**, in M24's exit review (2026-11-12), **and reproduced** on 2026-11-15 before the
fix, through a test fault that sheds one draw of a chunk (`faults::shed_row_chunk`), as a budget with no room
within its wait would. `a_page_in_rows_shed_at_a_chunk_answers_what_it_covered` in `shoal/tests/scans.rs`
sheds the fourth chunk's draw of a scan of 400 films of 8 KB on a node of one executor. Against the tree before
the fix:

```text
thread 'a_page_in_rows_shed_at_a_chunk_answers_what_it_covered' panicked at shoal/tests/scans.rs:677:5:
assertion `left == right` failed: the shed page was refused whole: ScanPageFigures { answered: 5, ended: [0, 3, 0, 2, 0, 0], in_rows: 5, in_partitions: 0, rows: 400, bytes: 3237056, tablets_refused: 0, refused: 1, unauthorized: 0, shed: 1, cancelled: 0 }
  left: 1
```

## The fix

- **A body knows how far it holds each partition whole.** As it writes a tablet's partitions it marks where
  it stands (`RowsBody::mark_whole`): at the tablet's start, every partition before it whole, and after each
  partition every row of which is in a chunk with none waiting, one whose last chunk ended at its end or whose
  every row the filter dropped. A partition the page stopped inside is never marked.
- **A shed cuts the body back to its last mark** (`cut_to_whole`): the chunks past it go and their draws are
  given back to the scan budget at once.
- **The page then ends short there, by its budget.** Its progress is the last partition of the tablet the
  body held whole, or as far as the tablets before it if none was, and the walk's own position inside a
  partition is not handed out. It is refused only when it covered nothing, and its scan asks it again from
  where it stood, as before.

## Alternatives rejected

**Ending the page at the last chunk whose head names a partition it finished (`Through`).** A partition the
filter dropped every row of writes no chunk, and a chunk that ended inside a partition followed by no row kept
leaves that partition whole with no `Through`, so the page would end before keys it had covered. The marks are
taken where the body is written, which knows both.

**Asking the shed chunk's draw again.** The draw has already waited its quarter second for room, and the
pages behind it wait in turn ([F135](../../features/scan-budget-floor.md)); a page that holds what it has and
waits again holds the budget longest when it is shortest.

## Invariants to uphold

- **A page that covered a key answers with it**, in rows as in partitions; it is refused only if it covered
  nothing.
- **A body is cut back only to a mark**, and a mark is taken only where every partition before it is whole in
  the body: never inside a partition, never at one the page stopped inside.
- **What a cut takes away is given back to the scan budget as it is cut**, its draws dropped with its chunks.

## Still open

Nothing of this item.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_page_in_rows_shed_at_a_chunk_answers_what_it_covered` (`shoal/tests/scans.rs`) | The shed page is refused whole and asked again, and no page ends by its budget |
| `an_answer_that_cannot_be_right_ends_its_scan_by_name` (`shoal/tests/scans.rs`) | A body cut back so that its chunks no longer fill it would be refused by the client, and the node's own check (Resolved #387) |

## Related

- [F143](../../features/scan-rows.md), whose body this is, and [F141](../../features/scan-pages-over-tablets.md),
  whose rule it now keeps.
- [Resolved #387](reached-never-in-a-refused-tablet.md), whose check holds every answer to its page before it
  is sent, a cut one included.
- [The *After M24* box](../../future-work/scans/whats-left-todo.md#finishing-after-m24), with which it was fixed.
