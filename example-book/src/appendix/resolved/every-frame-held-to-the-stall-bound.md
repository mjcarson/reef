# 397. A connection owing scans' answers escaped the stall bound by blocking on another frame, or by trickling

## Symptom

[Item 380](unread-scan-answers-bounded.md)'s stall bound ends a connection whose client takes none of a scan's answer
for `scans.stall_after`, so the answers it owes let their draws go. Two clients escaped it:

- **One that stopped reading while its node wrote another kind of frame.** A get's answer, a cancel's acknowledgement
  or another stream blocked its write relay for ever, and every scan answer queued behind it kept its draw.
- **One that took a little now and then.** Any byte taken started the bound again, so 4 KiB a quarter second kept a
  connection and its answers' draws for ever.

## Cause

The write relay watched only a scan stream's frames (`watch_scan_write`, `shoal-core/src/server/shard.rs`), and its
watch was of progress, not of a rate: it reset at every byte the socket took.

## Evidence

Found by the After M24 exit's review of the node half (2026-11-15), by reading. **Reproduced** on the tree before the
fix by two tests in `shoal/tests/scan_frames.rs`, each against a node whose stall bound is a second:

```text
thread 'a_connection_blocked_on_another_kind_of_frame_is_ended_at_the_stall_bound' panicked at shoal/tests/scan_frames.rs:1012:9:
a client blocked on a get's frames: its connection was never ended, its scan's draw kept

thread 'a_client_that_trickles_is_held_to_the_stall_floor' panicked at shoal/tests/scan_frames.rs:1012:9:
a client that trickled: its connection was never ended, its scan's draw kept
```

A trickle of a byte at a time escaped nothing: a receiver that frees a byte opens no window, so its node saw no
progress either. It takes reading a segment at a time.

## The fix

- **Every frame of a connection granted scans is watched** (`watch_write`): a whole frame, a cancel's acknowledgement
  and any stream's frame, whatever answer it carries, since a scan's answer it owes holds its draw while any frame waits.
  A connection not granted scans is written as it was.
- **A floor of bytes a bound** (`STALL_FLOOR`, 64 KiB): a connection that takes less than that in a window of
  `scans.stall_after` is ended, however often it took a little. 64 KiB in 30 s is about 2 KB a second, far under what a
  scan's share of a link gives each of a thousand connections streaming at once.

A connection standing aside on its link still does so after a second with nothing taken, and counts again at its next
byte. A write its socket takes at once, almost every frame, is polled once and never watched, so no timer is armed on
the path of every answer such a connection is written.

## Alternatives rejected

- **Watching only while the connection owes a scan stream.** A scan's answer queued behind a blocked frame is not yet in
  the relay's outbox, which the reproduction's get shows.
- **A deadline for each frame.** A paced scan frame among thousands of connections streaming on one link may honestly
  take longer than the bound.
- **A socket timeout** (`TCP_USER_TIMEOUT`). A client's kernel acknowledges a zero window, so the socket never times out.

## Invariants to uphold

- **Every frame a connection granted scans is written is watched**, never a scan's alone.
- **A window of the stall bound in which less than `STALL_FLOOR` was taken ends the connection.**
- **Only the relay watching its own write ends its connection**, as item 380's fix requires.
- **A write taken at once arms no timer**: the watch begins only once a write must wait.

## Still open

Nothing of this item.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_connection_blocked_on_another_kind_of_frame_is_ended_at_the_stall_bound` (`shoal/tests/scan_frames.rs`) | A client blocked on a get's frames keeps its scan's draw for ever |
| `a_client_that_trickles_is_held_to_the_stall_floor` (`shoal/tests/scan_frames.rs`) | A client reading a segment at a time keeps its connection and its draw for ever |

## Related

- [Resolved #380](unread-scan-answers-bounded.md), whose bound this completes.
- [F147](../../features/scan-link-pace.md), whose link a connection stands aside on.
