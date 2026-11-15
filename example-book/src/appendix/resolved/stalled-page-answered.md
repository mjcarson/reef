# 383. A scan's page given up for making no progress was never answered

## Symptom

A client's page that made no progress on its node for a whole query deadline (ten seconds unless
named) was given up, as it should be, and its client heard nothing: no answer, no error frame, on a
connection that stayed open. The client's driver waited out the rest of its own 45 seconds for the
page before it asked again, so every stall cost a scan some 35 seconds of silence. The node counted
the page as refused `Timeout`.

## Cause

A page's task waits for its answer with `answered_while_moving`
(`shoal-core/src/server/shard/walk.rs`), which gives the page up once its watch shows no progress for
a deadline, and says so on the watch (`PageWatch::abandon`) so the page stops where it waits
([item 355](scan-pages-served-in-turn.md)). The task then builds the `Timeout` refusal.

A cancelled page must be answered by its cancel alone, so before it sends anything the task dropped
its reply if the watch was abandoned (`shoal-core/src/server/shard/scan.rs`, `handle_scan_frame`).
But the watch had one flag for two things: *the client cancelled, or left*, and *the waiter gave up*.
The waiter had just set it, so its own refusal was always dropped.

## Evidence

**Found by reading the source**, by both independent readers of M24's exit review (2026-11-12), **and
reproduced.** `a_page_given_up_for_no_progress_is_answered_a_timeout` in `shoal/tests/scan_frames.rs`
holds a page's reads on a node whose query deadline is one second, and waits eight for any frame.
Against the tree before the fix:

```text
thread 'a_page_given_up_for_no_progress_is_answered_a_timeout' panicked at shoal/tests/scan_frames.rs:801:29:
a page given up for no progress was never answered: Elapsed(())
```

## The fix

The watch says the two things apart. `PageWatch::unask` (`shoal-core/src/server/scans.rs`) is what a
cancel and an ended connection call: nobody waits for an answer, and the page stops. `abandon` alone
is the waiter's, whose client still waits. The task drops its reply only for a page that was
unasked, so a page given up for no progress is answered `Timeout` under its id, once.

## Alternatives rejected

**A flag local to the task for "the waiter timed out".** A cancel that arrives between the waiter's
timeout and the reply's send would then be answered twice, by the cancel and by the timeout. The
watch is what the cancel marks, so the watch is where the two are told apart.

**Leaving it to the client's own wait.** The client's wait is the bound for a node that says nothing
because it cannot; a node that knows it gave a page up says so.

## Invariants to uphold

- **A page is answered once**: by its answer, by one error frame, or by its cancel's. A page its
  waiter gave up is the second.
- **Only a cancel and an ended connection `unask` a page.** Anything else that gives a page up
  `abandon`s it, and its client is told.
- **A page's watch is unasked before its cancel is recorded**, as it was abandoned before.

## Still open

Nothing of this item.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_page_given_up_for_no_progress_is_answered_a_timeout` (`shoal/tests/scan_frames.rs`) | The page is never answered, and the test's eight second wait fails it by name |
| `a_cancelled_page_stops_at_its_next_step` (`shoal/tests/scan_frames.rs`) | Were `unask` not to stop a page or drop its answer, a cancelled page would be answered twice or read on |

## Related

- [F144](../../features/scan-pages-on-the-wire.md), whose task this is.
- [Item 355](scan-pages-served-in-turn.md), which gave a page a watch its waiter gives it up on.
