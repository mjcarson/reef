# 403. A client's write dropped part way through a frame left the rest of it to the next frame on its connection

## Symptom

A frame a client wrote in several calls, because its socket had less room than the frame, could be dropped between two
of them: its caller's timeout, a dropped object operation, or a scan page's front aborted with its ask
([Resolved #394](scan-place-search-on-its-own.md)). The connection went back to its pool with the start of a frame on its
socket, and the next frame anyone wrote there was read by the node as that frame's rest. It failed to validate and ended
the connection with every answer owed on it ([Resolved #362](unreadable-bundle-ends-its-connection.md)); or, where the
first frame's length ran past the bytes that followed, the node held the connection waiting for bytes that never came.

## Cause

Every frame a client writes goes through `ConnShared::write` (`shoal-client/src/client/cancel.rs`), which loops on
`write_vectored` until every byte is taken. Nothing said a write had stopped part way: the pool's `has_broken` and
`is_valid` asked only whether the connection's reader had stopped and whether its socket still had a peer, and both
were true of a connection holding half a frame.

## Evidence

Found by the After M24 exit while finishing item 394's fix (2026-11-15), by reading. **Reproduced** by
`a_write_dropped_part_way_leaves_its_connection_torn` (`shoal-client`'s `client::cancel::tests`), which writes a frame
of 64 MiB to a peer that never reads, drops the write after 200 ms, and asks whether the connection is broken, on the
tree before the fix:

```text
thread 'client::cancel::tests::a_write_dropped_part_way_leaves_its_connection_torn' (3299985) panicked at shoal-client/src/client/cancel.rs:490:9:
a frame cut part way leaves its connection torn
```

## The fix

A write arms a guard once it holds the connection's lock, before its first byte, and disarms it only once its last
byte is taken (`TearGuard`). A write dropped between, or one that failed, leaves the guard armed, and its drop marks the
connection torn. A torn connection is broken by `ConnShared::broken`, which the pool's `has_broken` and `is_valid`
read, so the pool drops it rather than hand it out; and `write` and `flush` refuse it, so a holder outside the pool, a
cancel among them, writes nothing a node would read as the rest of the frame. Dropping it closes its write half, the
node ends the connection at the cut frame, and the answers owed on it fail as a connection lost does.

## Alternatives rejected

- **Finishing the frame in a task of its own when its writer is dropped.** It would keep the connection, at the cost of
  a task holding the frame's buffers past their caller, which borrowed them; and a write dropped because its peer
  stopped reading would hold that task as long.
- **Marking the connection dead in the reader's map** (`dead_conns`). That map is the reader's, and a connection is
  taken out of it at its first check, which a torn one must fail at every check.

## Invariants to uphold

- **Nothing is written on a connection after a write that stopped part way**, by a frame or a cancel.
- **A write is armed from its first byte to its last**: arming it before the lock is held would tear a connection
  whose write was dropped waiting for its turn, with nothing written.

## Still open

Nothing.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_write_dropped_part_way_leaves_its_connection_torn` (`shoal-client`'s `client::cancel::tests`) | A connection a write left half a frame on is called whole, and the next frame and a cancel are written after it |

## Related

- [Resolved #394](scan-place-search-on-its-own.md), whose page front, aborted with its ask, is one of the writers that
  could stop part way.
- [Resolved #362](unreadable-bundle-ends-its-connection.md), how a node ends a connection whose bytes do not validate.
- [F75](../../features/client-cancel.md), why a connection's write half is shared.
