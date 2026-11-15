# 395. A crafted page made a saved position's search for its keys quadratic on its shard's loop

## Symptom

A client could hold a shard's loop for seconds with two pages. A page of one row from a deep tablet's start saved its
keys ([F150](../../features/saved-positions.md)), and a second page of one row from near the tablet's end, asked within
ten seconds and with no key added meanwhile, made the position skip to its start one bucket at a time. Every get and
write on that shard waited, and the client could repeat it.

## Cause

A position's tail was cut into `tail / take` buckets, one a key at a page of one row, and a page past the sorted keys
skipped buckets one at a time with `next_end`, a search of every bucket's end from the first: a skip of `k` buckets cost
`k` squared. Nothing bounded the buckets or the skip.

## Evidence

Found by the After M24 exit's review of the node half (2026-11-15), which measured the module alone in release: 349 ms
at 62,914 keys, K_max on the lab's nodes, and 3.55 s at 200,000. **Reproduced** by
`a_far_start_on_a_deep_tablet_is_found_at_once` (`server::tables::saved`), on the tree before the fix, in a debug build:

```text
thread 'server::tables::saved::tests::a_far_start_on_a_deep_tablet_is_found_at_once' panicked at shoal-core/src/server/tables/saved.rs:381:9:
a far start took 11.888666631s to find
```

## The fix

A position's buckets are at most `MAX_BUCKETS` (1,024), each of at least `MIN_BUCKET_KEYS` (64) keys where the tail has
them, and a page whose start is past every sorted key goes straight to its start's bucket by arithmetic (`bucket_of`),
every bucket before it holding only keys before the start, sorts that bucket alone and goes on from it. The next
unsorted bucket is kept by index. So no page does more than a bucket's work, at most a 64th of its tablet's keys.

## Alternatives rejected

- **A binary search of the buckets' ends.** As bounded, and arithmetic needs no search: the buckets are of equal width.
- **No position for a page of few rows.** A crafted page of a thousand rows would still skip a bucket at a time.

## Invariants to uphold

- **No page does more than a bucket's work to find its keys**, and a position has at most `MAX_BUCKETS` buckets.
- **Every bucket before a start's bucket holds only keys before the start**, which is what lets a page skip them unread.

## Still open

Nothing.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_far_start_on_a_deep_tablet_is_found_at_once` (`shoal-core`'s `server::tables::saved`) | A far start on a deep tablet is found a bucket at a time |
| `a_saved_tablets_keys_come_out_in_order_a_page_at_a_time` (`server::tables::saved`) | A page of one row through a position's buckets hands out a key out of order, twice or not at all |

## Related

- [F150](../../features/saved-positions.md), whose positions these are, and
  [Resolved #398](saved-position-costs.md), their costs.
