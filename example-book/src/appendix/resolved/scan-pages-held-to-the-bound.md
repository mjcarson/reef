# 386. A scan asked more pages of a connection than its node serves, on a node of more than a few executors

## Symptom

A scan alone on a node of more than a few executors had pages shed all the time. A node serves at most
eight pages of scans in flight a connection and sheds the ninth `Shedding`
([F144](../../features/scan-pages-on-the-wire.md)). A client's scan asks two pages an executor once it has
learned them, up to 64 in flight, and wrote each on whichever connection its pool handed out, so on titan's
node of five shards a scan of 1,004 pages had 946 of them asked again, about a page shed for each one
answered ([M24's exit](../../future-work/scans/m24-exit.md#what-a-row-costs-a-scan-alone)). Each shed was
decoded and judged by the node before it was refused, and its stream backed off up to 250 ms before it asked
again. The sheds cost the node's threads 22.4 µs of cpu a row against 17.3 at one stream a shard, and the
scan no time while its device bound it. Nothing was handed out twice or lost.

## Cause

The bound is the node's and the client did not keep to it. The client wrote a page on a connection taken
from the pool its scans' pages go on and gave the connection back once the frame was written
(`ask`, `shoal-client/src/client/scans.rs`). Nothing counted the pages a connection had in flight, and the
pool hands out the connection it has idle, which is every connection a moment after its frame is written,
so whatever the pool's size most pages went on one connection. The pool held two connections unless set,
the number `StreamConfig::dedicated_connections` sets apart for long streams, sixteen pages' room for a scan
that asks up to 64. The bound lived only in the engine (`SCAN_PAGES_A_CONNECTION`, `shard/scan.rs`), so the
client could not have read it.

## Evidence

**Found by reading the source**, in M24's exit review (2026-11-12), **measured** at M24's lab exit the same
day, **and reproduced** on 2026-11-15 before the fix. `a_scan_asks_no_more_pages_of_a_connection_than_its_node_serves`
in `shoal/tests/scans.rs` reads 3,000 films alone in pages of 64 KiB from a node of six executors and counts
the node's sheds. Against the tree before the fix:

```text
thread 'a_scan_asks_no_more_pages_of_a_connection_than_its_node_serves' panicked at shoal/tests/scans.rs:586:5:
assertion `left == right` failed: the node shed pages of a lone scan: ScanPageFigures { answered: 350, ended: [0, 338, 0, 12, 0, 0], in_rows: 350, in_partitions: 0, rows: 3000, bytes: 24285248, tablets_refused: 10236, refused: 48, unauthorized: 0, shed: 48, cancelled: 0 }
  left: 48
 right: 0
```

## The fix

**Decided with the user on 2026-11-15: the client holds to the node's bound.** The other way, a node's bound
of its shards' scan classes, would have given up the bound on what one connection may hold of the scan
budget ([item 380](unread-scan-answers-bounded.md)).

- **The bound is the protocol's.** `SCAN_PAGES_A_CONNECTION` is in `shoal-proto`
  (`shared/protocol/scan/frames.rs`), and the engine and the client read it there.
- **A connection counts this client's scan pages on it.** A connection's shared half
  (`ConnShared::scan_pages`, `client/cancel.rs`) is taken a place in by `take_scan_place`, which never goes past
  the bound however many asks race for its last place.
- **A page takes its place before its slot is made.** `search_for_a_place` (`client/scans.rs`), ~~`place_a_page`~~
  since [Resolved #394](scan-place-search-on-its-own.md) inside the page's front, a task of its own, looks at the pool's
  connections one by one, holding each full one while it looks at the next, so the pool hands out another or
  opens one, up to every connection it may open. Only one ask looks at a time (`ScanPlaces::choosing`), so two
  asks never hold each other's connections and a burst of asks opens connections one at a time. When every
  connection the pool may open is full of this client's pages, the ask waits for one to give its place back
  (`ScanPlaces::freed`), within its wait for its answer.
- **A page's place is given back as its answer is delivered.** The place rides the page's slot
  (`PageSlot::place`), and the reader gives it back as it hands the answer over, the sweep of a connection that
  died gives back the places of the pages it fails, and the ask gives it back however it ends. It is given back
  once, by whichever is first (`ScanPlace::release`). A scan whose caller pauses is not polled, so a place held
  until its ask next ran would be held for as long as its caller paused; and the map of slots drops what it
  removes only later, so a place is never left to a drop.
- **A scan's pages have connections of their own, eight unless set.** `StreamConfig::scan_connections`, apart
  from `dedicated_connections`, holds the 64 pages a scan has in flight at the node's eight a connection; zero
  puts a scan's pages on the shared pool, still eight a connection.
- **A client says how many of its pages are owed** (`Shoal::scan_pages_owed`), which a test reads.

## Alternatives rejected

**The node's bound following its shards' scan classes**, `shards × scans.pages_in_flight`. A client would
shed nothing and the node would bound nothing a connection holds of the scan budget: one connection could
queue every shard's whole class, and a client that never reads would hold all of it
([item 380](unread-scan-answers-bounded.md)).
The user chose the client's side.

**One connection a stream.** A node of 32 executors would take 64 connections a scan, every one paced on its
link and holding its kernel's buffers, for pages a connection of eight serves as well.

**A shed answered by asking the next connection.** The node would still decode and judge every page past the
bound, which is what the shed cost; a client that knows the bound sends none of them.

**Counting a page until its whole answer is read by the scan.** A scan's futures run only while its caller
waits for a row, so a paused scan would hold its places until its caller came back, and another scan on the
same client would wait for it. The reader delivers every answer whether or not a scan is polled, so the reader
gives the place back.

## Invariants to uphold

- **A client never has more of its scan pages in flight on a connection than `SCAN_PAGES_A_CONNECTION`.** A
  page is written only on a connection it took a place on (`take_scan_place`), and the place is taken before
  the frame is written.
- **A place is given back once, by the first of the reader's delivery, the dead-connection sweep and the ask's
  end** (`ScanPlace::release`), never left to a drop.
- **One ask looks for a place at a time**, holding only the connections it looked at; nothing awaited while
  holding them but the pool's next connection.
- **The client's count of a page lasts at least as long as the node's, but for a page it cancelled**: the node
  counts a page from its frame to its answer, and the client from before the frame to after the answer, so the
  node never sees more than the client let it. A cancelled page is the exception, *Still open* says how.
- **The bound is read from `shoal-proto` on both sides.**

## Still open

- ~~**A page's search for a place held its client's placement lock while its scan's caller paused**~~ fixed by
  [Resolved #394](scan-place-search-on-its-own.md): a page's whole front, its search and its frame written, is a
  task of its own. And a client that sends no
  cancels gives a page's place back before its node does
  ([item 402](../known-issues.md#402-a-client-that-does-not-cancel-abandoned-pages-gives-a-pages-place-back-before-its-node-does)).
- **A cancelled page's place is given back when its ask is dropped**, before its node has seen the cancel. The
  cancel goes ahead of the next frame on that connection, but the node counts the page until its task ends,
  so a page written straight after it can be shed. It is retriable, and the count of a page until its answer
  is written, with a cancel releasing it at once, is [item 380](unread-scan-answers-bounded.md)'s,
  fixed with the count a principal.
- **Every scan of a client shares its places**, `split(n)`'s parts among them, in the order they ask. A split
  scan's parts still ask more pages of a shard than its scan class admits, and those are shed by the class,
  as before.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_scan_asks_no_more_pages_of_a_connection_than_its_node_serves` (`shoal/tests/scans.rs`) | A lone scan on six executors is shed at the node's eight a connection, and asks pages again |
| `a_paused_scan_holds_no_places_its_answers_left` (`shoal/tests/scans.rs`) | With the places given back only when an ask runs again, two paused scans hold seven of the eight places after their answers came |
| `a_scan_takes_connections_of_its_own` (`shoal/tests/scans.rs`) | A scan's pages went on a connection set apart for long streams |

## Related

- [F145](../../features/scan-driver.md), whose driver this is, and [F144](../../features/scan-pages-on-the-wire.md),
  whose bound it is.
- [M24's exit](../../future-work/scans/m24-exit.md), which priced it.
- [Resolved #384](scan-patience-counts-its-own-wait.md), which the sheds fed.
- [The *After M24* box](../../future-work/scans/whats-left-todo.md#finishing-after-m24), whose cursor sent ahead
  gives a stream a second page in flight, which is why this was fixed first.
