# The store

`Store<B: Backend>` reads and writes plans, their nodes and their pages by version. Its logic (each plan's lock,
the versions, the full-key checks) is written once, over a backend that only reads rows and makes
conditional writes the way shoal does: `ShoalBackend`, or `MemoryBackend`, its in-memory twin,
which needs no shoal and builds on stable. `tests/store.rs` runs every case against both, so shoal
is the twin's oracle.

- **A put names the version it read**, or `None` for a new node. One that isn't as expected
  comes back as `Put::Stale`, with whether the node exists, is missing or moved on.
- **A plan is listed with an empty current revision**, so a put always has somewhere to go.
- **Writes to a plan take its lock alone, and reads share it**, since reef-service is shoal's
  only writer.
- **Every row read is checked against its full key**, so a partition hash collision is an
  error, never the wrong row.
- **A project names a repo** (`Store::add_project`), located at its main worktree's root, so a
  linked worktree is its main repo. A name or a repo another project has is refused, and
  `Store::find_project` finds the project a working directory belongs to. A plan's project must
  be registered.
- **A whole plan is written as a new revision** (`Store::write_plan`): named `pending` on the
  plan's row, its rows written, then made current by one conditional write, and the revision it
  replaced pruned. A write killed midway leaves the old revision current and whole, and the next
  write prunes what it left. `tests/common/faulty.rs` kills writes where a case asks it to.
- **A plan's pages are a revision of their own.** A whole-plan write that carries pages writes
  them under its new revision and makes them current in the same flip, so an import lands whole.
  The plan's row names the pages' revision apart from its nodes', so a batch of node writes
  leaves the pages where they are rather than copying them, and each table is pruned only of
  the revisions it doesn't read. A page edit (`Store::edit_page_checked`) is one row's
  conditional write in the pages' revision. A plan's pages hold at most 32 MiB, and a bulk write
  is sent in bundles of at most 256 rows and 16 MiB, under shoal's 64 MiB frame.
- **A project's queue is one row an entry** (`Store::edit_queue`), sorted by a rank string
  (`rank`), so an edit writes one row. A move writes its new row, then deletes its old one; of a
  node's two rows the later write wins, and the next edit prunes the other. A rank grown past 32
  characters spreads the queue anew, an entry at a time, last first.
- **Each plan's entries are a queue of their own** (C94), in the same partition. A place is
  found among the node's own plan's entries, then its rank is made between the two rows beside
  that place in the whole partition, since the rank is the partition's sort key and two plans'
  rows must never share one. A node beside another plan's entry is refused. Each edit is kept
  under `queue/<plan>` and under the project's `queue`, so the project's history still spans
  every plan.
- **A move takes a node's rows with it** (`Store::move_nodes`, C99), under the project's queue,
  work, comment and decision locks and both plans' write locks, judged on both plans. Each step
  can run twice:
  1. its work record, revisions, their files and decisions are copied to its new name where
     none is stored, and its history is copied under its own keys;
  2. its comments are copied with new numbers, each naming the one it came from, and those
     left behind deleted;
  3. the plan it joins is written;
  4. what it left behind is deleted;
  5. its own plan is written without it;
  6. its queue entry moves to a rank just after its own.

  A node in both plans with its newest edit the same in both histories is one a killed move
  left, so running the move again finishes it.
- **A node's work record is one row** (`Store::start_work`), in its project's partition by
  `plan/id`, so the node a worktree is on is the one started there last, across the project's
  plans. A start is written where no record is, or over the version of the one it replaces, and
  is later than every start before it. A capture's SHAs (`Store::add_capture`) join the start
  its pinned commit names, and no other.
- **A node's revisions are a row each, and their files a row each** (`Store::revisions`), the
  files written first and the revision `if_absent` after them, so a revision that reads has
  every file it lists. They count from 1 for the node, through restarts. A file whose row would
  pass 32 MiB is kept without its hunks.
- **A plan's comments are a row each** (`Store::threads`), in the plan's partition by number,
  so a node's threads and the plan's are each one read. Every comment, a reply included, takes
  the number after the plan's last, through restarts. A thread's first comment holds its target
  and state, and a mark writes it again by version; a reply is a row of its own. Where a line's
  thread was followed to sits on that row too, which a capture's follow writes by version, and
  so does each revision linked to it as one that answered it, which a resolve writes.
- **Each node's, page's and queue's edits are a row each** (`Store::node_history`), in a
  partition of the subject's own, keyed newest first, so a read with a limit takes the newest.
  A page of it reads one row past its limit, to know whether an older one is kept, and names its
  last row's key as the cursor onto the next page, which reads past that key in the partition;
  keys are unique, so no page skips or repeats an edit. A start keeps an entry in its node's
  history too, under the project's work lock, since its work record keeps only the latest.
  So do acts with no history of their own (C89): a project's registration (`project`) and a
  plan's listing (`plan/<plan>`), joined into the listings a person reads, never the service's
  own reads; each loop act that changed something (`loops`), under the loops' lock, a stop's
  entry naming what it let go of, since its row is gone; and each act on a user (`users`, under
  the empty project).
  A write's entries are kept once it has landed, under its lock, with one stamp from the store's
  clock, always past the last it took; its key ends with a number the store drew when it opened,
  so a store opened again never takes a key the last one wrote. A history that fails to keep is
  warned of, and leaves its write standing. Only what changed is kept: a put that left its node
  as it was, or a whole-plan write's nodes and pages left so, keep nothing.
- **Each write to a plan's nodes or pages keeps one entry in the plan's activity** (C119,
  `ActivityRow`): when, who, the nodes and pages it touched, and the nodes it took to done, set
  done or added done. `keep_edits` tells it from the write's `node/…` and `page/…` subjects, a
  start's left out. It is kept under the write's own key, in the first bundle of its edits, and
  fails with them. `Store::plan_updated` reads the newest entry, else the plan's listing.
  `Store::plan_done` reads a window from its end back, a page at a time, stopping at the first
  entry older than its start, and counts only the nodes done now, so one set back isn't counted.
  A node moved in done counts in neither plan, since its going done was its old plan's.
- **Every loop is a row of one partition** (`Store::edit_loop`), under its pane's key, so a
  pane keeps one loop. Every change reads them all under one lock and writes the one row, a
  start `if_absent` and anything else by version, so two loops taking one node at once are
  taken one at a time. A stop deletes the row by version.
