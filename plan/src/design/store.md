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
- **A batch of several writes is a revision too** ([C62](../milestones/m3.md)). One put or one
  removal is one row's conditional write, which lands whole. A batch of more (a group with its
  members, or several removals) is written as a new revision by the same steps, so a batch
  killed midway leaves the plan as it was. The revision carries each node the batch doesn't
  touch at the version it was at, so another writer's edit read before the batch still gets in
  after it. A batch then costs a revision's write and prune, 2.2 ms and 2.0 ms for 202 nodes in
  X2, rather than 0.15 ms a put.
- **A plan's pages in revisions of their own** ([C28](../milestones/m6.md)). A page is a row in
  a partition keyed by project, plan and revision, sorted by path, with a version and its
  markdown. A whole-plan write that carries pages writes them under its new revision, beside its
  nodes, and its flip makes both current. The plan's row names the pages' revision apart from the
  nodes' (`pages_current`, `pages_retired`), so a batch of node writes, which is a revision of
  its own, leaves the pages where they are: the example book's pages are 2.4 MB, which a batch
  would otherwise copy and prune. A revision can then be current for pages and retired for
  nodes, so each table is pruned only of the revisions it doesn't read: nodes of `pending` and
  `retired`, pages of `pending` and `pages_retired`. A page edit is one row's conditional write
  in the pages' revision, and an import moves every page's version on, as a node's.
- **Bundles held under shoal's frame.** A bulk write's bundles hold at most 256 rows and 16 MiB,
  and a plan's pages at most 32 MiB together, since a plan's pages are read as one partition and
  a frame is 64 MiB by default.
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
- **Each table arrives with the node that needs it.** The queue, a plan's pages, work records,
  comments, loop state and histories each add a table. While reef is in development a table may also change, at the
  cost of a fresh data directory, until shoal can migrate it.
- **Persistent tables throughout.** Ephemeral tables are lost on restart and never evicted.

## Ranks

The user's queue is an ordered list that changes one entry at a time. With a `String` sort key
and no transactions, rewriting every row's position on each move would be slow and unsafe.

### Design choices

- **Fractional rank strings.** Each entry's sort key is a rank such as `a0`, `a0V` or `a1`, and
  a new rank can be made before the first, after the last, or between any two. Moving an entry
  rewrites only that entry's row.
- **A move writes its new row, then deletes its old one**, which must still be the row read. Of
  two rows for one node, the one with the higher `seq` wins, since the new row's write is the
  move, so a move killed between the two reads as made. The next edit prunes the other row.
- **Rebalanced when long, one entry at a time.** When a placement would make a rank longer than
  32 characters, every entry moves, last first, to a short rank after the last one. The entries
  moved sort after those not moved yet, so the queue reads in order after every step, and a
  rebalance killed midway leaves at most one entry twice. That needs no revision, which a
  rewrite of the whole partition would.
- **Writes take a lock per project; reads take none**, since every step of a write leaves a
  queue that reads in order.
- **A queue per plan, in its project's partition** ([C94](../milestones/m11.md)). Each entry names a plan and
  a node, and a plan's entries are a queue of their own: a place is found among them alone, and
  the rank is made between the two rows beside that place in the whole partition, since the rank
  is the partition's sort key and two plans' rows sharing one would write over each other. A
  node beside another plan's entry is refused. The project's order across its plans is still
  read whole, for a `next` over every plan.

## Work records

`reef start` records where a node's work happens and what it started from, and each capture
of its changes adds the SHAs it read ([C30](../milestones/m7.md)). A capture run in a worktree must find
the node it is on, across the project's plans, with no secondary index to ask.

### Design choices

- **One row a node, in its project's partition**, sorted by `plan/id`. The node a worktree is
  on is the one started there last, found by reading that one partition.
- **The record as one JSON field**, as a node's spec is, so what `reef start` and a capture
  read can change without changing the table.
- **A start is written whole, by version.** A node not started is written `if_absent`, and one
  started again only over the version its start named, captures and all. A second start of a
  node is refused as stale, rather than taking the first one's place unseen.
- **Each start is later than the last in its project**, by at least a millisecond, so the
  latest start in a worktree is never a tie.
- **A capture names its start** by the commit its pin points at, so the SHAs of a capture
  taken before a restart never join the new start's.
- **A capture adds to its record after its revision**, naming the revision it kept or the
  latest one it matched.
- **Writes take a lock per project; reads take none.** A start is one row's write, and a
  capture its revision's rows, then the record's.

## Revisions

Each capture of a node's changes is kept as a revision of its files, hunks and lines
([C32](../milestones/m7.md)), which the Changes tab lists, and a file's diff is opened from.

### Design choices

- **A row a revision, in its node's partition**, keyed by project and `plan/id` and sorted by
  its number padded to ten digits. It lists its files without their hunks, so a node's
  revisions are listed by reading one partition. Its record is one JSON field, beside a
  version a later write can name.
- **A row a file, in its revision's partition**, keyed by project, `plan/id` and the revision's
  number, sorted by the path it is shown under. It holds the file's hunks and lines as JSON, so
  opening a file reads one row.
