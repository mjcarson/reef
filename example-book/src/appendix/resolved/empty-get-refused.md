# 381. A get that named no partition key was never answered

## Symptom

A get whose key list was empty, `MovieGet::new(vec![])`, never came back. The client's `send_one`
waited without end: no rows, no error, no timeout. Every other query of the same bundle was answered.

## Cause

A coordinator routes a query by the partitions it names. `route_archived` groups a get's keys by the
shard that owns each and pushes one share a shard (`shoal-core/src/server/routing.rs`). A get with no
keys pushes no share.

The coordinator then sent nothing, recorded no gather, since a gather is recorded only for a query
split over more than one shard, and answered nothing. A query with no share had nobody to answer it
and no deadline to expire it, so its slot in the bundle stayed owed for the life of the connection.

The scans plan's contract (P24) says a get with no keys is refused. It was never a refusal. It was a
hang.

## Evidence

**Reproduced.** `a_get_with_no_keys_is_still_refused` in `shoal/tests/scans.rs`, written for M24's
acceptance row, was run against the tree before the fix. It printed

```text
test a_get_with_no_keys_is_still_refused ...
```

and nothing more. The test binary sat at 0.2% of a cpu for three minutes until it was killed, and a
second run of that test alone was ended by its 60 second limit at the same line.

## The fix

The coordinator refuses a query that routed to no shard, in `Shard::route`
(`shoal-core/src/server/shard.rs`), straight after the query is routed: `InvalidRequest`, "this query
names no partition key, so it reads nothing: a table is read whole only by a scan". The refusal goes
through the path a read refused while routing already takes (`refused_reads`), so it is answered in
its own table's variant, under its own index, after the ring's borrow is over.

Nothing else of the bundle is touched: a get that names a key beside it is answered as ever.

## Alternatives rejected

**Answering no rows.** An empty answer is what a get of keys that do not exist returns. A caller that
built an empty key list by mistake would read "nothing matched", and a caller that hoped an empty
list meant every row would read "the table is empty".

**Refusing it in the client.** A client of another build, or a raw socket, would still hang its own
connection's slot. The node has to answer every query it accepts.

**Reading the table.** P24: no query becomes a scan by naming no key. A scan is asked for as a scan,
by a principal holding `scan` ([F145](../../features/scan-driver.md)).

## Invariants to uphold

- **Every query a coordinator accepts is answered once**, whatever it routes to. A new query kind
  whose routing can push no share is refused here, not left.
- **No query reads a table by naming no key.**

## Still open

Nothing of this item. SHQL's `SELECT` requires a key in its grammar and was never affected.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_get_with_no_keys_is_still_refused` (`shoal/tests/scans.rs`) | A get of no keys, with a filter or without, is never answered; the test's ten second bound fails it by name |

## Related

- [F145](../../features/scan-driver.md), whose acceptance row found it.
- [P24](../../future-work/scans/contract.md) and [W10](../../future-work/scans/api.md): explicit and
  granted.
- [Resolved #33](gather-expiry.md): the other way a query was left unanswered, a gather with no
  deadline.
