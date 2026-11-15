# 396. A principal's burst of scan pages could hold more than its share of the scan budget

## Symptom

[F148](../../features/scan-shares.md) holds one principal's unwritten scan answers to half the node's scan budget, but a
principal asking many pages at once could hold the whole of it. On a node whose scan class read the burst together,
four connections each asking eight pages of the most answer, none of them read, took every byte of the budget, and
other principals' pages were shed for the budget, not their own.

## Cause

A page's principal was held to its share at its admission by what its answers waiting to be written held
(`handle_scan_frame`, `shoal-core/src/server/shard/scan.rs`). A page adds to that count only once its answer is built,
after it has drawn and read, so every page admitted before the first answer was counted saw a principal holding
nothing. The overshoot was every page a node's scan classes read at once.

## Evidence

Found by the After M24 exit's review of the node half (2026-11-15), by reading. **Reproduced** by
`a_principals_burst_of_pages_is_held_to_its_share` (`shoal/tests/scan_frames.rs`), on the tree before the fix, on a node
whose scan class reads 32 pages at once:

```text
thread 'a_principals_burst_of_pages_is_held_to_its_share' panicked at shoal/tests/scan_frames.rs:1118:5:
a burst held 67108864 bytes of a 67174400-byte budget: ScanPageFigures { answered: 6, ..., shed: 26, ..., shed_held: 0 }
```

At the default class of two pages a shard the class shed the burst first, and the same burst held its share.

## The fix

A page reserves what it will first draw against its principal as it is admitted (`HeldBytes::reserve`), by one compare
and swap that refuses it if the principal's count and the page would pass the share, unless the principal holds nothing
yet. The reservation is held by the page's task until its answer's own count is made, then dropped, so the count never
dips under what the page holds. A page refused is shed by name as before.

## Alternatives rejected

- **Counting the page's draw against its principal once drawn.** The draw is awaited, and two shards' pages could each
  pass the check before either drew.
- **Holding a principal to a count of pages.** Pages differ in what they hold; the share is of bytes.

## Invariants to uphold

- **A page is counted against its principal before it draws**, and its reservation lasts until its answer is counted.
- **The reservation is one compare and swap**, so pages of one principal admitted at once on several shards never pass
  its share between them.

## Still open

Nothing of this item. On a node that authenticates nobody every connection is one principal, so one client's answers
can hold the share every other client's pages are held to:
[item 401](../known-issues.md#401-on-a-node-that-authenticates-nobody-one-slow-reader-sheds-every-clients-scans).

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_principals_burst_of_pages_is_held_to_its_share` (`shoal/tests/scan_frames.rs`) | A burst of one principal's pages holds the whole scan budget |
| `a_principals_unwritten_answers_hold_at_most_its_share_of_the_budget` (`shoal/tests/scan_frames.rs`) | One principal's unread answers run the budget dry |

## Related

- [F148](../../features/scan-shares.md), whose share this holds to, and
  [Resolved #380](unread-scan-answers-bounded.md).