- **Files first, then the revision.** A capture's file rows go in bundles, as a bulk write's
  do, and its revision's row then goes `if_absent`. A revision that reads has every file it
  lists. A capture killed between the two leaves file rows no revision lists, which a read of
  a file never serves, since it reads the revision's listing first. The next capture is kept
  under the same number, over them.
- **A file row is held under half a frame.** A file whose JSON would pass 32 MiB is kept
  without its hunks, marked too large, with its counts still said.
- **Every row checked against its full key on read**, its project, node and number, so a hash
  collision is caught rather than served as another node's changes.

## Comments

Each comment on a node's work is a row of its plan's ([C34](../milestones/m7.md)): a thread on a line of
one of the node's revisions, or on the node as a whole, and the replies in it, which the
Changes and Comments tabs show.

### Design choices

- **A row a comment, in its plan's partition**, keyed by project and plan and sorted by its
  number padded to ten digits. A node's threads and the plan's are each read from the one
  partition and grouped into threads as they are read, so the plan-wide list of open threads
  is one read. Its record is one JSON field, beside a version a mark can name.
- **Numbered in the plan, through restarts.** Every comment, a reply included, takes the number
  after the last one in its plan, so one number names a thread wherever in the plan it is read
  from, as a `Reef-Resolves` trailer captured in another node's worktree must
  ([C37](../milestones/m7.md)). The next number is read from the partition under the project's comment
  lock, and written `if_absent`.
- **A thread's first comment holds the thread.** Its target and state sit on its first row,
  which a mark writes again over the version it read. A reply is a row of its own naming its
  thread, so a reply and a mark never write the same row, and a reply leaves the thread's
  version where it was.
- **A line comment names a stored line.** A thread on a line is kept only if its revision lists
  the file and the file has a line of that number on that side, read from the file's row.
- **A thread's resolutions on its first row** ([C37](../milestones/m7.md)). Each revision linked to it
  as one that answered it, with the commit whose trailer named it, sits beside its state, so
  a trailer or a hand's resolve writes the one row over the version it read. A row kept
  before has none, and reads as such.
- **A thread's trail on its first row** ([C36](../milestones/m7.md)). Where its line went in each later
  revision, and the revision that lost it, sit beside its target, which stays where the
  thread was started. A capture's follow reads the plan's comments and the node's revisions
  under the comment lock, and writes each thread it moved over the version it read; a row
  another process wrote since is left for the next follow. A file is read once a follow.
- **Each comment names its author** ([C50](../milestones/m10.md)). Who wrote it, the user signed in and
  whether Claude wrote it for them, sits in its record beside the writer it always kept, and so
  does each mark a thread takes, each revision a resolve links, and who started and who submitted
  or discarded a review. A row kept before names nobody, and reads as such.
- **Writes take a lock per project; reads take none.** Each write is one row, so the plan's
  comments read whole after every step.
- **Every row checked against its full key on read**, its project and plan, so a hash collision
  is caught rather than served as another plan's comments.

## Loops

Each terminal loop is a row ([C39](../milestones/m8.md)): the pane its session runs in, the project or
plan it feeds that session, the node it holds, the node whose end it acted on, and whether it
is paused.

### Design choices

- **Every loop in one constant partition**, as projects are, sorted by its pane's key, such as
  `zellij:reef:0`, `tmux:<socket>:%3` or `claude:<pid>`. A pane keeps one loop, and a node
  another loop holds is found by reading the one partition.
- **Every change under one lock.** A change reads every loop, judges it against them all, and
  writes the one row, so two loops taking one node at once are taken one at a time, and the
  second is refused, naming the first.
- **A start `if_absent`, every other change by version, and a stop a delete by version**, so
  a write from another process between the read and the write is caught as an error.
- **Its record is one JSON field**, beside its version, so what a loop keeps can change
  without changing the table.

## Users and tokens

reef's users and the tokens they sign in with ([C48](../milestones/m10.md)), a row each in a constant
partition, as projects and loops are.

### Design choices

- **A user is a row under their name,** the argon2id hash of their password and whether they are
  an admin kept as one JSON field. The store never sees a password.
- **Added under one lock, judged against every user.** Adding a user reads them all and hands
  them to the caller's judge, which refuses the add or says whether the new user is an admin, so
  two callers can't both add the first. The write is `if_absent`, so a name taken is said so
  rather than written over.
- **A token is a row under its SHA-256,** so a request's token is found by one get, and the rows
  can't be read back into tokens. A token is kept `if_absent`, and revoked by a delete over the
  version read.
- **Every row is checked against its full key on read,** so a hash collision is caught rather
  than served as another user's.
- **A user's tokens are found by reading the whole partition** ([C77](../milestones/m10.md)), since a token
  is keyed by its hash alone. Only listing tokens, revoking one by its id and removing or
  resetting a user read it.
- **Removed under the same lock, never the last admin.** The judge sees every user before anything
  is said of who exists, then the user's row is deleted over its version, and each token of
  theirs after it. A token issued before its user was added signs nobody in, so a token a removal
  missed never signs in a new user of the name.
- **A password is rewritten over the version read,** so a change checked against the current
  password can't write over one made since.

