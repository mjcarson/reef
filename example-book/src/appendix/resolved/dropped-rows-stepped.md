# 382. A scan in rows held its shard for every row its filter dropped

## Symptom

A scan answered in rows whose filter kept little held its table shard's thread for a whole tablet
page at a time. A page may read up to 4 MiB of partitions; a filter that kept none of their rows read,
validated and compared every one of them in one run, with nothing else on the shard served until it
ended. A filter of many values made each row dearer, and a spec may carry 64 KiB of them.

Any principal holding `scan` on a table could ask such a page, again and again.

## Cause

The rows encoder (`shoal-core/src/server/tables/walk/rows.rs`, `encode`) gathers the rows that pass a
filter into chunks of about 64 KiB and tells the page's steps of each chunk as it is written, in
`flush`. That was the only place it told them. A row the filter dropped added nothing to its chunk,
so a run of dropped rows never reached `flush`, and the page's steps, which are what offer a scan's
core up ([F117](../../features/scan-class.md)), were told of nothing.

The module's own header said a page's rows never hold the loop for more than a step's bytes. That
held only for the rows a filter kept.

A filter's cost is also not in a row's bytes. The generated check is a linear search of each
filtered field's values, so a row of a hundred bytes held to eight thousand values is eight thousand
comparisons.

## Evidence

**Found by reading the source**, by both independent readers of M24's exit review (2026-11-12), **and
reproduced.** `a_filter_that_keeps_nothing_still_moves_its_scan` in `shoal/tests/scan_rows.rs` was
given a count of the bytes a page's steps were told of and run against the tree before the fix:

```text
thread 'a_filter_that_keeps_nothing_still_moves_its_scan' panicked at shoal/tests/scan_rows.rs:455:9:
a page that read 65528 bytes and kept none told its steps of 0
```

## The fix

Every row read is counted into the page's steps by its share of its piece's bytes, kept or not,
before the filter is asked. A row held to a filter then offers the core up once more
(`Steps::offer`, `shoal-core/src/server/steps.rs`), whatever was counted: the check is a read of the
executor's own state, and yields only when something waits.

`RowsBody::stepped` is what a page's steps were told of while its body was made, which the test
reads.

## Alternatives rejected

**Counting a dropped row into its chunk's bytes.** Chunks would end early and empty, and a chunk of
no rows is no chunk; the bound that matters is the loop's, not the chunk's.

**A cap on a filter's values.** A spec is already held to 64 KiB, and a get's filter has no such cap.
A cap changes what a scan may ask, which is the contract's to say; the hold is cured without it.

**Weighing a row by its filter's values.** The encoder is generic over a table's filter, which names
no count of its values, and a weight is a guess where an offer after each row is exact.

## Invariants to uphold

- **Every row a page reads is a step's work, kept or not.** A new reason to drop a row, or a new
  body, counts the row before it decides.
- **Work whose cost its bytes do not say offers the core up itself** (`Steps::offer`).
- **A driver's page never yields**: `offer` and `take` do nothing for steps that do not yield.

## Still open

One row held to a filter of thousands of values is still one run, tens of microseconds at a spec's
largest. A piece's validation is one run too, as it was: a record handed out where its read put it
may be a mebibyte ([F133](../../features/bounded-records.md)), and
[O173](../optimizations.md#o173-a-record-is-validated-up-to-three-times-on-a-page-that-cuts-it) is
the walk validating it more than once.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_filter_that_keeps_nothing_still_moves_its_scan` (`shoal/tests/scan_rows.rs`) | A page that read 64 KiB and kept nothing told its steps of none of it |

## Related

- [F143](../../features/scan-rows.md), whose encoder this is, and [F117](../../features/scan-class.md),
  whose steps it takes.
- [Item 373](whole-record-holds.md): the other way a page held its shard, through a whole record's
  verify.
