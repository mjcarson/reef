# X2: reef's rows in shoal, priced

A scratch binary that stores the example book's plan in an in-process shoal pool, laid out the way
`plan/src/design/store.md` proposes, and times it. Each command starts its own pool over its data
directory, so a run killed midway is followed by one that opens what it left on disk.

It needs shoal's `ScanSupport` checkout at `../../../shoal` and glommio at `../../../glommio`,
beside reef, and nightly Rust. It is a workspace of its own, which reef's root workspace excludes.

## The rows

| Table | Partition | Sort | Holds |
|---|---|---|---|
| `ProjectRow` | the constant `"projects"` | name | the repo's path |
| `PlanRow` | project | plan name | `current`, `pending` and `retired` revisions, and a `version` |
| `NodeRow` | (project, plan, revision) | id | a `version`, status, group, written order, the spec as JSON, the body |

- **The plan's listing row is its pointer row.** An import names its revision `pending` before
  writing a row of it. It then flips `current` to it and names the old one `retired`, in one
  conditional write. The next write prunes whatever either names.
- **A put names the version it read** (`if_matches`), and a new node must be absent
  (`if_absent`). An import moves every node's version on, so an edit read before an import is
  refused after it.
- **Writes to one plan are serialized** by a lock in the store's process, the only writer.
- **Every row read is checked against its full key**, so a partition hash collision is caught.
- A book keeps most nodes inline in a group's `nodes:`. Each member is lifted into a row of its
  own, its `group` filled in, in depth-first written order.

## Running it

`.cargo/config.toml` sets `-Ctarget-cpu=native` for the host alone.

```sh
cargo +nightly test --release                      # the four acceptance tests
cargo +nightly run --release -- import             # the book as a new revision
cargo +nightly run --release -- verify             # load it and compare it with the book
cargo +nightly run --release -- stale              # two writers from one version
cargo +nightly run --release -- race [--no-lock]   # a put between an import's rows and its flip
cargo +nightly run --release -- import --chunk 25 --die-after 100   # an import killed midway
cargo +nightly run --release -- bench --rounds 10  # the timings below
```

Data goes to `data/run` (or `--data DIR`), never a tmpfs `/tmp`.

## Timings

`bench --rounds 10`: ten timed rounds after three warm-up rounds, on a 32-thread machine with an
NVMe disk (XFS), a two-shard pool with 256 MiB, nightly 1.100, release build. The data directory
is wiped first. A load is the plan row's get, the revision's get, decoding every spec, and
`Graph::build`. The book's load is mdbook-reef's walk, parse and `Graph::build`, without rendering.

| What | Median (lowest to highest), ms |
|---|---|
| The book loaded from its markdown | 2.311 (2.240 to 2.563) |
| The plan loaded from shoal, specs as JSON | 0.285 (0.265 to 0.297) |
| The same, specs as YAML | 1.204 (1.177 to 1.289) |
| The plan loaded by a fresh process, off the archive | 0.808 (0.777 to 0.881) |
| A rewrite: old versions read, revision reserved, 202 rows in one bundle, flip | 2.190 (1.948 to 2.359) |
| Pruning the replaced revision: its keys, 202 deletes, `retired` cleared | 2.022 (1.709 to 2.413) |
| One put: the plan's row read, the node written over its version | 0.151 (0.117 to 0.192) |
| 202 specs decoded, JSON / YAML | 0.091 / 0.899 |
| 202 specs encoded, JSON / YAML | 0.040 / 0.637 |
| The pool, start to ready | 32 (21 to 38) |

A revision's rows hold 77,760 bytes with JSON specs and 74,410 with YAML. Every spec reads back
equal from both formats.

## What it found

The plan records these in `plan/src/design/store.md`, under *What X2 found*.

- **Draining a bundle's stream hides refused and failed writes.** `next()` raises only frame
  errors, so X1's drain loop would accept a refused conditional write. Bulk writes go through
  `exec`, which checks every answer, and single writes through `send_one`.
- **Without the lock, a put during an import is lost.** It is written into the revision the
  import is about to retire and acknowledged. With the lock it waits for the flip and is refused
  as stale.
- **A projection must carry the row's partition key.** Shoal's docs say F27 lifted that, but the
  derive on `ScanSupport` still refuses a projection without one.
- **Stopping a pool cleanly takes up to 3 s.** The shards' shutdown watcher polls its flag every
  3 s (`shoal-core/src/server/shard.rs:2918`). Starting one takes about 30 ms.
- **The first restart still finds rows resident**, while it compacts the intent log into
  archives. Only later restarts read the partition off the archive.