## Remotes

The git remote a project was registered by ([C49](../milestones/m10.md)), a row each in a constant
partition, as projects, loops and users are.

### Design choices

- **A remote project is two rows.** Its project's row keeps its clone's path where a local
  project keeps its repo's, and a `RemoteRow` beside it keeps the URL, the branch, the clone's
  path again and the commit last fetched, as one JSON field. Projects needed no new field, so no
  fresh data directory.
- **The project's row is written last.** A registration writes its remote's row, then its
  project's `if_absent`, so one killed between the two lists no project. A remote joins its
  project only if both name the same clone, so a row left that way never attaches to a later
  project of that name, and a later registration of it writes over it.
- **One repo, one project, by its URL's key:** its host and path, with its scheme, user, port,
  trailing `/` and `.git` left out, so `git@host:o/r.git` and `https://host/o/r` are one repo.
  Two registrations of it are judged under the lock every registration holds.
- **A fetch writes over the version read**, whether its branch moved or not, so the row says
  when it was last fetched, and two fetches can't both land.
- **Each other branch a plan follows is kept in the same record** ([C92](../milestones/m11.md)), a map of
  each branch to the commit last fetched and when, so a row written before reads back with none.
  A branch that fails to fetch keeps its last tip, and one no plan follows any more is left in
  the map rather than taken out, so a fetch running beside a branch being set never drops it.

## Plans' branches

The branch each plan follows ([C92](../milestones/m11.md)), a row each in its project's partition, sorted by
plan, as the plans themselves are.

### Design choices

- **A table of its own, not a field of `PlanRow`.** `PlanRow` has no JSON field, so a branch in
  it would change the row and need a fresh data directory. A plan with no row follows its
  project's own branch, as every plan did before.
- **A change is judged under the plan's write lock.** The store hands the caller's check the
  plan as it stands and both branches, and writes the row only if the plan's docs at the new
  branch add no problem. The row is written `if_absent` for a plan with none, else over the
  version read.
- **Every checked write reads the branch under the same lock.** A put, a whole-plan write and a
  page edit each read the plan's branch row inside the lock their check runs under, so none is
  judged at a branch a change replaced meanwhile.
- **Kept in the plan's history**, under `plan/<plan>`, as an `Edited::Branched` naming both
  sides.

## History

Who made each edit of a node, a page or a project's queue, and what it changed
([C50](../milestones/m10.md)). A node's, a page's and a queue's rows hold no record a field could be added to,
and changing one would take a fresh data directory, so their edits are rows of a table of their
own.

### Design choices

- **One table, a partition a subject.** An `EditRow` sits in its project's partition under its
  subject: `node/<plan>/<id>`, `page/<plan>/<path>` or `queue`. A plan's name and a node's id
  never hold a `/`, so no two subjects share one, and a node removed and added again keeps one
  history.
- **Keys count down, so a read takes the newest first.** A key is `u64::MAX` less the edit's
  stamp, padded to twenty digits, so a get with a limit reads the newest edits and stops: 200 of
  them, a page. The stamp is the time in milliseconds, but always past the last one the store
  took, so one store's edits to a subject never share a key and need no read first. The key ends
  with a number each store draws when it opens, so a store opened again after a restart never
  writes over the last one's edit of the same millisecond.
- **A page's last key is the cursor onto the next** ([C88](../milestones/m10.md)). A page reads one row past
  its limit, to know whether an older one is kept, and if one is, names its last row's key as
  `older`. The next page reads the partition past that key, shoal's `SortRange::after`, so it
  costs a page however far back it is. Keys are unique where times aren't, so no page skips or
  repeats an edit two stores made in one millisecond. A cursor not shaped as a key is refused
  rather than read. The CLI's `--all` and the app's **Older edits** read page after page; the app
  reads from the newest each time, so an edit while older ones are shown leaves no gap.
- **Written once, after the write it records, under the same lock.** An entry is kept once its
  write has landed and before the change is published, so a client that reads again on the change
  finds it. A history that fails to keep is warned of and leaves its write standing, with a gap:
  an entry kept first would outlive a write that then failed, and failing a write that landed
  would send its writer back to a stale refusal.
- **What changed, not the node again.** A node's entry says its status went from one to another,
  a criterion was marked or named another test, which other keys were edited, its description by
  part, and whether its body was. A page's says it was added, its text edited or removed, and a
  queue's names the node queued, moved or taken off and where. A put that left its node as it
  was keeps none, and so does a whole-plan write for each node and page it left so, so importing
  the same plan again keeps nothing.
- **A start is an entry of its node's** ([C90](../milestones/m10.md)), with no version, since a start leaves
  the node's spec as it was. It names when the work record says it started, the branch and the
  commit, and whether it started again; a stale start keeps none. A node's work record holds
  only its latest start, so its history is where an earlier one is still seen.
- **Its record is one JSON field,** so what an entry keeps can change without changing the table,
  and a kind of edit a newer reef keeps reads as one this reef can't, rather than failing the
  history.
- **Who placed each queue entry is read from the queue's history,** its newest edit that queued
  or moved the node, among the newest 200.
