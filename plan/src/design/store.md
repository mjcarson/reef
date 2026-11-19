# The store

## Context

Shoal stores rows of Rust types declared with derive macros: `ShoalSortedTable` for many rows
per partition ordered by a sort key, and `ShoalUnsortedTable` for one row per partition. reef
builds against shoal's `ScanSupport` branch ([Shoal in the build](architecture.md#shoal-in-the-build)).
Its limits there shape reef's rows:

- **No client table scans yet.** The branch rebuilds the engine's walk for scans (M23 there),
  but the client side (M24 there) is not built. Until it is, anything reef lists needs a known
  partition key.
- **No secondary indexes and no transactions.** A bundle is a batch, not a transaction.
- **Sort keys are a single `String`.** Integer and composite sort keys don't compile: `RkyvSupport`
  is implemented for `String` alone (item 207 there). Composite *partition* keys do compile.
- **Partition keys hash to a `u64`, and collisions go unnoticed.** In an unsorted table a
  collision silently overwrites a row.
- **No table migrations yet.** Shoal can't yet remove, rename or retype a row's field. It will
  migrate tables later. Until then reef ignores the problem while it is in development, and a
  schema change may need a fresh data directory.
- **Conditional writes exist.** `if_absent` and `if_matches(filter)` give compare-and-set.

## Rows and versions

### Design choices

- **Listing partitions with known keys.** All projects sit in one constant partition, sorted by
  name. A project's plans sit in a partition of that project's, sorted by plan name. A plan's
  nodes sit in a partition of that plan's revision, sorted by id.
- **A node's spec as one serialized field.** The YAML (or JSON) of its `NodeSpec` sits beside the
  few fields reef filters or sorts on: id, status, group and its written order. reef's keys can
  then change without changing a shoal table.
- **A written `order` field.** Shoal sorts a partition by id, but ready work and layout fall
  back to the order a plan was written in. Without it, an imported plan would reshuffle.
- **Versions by conditional write.** Each node row carries a version, and a put names the one it
  read. A stale write is refused with `if_matches`.
- **Revisions behind a pointer row.** A bulk write (an import, a whole-plan edit) writes a new
  revision's rows, then makes it current with one conditional write to the plan's pointer row.
  A write killed midway leaves the old revision whole.
- **Every row checked against its full key on read**, so a hash collision is caught instead of
  being served as the wrong row.

### Open questions

- What a 202-node plan costs to load and to rewrite as a revision, measured on the example book
  (X2, `plan-load-ms`).
- How old revisions are pruned, with no scans to find them: the pointer row can keep a list of
  the revisions still kept.
- Whether the listing partitions give way to scans once `ScanSupport`'s client side (M24 there)
  is built.

## The schema

### Design choices

- **One `#[shoal::db]` for reef**, in a schema crate that both the service and any shoal node
  program use. Client and server must agree on its fingerprint, and a test pins it so a change
  is deliberate.
- **Each table arrives with the node that needs it.** The queue, work records, comments and
  loop state each add a table. While reef is in development a table may also change, at the
  cost of a fresh data directory, until shoal can migrate it.
- **Persistent tables throughout.** Ephemeral tables are lost on restart and never evicted.

## Ranks

The user's queue is an ordered list that changes one entry at a time. With a `String` sort key
and no transactions, rewriting every row's position on each move would be slow and unsafe.

### Design choices

- **Fractional rank strings.** Each entry's sort key is a rank such as `a0`, `a0V` or `a1`, and
  a new rank can be made before the first, after the last, or between any two. Moving an entry
  rewrites only that entry's row.
- **Rebalanced when long.** When ranks grow past a length, the whole partition is rewritten with
  short ranks, as one revision.
- **One queue per project, spanning its plans.** Each entry names a plan and a node, and a
  loop can be narrowed to one plan.
