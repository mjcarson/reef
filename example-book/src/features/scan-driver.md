# F145. The client's scan driver

A standalone node answered a scan's page on the wire, and nothing asked one but a test's raw socket.

Now `client.scan(MovieScan::new())` reads a table whole. The client asks the node for pages, learns
which executor hosts which tablets from the pages' own refusals, hands out each page's rows once its
answer is whole and every chunk of it validated, and keeps a resume token a later run goes on from.

```rust
let mut rows = client.scan(MovieScan::new()).start().await?;
while let Some(movie) = rows.next().await {
    index.add(movie?);
}
```

## Context

[W4](../future-work/scans/cursor-and-pages.md) is the design of a scan's pages and its token, and
[W10](../future-work/scans/api.md#the-client) of the client's surface. The frames are
[F140](scan-frames.md)'s, the scan struct [F143](scan-rows.md)'s, and the node's half
[F144](scan-pages-on-the-wire.md)'s. This is the sixth step of
[M24](../future-work/scans/whats-left-todo.md#finishing-m24).

The driver follows the object client's shape ([F80](object-wire.md)): one frame of its own type on a
connection set apart for long streams, its answer routed back by its id, its body assembled from data
frames by the connection's reader.

## What it does

### A scan and its pages

`Shoal::scan` takes a table's `{Row}Scan` and returns a `ScanBuilder`; `start` returns a `Scan`
(`shoal-client/src/client/scans.rs`). Since [F146](shql-scan.md) both are typed by what a row is
handed out as, `ScanBuilder<'a, S, T>`, and a scan parsed from SHQL runs on the same driver.

| On the builder | Does |
| --- | --- |
| `level(level)` | Reads each page at a level, the table's policy unless named |
| `page(rows, answer_bytes, work)` | Bounds each page; zero is a page's default |
| `streams_per_shard(n)` | Asks `n` pages at once of each executor, two unless named |
| `tablets(set)` | Reads only some tablets |
| `split(n)` | Cuts the scan into `n` builders over disjoint ranges of tablets |
| `patience(time)` | How long the scan goes on asking with no page moving it on, a minute unless named |
| `reads()` | The tablets the scan reads |

| On the scan | Does |
| --- | --- |
| `next()` | The next row, or none once the scan has ended |
| `next_page()` | The next page of rows, whole; a page may hold no row |
| `checkpoint()` | The scan's resume token |
| `rows_past_checkpoint()` | How many rows `next` has handed out of a page the token does not cover yet |
| `stats()` | Pages, rows, bytes, pages asked again, tablets refused as hosted elsewhere, the longest data frame |

A scan's rows come back as the scan's projection, in no order across tablets. Its limit is kept by
the client: a scan that has handed out its limit asks no more and cancels what it has in flight.

A scan answered in partitions (`MovieScan::new().partitions()`) is decoded into the same rows. A
scan with a filter or a projection that asks for partitions is refused by the client before a page is
sent, as the node would refuse it.

### Streams, learned from refusals

A client of a standalone node knows no executor. The scan begins as one **stream** over every tablet
it reads. A page asks its stream's tablets; the executor hosting the lowest reads its own and refuses
the others `HostedElsewhere`. The refused tablets leave the stream and become a stream of their own.
After as many pages as the node has executors there is a stream for each.

A stream has one page in flight. Once a page of it comes back with no tablet refused, the stream is
one executor's alone, and its tablets are shared out over `streams_per_shard` streams, each over
tablets of its own. A shard's own admission (`scans.pages_in_flight`) has the last word: a page past
it is shed and asked again.

### What a page's answer is held to

The connection's reader assembles an answer's body from its data frames into a buffer aligned to
sixteen, at most `SCAN_BODY_BOUND` long, and hands the answer and its body to the scan that asked.
The scan then:

1. holds the answer to the page it asked (`ScanAnswer::check_against`): its id, body, layout, tablet
   bits, every refused tablet one it asked, where it reached inside its tablets and no further back
   than its cursor, and a body no longer than twice the page's bytes and one row;
2. reads every chunk of the body (`RowsPage::read`, `PartitionPage::read`), which refuses all of it
   unless every chunk validates;
3. only then moves its cursors and queues the page's rows to be handed out.

An answer that fails any of these ends the scan with an error that names what was wrong. None of that
page's rows were handed out, and the scan's token resumes it from the pages before.

### Pages asked again

A page that was shed, timed out, or whose connection ended is asked again from where it began under
a fresh page id, after a wait that doubles from 2 ms to 250 ms. None of its rows were handed on, so
nothing is handed out twice. A page not answered within 45 seconds is given up and cancelled on the
connection that owes it. A scan with no page moving it on for its patience ends by the refusal it
last met. Its patience counts what the scan waits, from when a page last moved it on or its caller
last began to wait for one: ~~the time a caller took between pages counted too~~ a caller's own pause
between pages is never counted ([Resolved #384](../appendix/resolved/scan-patience-counts-its-own-wait.md)).

A page that hands out no row, or covers nothing because its budget or deadline was met first, never
ends a scan. A scan ends when every tablet it reads is finished.

Any other refusal ends the scan by name: `Unauthorized`, `WrongFingerprint`, `InvalidRequest` and the
rest are answered the same however often they are asked.

### The resume token

`checkpoint()` returns [F140](scan-frames.md)'s token: the scan's identity, the tablets not finished
and a cursor for each tablet read part way. It covers **exactly the pages handed out whole**. A page
`next` is part way through is not covered until its last row is taken; `rows_past_checkpoint` says
how many rows that is, which a scan resumed from the token hands out again.

`Shoal::resume(token, scan)` goes on from a token on any client. The token is held to the scan it is
handed by this client's own build of the table, before any page is sent:

| The token | Is refused |
| --- | --- |
| Does not check, or is too short | `TokenError::Damaged` |
| Is of a version this build does not read | `TokenError::UnknownVersion` |
| Was taken from a scan of another table, or another build of this one | `TokenError::WrongFingerprint` |
| Was taken from a scan with another filter, projection, level or body | `TokenError::WrongSpec` |

### A dropped scan

Every page in flight holds a guard. A scan that is dropped, ends by a refusal or meets its limit
drops its pages, and each guard cancels its page on the connection that owes it
([F75](client-cancel.md)), which the node stops at its next step.

## Design choices

**Executors are learned, not told.** A standalone node's hosting is its own business
([F47](local-rehome.md)) and may change at a restart. A stream that reads what it is given and is
told what it was refused needs no map, costs one extra page an executor at the scan's start, and
keeps working across a rehome.

**The reader assembles, the scan judges.** The connection's reader does not know which page an
answer is for, so it bounds a body by a constant and the scan holds the answer to its page afterwards.
The alternative, a table of pages in flight the reader consults, would put a lock on every data
frame's path.

**A fresh id every ask.** A cancel covers arrivals under its id before it. An ask given up and asked
again under the same id could be cancelled by its own predecessor's cancel.

**The limit is the client's.** A node's page knows nothing of the pages before it, and a limit
shared out over streams would need a count kept between them. The client stops when it has enough.

**The token is not signed.** [Q36](../future-work/scans/contract.md#q36-the-cursor-and-the-frames-2026-11-06)
settled this: every page a resumed scan asks is authorized on its own, so a forged token reads
nothing its bearer could not scan anyway.

## Alternatives rejected

**Handing rows on as chunks arrive.** It lowers the time to a first row and makes a page cut off
part way hand rows out twice when it is asked again, unless the client deduplicates by key.

**One page a connection, held until answered.** The object client first did this and every other
operation queued behind it (item 230). A page's connection goes back to its pool once its frame is
written.

**Marking the connection dead on a page that timed out**, as an object attempt does. A scan's
connection always has cancel, so the page alone is cancelled and the pages of other streams on that
connection are left alone. A connection whose server stopped reading is still discarded when a
page's frame cannot be written.

**Asking each tablet in a page of its own.** 4,096 pages a scan of an empty table, where one page an
executor finishes every tablet its counters call empty.

## Limitations

- **A standalone node only.** A cluster node grants no `CLIENT_CAP_SCANS` until
  [M25](../future-work/scans/milestones.md); a scan of one is refused by the client by name.
- **No order across tablets**, and none promised within one beyond ascending keys.
- **A page is held whole in memory** before its rows are handed out: at most twice its bytes and a
  row, a megabyte or so by default, a stream.
- ~~**The reader bounds a body by `SCAN_BODY_BOUND`**, 72 MiB, before the scan can hold it to its page.
  A node that lied about a body's length could make a client allocate that much a page before the
  scan refused it ([item 388](../appendix/known-issues.md)).~~ Since
  [Resolved #388](../appendix/resolved/scan-body-held-to-its-page.md) **the reader holds a body to its own
  page's bound** before it allocates, the bound a page's slot carries (`Shoal::track_page`); a body past
  it is dropped unassembled and the scan ends by name.
- ~~**A scan asks two pages an executor, and a node serves eight a connection**: on a node of more
  than a few executors the pages past eight are shed and asked again
  ([item 386](../appendix/known-issues.md)).~~ Since [Resolved #386](../appendix/resolved/scan-pages-held-to-the-bound.md)
  **a scan holds each connection to the node's eight**: a page takes a place on a connection with room before
  it is written, the reader gives it back as it delivers the answer, and a scan's pages have connections of
  their own, `StreamConfig::scan_connections`, eight unless set. **Every scan of a client shares the places**,
  and an ask with none waits for one.
- **A scan ended by its limit has a token that covers its whole last page**, the rows past the limit
  with it. A limit ends a scan; it is not a place to resume from.
- **`rows` is a soft bound**, as the node's is: a page may pass it by a partition.
- **The client keeps no floors.** A standalone node has no replicas to read behind; floors are
  [M25](../future-work/scans/milestones.md)'s.
- **A token names tablets by twelve bits.** A scan resumed after a change of the tablet count is
  M25's concern, and the frames carry the bits for it.
- ~~**A client that never reads holds its pages' draws** on the node (item 380), eight pages a
  connection at most.~~ Since [Resolved #380](../appendix/resolved/unread-scan-answers-bounded.md) it holds them
  for `scans.stall_after` at most, and its principal at most half the node's scan budget
  ([F148](scan-shares.md)).

## Invariants to uphold

- **No row of a page is handed on until its whole answer is in, held to the page that asked it, and
  every chunk of it validated.**
- **A short or empty page never ends a scan.** Only every tablet finished, the limit met, or a
  refusal does.
- **A checkpoint covers exactly the pages handed out whole**, never one in flight, read and waiting,
  or handed out in part.
- **Every ask has a fresh page id.**
- **A page's count of what is owed is held by its guard**: every way an ask ends early cancels it.
- **A stream has one page in flight, and no tablet is in two streams.**
- **Every frame is written through `ConnShared::write`**, so a queued cancel goes first.

## Performance

The driver adds nothing to a get's or a write's path but one arm of the reader's match on a frame's
type. ~~A scan's own rate and cost are measured at M24's lab exit, on the scan's own client and link;
no figure is claimed here.~~ [M24's lab exit](../future-work/scans/m24-exit.md) measured them
(2026-11-12, titan the node, europa the client over 1 GbE, `performance`): a scan of a million
archived kibibyte rows took 59.4 s alone, 16.7 MiB a second, its device's share, and 270 s beside the
reference cell on the same client; of resident rows 9 s, 110.6 MiB a second, the link. The driver's
cpu was 0.69 µs a row. At its default of two streams a shard on a node of five shards a page was shed
for nearly every page answered ([item 386](../appendix/resolved/scan-pages-held-to-the-bound.md), resolved
since by the client holding its pages to the node's bound), which cost the node cpu and the scan no time.

## Tests

`shoal/tests/scans.rs`, each against a standalone node in process:

| Test | What breaks if the feature is reverted |
| --- | --- |
| `a_short_or_empty_page_never_ends_a_scan` | A filtered scan in small pages stops at its first empty page |
| `a_callers_pause_is_not_counted_against_a_scans_patience` | A scan whose caller paused past its patience ends at the next page cut off or shed ([Resolved #384](../appendix/resolved/scan-patience-counts-its-own-wait.md)) |
| `a_resume_token_resumes_and_is_refused_for_another_table` | A token resumes nothing, hands out a page twice, or resumes another table's scan |
| `a_page_cut_off_mid_answer_is_asked_again_and_hands_out_nothing_twice` | A page whose connection ended half way through its body ends the scan or repeats rows |
| `an_answer_that_cannot_be_right_ends_its_scan_by_name` | A chunk whose head lies hands rows out, or the scan cannot be resumed |
| `a_tablet_another_shard_hosts_is_asked_in_a_stream_of_its_own` | A node of two executors is read by one stream, or half its tablets never |
| `a_scan_split_in_parts_reads_every_tablet_once` | Parts overlap or leave a tablet out |
| `a_scan_hands_out_every_live_partition_once` | A scan beside writes breaks P20 to P22, judged by `shoal_model::scan::check` |
| `a_scan_needs_its_grant` | `read` alone scans |
| `a_get_with_no_keys_is_still_refused` | A get that names no key reads its table |
| `a_scan_frame_under_ktls_is_written_in_pieces` | A data frame of a scan's body passes `SCAN_DATA_FRAME_BYTES` |

## Related

- [F140](scan-frames.md), [F143](scan-rows.md), [F144](scan-pages-on-the-wire.md): the frames, the
  scan struct and the node's half.
- [F75](client-cancel.md): what a dropped page's cancel is.
- [W4](../future-work/scans/cursor-and-pages.md), [W10](../future-work/scans/api.md): the design.
- [Resolved #380](../appendix/resolved/unread-scan-answers-bounded.md): a client that never reads.
