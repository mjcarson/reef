# 380. A client that never read its scans' answers held their draws on the scan budget

## Symptom

A client that asked a node for pages of a scan and read none of the answers held the node's scan budget for as
long as its connections lived. A page's answer holds its draw on the scan budget until its last frame is written
([F144](../../features/scan-pages-on-the-wire.md)), which is what keeps a slow client from growing a node's
memory; so a client that reads nothing keeps its pages' bytes drawn, and every other scan's page was shed
`Shedding` for the budget until it went. No get and no write was delayed: the budget is the scans' own.

Three things made it worse than a slow client's due:

- **One connection could hold the whole budget.** The item said eight pages' draws a connection, 32 MiB; the
  *After M24* plan's review found the bound counted a page only while it was read, so a connection's ninth page
  was read and answered beside eight unwritten, 36 MB on one connection, and its pages went on until the budget
  itself ran dry.
- **Nothing bounded how long an answer may wait unwritten.** A connection whose client stopped reading kept its
  draws until its keepalive found it gone, which a socket whose window is shut never does.
- **A connection its node had ended held them too.** Once its write relay ended, told a frame of its could not be
  right, its read relay waited on a client that sent nothing, and an answer queued meanwhile kept its draw.

## Cause

- **The per-connection bound counted the wrong thing.** A page was counted from its admission until its answer was
  queued to its connection's writer (`Walks::serve`, `unserve` at `ScanServed`, `shard/scan.rs`), so pages whose
  answers waited unwritten were counted by nothing but the budget.
- **The write relay had no clock on a scan's frame.** `write_all_slices` waits for the socket to take every byte,
  and a client that reads nothing takes none.
- **The relays ended apart.** The read relay ends when the client sends something or goes; the write relay's end
  told it only through the backlog, which it looks at only after it reads a frame.
- **Nothing counted what one tenant held.** The budget is the node's, and every connection drew on it alike.

## Evidence

**Established by reading the source and by a test** written for F144 (2026-11-10):
`a_scan_frame_that_cannot_be_right_ends_its_connection` first failed at *a page whose connection ended kept its
draw*, and `a_pages_answer_holds_its_draw_until_written` shows the held draw itself. **Each part was reproduced
before its fix** on 2026-11-15, in `shoal/tests/scan_frames.rs`, on the tree before it:

```text
thread 'a_connection_owes_no_more_than_eight_pages_written_or_not' panicked at shoal/tests/scan_frames.rs:870:13:
assertion `left == right` failed: ScanPageFigures { answered: 9, ended: [0, 9, 0, 0, 0, 0], in_rows: 9, in_partitions: 0, rows: 18045, bytes: 37751760, tablets_refused: 0, refused: 0, unauthorized: 0, shed: 0, cancelled: 0 }
  left: (9, 0)

thread 'a_client_that_reads_nothing_is_ended_at_the_stall_bound' panicked at shoal/tests/scan_frames.rs:956:9:
an answer its client took none of kept its draw past the bound

thread 'a_connection_its_node_ended_holds_no_draw_while_its_client_stays' panicked at shoal/tests/scan_frames.rs:1011:9:
a page of a connection its node ended held 4215688 bytes while its client stayed

thread 'a_stalled_connection_takes_no_share_of_its_link' panicked at shoal/tests/scan_frames.rs:1050:9:
a connection whose client takes nothing kept its share of its link

thread 'a_principals_unwritten_answers_hold_at_most_its_share_of_the_budget' panicked at shoal/tests/scan_frames.rs:1102:5:
assertion `left == right` failed: the budget ran dry before the principal's share was held to
  left: 1
 right: 0
```

## The fix

In three parts, committed apart, the last with [F148](../../features/scan-shares.md):

- **A connection owes at most eight pages, their answers written or not.** Each page takes a place among its
  connection's owed pages (`OwedPage`, `server/scans.rs`; `Walks::serve`), which rides its answer's body beside its
  draws until the last frame is written (`AnswerHolds` in `HeldRoot`); a cancel gives it back as it is read, the
  page found by its id while its answer waits unwritten, and the connection's end gives every one back. A
  connection's ninth page is shed by name.
