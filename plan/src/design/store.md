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
  nodes sit in a partition keyed by project, plan and revision, sorted by id.
- **A node's spec as one JSON field.** The JSON of its `NodeSpec` sits beside the few fields reef
  filters or sorts on: id, version, status, group and its written order. reef's keys can then
  change without changing a shoal table. JSON decodes ten times faster than YAML and every spec
  reads back equal. YAML stays the form the CLI prints and reads.
- **A node a row.** A book keeps most nodes inline in a group's `nodes:` list. Each member gets a
  row of its own, its `group` filled in as the group that listed it.
- **A written `order` field.** Shoal sorts a partition by id, but ready work and layout fall
  back to the order a plan was written in. Without it, an imported plan would reshuffle. A
  member's order follows its group's, depth first.
- **Versions by conditional write.** Each node row carries a version, and a put names the one it
  read with `if_matches`. A new node is written `if_absent`. A stale write is refused as shoal's
  `Refused { reason: RowMismatch }`.
- **Writes to a plan serialized in the service.** reef-service is the only writer, so a lock per
  plan in its process keeps a put from landing in a revision an import is about to retire. The
  conditional writes still refuse a client's stale edit, and a second writer's change to the
  plan's row.
- **Revisions behind the plan's own row.** The plan's listing row is its pointer row, with
  `current`, `pending` and `retired` revisions and a version. A bulk write (an import, a
  whole-plan edit) works in these steps:
  1. Name the new revision `pending`.
  2. Write its rows.
  3. Flip, in one conditional write: the new revision `current`, the old one `retired`.
  4. Prune the old one.

  The next write prunes whatever a killed write left named in either. An import moves every
  node's version on, so an edit read before it is refused after it.
- **Every write's answer checked.** Bulk writes go through shoal's `exec`, and single ones
  through `send_one`. Draining a bundle's stream would accept a refused or failed write.
- **Every row checked against its full key on read**, so a hash collision is caught instead of
  being served as the wrong row.
- **Only the current revision is kept.** Pruning lists a revision's keys through a projection and
  deletes each row in one bundle, since shoal can't delete a partition. Shoal answers a delete
  of a row that isn't stored as `QueryDidNotSucceed`, a query that found nothing, which the
  store reads as done, so a prune that is tried again is harmless. A key from another partition
  is refused before anything is deleted, since deleting by this revision's key would delete
  the other's row.
- **A failed write is left as a killed one would be.** The store cleans nothing up when a write
  fails partway. The rows it left are named on the plan's row either way, and the next write,
  a put or a bulk write, prunes them. A prune that fails after the flip leaves the new revision
  current, and its old one retired for the next write.

### What X2 found

Tried in `spikes/x2-rows`, which imports the example book's 202-node plan into an in-process
pool, reads it back as the book's graph, and times it. Each command starts its own pool, so a
run killed midway is followed by one that opens what it left on disk.

- **A plan loads in 0.29 ms, eight times faster than from its markdown.** Loading means:
  - the plan's row and the revision's partition (202 rows, 78 KB);
  - every spec decoded;
  - `Graph::build`.

  That takes 0.29 ms warm, against 2.3 ms for mdbook-reef's walk and parse. A fresh process
  loads it off the archive in 0.81 ms. With YAML specs a warm load takes 1.2 ms.
- **A rewrite costs 2.2 ms, and pruning what it replaced 2.0 ms.** A rewrite is reading the old
  versions, reserving the revision, one bundle of 202 rows and the flip. A put of one node takes
  0.15 ms.
- **A killed import leaves the old revision whole.** An import aborted after 100 of its 202 rows
  left the old revision current, and every node read back. The next import found the 100 rows
  by the plan's `pending` and pruned them.
- **Without the lock, a put during an import is lost.** A put made between an import's rows and
  its flip is written into the revision being retired, acknowledged, and gone once the import
  is current. With the lock it waits for the flip and is refused as stale.
- **Draining a bundle's stream hides refused and failed writes.** `next()` raises only frame
  errors, so X1's drain loop would accept a refused conditional write.
- **A sorted get that finds nothing is an error too** (shoal's known issue 55). A get sent as a
  one-query bundle, with its `error()` checked, reads an empty partition as no rows.
- **A projection must carry the row's partition key.** Shoal's docs say F27 lifted that, but the
  derive on `ScanSupport` refuses a projection without one.
- **A composite partition key of two `String`s and a `u64` works**, in gets, deletes and
  conditional writes.
- **Stopping a pool cleanly takes up to 3 s**, since its shards poll the shutdown flag every 3 s.
  Starting one takes about 30 ms ([C57](../milestones/m2.md)).
- **The first restart still finds rows resident** while it compacts the intent log into
  archives. Later restarts read the partition off the archive.

### Open questions

- Whether the listing partitions give way to scans once `ScanSupport`'s client side (M24 there)
  is built.
- Whether a plan keeps older revisions, for undo or history. The plan's row would then list the
  revisions kept, so pruning still needs no scan.

## The schema

### Design choices

- **One `#[shoal::db]` for reef**, in reef-service's `schema` module, which `reef-shoal` uses
  too. Client and server must agree on its fingerprint, and a test pins it so a change is
  deliberate. The pin is the schema's `SCHEMA_ID` and each table's fingerprint, which leave out
  shoal's protocol version, so a shoal bump that keeps every row moves nothing.
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
