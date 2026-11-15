# 400. A reused response channel could hand its next query its last query's failure

## Symptom

A client's response channel pair, given back to the queue after a query's answer, could still hold a message that
query never read, and the next query to take the pair read it first: a `ConnectionLost` of the last query's connection,
as the next query's answer.

## Cause

A query is answered, and then removes its slot from the client's map and gives its channel pair back. A connection
that died between the two had its reader fail every slot still in the map, the answered query's among them, which put
a failure into the pair. Every path gave a pair back without draining it, and `track_slot` took a pair from the queue as
it was.

## Evidence

Found by the After M24 exit's review of the client half (2026-11-15), by reading, and older than this work.
**Reproduced** at the reuse by `a_reused_pair_holds_nothing_of_its_last_query` (`client::reused_pair_tests`), which
gives a pair holding a failure back and takes it, on the tree before the fix:

```text
thread 'client::reused_pair_tests::a_reused_pair_holds_nothing_of_its_last_query' panicked at shoal-client/src/client.rs:6973:9:
a reused pair handed its next query its last query's message
```

The race itself, a connection ending in the instant between an answer and its slot's removal, was not reproduced.

## The fix

A pair taken from the queue is drained before it is used (`reused_pair`): whatever its last query was sent after it
stopped reading goes with it.

## Alternatives rejected

- **Draining at every place a pair is given back.** There are eight of them; one place takes pairs.

## Invariants to uphold

- **A reused pair holds nothing of its last query** when its next query takes it.

## Still open

Nothing.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_reused_pair_holds_nothing_of_its_last_query` (`shoal-client`'s `client::reused_pair_tests`) | A reused pair hands its next query its last query's message |

## Related

- [Resolved #141](recycled-stream-channels.md), the late answer to a channel that belongs to another stream.