- **An unread answer's wait is bounded, and an ended connection holds nothing.** The write relay watches each scan
  frame's write for what its client takes (`watch_scan_write`, `write_all_slices_taken`): none for a second and the
  connection stands aside on its link ([F147](../../features/scan-link-pace.md)), so it takes no share from those
  reading; none for `scans.stall_after`, 30 seconds unless set, and the connection is ended, warned once and
  counted (`ScanPageFigures::stalled`), every answer it owes and their draws let go. And once the write relay ends,
  for any reason, it shuts the read side of its socket while it still holds the write half, so the read relay ends
  and the connection's pages are given up.
- **One principal's unwritten answers hold at most half the scan budget** (F148): what an answer holds is counted
  against its principal until it is written, and a page whose principal holds half or more is shed by name before
  it draws. A shard's scan class is shared fairly among principals that contend for it, too.

## Alternatives rejected

**A node's bound a connection of its shards' scan classes.** Item 386's other cure: it would have given up any
bound on what one connection holds, which is this item.

**Ending a stalled connection from a sweeper with `shutdown`.** The blocked write would return an error the relay
logs at ERROR, and a sweeper must never touch a descriptor its relay has closed, since numbers are reused. The relay
watching its own write ends the connection itself, with one WARN.

**A timeout on every answer's whole write.** A paced answer among many connections on one link may take seconds a
frame and is reading all the while; what tells a stopped client apart is that it takes nothing, so the watch is of
bytes taken, not of time.

**A count a connection of unwritten bytes.** A tenant opens connections; the share is the principal's.

## Invariants to uphold

- **A page's place among its connection's pages lasts until its answer is written or dropped**, and is given back
  once: by the answer, a cancel or the connection's end (`OwedPage::release`).
- **A cancel finds a page whose answer waits unwritten**: owed pages are kept by id, weakly, apart from the pages
  being read.
- **A scan frame's write is watched by bytes taken**; a connection that takes none for `scans.stall_after` is ended
  by its own write relay, never from outside it.
- **A write relay that ends shuts its socket's read side while it holds the write half**, so the descriptor is its
  own when it does.
- **A connection standing aside on its link counts there again as soon as its client takes a byte.**
- **What an answer holds is counted against its principal by a guard in its body** (F148's invariants).

## Still open

Nothing of this item. The After M24 exit's review found the stall bound could be escaped by a connection blocked on
another kind of frame, or by one that took a little now and then:
[Resolved #397](every-frame-held-to-the-stall-bound.md), which watches every frame of a connection granted scans
against a floor of 64 KiB a bound. And it found one principal's burst of pages could pass its share:
[Resolved #396](principal-share-reserved-at-admission.md).

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_connection_owes_no_more_than_eight_pages_written_or_not` (`shoal/tests/scan_frames.rs`) | A connection that reads nothing has a ninth page read and answered beside eight unwritten |
| `a_cancelled_unwritten_page_frees_its_place_before_the_next` (`shoal/tests/scan_frames.rs`) | A cancel of an answer waiting unwritten gives its place back only when the blocked writer drops it, and the page behind it is shed |
| `a_client_that_reads_nothing_is_ended_at_the_stall_bound` (`shoal/tests/scan_frames.rs`) | An unread answer keeps its draw for as long as its client keeps its socket |
| `a_connection_its_node_ended_holds_no_draw_while_its_client_stays` (`shoal/tests/scan_frames.rs`) | A page of a connection its node ended holds its draw while its client sends nothing |
| `a_stalled_connection_takes_no_share_of_its_link` (`shoal/tests/scan_frames.rs`) | A connection that stopped reading keeps its share of its link from the connections reading |
| `a_principals_unwritten_answers_hold_at_most_its_share_of_the_budget` (`shoal/tests/scan_frames.rs`) | One principal's unread answers run the budget dry |
| `a_pages_answer_holds_its_draw_until_written` (`shoal/tests/scan_frames.rs`) | Were an answer to give its draw back before it is written, a slow client would grow a node's memory |

## Related

- [F144](../../features/scan-pages-on-the-wire.md), whose draw an answer holds, and [F148](../../features/scan-shares.md),
  the count a principal.
- [Resolved #386](scan-pages-held-to-the-bound.md), the client's half of a connection's bound.
- [F147](../../features/scan-link-pace.md), whose link share a stalled connection stands aside on.
- [Resolved #238](object-lane-backpressure.md), the stalled lane whose bound this copies.
