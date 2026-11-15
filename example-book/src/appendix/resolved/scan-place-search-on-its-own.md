# 394. A scan whose caller paused held every other scan of its client from finding places for its pages

## Symptom

A client's scan whose caller stopped asking for rows part way through a page held every other scan of the same client
from writing a page. A pager waiting on its reader, a consumer slow over one page and a split scan's part did it to
the others. Their asks waited out the 45 s a page waits for a place, were asked again and waited again, and the scans
ended `Timeout` once their patience ran out.

## Cause

A page's place on a connection is found by `place_a_page` (`shoal-client/src/client/scans.rs`), which held one lock
for the whole client (`ScanPlaces::choosing`) across `pool.get_owned().await`. That await is pending whenever the pool
opens a connection. The search runs inside its ask, and its ask sits in its scan's flight, which is polled only while
the scan's caller waits for rows: a caller that paused left a search holding the lock, and the full connections it
held checked out, until it was polled again. tokio's lock hands itself to the next queued waiter whether or not
anything polls that waiter, so a paused scan's queued asks held it the same way.

## Evidence

Found by the After M24 exit's review of the client half (2026-11-15), by reading the source. **Reproduced** by
`a_paused_scan_holds_no_other_scan_from_its_places` (`shoal/tests/scans.rs`), on the tree before the fix: a scan of
eight streams a shard on a node of four executors, its caller stopped after three pages, and a second scan of the same
client.

```text
thread 'a_paused_scan_holds_no_other_scan_from_its_places' panicked at shoal/tests/scans.rs:500:19:
a scan beside a paused one found no place for its pages in thirty seconds
```

## The fix

~~The search is a task of its own (`search_for_a_place`, run by `tokio::spawn`), and the ask waits for it, aborting it
if the ask is dropped first (`AbortOnDrop`). So a search finishes whoever polls its scan, and gives its lock and the full
connections it held back at once. A place found for an ask that was dropped meanwhile goes back when the task's result
is dropped.~~

**A page's whole front is a task of its own** (`AskFront::write`, run by `tokio::spawn`): its search for a place
(`search_for_a_place`), its slot, its cancel's guard, its frame written and its connection given back. The ask waits for
the task and then for its answer, aborting the task if the ask is dropped first (`AbortOnDrop`). So a search finishes
whoever polls its scan and gives its lock and the full connections it held back at once, and **a page placed is
written**, so its answer comes and its reader gives its place back as it delivers it, whether or not its scan is polled
again. The front takes clones of what it needs of its client (its pool, its slots, its queue of channel pairs, its dead
connections and its canceller) and borrows nothing, so it runs on any of tokio's threads.

**The fix's first form ran the search alone as a task** (9697d055), and the after M24 exit's workspace run failed on it:
`a_paused_scan_holds_no_places_its_answers_left`, [Resolved #386](scan-pages-held-to-the-bound.md)'s row, which pauses
a scan with its connection full and asks a second scan's page beside it.

```text
---- a_paused_scan_holds_no_places_its_answers_left stdout ----
Error: Client(ConnectionPool("failed to get connection from pool: Timed out in bb8"))
```

Alone it failed about one run in eleven, *paused scans held 1 places after their answers came* (`scans.rs:921`). A
search that found its place after its scan's caller had paused had taken the place, and the frame was written by the
ask's own code after the task, which nothing polled: the place was held by a page no node had been asked for, so no
answer came to give it back. With the whole front a task, the row passed 25 runs of 25 alone and every scan suite with
it.

## Alternatives rejected

- **A lock that is not handed to a queued waiter** (`futures::lock::Mutex`). A search holding the lock across its own
  pending `get_owned` would still hold it while its scan was not polled.
- **A lock a scan.** Two scans' searches would hold each other's full connections, which the lock is there to prevent.
- **No lock, each search holding the full connections it looked at.** Searches at once take every connection between
  them and wait out the pool's timeout.
- **The search alone as a task**, the fix's first form: a place it found for a paused scan was held by a page never
  written, so no answer came to give it back.
- **Giving a place back when the search's task hands it to an ask nobody polls.** Nothing can tell an ask nobody polls
  yet from one about to be polled; a page written is answered, which gives the place back by the path every answer
  takes.

## Invariants to uphold

- **Nothing a scan's caller must poll holds the placement lock**: the search is its own task.
- **A page's place is taken only by a task that writes the page**: nothing between a place taken and its frame written
  waits for the scan's caller, so every place is given back by its page's answer, its cancel or its ask's drop.
- **A front is aborted with its ask**, and what it took for a dropped ask, its place and its slot, is given back when
  they are dropped; a page written before the abort is cancelled by its guard, as an ask dropped after its write is.

## Still open

Nothing.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_paused_scan_holds_no_other_scan_from_its_places` (`shoal/tests/scans.rs`) | A scan beside one whose caller paused finds no place for its pages |
| `a_paused_scan_holds_no_places_its_answers_left` (`shoal/tests/scans.rs`, item 386's) | With the search alone a task, a place found for a paused scan's page is held by a page never written, about one run in eleven |

## Related

- [Resolved #386](scan-pages-held-to-the-bound.md), whose places these are.
- [F145](../../features/scan-driver.md), the driver whose flight a paused caller leaves unpolled.
