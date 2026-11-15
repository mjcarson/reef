# 376. A scan's page over dead archived partitions read past its bound to the end of its range

## Symptom

A scan's page over a tablet of partitions whose every row was deleted read to the end of its range in
one page, however many dead records lay there, when its node's scan budget held them all. Under a
budget of 64 MiB, one page read every one of a tablet's 600 dead partitions, 1,200 records, and drew
6.3 MiB. Under a small budget the page still ended, but only because a wave's draw passed the whole
budget. Item 371's own test runs under a small budget, so it never saw the defect.

## Cause

[Item 371](scan-page-over-dead-partitions.md)'s fix counts the bytes of a partition the merge read and
found dead toward its page's byte bound (`WalkRows::passed`). It counted them through `handled`, the
bytes each arm of the merge took, which the page's steps take too.

[F133](../../features/bounded-records.md) moved an archived partition's fold into `fold_records` and
`fold_apart`, which take the page's steps a record at a time themselves. Its arm therefore stopped
setting `handled`, or the steps would have counted every byte twice. But `handled` also fed `passed`.
A dead archived partition then counted nothing toward the bound, and the page's waves planned past
every one of them to the end of its range. Item 371's invariant held only where a small budget ended
the page first.

Found by reading, in an independent review of [O162](../optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it)'s
design (2026-11-07): the same lines of the merge. Then reproduced.

## Evidence

**Established by reproducing it.** `a_page_over_dead_partitions_ends_at_its_bound_whatever_its_budget`
(`shoal/tests/scan_edges.rs`) is item 371's tablet of 600 dead partitions of eight kibibyte rows,
under a scan budget of 64 MiB. On the tree before the fix (`915a10ad`):

```text
test a_page_over_dead_partitions_ends_at_its_bound_whatever_its_budget ... FAILED
a page read past its bound of dead records: Scanned { pages: 1, partitions: 0, reads: 1200, most_drawn: 6303840 }
```

## The fix

The merge keeps two counts for an archived partition (`PendingWalk::finish`, `walk.rs`):

- `handled`, the work its arm counts into a step;
- `read_bytes`, the bytes its records hold.

A partition that hands out nothing counts the larger of the two toward the page's bound. A dead
partition's draws are given back as before. Since [F134](../../features/walked-records-uncopied.md),
so are the buffers of any read the page held for it alone, its records' alone and no other's.

Fixed in the same change as F134, which rewrote these lines.

## Alternatives rejected

- **Setting `handled` in the archived arm again.** The fold takes the page's steps a record at a time
  itself, so the page would count every byte twice and yield twice as often.
- **Counting in the fold.** The fold does not know whether its page hands the partition out, and the
  bound is the merge's to judge.

## Invariants to uphold

- **A partition that hands out nothing counts every byte the merge read of it toward its page's
  bound, whichever arm read it.** This is item 371's invariant, restated for an arm that steps on its
  own.
- **A step's bytes are counted once**, by whichever of the arm and its fold takes them.

## Still open

Nothing.

## Tests

| Test | Where | Breaks if |
| --- | --- | --- |
| `a_page_over_dead_partitions_ends_at_its_bound_whatever_its_budget` | `shoal/tests/scan_edges.rs` | A dead archived partition counts nothing toward its page's bound, and one page reads past every one of them, ended by nothing but its budget |
| `a_page_over_dead_partitions_moves_its_cursor_within_its_draw` | `shoal/tests/scan_edges.rs` | Item 371's: a page over dead partitions draws past what a page of live ones holds, or never moves its cursor |

## Related

[Item 371](scan-page-over-dead-partitions.md), whose invariant this restores;
[F133](../../features/bounded-records.md), whose move of the fold lost the count;
[F134](../../features/walked-records-uncopied.md), whose change fixed it.
