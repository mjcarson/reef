# 390. A scan of resident rows filled its client's link, and the client's own foreground waited behind it

## Symptom

At [M24's lab exit](../../future-work/scans/m24-exit.md) (2026-11-12), titan's standalone node
scanned a million resident rows to europa over 1 GbE. The scan sent 110.6 MiB a second of rows,
116 MiB a second on the wire: the whole link. The scanning client's own reference cell, half gets
and half inserts on other connections of the same client, went over
[W7's budget](../../future-work/scans/isolation.md#the-budget) beside it:

- its read p99 went from 0.46 ms to 1.6 ms, 3.50 [2.41, 4.82] times;
- its insert p99 rose 1.63 [1.31, 1.77] times;
- its rate fell by a sixth.

Both p99s were disjoint from the cell alone. titan resent twice the segments it resends with no
scan, every one a loss probe its peer had no need of.

## Cause

A scan of archived rows is paced by its device ([F119](../../features/scan-pacing.md),
[F128](../../features/device-time-pace.md)). A resident row is read from memory, and nothing paced
it on its way out. So a resident scan's pages were read as fast as the shards encode them. Their
data frames were written as fast as the socket took them, and TCP sent them as fast as the link
carried them. titan's NIC queue then stood full the whole scan, and every other answer leaving
titan waited in it: the cell's, and any other client's. W7 had said as much ("nothing here shares
out the link itself").

## Evidence

**Established by measurement** at M24's exit: eight rounds, both resident arms over the budget and
disjoint from the control, the shards each under a quarter of a core. The exit could not tell the
link from the node, since no arm held a resident scan under the link's rate.

**Reproduced in process before the fix was wired in.** `a_resident_scan_is_paced_to_its_links_share`
and `two_scans_on_one_link_share_its_pace` in `shoal/tests/scan_link_pace.rs` scan 5.1 MB of
resident rows, on loopback paced as a link of 80 Mbit/s at a share of 50%. Run with the pace
skipped at every frame, they failed, the rows coming as fast as loopback carries them:

```text
thread 'a_resident_scan_is_paced_to_its_links_share' panicked at shoal/tests/scan_link_pace.rs:153:5:
5104048 bytes came in 192.587912ms, faster than a share of 5000000 bytes a second allows
thread 'two_scans_on_one_link_share_its_pace' panicked at shoal/tests/scan_link_pace.rs:182:5:
10208096 bytes came in 353.636817ms, faster than one share of 5000000 bytes a second allows
```

**Placed on the link, for the part the link adds, by the lab's sweep**
([its record](../../future-work/scans/link-pace.md#the-sweep), 2026-11-13). It swept the share on
the exit's own resident arm over four rounds on titan, and set the cell beside it against the cell
alone, another client's cell, and scans that send almost nothing.

| Arm | Wire, MiB/s | Cell's read p99 ratio | Insert p99 ratio | Cell's read median, µs |
| --- | --- | --- | --- | --- |
| The cell alone | 1.0 | 1.00 | 0.98 | 242 |
| No pace, as at the exit | 113.0 | 3.33 | 1.59 | 552 |
| A quarter of the link, the default | 32.1 | 2.07 | 1.16 | 253 |
| An eighth | 16.6 | 1.88 | 1.16 | 244 |

The pace binds at every share, the scan's bodies at 95 to 100% of it, and takes off what the link
added. The link no longer holds the cell's median, and its read p99 is two thirds lower. Its insert
p99 is back to within a sixth of its own, and titan's NIC's softirq is down from 38% of its cpu to
10%. What is left does not move with the share, and another client's cell shows it as the scanner's
own does: ~~the node's own cost~~ placed since by a diagnostic: the reads' part titan's network path, any paced flow
out of its NIC costing as much, and the inserts' part the scan's own, item 391 below, resolved since
([Resolved #391](paced-scan-reads-and-inserts.md)).

## The fix

[F147](../../features/scan-link-pace.md), decided with the user on 2026-11-13: the node paces a
scan's answers to a share of the link they leave by, 25% unless set, the share chosen with the
user from the sweep.

The item is resolved as the link's part, also decided with the user once the sweep had run. The
node's part is item 391, which M24 ~~now waits~~ waited on. M24 was delivered once item 391 was resolved on
2026-11-14.

- A node reads its interfaces' speeds at its start.
- Every connection writing a scan's answer is paced by its own write relay. Its part is
  `scans.link_share` of its link's speed, shared out among the connections streaming one there,
  set as the socket's `SO_MAX_PACING_RATE`, with at most a data frame unsent in the kernel.
- Loopback is never paced. An interface that reports no speed is taken at `scans.link_speed`, or
  a gigabit.
- A client's scans ride connections of their own, so no object answer shares a paced one.

## Alternatives rejected

- **The client holding its scan back**, in bytes in flight or by asking paced. It protects only
  that client's own foreground, and only a client that does it.
- **Reading W7's clause as the device's**, the link left to the deployment.
- **A writer's token bucket.** A frame let go at an average rate still leaves at the link's speed,
  a 256 KiB burst at a time, and fills the queue for as long.
- **A fixed byte ceiling as the default.** Another share of every link.

F147's page says more of each.

## Invariants to uphold

- **Only a connection's own write relay sets its socket's pace.** A descriptor is reused once
  closed.
- **A connection counts on its link through a guard dropped when it owes no scan's answer.**
- **Every frame of a scan's answer is paced before it is written**, its opener included.
- **A paced connection keeps at most a data frame unsent in the kernel.**
- **A whole frame never asks the pace anything.**
- **Loopback is never paced** but by a test's fault.

## Still open

- ~~**What a paced resident scan still costs the foreground**, which no share removes: about 0.4 ms on the cell's
  read p99, which any paced flow out of the lab's NICs costs as much, and its inserts at 1.27 times, the scan's own,
  [item 391](paced-scan-reads-and-inserts.md).
  So W7's budget is still not held beside a resident scan, and M24 waits on it.~~ Resolved as
  [item 391](paced-scan-reads-and-inserts.md) on 2026-11-14:
  - The inserts' part was a scan's unpaced reads of its map, cured by Resolved #392.
  - The reads' part is titan's network path, judged against a paced flow of the scan's rate.
  - M24's confirmation held both beside a resident scan, and M24 is delivered.
- **A client on a slower link than its node's** is not protected by a share of the node's link, and
  M25's many nodes answering one client add up on the client's: a client's own asked pace is in
  [the todos](../todos.md#a-clients-own-asked-pace-for-its-scans).
- **The share is a link's, never a principal's**, beside
  [item 380](unread-scan-answers-bounded.md).

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_resident_scan_is_paced_to_its_links_share` (`shoal/tests/scan_link_pace.rs`) | 5.1 MB of resident rows come in 193 ms, faster than the share allows, and the node reports no link paced |
| `two_scans_on_one_link_share_its_pace` (same) | Two clients' scans together come at twice one share and more: 10.2 MB in 354 ms |
| `a_scan_over_loopback_is_not_paced` (same) | Were loopback paced, a node's own client would be held to a share of nothing |
| `a_connection_is_paced_only_while_a_scan_streams` (`shoal-core/src/server/link_pace.rs`) | The pace's rules: set at a scan's frame, shared out on the link, let go at another stream's frame, refused once and left |
| `a_links_share_is_divided_among_its_streaming_connections` (same) | Connections streaming together would take a share each |
| `a_scan_takes_connections_of_its_own` (`shoal/tests/scans.rs`) | A scan's pages would go on the connections that carry a client's objects, which its node paces |

## Related

- [F147](../../features/scan-link-pace.md), the fix.
- [M24's exit](../../future-work/scans/m24-exit.md), where it was found.
- [The lab's record](../../future-work/scans/link-pace.md), the sweep.
- [W7](../../future-work/scans/isolation.md#network), the rules on the wire.
