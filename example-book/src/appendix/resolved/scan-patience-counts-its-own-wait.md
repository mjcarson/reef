# 384. A scan's patience counted the time its caller took between pages

## Symptom

A scan whose caller paused between pages for longer than the scan's patience (a minute unless named)
ended at the first page that was shed, timed out or cut off after the pause, with that page's
refusal, where a caller that never paused had the page asked again. `shoalctl`'s paging does exactly
this: it asks a page when its reader scrolls to the end of the last.

## Cause

The driver asks a page again while `progressed.elapsed() < patience`
(`shoal-client/src/client/scans.rs`, `settle`), where `progressed` was when a page last moved the
scan on. Pages a scan has in flight are settled only when its caller next asks for one. So a caller
who took longer than the patience to come back found `progressed` already too old, and the first
retriable refusal it settled ended the scan. A page that covered nothing ended it the same way.

Patience is meant to bound how long a scan goes on asking with nothing answered. The caller's own
time is not that.

## Evidence

**Found by reading the source** in M24's exit review (2026-11-12) **and reproduced.**
`a_callers_pause_is_not_counted_against_a_scans_patience` in `shoal/tests/scans.rs` starts a scan
patient for half a second through a forwarder that cuts its first page off part way, and takes a
second before it asks for a page. Against the tree before the fix:

```text
Error: Client(Server { query_id: Some(01a1cdde-c487-7191-a1b3-b0227b181299), index: None, code: ConnectionLost,
msg: "the connection to the server failed: IO(Kind(UnexpectedEof))" })
```

## The fix

`take_page` sets `progressed` to now the first time a call waits for a page, after it has launched
what it may. Patience then counts from when a page last moved the scan on or its caller last began
to wait, whichever is later.

## Alternatives rejected

**Summing only the time spent awaiting.** The same bound with a counter to keep on every path that
awaits; the caller's wait beginning is the one moment that matters.

**Settling pages in the background.** A scan holds at most one read page and hands out nothing its
caller did not ask for; a task that settles for it would hold more, and would outlive a dropped scan.

## Invariants to uphold

- **A scan's patience counts what the scan waits, never what its caller takes.**
- **A short or empty page never ends a scan** within its patience
  ([F145](../../features/scan-driver.md)).

## Still open

A page in flight through a caller's long pause is still given up at its own 45 second wait when it
is next polled, and asked again; that costs a round trip, never the scan.
~~[Item 386](../known-issues.md) is what makes sheds routine on a node of many executors.~~ Item 386 made
sheds routine on a node of many executors until [it was resolved](scan-pages-held-to-the-bound.md): a client now
holds its pages on each connection to its node's bound.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_callers_pause_is_not_counted_against_a_scans_patience` (`shoal/tests/scans.rs`) | The scan ends `ConnectionLost` at its first page, cut off after its caller's pause |

## Related

- [F145](../../features/scan-driver.md), the driver, and [F146](../../features/shql-scan.md), whose
  paging pauses.
