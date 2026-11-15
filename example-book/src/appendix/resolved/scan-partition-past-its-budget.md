# 370. A partition the scan budget could not hold was shed on every ask, never refused by name

## Symptom

A scan of a table never finished once one of its partitions needed more than half the node's scan
budget. Every page asked for that partition was refused `Shedding`, "the scan budget is spent", after
its quarter second's wait, and asked again from the same cursor, which was shed the same way. The
bench's scan driver gave up on the tablet after a minute ("a page went unanswered"), and a driver's
`TabletWalk` after its tries, neither saying why. Nothing else was holding the budget: the page could
never have been given what it asked for.

Two pages met it:

- **A partition past half the budget.** Archived or resident, a sorted partition of 1.5 MiB under a
  budget of 2 MiB was shed at every ask, where [F118](../../features/scan-budget.md) says a page that
  needs more than the whole budget is refused `ResponseTooLarge` at once, by a code no scan asks again.
- **Two partitions each within it.** Two archived partitions of 0.9 MiB in one tablet, either of which
  the budget holds alone, were shed at every ask too: the page read both ahead together, and the first's
  copy found no room behind them.

The default budget is eight mebibytes a shard of the node, so a node of four shards met it at a
partition of 16 MiB, past the 4 MiB [the Q36 record](../../future-work/scans/contract.md#q36-the-cursor-and-the-frames-2026-11-06)
named for it, and a node of one shard at 4 MiB.

## Cause

A scan's page holds each of its archived partitions twice. A wave of the page's reads draws a record's
bytes before it is read. The merge then draws them again as it folds the record into the partition it
hands out, since the run's buffer is held beside the partition built from it
([F125](../../features/page-reads-in-runs.md)). A resident partition is the same: its encode steps
draw their parts, and the pieces its fold makes are drawn beside them before the parts are let go
([F126](../../features/encodes-in-steps.md)). A partition of `S` bytes therefore needs about `2S` of
the budget at its peak, and that peak is real.

`PageDraws::cover` (`shoal-core/src/server/tables/walk.rs`) drew each of those draws by itself through
`Budget::draw`, which refuses at once only a draw that is larger than the whole budget
(`Shed::never_fits`, `needed > limit`). Each of a partition's two draws is about `S`, so neither was
past the budget until `S` itself was. Between `L/2` and `L`, the second draw waited its 250 ms for room
that the page's own first draw held, and was shed. `short()` refused a page that held no partition,
by `Shedding`, which every driver asks again.

The second page met the same thing through its wave. `plan_reads` plans every partition a page's bound
reaches into a wave, at least one past the bound. Two partitions of 0.9 MiB under a 1 MiB page bound
are one wave of 1.8 MiB. The merge's copy of the first then needed 0.9 MiB more, past the 2 MiB budget,
before the page held anything.

## Evidence

**Established by reproducing it.** `shoal/tests/scan_edges.rs` runs a standalone node in process with a
scan budget of 2 MiB and every loop evicting what it has compacted, so partitions written before a start
are read from their archives after it. On the tree before the fix (`f631775c`, 2026-11-07):

```text
test a_partition_no_scan_budget_holds_is_refused_by_name ... FAILED
test a_wave_reads_no_further_ahead_than_its_budget_holds ... FAILED

---- a_partition_no_scan_budget_holds_is_refused_by_name stdout ----
assertion `left == right` failed: WalkRefusal { code: Shedding, msg: "the scan budget is spent:
1601624 bytes found no room in their wait", applied: None, shed: Some(Budget) } after 7 sheds
  left: Shedding
 right: ResponseTooLarge

---- a_wave_reads_no_further_ahead_than_its_budget_holds stdout ----
Error: Io(Custom { kind: Other, error: "the scan of two partitions within the budget ended
WalkRefusal { code: Shedding, msg: \"the scan budget is spent: 960984 bytes found no room in their
wait\", applied: None, shed: Some(Budget) } after 7 sheds" })
```

The 1,601,624 bytes are the merge's copy of the 1.5 MiB partition, drawn behind its own wave's read.
The test's two seconds of patience ran out after seven sheds. The defect was first read from the code
while [X19](../../future-work/scans/spikes.md#x19-the-edges)'s method was being planned, which is where
both tests come from.

## The fix

- **A page's draw is judged by what the page would hold, not by the draw alone.** `PageDraws::cover`
  refuses at once, with no wait, a draw that would take what the page holds past the whole budget:
  `held + more > limit` is a shed that never fits, whose `needed` is the page's whole holding.
- **A page that covered a key ends short there, whatever shed it** (`short`). Its next page starts at
  the partition that did not fit and asks for it alone. Only a page that covered no key is refused:
  `ResponseTooLarge` when its partition never fits, `Shedding` when it might after a wait.
- **The refusal names the partition** by its key (`ShoalError::ScanBudget::key`, `scan_refusal`):
  "partition `…` needs a scan's page to hold `…` bytes, past the whole scan budget of `…`; raise
  resources.scan_memory".
- **A wave plans no partition past its first that the budget could not hold beside the rest**
  (`plan_reads`, `PageDraws::could_hold`). What counts is the wave's partitions twice, for their reads
  and their merge's copies, and a run's gap for each. The first partition is always planned, and its own
  need is judged at its draw.

A partition the budget holds alone, about `L/2` less a few bytes, is handed out. One it does not hold
ends its scan by name, which [W4](../../future-work/scans/cursor-and-pages.md#bounds-and-short-pages)
says it does until M24 splits such a partition or does not.

## Alternatives rejected

- **Draw an archived record once.** The merge's copy is a second buffer: the fold copies a record it
  hands out as it lies into an aligned buffer of its own while the run's buffer still holds it
  (`walk_fold`, `aligned`). Counting it once would let a page allocate past its budget, which is the one
  thing F118 promises never happens. Handing a record out from its run's buffer instead of copying it
  would halve what an archived page holds. That is
  [O162](../optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it),
  an optimization, not a fix, built since as [F134](../../features/walked-records-uncopied.md): a record is
  now handed out where its read put it, so it is drawn once because it is held once.
- **Refuse a partition past `SCAN_MAX_BYTES`, 4 MiB**, as the Q36 record put it. That bound is a page's
  answer bytes, not its memory. A node whose budget holds a 6 MiB partition would refuse one, and a
  node of one shard would still shed one of 3 MiB. The budget is what measures memory, so it is what
  judges a partition.
- **Raise the default budget.** That moves the edge and leaves it: a larger partition sheds the same way.
- **Give up after a number of sheds**, in the drivers. A transient code for a permanent condition makes
  every client wait out its patience to learn what the shard knew at the first ask.

## Invariants to uphold

- **No page holds more than the whole scan budget**, counting every buffer it holds at once: ~~a record's
  read and its copy~~ a record's read, in whole blocks, and any copy made of it, a fold's pieces, an
  encode's parts and pieces ([F134](../../features/walked-records-uncopied.md)).
- **A page refused `ResponseTooLarge` for its budget covered no key**, and its partition alone needs
  more than the whole budget. A page that covered a key is never refused for its budget: it ends short.
- **`Shed::needed` of a page's draw is what the page would hold**, so `never_fits` is the page's whole
  need against the budget, not one increment's.
- **A wave always plans its first partition, and the rest only within the budget**, so a partition the
  budget holds alone is never shed behind its own wave.

## Still open

- **Whether M24 splits a sorted partition across pages**, filling Q36's reserved position, ~~is
  [X19](../../future-work/scans/spikes.md#x19-the-edges)'s to decide with the user~~ was decided by
  [X19](../../future-work/scans/the-edges.md#what-it-says-for-the-design) with the user (2026-11-07): it
  splits, from records of bounded size, and keeps a scan budget of at least the largest row, so no partition
  is refused for size once M24 lands. Until then a partition past half the budget ends its scan by name,
  ~~archived or resident~~ and since F134 an archived one only past the whole budget. The floor is built
  (2026-11-08, [F135](../../features/scan-budget-floor.md)): a node's scan budget is never less than its
  request body bound and a row's slack, so an archived row it admits always fits a page alone. A resident or
  overlaid row past half the budget is still refused
  ([item 377](../known-issues.md#377-a-resident-or-overlaid-row-past-half-the-scan-budget-is-refused-by-name)),
  and so is a sorted partition of many rows past the whole budget until M24 splits it.
- ~~**An archived partition is held twice by its page**: [O162](../optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it),
  required before M24 since X19.~~ Done (2026-11-07, [F134](../../features/walked-records-uncopied.md)):
  an archived partition is held once, where its read put it. A resident one is still held about twice, its
  encode steps' parts beside its fold's pieces.

## Tests

| Test | Where | Breaks if |
| --- | --- | --- |
| `a_partition_no_scan_budget_holds_is_refused_by_name` | `shoal/tests/scan_edges.rs` | A partition past half the budget is shed again rather than refused, its refusal stops naming its key, waits a quarter second first, or a resident one is not judged the same way |
| `a_wave_reads_no_further_ahead_than_its_budget_holds` | `shoal/tests/scan_edges.rs` | A wave plans a partition past its first that the budget cannot hold beside it, so its first partition is shed behind it |
| `a_scans_buffers_are_drawn_from_its_budget_and_shed_past_it` | `shoal/tests/scan_admission.rs` | A page's admission past the whole budget stops being refused at once, or a page short of room stops being shed after its wait |

## Related

[F118](../../features/scan-budget.md), the scan budget; [F125](../../features/page-reads-in-runs.md)
and [F126](../../features/encodes-in-steps.md), the waves and the encode steps that hold a partition
twice; [Resolved #371](scan-page-over-dead-partitions.md), the other edge of the same draws, found
beside it; [X19](../../future-work/scans/spikes.md#x19-the-edges), whose planning read it from the
code.
