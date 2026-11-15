# 398. Saved positions cost more than their bound said, and one table's could keep every other's out

## Symptom

[F150](../../features/saved-positions.md)'s positions cost more than it stated, in three ways:

- **A copy for nothing.** A page copied every key of its range to save it however large the range, and saved again at
  every page of a tablet written to, which moves the tablet's stamp at every write.
- **Uncounted bytes.** A position's buckets' bounds went uncounted against the bound of 4 MiB a thread.
- **Positions kept past their life.** A table's positions were dropped only by that table's own next page, so a table
  never scanned again kept its positions, and the room they held, for ever, and the positions of other tables could find
  no room.

## Cause

`capture_keeping` cloned the range's keys whenever a planned page selected some; nothing bounded a range by what a
position may hold, or held a tablet written at every page from saving. A position counted its keys' capacity alone,
and each table kept its own positions (`SavedPositions` in `ResidentPartitions`), expiring and evicting only its own.

## Evidence

Found by the After M24 exit's review of the node half (2026-11-15), by reading the source. Not reproduced by a test that
fails: a position counted wrong and a copy made for nothing leave no answer wrong. The fix's own tests hold what it
counts and keeps.

## The fix

- **One store a shard's thread** (`saved::take`, `saved::keep`), positions kept by table and tablet together, its oldest
  dropped first whatever its table, at most 32 a thread, and every position past its life dropped at any table's look.
  A table is known by an identity its resident partitions are given (`ResidentPartitions::table`).
- **A position's buckets counted** with its keys, again once its tail is cut.
- **No copy past what a position may hold**: a range of more than `MAX_SAVED_KEYS` (half the thread's bound) is not
  copied, and a tablet whose position was dropped for a moved stamp is not saved again for a second (`may_save`).

## Alternatives rejected

- **Positions drawn from the scan budget.** A table's walk has no handle on the node's budget.
- **Keeping a table's positions with its table and sweeping them on a timer.** A shard's thread has no timer for its
  tables; one store a thread is looked at by every page.

## Invariants to uphold

- **Every byte a position holds is counted while it lives**, its buckets' bounds with its keys.
- **One store a thread holds every table's positions**, evicted oldest first.
- **No range past `MAX_SAVED_KEYS` is copied to be saved.**

## Still open

Nothing.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_threads_positions_are_counted_and_bounded` (`shoal-core`'s `server::tables::saved`) | A cut position's buckets go uncounted, or an old table's position outlives the thread's bound |
| `a_position_serves_only_what_it_saved` (`server::tables::saved`) | A tablet whose stamp moved is saved again at once |

## Related

- [F150](../../features/saved-positions.md), and [Resolved #395](saved-position-bounded.md).
