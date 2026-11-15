# F143. A scan answered in rows, and a scan struct beside each get

A scan's page handed out its table's partitions as they lie, the whole of each, every field of every
row. Nothing could ask a scan for only the rows a filter matches, or only the fields a projection
names, and nothing a caller could write named a scan at all.

Now a page is answered in rows unless it asks for partitions: each row held to the scan's filter and
built as its projection on the shard, where the rows are, written as a get's answer in chunks a client
validates before it reads any of them. The table derives generate `MovieScan` beside `MovieGet`: a get
without keys, with the same filter, limit and projection. Nothing sends one over a wire yet.

## Context

[Q35](../future-work/scans/contract.md#q35-what-a-scans-answer-carries-2026-11-06) settled what an
answer carries, with the user, on 2026-11-06: rows encoded on the shard for every scan by default, and
partitions for an unfiltered, unprojected scan that asks for them. [F132](whole-partitions-in-answers.md)
built the body in partitions. This is the other body, and the request's half of both: the spec a page
carries, and the refusal of partitions for a scan that filters or projects.

It is the fourth step of [M24](../future-work/scans/whats-left-todo.md#finishing-m24), over the page
[F141](scan-pages-over-tablets.md) reads and the split [F142](split-partitions.md) cuts.

## What it does

### The spec, and the scan a caller builds

`ScanSpec<R>` is what a scan asks of each row: `filters`, the table's own `R::Filters`, and
`projection`, its `R::Projection`, the shapes a get carries
(`shoal-proto/src/shared/queries/scan.rs`). It archives with rkyv, and a page carries its bytes.
`ScanSpec::read` reads one a peer sent: in a buffer aligned to sixteen, validated as exactly its own
bytes with a bound on how deep its pointers nest, before anything of it is used. A spec is **plain**
when it has no filter and its projection is the whole row.

`TableScan<R, P>` is what a caller builds, and the derives name it: `MovieScan`, with `P` the row
until a projection is named.

```rust
let scan = MovieScan::new()
    .filters(MovieFilter { year: Some(vec![1982]), ..Default::default() })
    .limit(100)
    .projection::<MovieTitle>();
```

`.projection::<Q>()` changes the scan's type, so what its rows come back as is known where it is
built, and a projection of another table does not compile. `.partitions()` asks for the table's
partitions as they lie. `.spec()` is the scan's `ScanSpec`.

### Rows, written on the shard

`ScanAsk::in_rows(spec)` asks a page in rows. As each of the page's tablets is read, its partitions go
through `ShoalDatabase::scan_rows`, which the schema macro generates: it reads the spec as its table's,
and dispatches on the spec's projection to `rows::encode::<Row, Projection>`
(`shoal-core/src/server/tables/walk/rows.rs`).

The encoder reads each partition as the walk handed it out, validated where it lies, and takes its
live rows in order. A row the filter drops is counted as read and goes no further. A row that passes is
gathered: the whole row as a reference to where it lies in its partition's archive, never materialized,
or a projection built from it, only the fields it names read. That is a get's own path
([`RowRef`](grouped-responses.md), `Rearchive`, `ShoalProjection::from_archived`), so a scan adds no
encoder of rows.

Once the rows gathered hold about a step's bytes, 64 KiB, they are written as one chunk: a
`GetRows<P>`, the rows and the partitions each run of them came from, behind a chunk head of kind
`Rows`. Each chunk is drawn from the scan budget before it is made and its draw shrunk to what it came
to; the page's steps are taken after each, so no more than a step's bytes of rows are written without
yielding. ~~That was the only place they were taken, so a filter that kept little read a whole tablet
page of rows without yielding~~ Since [Resolved #382](../appendix/resolved/dropped-rows-stepped.md)
every row read is counted into the page's steps too, kept or not, and a row held to a filter offers
the core up after it. The partitions the rows came from, and what they drew, are let go as their tablet is done.

`RowsBody` is the page's answer: its chunks, the rows and partitions handed out, and the rows and
partitions read before the filter, which the answer's head carries.

### Bounds, with a filter

A page's answer bytes are its chunks'. Its work is what it read. So a page whose filter keeps little
reads on from tablet to tablet until it has read its work, 4 MiB unless asked, and ends there: with few
rows or none, and a position that moved.

### What is refused

At a page's admission, before anything is read: a spec that is not its table's filter and projection;
a page in rows with no spec; a page in partitions whose spec is not plain. Each is `InvalidRequest`, by
name. A bucket's generated table has no scan.

### The client's reader

`RowsPage<P>::read` is the twin of F132's `PartitionPage`
(`shoal-proto/src/shared/protocol/scan/rows.rs`), held to the same rule: nothing is allocated past
what the body can hold, every chunk is validated with bytecheck as exactly its own bytes, and no row
is handed out unless every chunk passed. A chunk's rows are its head's, its groups cover exactly its
rows, its partitions' keys never step back within a chunk or across them nor pass `reached`, and the
body's rows and partitions are those its answer declared. `keyed_rows` hands each row out beside the
key of the partition it came from.

## Design choices

**The spec is generic, and the derive names it.** `ScanSpec<R>` and `TableScan<R, P>` are written once
in `shoal-proto` over the table traits, as `UnsortedGet<R>` is, and a table's derive adds one line: a
type alias. So a schema that compiles against the client alone has its scans, and no generated code has
to be kept in step with the reader.

**A scan is typed by its rows.** A get's projection is a value, and its answer is read as whichever
type the caller says. A scan hands rows to a caller a page at a time over a long life, so the type is
fixed where the scan is built.

**The projection is chosen once a page's tablet, not once a row.** The schema's dispatch matches the
spec's projection and calls an encoder generic in it, as a get's dispatch does.

**A chunk is filled by what its rows hold where they lie.** The size of a serialized chunk is known
only once it is made, and a row's share of its partition's bytes is known before. So a chunk is about a
step's bytes, a little over or under, and the client holds a chunk to its body, not to 64 KiB.

**Rows are written tablet by tablet, inside the page.** A page's bounds are on what it answers and what
it reads. Only the encoder knows what a filter left, so it runs between a tablet's read and the next
tablet's bounds.

~~**A page whose rows cannot be written is refused whole.** Its partitions were read, but none of its
rows was handed out and its scan's cursors have not moved, so asking it again loses nothing. Ending it
short would have to say how far its rows got, which its partitions' position does not.~~ **A page whose
rows cannot be written is refused whole; one whose chunk the scan budget sheds ends short** since
[Resolved #389](../appendix/resolved/rows-page-shed-answers-what-it-covered.md): its body marks where it
holds each partition whole, which says how far its rows got, and a shed cuts it back to the last mark,
refusing the page only when it covered nothing.

## Alternatives rejected

**A scan variant of `QueryKinds`.** A scan answers more than once and routes by tablet
([W4](../future-work/scans/cursor-and-pages.md#alternatives-rejected)); and a new variant would move
every schema's fingerprint.

**A generated `{Row}Scan` struct, as `{Row}Get` is.** It would archive to the same bytes as the generic
spec and need a second copy of the builder in the derive for each table kind.

**Filtering on the client.** Every row of a table would cross the wire to be thrown away.
[X17](../future-work/scans/page-rebuilt.md#rows-encoded-on-the-shard-against-walked-partitions) priced
the shard's side at 80 to 280 ns a row.

**Rows encoded after the whole page is read.** A filtered page would have read to its answer's bytes
in partitions, and then answered a fraction of them.

## Limitations

- **Nothing sends a scan.** The page's frames are served by M24's next step and driven by the one
  after.
- **A scan's limit is the driver's.** A page knows nothing of it; the client stops asking once it has
  handed out that many rows.
- **A projection not declared on its table is answered as the whole row**, as a get of it is
  (`#[shoal(projections(...))]`).
- **A chunk is about 64 KiB, not at most.** One row larger than that is a chunk of its own.
- **A row is validated twice**: by the walk as part of its partition, and by the encoder when it reads
  the partition as rows.
- **A row's own map field is unguarded** against [item 368](../appendix/known-issues.md) in a body in
  rows, as it is in one in partitions.
- **A filter is a get's**: a set of values a field, the fields joined by `AND`.

## Invariants to uphold

- **A spec a peer sent is validated, with a bounded depth, before anything of it is read.**
- **Only a plain spec is answered in partitions.**
- **A row the filter drops is never in a chunk, and is counted as read.**
- **A chunk is drawn before it is made, and the page's steps are taken after it.**
- **Every row read is a step's work, kept or not**, and a row held to a filter offers the core up
  after it ([Resolved #382](../appendix/resolved/dropped-rows-stepped.md)).
- **A chunk's groups cover exactly its rows, in key order**, a partition that goes on from the chunk
  before first.
- **A chunk ends a key (`THROUGH`) only when that partition's last row is in it**: never the partition
  a page stopped inside.
- **The client hands out no row of a page until every chunk of it validated.**
- **`{Row}Scan` adds nothing to a schema's query enum**, so no fingerprint moves for it.

## Performance

Nothing on a query's path changed, and a scan in partitions does what it did. A scan in rows adds the
encoder's work to each page, a chunk a step in the scan queue. Nothing was timed here. X17 measured
the shape before it was built: rows cost the shard 80 to 280 ns each, 1.3 to 2.9 times what decoding
partitions costs a client. ~~M24's lab exit measures a client's cpu a row in rows and in partitions, and
the bytes each puts on the wire.~~ [M24's lab exit](../future-work/scans/m24-exit.md#what-a-row-costs-a-scan-alone)
measured it (2026-11-12, titan the node, europa the client, `performance`), a scan alone of a million
kibibyte rows, one a partition: in rows a row cost the client 0.69 µs and 1,085 bytes of wire, in
partitions 0.77 µs and 1,133 bytes; a filter keeping a row in a hundred and a projection of two
fields each read every record at the unfiltered scan's rate, and put 1% and 3.4% of its bytes on the
wire.

## Tests

| Test | Where | Breaks if |
| --- | --- | --- |
| `a_scan_in_rows_hands_out_the_rows_its_table_holds` | `shoal`, `scan_rows.rs` | A scan in rows misses a row or hands one out twice, resident or archived, or a chunk passes a step by much, or a body keeps a draw past what it holds |
| `a_filter_and_a_projection_are_applied_on_the_shard` | the same | A row the filter drops is handed out, a projection comes back as the whole row, or the answer does not say what was read before the filter |
| `a_filter_that_keeps_nothing_still_moves_its_scan` | the same | A page that keeps nothing reads past its work, answers bytes, or leaves its scan where it was |
| `a_sorted_table_is_answered_in_rows_across_a_split_partition` | the same | Rows of a partition cut across pages are lost or repeated, or come out of sort order |
| `a_page_whose_body_and_spec_do_not_go_together_is_refused` | the same | Partitions are answered for a filtered or projected scan, or a spec that is not its table's is read |
| `a_scan_struct_carries_a_gets_filter_and_projection` | `shoal-client-check`, `client_half.rs` | The derive stops generating a scan, it loses its filter, limit or projection, or it needs an engine to compile |
| `a_client_reads_a_scans_rows_without_an_engine` | the same | The rows reader needs an engine, or reads a body its answer did not describe |
| `a_page_of_rows_hands_out_its_rows_in_order` | `shoal-proto`, `scan/rows.rs` | Rows lose their order or their partition's key, or a partition across two chunks is counted twice |
| `a_page_of_rows_that_breaks_a_rule_hands_out_none` | the same | Any one check of a body in rows is dropped |
| `every_byte_of_a_body_in_rows_flipped_never_panics` | the same | A flipped byte panics the reader, or reads as a page its answer did not describe |
| `the_client_halfs_ids_are_frozen` | `shoal-client-check`, `client_half.rs` | A scan struct moves a schema's fingerprint |

## Related

- [Q35](../future-work/scans/contract.md#q35-what-a-scans-answer-carries-2026-11-06) and
  [W4, the body](../future-work/scans/cursor-and-pages.md#the-body-rows-or-partitions-on-request)
- [W10, the client](../future-work/scans/api.md#the-client), which this is the scan struct of
- [F132](whole-partitions-in-answers.md), the other body
- [F2](projections.md) and [F27](grouped-responses.md), a get's projections and its answer's shape
- [F141](scan-pages-over-tablets.md) and [F142](split-partitions.md), the page and the split
