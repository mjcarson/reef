# Milestones

~~**Nothing below is delivered, and no gate has been set**~~ M22 and M23 are delivered (2026-10-30 and
2026-11-04), and no gate after them is set; the contract that comes before them was agreed on
2026-10-29 ([below](#before-m22-the-contract-agreed)). This page is an order of work, drawn before
any [spike](spikes.md) has reported, so that its dependencies can be argued with. It becomes a plan
gate by gate: each is set once the questions it waits on are decided on
[W13](contract.md#questions-to-answer), with its acceptance tests named on their owning pages and its
evidence stated before its work starts.

M22–M28 continue the object storage part's [M11–M21](../../object-storage/milestones.md) and are stable
identifiers. Acceptance tests live on their owning W pages, indexed by
[W13](contract.md#the-acceptance-index), and each names one gate below.

**Every gate keeps the cluster's rules.** No gate adds a consensus protocol, a coordination service
or a clock that decides anything ([R7](../../distributed/overview.md#what-is-asked-of-it)), and no gate
is passed by weakening a clause of [the contract](contract.md#the-contract). And no gate that a client
can reach is passed without [W7](isolation.md#the-budget)'s budget measured beside it.

## How a gate is written

| Line | Holds |
| --- | --- |
| **Closed before it** | The questions of [W13](contract.md#questions-to-answer) decided, with their evidence on the record, before the gate's work starts |
| **Lands first** | The required rows of [W1](prerequisites.md#required) placed at its start. Each is a change of its own, and nothing stops it landing sooner |
| **Delivers** | What exists afterwards |
| **Acceptance** | The owning pages' rows that name this gate |
| **Evidence/exit** | What is measured or demonstrated beyond tests passing. Every number labelled by host, cpu and governor |
| *Not at this gate* | What a reader might expect here and will not find |

## The gates at a glance

| Gate | In a line | Closed before it | Lands first |
| --- | --- | --- | --- |
| Before M22 | ✅ The contract agreed, 2026-10-29 | P20–P26 | — |
| M22 | ✅ The harness and the facts: the bench's scan, the fixture's oracle, the baseline, delivered 2026-10-30 (F122) | — | The bench's scan kind and `scan` event |
| M23 | ✅ The walk rebuilt for load, the drivers moved onto it, judged by the budget and, since W42, by a scan's own rate and cost, delivered 2026-11-04 (F123 to F130) | Q33, Q34, Q37's default, Q62 | The scan class, the budget, device pacing, an admission class; one operation a record read, merges written in key order |
| M24 | ✅ Scans on one node: the frames, the client, SHQL, the grant, delivered 2026-11-14 (F140 to F147) | ✅ Q35 and ✅ Q36 (settled 2026-11-06), ✅ X19 (2026-11-07) | ~~Bounded records for large sorted partitions~~ ✅ (F133, 2026-11-07); ~~a walked record handed out uncopied~~ ✅ (F134, 2026-11-07); ~~a scan budget's floor~~ ✅ (F135, 2026-11-08) (X19's decisions) |
| M25 | Scans across a cluster: copies, floors, refusals, activation | ✅ Q38 (settled 2026-11-14, by design and by X38), ✅ Q39 (settled 2026-11-10, ahead of M24's grant) | — |
| M26 | Listing a bucket's objects, unordered | ✅ What a listing reads (X19, 2026-11-07) | A listing's fields kept apart from inline bytes (X19's decision) |
| M27 | Ordered scans by declared order | Q40 | — |
| M28 | The ordered listing | Q41 | — |

## The gates

### Before M22: the contract agreed

**Settled 2026-10-29**: P20–P26 agreed as written ([W13's record](contract.md#before-m22-the-contract-agreed-2026-10-29)).
~~**Not settled.**~~ It ~~is~~ was settled when P20–P26 on [W13](contract.md#the-contract) ~~are~~ were agreed, each
beside the violation that breaks it and the tests that own it, as the object store's P7–P19 were
before M11 ([S18's record](../../object-storage/contract.md#before-m11-the-contract-agreed-and-q27s-path-modelled-2026-10-09)).
No type, frame or file format of a scan is added at this gate.

### M22. The harness and the facts

**Delivered** (2026-10-30), as [F122](../../features/scan-oracle.md): the scan oracle, the fixture's `SCAN`
verb and a row that holds a scan beside writes to the oracle, and this part's acceptance tables joined
to the book's check, beside F115's scan kind and event and X16's baseline, which landed ahead of it.

**Closed before it.** The contract.

**Lands first.** The bench's scan kind and `scan` event ([W1](prerequisites.md#the-benchs-scan-kind-and-scan-event)):
✅ landed on 2026-10-29 as [F115](../../features/bench-scans.md).

**Delivers.**
- The bench's scan operation kind, ~~`list` kind~~ and `scan` event, the arm `{mix}/b{bundle}/scan` cut by
  `cut_background` ([W12](performance.md#what-the-driver-gains)) ✅ (F115). The `list` kind is M26's, since a
  listing needs M26's engine: W12's row for it, `a_list_kind_drives_a_buckets_listing`, gates M26.
- The fixture's scan oracle: a check of P20 to P22 for each group against the committed history, and
  nothing across groups, so no test can lean on what P23 withholds ✅ (F122, `shoal_model::scan`).
- A `SCAN` verb in the fixture that drives the walk as a scan would, ~~every shard's pages at once~~ its
  pages one at a time, each at its group's floor ✅ (F122): a node's pool is not `Sync`, so the child
  cannot share its walk among threads without an engine change this gate does not make, and pages in
  flight together are the bench's (F115) and M24's driver's.
- This part's acceptance tables joined to `shoal-bench/tests/acceptance_tables.rs`, which learns
  M22–M28, so a test name is unique across all three parts and every test of a delivered gate is a
  function in the workspace ✅ (F122), which found [W42](scan-speed.md)'s copy of W5's
  `a_page_reads_its_archives_in_runs` row.
- [X16](spikes.md#x16-the-walk-beside-the-foreground)'s baseline on the record: ✅ taken ahead of the gate on
  2026-10-29 ([its record](walk-beside-the-foreground.md)).

**Acceptance.** [W12](performance.md#acceptance-tests)'s M22 rows and W13's oracle test.

**Evidence/exit.** X16's tables: what the walk as it stands costs the reference cell on each host
(✅ [taken](walk-beside-the-foreground.md#the-tables), by member where the hosts differ).

*Not at this gate*: any change to the engine, the wire or the client.

### M23. The walk rebuilt for load

**Delivered** (2026-11-04), as [F123](../../features/walk-and-scan-counters.md) to
[F130](../../features/loop-steps-by-kind.md): a scan's page planned, read in runs and paced in device time,
its encodes and folds in steps, the drivers on the rebuilt walk, and a loop's steps counted by kind. Its
exit was run on the lab and run again ([the record](m23-exit.md)), and closed with the user once item
359's twin showed the read clause's rise is the cell's own
([the decisions](m23-exit.md#decided-with-the-user-2026-11-04)).

**Closed before it.** Q33, the engine, by X16 and [X17](spikes.md#x17-a-page-rebuilt); Q34, the budget
and the pace, by X16 and [X18](spikes.md#x18-table-reads-paced-by-idle-time); Q37's default, on the
shards under a ceiling; and since [W42](scan-speed.md) Q62, a tablet's records kept together, by
[X37](spikes.md#x37-a-tablets-records-together-priced).

**Lands first.** The scan class on every table shard, the budget for transient read buffers, table
reads paced by a device's idle time, and an admission class apart from the drivers'
([W1](prerequisites.md#required)): ✅ all four landed on 2026-10-29 as F116 to F119, for a scan's pages
alone, the drivers moved onto them here. Since [W42](scan-speed.md) (2026-10-29), for its rate, also one
operation a record read and merges written in key order, ~~neither started~~ ✅ both landed on 2026-10-30
as [F120](../../features/one-operation-a-record-read.md) and [F121](../../features/merges-in-key-order.md).

**Delivers.**
- [W5](tablet-reader.md)'s six steps (✅ the plan, [F124](../../features/scan-page-planned.md), the archive reads, [F125](../../features/page-reads-in-runs.md), the encodes in steps, [F126](../../features/encodes-in-steps.md), and the folds and the check, [F127](../../features/folds-in-steps.md), 2026-10-31): admission; a plan on the loop that serializes nothing; archive
  reads ~~in runs, in disk order,~~ several in flight, and in runs where a tablet's records lie together
  ([W42](scan-speed.md)), in idle time under a ceiling; resident partitions encoded in steps
  from a second inbox, folded over fresh lookups; folds, filters and encodes in the scan queue; the
  check at the page's end.
- The object store's drivers, backfill, copier, light scrub and reclaimer, on the rebuilt engine, with
  their admission kept: ✅ [F129](../../features/drivers-on-the-rebuilt-walk.md) (2026-10-31), M23's seventh step, the backup archive's walk
  left on the capture, and the backfill asking a shed page again
  ([Resolved #345](../../appendix/resolved/backfill-shed-page.md)).
- A node's counters of walks and scans: ✅ [F123](../../features/walk-and-scan-counters.md)
  (2026-10-31), M23's first step.
- The pace in device time, as M23's plan decided with the user on 2026-10-31: a share of each
  device's busy time a second, 75 ms, learned from the scans' own reads under the byte ceiling, ~~four~~
  sixteen scan reads in flight a device since [O146](../../appendix/optimizations.md#o146-four-scan-reads-a-device-held-a-scan-alone-to-two-thirds-of-its-rate),
  O143's waits cut, and a page its device holds back ending short inside its deadline:
  ✅ [F128](../../features/device-time-pace.md) (2026-10-31), M23's sixth step, its reads paced a group
  at a time since [item 346](../../appendix/resolved/paced-reads-alone.md).

**Acceptance.** [W5](tablet-reader.md#acceptance-tests)'s and [W7](isolation.md#acceptance-tests)'s M23
rows.

**Evidence/exit.** The budget held on the lab beside a full scan driven in process, of a table larger
than memory after a restart and of `MovieByKeyword`, eight rounds a host; every test on F87's page
unchanged; the drivers' own figures no worse than before. Since [W42](scan-speed.md): a scan of `Big`
alone at its ceiling on the lab's SSDs, within [W12](performance.md#the-budgets)'s cost budget, and
beside the cell within the foreground's.

**The exit, run on the lab 2026-10-31** ([its record](m23-exit.md)), at the exit's commit with every
scan setting its default. Held: the cell's read p99 beside a whole scan of `Big`, 1.07 times, and of
`Keyword`; no `Resident` eviction pass; ten partitions a device read and more for `Big` loaded once,
nine and more after an hour's churn at both segment sizes; the drivers' figures no worse. Open, for the
user to decide on the record's evidence: the insert clause, which W7's ratio misses because the
cell's feed wraps inside an arm this long, and which before the wrap holds beside `Big` and rises
beside `Keyword` no further than the cell alone does over the same seconds; loop steps
past 100 µs, about one in 460, nine in ten of them steps of large resident partitions, which item
347's bound does not hold
([item 350](../../appendix/resolved/encode-copy-deadline.md)),
and the rest light steps on shards their own io_uring workers preempt
([O147](../../appendix/optimizations.md#o147-each-shards-io_uring-workers-run-on-the-shards-own-cpu));
a Zen1 host's cpu a partition, 22.6 µs gross and 10.1 above its idle node at today's segments, 12.2
gross at 40 MiB ([O149](../../appendix/optimizations.md#o149-larger-wal-segments-keep-a-tablets-records-together-four-times-longer));
and the share, which buys a 970 EVO about 20 MiB a second alone where the plan expected 32. The quick
passes before the rounds found [items 346](../../appendix/resolved/paced-reads-alone.md) and
[347](../../appendix/resolved/encode-steps-by-rows.md), O146 and O148, each fixed before the rounds,
and the rounds [items 348 to 350](../../appendix/known-issues.md#348-a-failed-devices-left-slice-waits-out-the-readers-grace-and-a-move-that-needs-it-waits-with-it),
filed.

**Decided with the user the same day** ([the decisions](m23-exit.md#decided-with-the-user-2026-10-31)):
the insert clause read as W7 defines it, on the exit's rounds run again with a cell whose feed does
not wrap and a control beside them; item 350 M23's, fixed before those rounds, and the step clause
read on a step's own work, since O147 follows M23; W12's cost read above the idle node, so held at
10.1 µs, and O149 after M23; the share kept at 75 ms, W12's rate for M23 what it buys, read again at
O149's segments once that is done. What is left is drawn under
[Finishing M23](whats-left-todo.md#finishing-m23). The cell is built since 2026-11-01: 1,100,000 rows
whose feed never wraps inside an arm, and the rounds again run at 6 GiB a node, decided with the user,
since a cell that never wraps keeps every row it inserts
([the cell for the rounds again](m23-exit.md#the-cell-for-the-rounds-again)). Its check round found
that the cell's inserts fall about 75 s into every arm, wrap or not
([item 351](../../appendix/resolved/the-cells-fall-is-the-drives.md)). Item 351 is M23's, decided with the
user the same day, found and fixed before the rounds again. ~~found and fixed~~ It was resolved the same
day: the lab's 970 EVOs flush three times slower after about 75 s of synced writes, so the rounds
again warm every cell arm 100 s before it measures. Item 350 is fixed since 2026-11-01: a lab run of
F130's counts named its steps as encode steps that took three to five times too many rows, and a
piece's copy now ends at a deadline ([Resolved #350](../../appendix/resolved/encode-copy-deadline.md)).
The same rounds on the fixed build confirmed it. The run also found light steps past 100 µs that F130
counts as their own cpu, on kernels that charge a cpu's interrupts to the thread it interrupts
([item 353](../../appendix/resolved/capture-allocations.md)),
M23's to attribute before the rounds again, decided with the user the same day, and attributed on
2026-11-02 as the light steps' own work, not the interrupts their cpu takes, `Big`'s long captures fixed
in M23 before the rounds again, decided with the user the same day, and fixed the same day, the
allocator inside a capture's allocations ([Resolved #353](../../appendix/resolved/capture-allocations.md)); and the confirmation
run a scan's tablet beside the cell given up after a minute, its pages starved at their device's pace,
fixed before the rounds again ([Resolved #355](../../appendix/resolved/scan-pages-served-in-turn.md)).

**The exit's rounds again, run on the lab 2026-11-03** ([their record](m23-exit.md#the-rounds-again)):
eight rounds at `b3a4d7ef`, with four controls among them, the cell alone where a `Keyword` capture
scans. Held: the insert clause, read as W7 defines it over the whole arm, 1.04 times beside `Big` and
1.00 beside `Keyword`, the control's inserts rising as far; no eviction pass; no feed's wrap. Missed:
the read clause beside `Big`, 1.14 times. The cell alone fell over the same seconds, so the rise ~~is~~
looked like the scan's. The scan did not get dearer: the cell reads faster without it than it did
([item 359](../../appendix/resolved/the-cells-own-climb.md)). Also missed, the step clause, on the remainders already placed after M23. Both
~~are~~ were the user's to decide before M23 closes ([the record](m23-exit.md#for-the-user-to-decide-1)).

**Item 359's runs (2026-11-04)** showed the read clause's 1.14 is the cell's own climb at that place in a
capture, not the scan's. fio replaying a scan's reads on the Zen1 drives cost the cell nothing. A twin of
the scan arm, its scan paced to a seventy-fifth of the share, rose as far, 1.11 times
([Resolved #359](../../appendix/resolved/the-cells-own-climb.md)).

**Closed 2026-11-04**, decided with the user ([the decisions](m23-exit.md#decided-with-the-user-2026-11-04)):
- the read clause beside `Big` held, judged against the twin of its arm;
- the step clause met with its remainders apart, each after M23: europa's `Keyword` encode steps
  (Resolved #350's Still open), O154 and O147;
- every other clause as read before.

Found by item 359's first run and filed after M23:
[O155](../../appendix/optimizations.md#o155-two-scan-reads-a-device-collapse-a-scans-rate-ninefold),
two scan reads a device collapsing a scan's rate ninefold, applied since (2026-11-08) as a floor of four
reads a device.

*Not at this gate*: anything a client can call. Isolation is proven before any client can scan, for the
reason X9 ran before the object store's first device: what a design costs a table's tail decides the
design.

### M24. Scans on one node

**Delivered** (2026-11-14), as [F140](../../features/scan-frames.md) to [F146](../../features/shql-scan.md), with
[F147](../../features/scan-link-pace.md)'s pace. It brought:

- a scan's frames, token and grant;
- its pages over a shard's tablets, a sorted partition split across them, and its answer in rows;
- the page on the wire, the client's driver and SHQL's `SCAN`;
- a scan's answers paced by a share of their link.

Its exit was run on the lab ([the record](m24-exit.md)). It was run again at the default share on
[item 392](../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md)'s fix, with a twin of the scan in every round
([the confirmation](link-pace.md#the-confirmation)).

It closed on the decisions taken with the user ([2026-11-13](link-pace.md#decided-with-the-user-2026-11-13)):

- the read clause beside a resident scan is judged against that twin, and its arms overlapped it;
- the insert clause held at the median, at 1.00 to 1.04 times;
- the step clause is met with its remainder apart, after M24
  ([O180](../../appendix/optimizations.md#o180-a-paced-resident-scans-encode-steps-run-past-their-bound-on-their-own-cpu-about-once-in-ninety));
- [item 386](../../appendix/resolved/scan-pages-held-to-the-bound.md) stays open after it (resolved since, on
  2026-11-15, with the *After M24* box).

**Closed before it.** Q35, what an answer carries, ✅ settled 2026-11-06 with the user
([the record](contract.md#q35-what-a-scans-answer-carries-2026-11-06)); Q36, the cursor and the frames,
✅ settled the same day by design ([the record](contract.md#q36-the-cursor-and-the-frames-2026-11-06));
and [X19](spikes.md#x19-the-edges), the edges, which decides the bounds M24 must hold and, with the
user's leave, whether M24 fills the cursor's reserved position and splits a sorted partition across
pages: ✅ reported 2026-11-07 ([its record](the-edges.md), [the decisions](contract.md#x19-measured-the-edges-decided-with-the-user-2026-11-07)).
No partition may fail or go missing from a scan, so M24 splits, and refuses no partition for its size.

**Lands first**, decided by X19 with the user:
- ✅ **A large sorted partition written as records of bounded size** by the merges, as fragments are
  ([F61](../../features/fragmented-partitions.md)), so a page verifies and folds one bounded record at a time.
  This cures [item 373](../../appendix/resolved/whole-record-holds.md). **Done** (2026-11-07,
  [F133](../../features/bounded-records.md)): records of at most 1 MiB, a page folding one a step, a get
  reading them in place. What it leaves M24 is the split across pages: a page still holds and draws the
  whole partition.
- **A walked archived record handed out from its read buffer**, never copied
  ([O162](../../appendix/optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it)).
  **Done** (2026-11-07, [F134](../../features/walked-records-uncopied.md)): every walk reads its archives into
  heap buffers its page carries across threads, a whole record is handed out where its read put it, and a page
  draws copy room only for what a fold over fragments makes.
- **A scan budget's floor of the largest row a node admits**, so any row fits alone and waits its turn,
  never refused for size. **Done** (2026-11-08, [F135](../../features/scan-budget-floor.md)): a node's scan
  budget is never less than its request body bound and a row's slack, about 64 MiB at the defaults, and the
  scan budget serves its waiting draws in turn. A resident or overlaid row, which a page still holds about
  twice, is refused past half the budget
  ([item 377](../../appendix/known-issues.md#377-a-resident-or-overlaid-row-past-half-the-scan-budget-is-refused-by-name)).

**Delivers.**
- **A sorted partition too large for a page split across pages**, the cursor's reserved sort-key position
  filled, and [P20](contract.md#the-contract) restated for it: each live row once (X19).
- **A page's bounds as W4 has them**, 1,000 rows, 1 MiB of answer bytes and 4 MiB of work, the work bound's
  maximum 4 MiB (X19), and the tablets the counters call empty skipped on the loop.
- `ScanPage` and `ScanAnswer`, `CLIENT_CAP_SCANS`, refusals by name, `Cancel` of a page
  ([W4](cursor-and-pages.md)), with the bytes Q36 fixed: fixed heads of 96 and 80 bytes, a page's
  progress one key, cursors that are keys, chunks of a 32 byte head in data frames of 256 KiB, and the
  tablets another shard hosts refused `HostedElsewhere`.
- An answer's two bodies: rows encoded on the shard by default, and partitions for an unfiltered,
  unprojected scan that asks, under `PARTITION_LAYOUT`; each page's rows handed on once its answer is
  whole and validated ([W4](cursor-and-pages.md#the-body-rows-or-partitions-on-request), Q35). The body
  in partitions is built since [F132](../../features/whole-partitions-in-answers.md): M24 carries
  `PartitionBody` in `ScanAnswer`'s data frames, names its layout and counts in the answer's head, hands a
  client's body to `PartitionPage` with the bound of the page it asked, and adds `.partitions()`, the
  request's body kind, its refusals of a filter, a projection, a listing and a generated table, and
  `UnknownLayout`.
- The client's driver against one node: streams by hosting shard, resume tokens, `split(n)`
  ([W6](distribution.md), [W10](api.md#the-client)).
- `MovieScan` generated beside each get, with its filter, limit and projection.
- SHQL's `SCAN`, and `shoalctl`'s paging.
- The `scan` privilege, read from a standalone node's file.
- Scan counters in a node's figures.

**Acceptance.** The M24 rows of [W2](ordering.md#acceptance-tests), [W4](cursor-and-pages.md#acceptance-tests),
[W7](isolation.md#acceptance-tests), [W10](api.md#acceptance-tests) and [W13](contract.md#acceptance-tests).

**Evidence/exit.** The budget beside a client's scan over the wire to a standalone node on the lab, the
foreground on the scan's own client and link too, as [X20](scylladb-scan-sources.md#what-the-run-says)'s
rounds carrying rows had it, with the hosts' TCP counters read beside it; and the scan's rate a node
reported.

**The exit was run on 2026-11-12** ([M24's exit](m24-exit.md)): eight rounds, titan the node and
europa the client over 1 GbE. Beside a scan of archived rows the budget held, in rows, in partitions,
filtered and projected, with no eviction pass and no long step at the median. **Beside a scan of
resident rows it missed**: the scan filled the link and the cell's read p99 rose 3.5 times
([item 390](../../appendix/resolved/resident-scan-fills-the-link.md)). M24 is not marked delivered: what the clause
asks of the link is [the user's to decide](m24-exit.md#for-the-user-to-decide).

**The user decided on 2026-11-13 that the node paces a scan's answers to a share of its link**
([F147](../../features/scan-link-pace.md), 25% unless set). [Its sweep](link-pace.md#the-sweep) cured the link's
part: at a quarter the cell's read median is its own and its p99s fall from 3.33 and 1.59 times to 2.07 and
1.16. No share holds the budget: a resident scan costs the reads beside it ~~on its node~~ about 0.4 ms at p99 at any
rate ([item 391](../../appendix/resolved/paced-scan-reads-and-inserts.md)).
~~**M24 stays open behind item 391**, and its exit's resident half is run again once 391 is cured.~~
Item 391's insert part was a scan's unpaced reads of its map, cured by
[Resolved #392](../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md), and its read part is titan's network
path, judged against a twin. [The confirmation](link-pace.md#the-confirmation) ran the exit's resident half again over
eight rounds on 2026-11-14 and held both, and **M24 is delivered**
([Resolved #391](../../appendix/resolved/paced-scan-reads-and-inserts.md)).

*Not at this gate*: a cluster, a listing, an ordered scan.

### M25. Scans across a cluster

**Closed before it.** Q38, copies and levels: ✅ settled 2026-11-14
([the record](contract.md#q38-which-copies-a-scan-reads-and-its-levels-2026-11-14)), ~~in part~~ by design, and
the copy a scan prefers ~~left to~~ by [X38](copies-two-ways.md) the same day, the user's decision: followers then
leaders; Q39, whether `all` includes `scan`, with the user: ✅ settled 2026-11-10, ahead of M24's grant, `all`
includes `scan` ([the questions](contract.md#questions-to-answer)). Nothing is left open before it.

**Under way since 2026-11-15**, its plan approved with the user and drawn down to its steps under
[Finishing M25](whats-left-todo.md#finishing-m25): a refusal carries no applied index, the bench scans through
client pages by default, items 393 and 401 to 404 are fixed with it, and it is delivered once its lab exit is run.
Six rules of Q38's record were restated by the plan's review ([the amendments](contract.md#amended-at-m25s-plan-2026-11-15)).

**Delivers.**
- The driver over a cluster: voters only, ~~followers first and spread~~ followers then leaders since X38, every
  tablet dealt once to a lane a member and a member out of tablets taking one it follows, then one it leads,
  floors a group, one barrier a group at `Quorum`, every refusal routed around, a move, a failover and a restart
  carried through, a scan resumed by a fresh client from its token ([W6](distribution.md#choosing-copies)).
- Q38's rules on a node's page and the client's driver: a copy that is not a voter, by its map or its
  applied membership, refused `Unavailable`; a scan's level resolved once at its start; a floor waited for
  at most 250 ms; a page's groups' barriers taken together as a get's, a failed one refusing its group's
  tablets; a caller's session tokens the floors a scan begins with (`.after(token)`); and a node's figures
  counting a scan's barriers and floor waits ([W4](cursor-and-pages.md#the-cursor-and-the-resume-token), W6).
- The scans' wire version, activated as every addition since F48 is, and `scan` grantable in the
  committed grants only behind it ([W10](api.md#grants)), a bootstrap's seeded grants held to it too (item 404).
- `shoaladm bench` scanning a cluster through the client's pages, followers or leaders as a spec names them (the
  user's decision with the plan).

**Acceptance.** The M25 rows of [W4](cursor-and-pages.md#acceptance-tests),
[W6](distribution.md#acceptance-tests), [W10](api.md#acceptance-tests) and [W13](contract.md#acceptance-tests).

**Evidence/exit.** The budget on the lab's three nodes beside a scan at its defaults; a scan through a
node's kill and through a rebalance in the fixture with P20 checked by the oracle; a scan's figures on
followers beside the same scan forced onto leaders.

*Not at this gate*: a listing, an ordered scan, a coordinator for thin clients.

### M26. Listing a bucket's objects

**Closed before it.** What a listing reads against the inline threshold, by [X19](spikes.md#x19-the-edges):
✅ decided 2026-11-07 with the user ([the record](contract.md#x19-measured-the-edges-decided-with-the-user-2026-11-07)).
At the default 16 KiB a listing read 58 times its entries' fields, so they are kept apart.

**Lands first.** A listing's fields kept apart from an object's inline bytes, required (below).

**Delivers.** The `BucketListing` target: live entries, every entry of a row, the listing's projection,
never an object's bytes; `client.bucket::<M>().list()`; the bucket's `scan` grant; `shoalctl`'s bucket
listing; the bench's `list` kind ([W9](bucket-listing.md)).

**Acceptance.** [W9](bucket-listing.md#acceptance-tests)'s and [W12](performance.md#acceptance-tests)'s
M26 rows.

**Evidence/exit.** The budget beside a listing on the lab; entries a second a node, for a bucket of
inline objects and one of striped ones.

*Not at this gate*: path order, a prefix, a delimiter.

### M27. Ordered scans by declared order

**Closed before it.** Q40, how a declared order is kept, by [X21](spikes.md#x21-declared-order-priced).

**Delivers.** The `place`, `order` and `cover` attributes and a table's `spread`; `OrderedKey` and the
derive's encodings; the order index in each tablet, kept at apply and compaction and carried by an
install, a restore, a move, a rehome and a scrub; collisions refused by name; ordered scans and spread
merges in the driver; generated placement and order types ([W8](declared-order.md)).

**Acceptance.** [W8](declared-order.md#acceptance-tests)'s M27 rows.

**Evidence/exit.** X21's prices confirmed on what was built: gets and inserts of a declared-order table
beside the same table placed by its whole key, and ordered pages a second, covering and not.

*Not at this gate*: an ordered listing.

### M28. The ordered listing

**Designed, later.** **Closed before it.** Q41, by [X22](spikes.md#x22-the-ordered-listing-two-ways).
**Delivers.** A listing in the byte order of paths, with a prefix, a delimiter's common prefixes and a
continuation, on the layout X22 chose ([W9](bucket-listing.md#the-ordered-listing-a-later-gate)).
**Acceptance** and **evidence** are set at its gate.

## Beside the gates

~~None of these is needed by a gate, and each can be added later without changing anything built:~~
None of these is needed by a gate, and each could be added later without changing anything built.
**Since 2026-10-31 every one is planned**, decided with the user once the spikes before M23 had
reported, each where it costs least or can first be measured, which
[the work beside the gates](whats-left-todo.md#the-work-beside-the-gates) draws. ~~O144 and O145, added
after M23's exit, are optional and not planned.~~ O144 and O145, added after M23's exit, are optional, and
were worked through (2026-11-08 and 09) with the rest of the plan's *After M23, optional* box.

**After M23**, on the rebuilt walk:

- ~~**Larger or consolidated merge blocks** ([W42](scan-speed.md#the-levers-in-the-order-they-land)). A
  larger block is a setting, not code: a merge's block is a WAL segment
  (`cluster.replication.segment_bytes`, which an inventory renders), so M23's lab exit churns at
  10 MiB segments and at 40 MiB, which sets the default and says whether an archive pass consolidates a
  scanned table's blocks as well ([X37](tablet-records-together.md#after-churn)).~~
  **Larger merge blocks, by larger WAL segments**
  ([O149](../../appendix/optimizations.md#o149-larger-wal-segments-keep-a-tablets-records-together-four-times-longer)),
  after M23 as the user decided on 2026-10-31. M23's exit churned at both sizes. It found
  consolidation not needed, nine partitions a device read and more after an hour's churn (Q62). It
  also found four times a tablet's records together a read at 40 MiB, and left the default to what a
  larger segment costs a node's foreground, its memory and its start, measured on their own. Once
  that is done, **W12's rate is read again at the segments it sets**, where the same share of
  75 ms bought a 970 EVO 35 MiB a second. ✅ **Done** (2026-11-09): measured side against side on the
  lab, 40 MiB is the default, no figure the user's rule named disjointly worse, and W12's rate read again
  in the same rounds, 26.7 MiB a second of a Zen1 host's 970 EVO against 13.9 at 10 MiB
  ([the record](../../appendix/optimizations.md#o149-larger-wal-segments-keep-a-tablets-records-together-four-times-longer)).
- **A shard's io_uring workers off its cpu**
  ([O147](../../appendix/optimizations.md#o147-each-shards-io_uring-workers-run-on-the-shards-own-cpu)),
  after M23 as the user decided on 2026-10-31. Every ring's io-wq workers have their executor's
  one-cpu affinity, so a WAL batch's sync preempts its shard's loop, a foreground step's as much as
  a scan's. Moving them changes every sync's path, so it is measured on its own, beside the cell's
  p99. ✅ **Done** (2026-11-10): built as [F139](../../features/io-workers-off-the-shards.md), an opt-in,
  and measured on the lab. Off their cpus the Zen1 shards waited a tenth as long, and the cell lost a
  quarter of its inserts to syncs sent through fewer of the drive's queues
  ([O171](../../appendix/optimizations.md#o171-a-nodes-syncs-sent-from-fewer-cpus-go-through-fewer-of-its-drives-queues)),
  so `own` stays the default
  ([the record](../../appendix/optimizations.md#o147-each-shards-io_uring-workers-run-on-the-shards-own-cpu)).
- **A page's reads let go as they fold, and partitions together in an encode step**, optional
  ([O144](../../appendix/optimizations.md#o144-a-scan-page-holds-its-reads-beside-the-partitions-folded-from-them),
  [O145](../../appendix/optimizations.md#o145-a-planned-partitions-encode-step-is-a-message-through-its-loop-each)):
  each was to be weighed at M23's exit. The exit reported no scan budget peak, and the drivers'
  figures were no worse, so neither is M23's. O144 is moot since
  [F134](../../features/walked-records-uncopied.md), a page handing out its reads' own buffers, and O145
  is applied (2026-11-08) as [F137](../../features/encode-steps-together.md).
- **Always-on measuring logic behind a feature flag, measured**, optional
  ([O153](../../appendix/optimizations.md#o153-always-on-measuring-logic-behind-a-feature-flag-measured)),
  filed at the user's request on 2026-11-01. F130's clock, item 353's fault counts and the counters a
  query or an object operation bumps each cost about a microsecond or less; whether their sum is worth
  a feature that compiles them out of a product build is measured on the rebuilt walk first, beside
  the cheaper ways of reading a figure only where it is judged. Measured on the lab (2026-11-09) and
  not taken: a build with every candidate figure stubbed out moved nothing past its noise.
- **Polled NVMe queues on a host, and the WAL on a device of its own** ([W42](scan-speed.md#the-levers-in-the-order-they-land)):
  a host's settings and a deployment's choice, the two roots already allowing the second, priced on
  the rebuilt walk, where runs cut the ring's share of a node's samples from about a quarter to under
  a tenth.

**With M24**, as it first defines what an answer carries, so no later wire version is needed:

- ✅ **Whole partitions in answers** for an unfiltered scan, the client decoding them
  ([Q35](contract.md#questions-to-answer)); [X17](page-rebuilt.md#rows-encoded-on-the-shard-against-walked-partitions)
  moved it forward. Its shape is settled since 2026-11-06 ([Q35's record](contract.md#q35-what-a-scans-answer-carries-2026-11-06)):
  an opt-in, `.partitions()`, refused with a filter or a projection, under a layout version of its own.
  **Done the same day** ([F132](../../features/whole-partitions-in-answers.md)), below M24's frames as the
  user chose: one archived definition in `shoal-proto` the engine's partitions archive as,
  `PARTITION_LAYOUT`, the chunk head, the shard's writer and the client's reader. M24 carries it and adds
  the request's half.

**After M24**, once a client scans one node:

- ✅ **The next cursor sent ahead of a page's rows**, so a stream's next request leaves sooner
  ([W4](cursor-and-pages.md#alternatives-rejected)), which needs no new frame and is priced on M24's
  streams. Priced by [X39](cursor-sent-ahead.md) on 2026-11-15 and not taken: it bought a scan nothing at two
  streams a shard and cost the cell's reads 6% at one. Since Q36 an answer's head, with its `reached`, goes before its body's data frames, and every
  chunk names the last key it finished (`Through`), so neither this nor rows handed on chunk by chunk
  needs a frame of its own.
- ✅ **A scan connection handed to the executor of its slot**, saving the hop of a page's bytes
  ([W6](distribution.md#streams)). Priced by [X41](answer-hop-priced.md) on 2026-11-15 and not taken: the hop cost
  an answer 1.07 times the cpu, 0.07% of a Zen1 core at the default share.
- ✅ **Saved positions between pages** ([W5](tablet-reader.md#saved-positions-optional)), priced on a
  table whose tablets take many pages. X17 (2026-10-30) found nothing to save on the lab's tables,
  every tablet one page. Priced by [X40](saved-positions-priced.md) on 2026-11-15 at the depth a lab node holds
  of the smallest rows and built as [F150](../../features/saved-positions.md).
- ✅ **A client helper that steps through keys a caller names**, as pipelined gets ([W2](ordering.md#alternatives-rejected)),
  built as [F149](../../features/key-steps.md) on 2026-11-15.
- ✅ **A count of scans a principal**, so one tenant cannot take every shard's class
  ([W7](isolation.md#admission)), built as [F148](../../features/scan-shares.md) on 2026-11-15.

**Taken up on 2026-11-15**, the user asking for every row ([Finishing After M24](whats-left-todo.md#finishing-after-m24)
draws the steps). The cursor sent ahead, saved positions and the hand-off are each measured first, by
[X39](spikes.md#x39-the-next-cursor-sent-ahead-priced), [X40](spikes.md#x40-saved-positions-between-pages-priced) and
[X41](spikes.md#x41-a-scan-answers-hop-between-shards-priced), and kept only where a rule written before the run says
so. A count a principal is a fair share of each shard's scan class when principals contend, and half the scan budget
at most for one principal's unwritten answers. Items 380, 386, 387, 388 and 389 are fixed with the rows they lie on.

**Done on 2026-11-15**, every row green: the count a principal, the helper and saved positions are built
([F148](../../features/scan-shares.md), [F149](../../features/key-steps.md), [F150](../../features/saved-positions.md)),
the cursor sent ahead and the hand-off are measured and not taken, and items 380 and 386 to 389 are fixed. The exit's
review found seven more, items 394 to 400, all fixed, and filed items 401 and 402 and O183; its workspace runs found
item 394's first fix wanting, finished, and a row of F148's asserting what its premise did not need, changed, and
item 403 was filed while finishing the first. The reference cell before and after
on titan was within on every figure ([Finishing After M24](whats-left-todo.md#finishing-after-m24) says what each
found).

**After M25**, once scans run across a cluster:

- **Aggregates pushed to the shards**, a count or a sum answered by each page as a partial, as
  ScyllaDB's parallelized aggregation does ([W3](prior-art.md#scylladb)).
- **A scan coordinated by a node**, for a client that is not Rust ([Q42](contract.md#questions-to-answer)).
- **Scan work on executors of their own**, Q37 asked again then ([W7](isolation.md#cpu)); X17's and
  X18's cpu a partition at a ceiling's rate gave no reason to move it sooner.

**Before M26**, decided by [X19](spikes.md#x19-the-edges), which prices a listing's bytes, ✅ **required** since
2026-11-07, with the user:

- **A listing's fields kept apart from an object's inline bytes**, so a listing reads less
  ([W9](bucket-listing.md#what-a-listing-reads)); it changes what a bucket stores, so it costs least
  before buckets hold much. X19 measured a listing at the default threshold reading 18,139 device bytes an
  entry for 312 of fields, 901 entries a second on titan at the pace.

## The order is a claim

The order above argues for itself:

- **The engine before the wire.** A scan's cost to the queries beside it is proven on the rebuilt walk,
  driven in process, before any client can ask for one: the same reason X9 came before the object
  store built a device.
- **One node before a cluster.** The frames and the client against one node, then the driver's
  copies, floors and refusals across many, as M14 came before M15.
- **Scans before a listing.** A listing is a scan with a target and a projection.
- **The listing before ordered scans**, by the decision of 2026-10-25: an unordered listing first.
- **Ordered scans before the ordered listing.** Every way to list in path order stands on W8's index.

Two places in it are convenience, not dependency: M26 needs M25 and nothing after it, and M27 could
start beside M26 once X21 reports.

## Related

[W1](prerequisites.md) for the rows the gates begin with; [Spikes](spikes.md) for what has to report
first; [W13](contract.md) for the questions and the record; [What's left to do](whats-left-todo.md) for
all of it drawn.
