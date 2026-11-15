# F144. A scan's page on the wire

The scan frames were bytes nothing sent, and a page was read only by a caller in the same process.
The only scan a connection could ask for was the admin walk, an administrator's, answered in JSON with
no row in it.

Now a standalone node answers a `ScanPage` on any connection it granted scans to: it judges the page,
refuses by name what it will not serve, reads it, and answers a `ScanAnswer` whose body follows in data
frames written straight from the buffers the page was read into. A `Cancel` stops a page at its next
step, and a page's answer holds its draw from the scan budget until its last frame is written. Nothing
drives it yet but a test's raw socket: the client is M24's next step.

## Context

[W4](../future-work/scans/cursor-and-pages.md#a-page-on-the-wire) is the design, fixed by
[Q36](../future-work/scans/contract.md#q36-the-cursor-and-the-frames-2026-11-06). The frames are
[F140](scan-frames.md)'s, the page [F141](scan-pages-over-tablets.md)'s and
[F142](split-partitions.md)'s, its two bodies [F132](whole-partitions-in-answers.md)'s and
[F143](scan-rows.md)'s. This step joins them to a connection, and is the fifth of
[M24](../future-work/scans/whats-left-todo.md#finishing-m24).

It follows the object frames' path through a shard ([F80](object-wire.md)): a frame of its own type
read by the connection's read relay, a task on the coordinating shard, an answer streamed by the write
relay from parts.

## What it does

### The capability

A client asks for `CLIENT_CAP_SCANS` in its hello. A node grants it only when it answers scans, which
is a standalone node until a cluster activates its scans' wire version at M25, and only beside
`CLIENT_CAP_STREAMS` and `CLIENT_CAP_CANCEL` (`granted_caps`). A `ScanPage` on a connection that was
not granted it ends that connection.

### A page, from the socket to its shard

The read relay checks a page's frame against `MAX_SCAN_PAGE_FRAME` before it allocates for it, reads
it, and decodes it; a frame that cannot be right ends the connection there. A page that decodes goes
to the shard coordinating the connection (`handle_scan_frame`,
`shoal-core/src/server/shard/scan.rs`), which judges it in this order and answers the first refusal
with one `Error` frame under the page's id:

| The page | Is answered |
| --- | --- |
| Names a listing, or tablets cut by another count of bits | `InvalidRequest` |
| Names a table the schema does not have | `UnknownOperation` |
| Names a bucket's generated table, whoever asks | `GeneratedTable` |
| Names a table not in the committed schema | `UncommittedTable` |
| Comes from a principal without `scan` on the table | `Unauthorized` |
| Names a fingerprint that is not the table's | `WrongFingerprint` |
| Asks for partitions in a layout the node does not write | `UnknownLayout` |
| Names a bound past its most | `InvalidRequest` |
| Carries a spec that is not its table's, or asks partitions of a filtered or projected scan | `InvalidRequest` |
| Is one more than a connection may have in flight, eight | `Shedding` |
| Finds the scan class full, or no room in the scan budget | `Shedding` |

A page under an id its connection already has in flight is a frame that cannot be right: the
connection is told so and ended.

A page that passes is given a watch and an attempt, and a task of its own. The task draws the page's
first tablet's bytes from the scan budget in its turn, off the loop, hands the page to the executor
hosting its lowest tablet ([F141](scan-pages-over-tablets.md)), and waits for it for as long as it
keeps moving.

### The answer

A page that was read is answered `ScanAnswer`: the page's id, its head and sections, then its body in
`Data` frames under the same id. The body is handed to the connection's writer as the range an object
read's answer is: each chunk's head, each root where the page left it, and each root's padding as a
run of zeros, with nothing of a root copied (`PartitionBody::into_range`, `RowsBody::into_range`). The
writer cuts it into frames of at most `SCAN_DATA_FRAME_BYTES`, 256 KiB, and takes its other answers
between them.

**Every root holds the page's draw.** The draw goes back once the last root is dropped: when the
answer's last frame is written, or when the answer is dropped unwritten because its page was cancelled
or its connection ended.

### Cancel, and a client that leaves

`Cancel` names a page's id as it names a bundle's ([F75](client-cancel.md)). The coordinating shard
marks the page's watch before it records the cancel, so the page stops where it waits: its reads end
at their next step, its draws stop waiting for room, and its answer, if it had one, is never written.
The cancel is answered once, by one `Cancelled` under the page's id, whether or not the page had been
answered. A connection that ends gives up every page it had in flight the same way.

A page that makes no progress for a whole query deadline is given up by its own waiter and answered
`Timeout` under its id. ~~It was answered nothing~~: the page's watch had one mark for a cancelled
page and a given-up one, and a cancelled page's answer is dropped. The watch now says them apart
(`PageWatch::unask` for a cancel and an ended connection), and only an unasked page is answered
nothing ([Resolved #383](../appendix/resolved/stalled-page-answered.md)).

### What a node counts

`NodeStats.scans.client` (`ScanPageFigures`) counts the pages a node's connections asked: those
answered, by why each ended and by body, their rows and bytes, the tablets refused, and the pages
refused, shed, refused for want of `scan`, and cancelled. `scans.budget_used` is the scan budget's
bytes drawn now. A page's reads are still counted as the scan class's
([F123](walk-and-scan-counters.md)).

## Design choices

**The object answer's path, not a new one.** A scan's answer is a head and a range of parts, exactly
as a read of an object is, so the write relay's streaming, its interleaving, its cut on a cancel and
its backpressure hold for it with one more kind of reply (`ReplyKind::Scan`).

**The draw rides in the bytes.** A buffer the writer holds carries the permits it was read under
(`HeldRoot`), so nothing has to remember to give them back: when the writer is done with the bytes,
however it came to be done, they go.

**Refused by name where the node could have served another page.** Only what no client of this build
could send ends a connection. A client that asks too much, or of a table it may not scan, learns why
and keeps its connection, and its other pages.

**`GeneratedTable` before the grant.** A bucket's rows are reached only through its bucket, whoever
asks, an administrator too, as for a query ([F79](bucket-schema.md)).

**The grant is `scan`, never `read`.** A principal that may get a table's rows by key may not read it
whole unless it was granted that ([F140](scan-frames.md)).

**The first draw comes before the scan class's place.** A page waits its turn for the budget before it
is admitted, as an admin's scan page does ([F135](scan-budget-floor.md)), so it holds no place of
`scans.pages_in_flight` while it waits for room.

**A page nobody reads is bounded twice.** A connection has at most eight pages in flight, each of at
most 4 MiB of answer, and the scan budget bounds them all together: pages beside them are shed, and
the node's memory does not grow.

## Alternatives rejected

**Copying the body into one buffer for the writer.** A megabyte copied a page, on the shard's thread,
to save a few slices in a vectored write.

**Releasing the draw when the page is handed to the writer.** An answer waiting on a slow client would
hold memory no budget sees ([W4](../future-work/scans/cursor-and-pages.md#cancel)).

**A cancel that only drops the answer.** F75's cancel never stops work already running. A page can be
megabytes of reads; it checks its watch at every step, so it stops.

**Ending the connection for every refusal.** A driver runs many streams on few connections, and one
table's missing grant would end the scans beside it.

## Limitations

- **Standalone nodes only.** A cluster node grants no connection scans and answers none; floors,
  groups, read levels and routing by copy are M25's. A page's read level and floors are read and set
  aside.
- **No client sends a page yet.**
- ~~**An answer queued to a connection that does not read holds its draw** until the connection's read
  side ends too, which a silent client keeps open until its keepalive~~ Since
  [Resolved #380](../appendix/resolved/unread-scan-answers-bounded.md) a connection owes at most eight pages until
  their answers are written, one whose client takes nothing for `scans.stall_after` is ended, and a write relay
  that ends shuts its read side, so an unread answer holds its draw no longer than that.
- **A page's bound of eight a connection is a constant.**
- ~~**A count of scans a principal is not kept**: one principal's connections can take every place of
  the scan class ([W7](../future-work/scans/isolation.md)).~~ Since [F148](scan-shares.md) a shard's scan class is
  shared fairly among the principals contending for it, and one principal's unwritten answers hold at most half
  the scan budget.
- **A read level of `Quorum` is not refused on a standalone node**; it has one copy.
- **A spec that is not its table's is refused by name**, where W4 would allow ending the connection.

## Invariants to uphold

- **A node grants scans only where it answers them, and only beside streams and cancels.**
- **A page's frame is held to `MAX_SCAN_PAGE_FRAME` before anything is allocated for it.**
- **The refusals keep their order**: a generated table before the grant, the grant before the
  fingerprint.
- **A page is answered once**: by its answer, by one error frame, or by its cancel's.
- **Every root of a body holds the page's draw**, and nothing else gives it back.
- **A page's watch is marked before its cancel is recorded, and when its connection ends.** Only
  those two `unask` it; a page its waiter gave up is abandoned and still answered
  ([Resolved #383](../appendix/resolved/stalled-page-answered.md)).
- **A page is forgotten by its coordinating shard exactly once**, when its task says it is done.
- **No data frame of a scan's body is longer than `SCAN_DATA_FRAME_BYTES`.**
- **Nothing is awaited on the loop for a page** but the sends of its refusals.

## Performance

A query's path gained one comparison of a frame's type in the read relay and one arm in the loop's
dispatch. A scan's answer is written from the buffers its page was read into, as an object read's is.
~~Nothing was timed here: M24's exit measures the reference cell before and after these changes on a
Zen1 host, and a scan's rate over the wire on the lab.~~ The reference cell is measured since
2026-11-12, below; a scan's rate over the wire is the lab exit's
([M24's exit](../future-work/scans/m24-exit.md), run the same day).

### The reference cell before and after M24

The grid's reference cell (`macro/grid/unsorted/r50/1024`) on titan (Zen1, a Ryzen Embedded V1756B,
four cores) under `performance`, by
[the lab's procedure](../performance/benchmarking.md#before-and-after-on-the-lab), on 2026-11-12
(`shoal-spike-scan/results/m24-ab/`, its predictions written first):

- **Two builds of `shoal-workload`**, each for znver1 in a worktree of its own, against the same forks:
  - before, `5a1c162f`, the last commit before M24's first;
  - after, `01950968`, every step of M24 (F140 to F146) and its exit review's four fixes.
- **O156's scratch configuration**: two shards on cpus 2 and 4, the client on 6 and 7, read from a
  run's threads in the first round.
- **The host held still**: titan's tmdb node stopped, locked memory unlimited (no run's log warns of
  unregistered buffers), and the storage wiped before every run. Both were put back after.
- **Every run short**: 20,000 operations, two or three seconds, well inside the 970 EVO's rested
  flush ([Resolved #351](../appendix/resolved/the-cells-fall-is-the-drives.md)).
- **Eight rounds**, the first side alternating.

Two arms were run beside the cell, since half the cell's operations are writes that wait about 7 ms
on a sync and would hide a small cost a query: the same cell reading alone
(`macro/grid/unsorted/r100/1024`), and the storage-free control (`macro/get_ephemeral`). Neither is
the cell, and neither is judged as it.

Each side's median with its lowest and highest:

| Arm | Figure | Before | After | After ÷ before |
| --- | --- | --- | --- | --- |
| The cell | Operations a second | 8,087 [7,975–8,233] | 8,064 [7,888–8,209] | 1.00, within |
| | Read p50 µs | 171 [165–179] | 170 [164–178] | 0.99, within |
| | Read p99 µs | 393 [357–425] | 383 [355–408] | 0.97, within |
| | Read mean µs | 180 [174–192] | 181 [174–188] | 1.01, within |
| | Write p50 µs | 6,948 [6,776–7,023] | 6,962 [6,802–7,092] | 1.00, within |
| | Write p99 µs | 14,782 [14,098–16,109] | 14,896 [14,232–15,233] | 1.01, within |
| | Write mean µs | 7,307 [7,176–7,401] | 7,305 [7,189–7,513] | 1.00, within |
| Reads alone, 24 rounds | Operations a second | 71,414 [61,648–73,969] | 71,925 [63,220–74,244] | 1.01, within |
| | Read p50 µs | 389 [368–493] | 388 [370–468] | 1.00, within |
| | Read p99 µs | 825 [794–988] | 844 [791–1,003] | 1.02, within |
| | Read mean µs | 408 [392–480] | 402 [392–460] | 0.98, within |
| The control | Operations a second | 67,161 [59,732–69,676] | 66,494 [62,956–68,914] | 0.99, within |
| | Get p50 µs | 227 [214–265] | 229 [217–247] | 1.01, within |
| | Get p99 µs | 401 [364–443] | 402 [370–431] | 1.00, within |
| | Get mean µs | 227 [219–256] | 230 [221–242] | 1.01, within |

What this says:

- **M24 costs the cell nothing that this can see.** Every figure of the cell is within, its rate and
  its writes' mean the same to three figures.
- **Nor a read by itself.** After eight rounds the reads-only arm's mean stood 2.8% higher, its ranges
  overlapping, so it was run sixteen rounds more before anything was said of it: over the
  twenty-four its mean is 0.98, the after side the slower in eight of them. That was the rounds'
  noise: this arm's rounds range over a sixth of its rate.
- **What it cannot say**: a cost under about 2% of a read. The arms' ranges are that wide on a host
  whose runs are two seconds long, and a cost that small would need the criterion layer, not this.

## Tests

| Test | Where | Breaks if |
| --- | --- | --- |
| `a_page_given_up_for_no_progress_is_answered_a_timeout` | `shoal`, `scan_frames.rs` | A page its waiter gave up is never answered, and its client waits out its own 45 seconds ([Resolved #383](../appendix/resolved/stalled-page-answered.md)) |
| `a_scan_is_refused_without_the_capability` | `shoal`, `scan_frames.rs` | Scans are granted without streams or cancels, or a page is read on a connection not granted them |
| `an_unfiltered_scan_in_partitions_hands_out_the_rows_a_scan_in_rows_does` | the same | Either body loses or repeats a row over the wire, or a data frame passes 256 KiB |
| `partitions_are_refused_with_a_filter_a_projection_or_a_listing` | the same | A page the node will not serve is read, is refused under the wrong name, or costs its connection |
| `a_page_asking_a_layout_the_node_does_not_write_reads_nothing` | the same | A page of another layout is admitted, captured or drawn for before it is refused |
| `a_scan_frame_that_cannot_be_right_ends_its_connection` | the same | A malformed page, or one under an id in flight, is served, or ends another connection |
| `a_cancelled_page_stops_at_its_next_step` | the same | A cancelled page reads on, is answered twice, or keeps its draw |
| `a_pages_answer_holds_its_draw_until_written` | the same | An unwritten answer's draw goes back early, or a written or abandoned one's never does |
| `a_node_counts_its_scans` | the same | A page is not counted, or counted under the wrong end, body or refusal |

## Related

- [W4, the cursor and the pages](../future-work/scans/cursor-and-pages.md)
- [F80](object-wire.md), the path an object's answer takes; [F73](bodies-across-frames.md), the data
  frames; [F75](client-cancel.md), the cancel
- [F116](scan-admission.md), [F118](scan-budget.md) and [F135](scan-budget-floor.md), what admits a
  page and what it draws from
- [F140](scan-frames.md) to [F143](scan-rows.md), the steps before this one
