# What's left to do

**The order of the work, drawn.** The pages before this one say what has to be done and why; this one
says in what order, ~~on two diagrams: the work up to the first gate, and the gates after it~~ on a
diagram of each part: the work up to the first gate, the gates, what is left of M23, and the work
beside the gates; and at its end on [one diagram of the whole plan](#the-whole-plan). ~~As of
2026-10-25, when this plan was drawn, nothing on either diagram is done.~~ As of 2026-10-29 the contract
is agreed ([W13's record](contract.md#before-m22-the-contract-agreed-2026-10-29)), and the bench's scan
kind and event ([F115](../../features/bench-scans.md)), the admission class
([F116](../../features/scan-admission.md)), the scan class ([F117](../../features/scan-class.md)), the
budget ([F118](../../features/scan-budget.md)) and the device pacing ([F119](../../features/scan-pacing.md))
are built, and X16 is measured ([its record](walk-beside-the-foreground.md)), so M22's three inputs are
in and X17 and X18 can start; X20, optional, is read and run ([its record](scylladb-scan-sources.md)); and
since [W42](scan-speed.md), the same day, two more W1 rows and X37 feed M23, which is now judged on a
scan's own rate and cost too. ~~Nothing else on either diagram is done.~~ Since 2026-10-30 those two rows
are built ([F120](../../features/one-operation-a-record-read.md), [F121](../../features/merges-in-key-order.md)) and
M22 is delivered ([F122](../../features/scan-oracle.md)), so the first gate is behind the plan; ~~nothing else is
done~~ and X37, both halves of X17 and X18 are measured ([X37](tablet-records-together.md),
[X17](page-rebuilt.md), [X18](table-reads-paced.md)), X17 drawn as its two halves. ~~Nothing else is done.~~
Since 2026-10-31 M23's seven steps are built (F123 to F129) and its exit is run on the lab
([its record](m23-exit.md)): its box stays open, ~~since four of its clauses are the user's to read on the
record's evidence, and its step clause waits on
[item 350](../../appendix/resolved/encode-copy-deadline.md)~~
since the user decided its four open clauses the same day ([the decisions](m23-exit.md#decided-with-the-user-2026-10-31)):
item 350 is fixed and the exit's rounds are run again with a cell whose feed does not wrap, which
[Finishing M23](#finishing-m23) draws, and O147 and O149 follow M23. Since 2026-11-01 that cell is
built and checked on the lab ([its record](m23-exit.md#the-cell-for-the-rounds-again)). It found item
351, the cell's own inserts falling about 75 s into every arm, wrap or not, which the rounds again
~~wait for too~~ waited for too. Item 351 is resolved the same day: the lab's 970 EVOs flush three
times slower after about 75 s of synced writes, so the rounds again warm every cell arm past it
([Resolved #351](../../appendix/resolved/the-cells-fall-is-the-drives.md)). Item 350 is fixed the same
day, once a lab run of F130's counts named its steps as encode steps that took three to five times
too many rows ([Resolved #350](../../appendix/resolved/encode-copy-deadline.md)); ~~its box turns green
once the same rounds on the fixed build confirm it~~ the same rounds on the fixed build confirmed it, and
its box is green. Item 353, which that run found, is M23's to attribute before the rounds again, decided
with the user the same day. ~~Nothing else is done.~~ It is attributed since 2026-11-02, by six of the
exit's rounds that counted each long step's faults and each shard's steps beside its cpu's interrupts,
two of them with the NIC's interrupt moved off the shards: the light steps are their own work, not the
interrupts their cpu takes, and its box is green. Its fix, a capture timed by its parts and the part that
holds a long one bounded, ~~is under way since the same day~~ is done since the same day, and its box is
green: no part's work held them, but the allocator inside a capture's allocations, so every node's
allocator hands freed pages back at 100 ms and a capture allocates each collection once
([Resolved #353](../../appendix/resolved/capture-allocations.md)). ~~Nothing else is done.~~ The exit's
rounds were run again on 2026-11-03, with four controls among them, and their box is green
([their record](m23-exit.md#the-rounds-again)). The insert clause held over the whole arm beside both
scans. The read clause beside `Big` missed at 1.14 times, since the cell reads faster without a scan
than it did
([item 359](../../appendix/resolved/the-cells-own-climb.md)).
The step clause missed, on remainders already placed after M23. ~~Both are the user's to decide before
M23 closes. Nothing else is done.~~ Item 359's runs showed the 1.14 is the cell's own climb, not the
scan's: a twin of the arm with its scan paced to almost nothing rose as far
([Resolved #359](../../appendix/resolved/the-cells-own-climb.md)). **M23 closed on 2026-11-04**, decided with the user
([the decisions](m23-exit.md#decided-with-the-user-2026-11-04)): the read clause held against that
twin, and the step clause met with its remainders apart, each after M23. Every box of
[Finishing M23](#finishing-m23) is green. M24 is next, under way since 2026-11-10 and drawn down to its
steps under [Finishing M24](#finishing-m24). Since 2026-11-06 Q35 is settled with the user
([its record](contract.md#q35-what-a-scans-answer-carries-2026-11-06)): rows encoded on the shard by
default, and partitions for an unfiltered, unprojected scan that asks, which the box *Whole partitions in
answers* ~~builds with M24~~ built ahead of it the same day, and its box is green
([F132](../../features/whole-partitions-in-answers.md)): the body of an answer in partitions, below M24's
frames, which M24 carries. Item 365, found on the way, is filed under *Found on the way, any time*.
Q36 is settled the same day by design ([its record](contract.md#q36-the-cursor-and-the-frames-2026-11-06)):
a page's progress one key, cursors that are keys with a sort-key position reserved, floors a group
learned from the answers, and fixed heads for `ScanPage` and `ScanAnswer`. The user settled it before
X19, which ~~still comes~~ came before M24 and decided whether M24 splits a sorted partition across pages in the
reserved position. Item 366, found on the way, is filed beside 365, and F132's items 367 and 368 beside
them. **X19 reported on 2026-11-07** ([its record](the-edges.md)), and its box is green. The user decided from it
([the decisions](contract.md#x19-measured-the-edges-decided-with-the-user-2026-11-07)) that no partition may fail or
go missing from a scan: M24 splits a sorted partition too large for a page across pages, and three pieces of work
land before it, on [the work beside the gates](#the-work-beside-the-gates). A listing's fields kept apart is
required before M26. **M24 was delivered on 2026-11-14**, once
[the confirmation](link-pace.md#the-confirmation) held its exit's resident half at the default share on item 392's fix,
with the scan's twin in every round. Every box of [Finishing M24](#finishing-m24) is green, and M25 is next.
Q38, which copies a scan reads and how its levels are served, ~~is the one question left before it~~ was the one
question left before it, **settled on 2026-11-14**. Its levels and which copies may be read at all are settled by
design. The user asked to have measured which of a tablet's voter copies a scan prefers, followers then leaders
against every voter alike: spike [X38](spikes.md#x38-which-copies-a-scan-reads-two-ways), drawn before Q38 on the
gates' diagram. It read the two as alike, and the rule gave the tie to followers then leaders
([its record](copies-two-ways.md)). Its control ran the bench's rotated driver at the same commit and found what both
options share, dealing and taking, worth 5.1 times beside the cell. It also found that walks of one tablet at a time,
asked in no key order, read their map ahead for nothing: [O181](../../appendix/optimizations.md#o181-a-walk-of-one-tablet-in-no-key-order-reads-its-maps-sixteen-pages-ahead-for-its-next-walk-to-miss),
which is optional and drawn under *Found on the way*. Nothing is left open before M25. **M25 is under way since
2026-11-15**, drawn down to its steps under [Finishing M25](#finishing-m25).

**How to read them.**

- An arrow points from a piece of work to what requires it. Things on one level, with no arrow
  between them, can be done in parallel.
- A green box with a ✅ is done.
- A box titled *(optional)*, with a dashed border, can be left out without anything built having to
  change; a dashed arrow means *helps*, not *requires*. Each one's reason is on the page that lists
  it.
- A box beside a gate lands no later than the start of that gate, and nothing stops it landing
  sooner ([W1](prerequisites.md#the-order)).
- A box named for an item or an O number is a defect or an optimization found on the way, drawn
  where it belongs: before what it blocks, after what it is scheduled after, or under *Found on the
  way, any time* when no gate waits on it.

The full list of what each gate requires is on [the milestones page](milestones.md#the-gates-at-a-glance);
the diagrams draw an arrow only where no longer path already implies it.

## Before the first gate

Everything in the first box can start now. The spikes after it wait on X16's baseline, which says
which part of the walk hurts a foreground first.

```reef
focus = "w1"
direction = "LR"
page = "../../plan.md"
```

The two W1 rows and X37 that [W42](scan-speed.md) added feed M23, on the second diagram. X20 feeds no
gate. It read ScyllaDB's scan path at a pinned release and ran a ScyllaDB on the lab
beside X16's foreground; [W3](prior-art.md) marks what it confirmed, and W13 records its run as
evidence for Q33, Q34 and Q37 ([the record](contract.md#x20-run-evidence-for-q33-q34-and-q37-none-settled-2026-10-29)).

## The gates

```reef
root = "scans-plan"
expand = "none"
page = "../../plan.md"
```

The chain is the claim [the milestones page](milestones.md#the-order-is-a-claim) argues for: the engine
before the wire, one node before a cluster, scans before a listing, the listing before ordered scans,
ordered scans before the ordered listing. Two of its places are convenience: M26 needs M25 and nothing
after it, and M27 can start beside M26 once X21 reports, which the diagram draws as the siblings they
are. ~~The rest of the optional work, a stepped-keys helper, scan work on executors of its own, the next
cursor sent ahead of a page's rows and a count of scans a principal, is on the milestones page; it
changes no box here.~~ The work that was optional, any time, is planned since 2026-10-31 and drawn
below, beside the gates it follows.

## Finishing M23

~~M23's box on the diagram above stays open until the work drawn here is done.~~ The work drawn here is done, and M23's box on the diagram above is green since 2026-11-04 ([the decisions](m23-exit.md#decided-with-the-user-2026-11-04)). Its seven steps are built,
and its exit ran on the lab on 2026-10-31. The exit's quick passes found four things, fixed before its
rounds; the rounds left four clauses to the user, decided the same day
([the decisions](m23-exit.md#decided-with-the-user-2026-10-31)). Two of those decisions leave work in
M23: item 350, attributed and then fixed, and a cell whose own feed does not wrap inside an arm. The
cell is built since 2026-11-01 ([its record](m23-exit.md#the-cell-for-the-rounds-again)): 1,100,000
rows, and the nodes at 6 GiB, decided with the user, since a cell that never wraps keeps every row it
inserts. Its check round found that the wrap was not what changed the cell's inserts: they fall about
75 s into every arm, wrap or not, scan or not
([item 351](../../appendix/resolved/the-cells-fall-is-the-drives.md)).
Item 351 is M23's, decided with the user the same day, and is found and fixed before the rounds again.
~~It is~~ It was the lab's 970 EVOs: about 75 s into the cell's synced writes their flush goes from
0.82 ms to 2.83 ms and stays until they rest, and fio alone does the same with no Shoal unit up. So
the rounds again warm every cell arm 100 s before it measures, decided with the user on 2026-11-01
([Resolved #351](../../appendix/resolved/the-cells-fall-is-the-drives.md)). The exit's rounds are then
run again, a control beside them, and the insert and step clauses read on those rounds. The other two clauses are read on the evidence there is: W12's cost above the idle node,
and W12's rate as the share's.

The rounds again ran on 2026-11-03 at `b3a4d7ef`, with every fix above
([their record](m23-exit.md#the-rounds-again)). There were eight rounds and four control rounds, one
after every two, decided with the user the same day. A control round is the cell alone in a
`Keyword` capture's place, for longer than any whole scan, read a second at a time beside the scan
arms. Their box is green. They leave a box after it, the read and step clauses decided with the
user, before M23 closes:

- **The insert clause held** over the whole arm, 1.04 times beside `Big` and 1.00 beside `Keyword`.
  The control's inserts rose as far.
- **The read clause beside `Big` missed**, 1.14 times [1.10, 1.17], where the first exit read 1.07.
  The cell alone over the same seconds fell, so the rise ~~is~~ looked like the scan's. The scan did not get dearer:
  the cell reads about half a millisecond faster without it since `345e17dd`, and its p99 beside the
  scan stayed at 5.8 ms
  ([item 359](../../appendix/resolved/the-cells-own-climb.md)).
- **The step clause missed**, on the remainders the user has placed after M23: europa's `Keyword`
  encode steps (Resolved #350's), `Big`'s long captures (O154's) and steps past 100 µs by the wall
  alone (O147's), at the confirmation runs' rates.

~~Whether item 359 is M23's, or W7's read clause is restated for a scan as decision 1(c) of M23's plan
allows, is the user's to decide ([the record](m23-exit.md#for-the-user-to-decide-1)), and so is
whether the step clause is read as met with those remainders apart.~~ Item 359's runs (2026-11-04)
showed the 1.14 is the cell's own climb: the arm with its scan paced to almost nothing rose as far.
So the user decided the same day ([the decisions](m23-exit.md#decided-with-the-user-2026-11-04)):
- item 359 is resolved as not the scan's;
- the read clause is held, judged against that twin;
- the step clause is met, its remainders apart, each after M23.

M23 closes, and its box is green on every diagram of this plan.

```reef
focus = "m23steps"
direction = "LR"
page = "../../plan.md"
```

Counting a loop's steps by kind, a page's capture apart from an encode step, with each step's bytes,
rows and own cpu beside its wall time, ~~comes~~ came first: item 350 is not yet reproduced, and its three
candidates need different fixes. It is done since 2026-11-01
([F130](../../features/loop-steps-by-kind.md)): a node counts each kind's steps past 100 µs by the
wall and on their own cpu, and what a long step held, and `m23-report.py` prints them by arm and
member. Item 350's lab run reads them first, decided with the user the same day. That run (2026-11-01, four of the
exit's rounds) named item 350's steps as encode steps, held on their own cpu with three to five times
the rows of their arm's steps, and a piece's copy now ends at a deadline
([Resolved #350](../../appendix/resolved/encode-copy-deadline.md)). ~~Its box turns green once the same
rounds on the fixed build confirm it.~~ The same rounds on the fixed build confirmed it: `Keyword`'s
long steps hold 1.3 to 3.5 times their arm's rows where they held 3 to 6.5, and on the Zen1 hosts they pass
100 µs as often as the light steps every arm holds. The run also found those light steps past 100 µs
that F130 counts as their own cpu, on kernels that charge a cpu's interrupts to the thread it
interrupts
([item 353](../../appendix/resolved/capture-allocations.md)),
~~drawn under what was found on the way until it is decided whether it is M23's~~ and the user decided
the same day that it is M23's to attribute before the rounds again, so the step clause is read knowing
whether a step's own cpu is its work; a fix waits for what that shows. It is attributed since
2026-11-02, and its box is green: on the shard whose cpu took every NIC
interrupt the light steps passed 100 µs as often as on the others, moving the NIC changed nothing, they
followed no softirq, almost none faulted, and `Big`'s captures passed thirty times as often as `Keyword`'s
on the same cpus at about the same length. They are the steps' own work, `Big`'s long captures in the part
of a capture past its keys
([item 353](../../appendix/resolved/capture-allocations.md)).
The user decided the same day that its fix is M23's, before the rounds again: a capture timed by its
parts to name the one that holds a long `Big` capture, then that part bounded, as items 347 and 350
bounded an encode step; the rare long encode steps, about once in 10,000, are read by the step clause as
the steps' own. Since 2026-11-02 a capture is timed by its parts, each by the wall
([F130](../../features/loop-steps-by-kind.md), extended), and ~~six of the exit's rounds on that build name
the part; its bound follows from what they name, decided with the user the same day, and its box turns
green once six more rounds on the fixed build confirm it~~ six of the exit's rounds on that build named
no part: each long capture held one part 100 to 300 µs, whichever ran when an event of its own size
came. Two rounds with the allocator's hand-back of freed pages off halved them, the plan's nearly all
gone, so every node's unit now runs mimalloc's purge at 100 ms, and the view's own parts are timed for
the rest, decided with the user the same day. ~~Two rounds on that build name the view's, six more
confirm it, and its box turns green then.~~ Two rounds on that build found the 100 ms delay moved the
plan's half little and the view's held by its delta's copy, its allocations' and not its work's, so a
capture now allocates each of its collections once at its size, decided with the user the same day;
~~two rounds show whether its long captures fall, six more confirm it, and its box turns green then~~
six rounds on that build confirmed it, and its box is green
([Resolved #353](../../appendix/resolved/capture-allocations.md)): the Zen1 hosts' long `Big` captures
fell from 21.8 to 11.4 a 10,000, the plan's from 9.1 to 2.1. The rest, inside the view's few
allocations left, the step clause reads as the steps' own, and O154 is filed for them. The step clause is
read on a step's own work, and a step past 100 µs by the wall alone is named as O147's, which follows
M23. Items 348 and 349, also found by the
exit's rounds, are not M23's: each was the same before M23. Item 349 is drawn below, ~~and~~ fixed since 2026-11-05
([Resolved #349](../../appendix/resolved/shed-pages-logged-quietly.md)), and item 348,
the object store's, on [its plan's diagram](../../object-storage/whats-left-todo.md#the-gates).

## Finishing M24

M24's plan was agreed with the user on 2026-11-10. Three things were decided with it:

- **The lab exit is part of the gate**: M24 is built, its exit run on the lab, and only then marked
  delivered, as M23 was.
- **`all` includes `scan`**, which settles [Q39](contract.md#questions-to-answer) ahead of M25, since
  M24 reads the `scan` privilege from a standalone node's file and has to say what `all` means there.
- **M24 is built in steps, a feature page and a commit each**, drawn here in the order they are built.

Each step lands with its acceptance rows passing. The frames come first, since every later step speaks
them; the page over a shard's tablets, with its bounds, and then the split of a sorted partition across
pages, are the engine's half and are proven in process before anything is on a wire; the answer in rows follows
it, then the page served to a connection, the client's driver, and what a caller sees: SHQL's `SCAN`,
`shoalctl`'s paging, the bench's client scan and the example. The exit is a workspace run and its
counts, a security review of the gate's changes, the reference cell before and after on a Zen1 host,
and the budget beside a client's scan over the wire to a standalone node on the lab
([the milestones page](milestones.md#m24-scans-on-one-node) has its evidence).

The frames are built since 2026-11-10 ([F140](../../features/scan-frames.md)), and their box is green:
`ScanPage`, `ScanAnswer` and the resume token as bytes in `shoal-proto`, the three refusal codes, the
capability bit, and `scan` as a privilege `read` never implies. Nothing sends one yet.

The second step was cut in two as it was built, and the steps after it renumbered: the page and its
bounds first, then the split of a sorted partition too large for a page, which changes the walk of one
tablet where the page does not. **The page is built** since 2026-11-10
([F141](../../features/scan-pages-over-tablets.md)), its box green: a page over the tablets of one
executor, read in ascending order until its rows, its answer bytes, its work, its deadline or the end
of its tablets, a tablet another executor hosts refused `HostedElsewhere`, a tablet that holds nothing
skipped on the loop, and its progress one key. It is a task over M23's walk of one tablet, admitted
once.

**The split is built** the same day ([F142](../../features/split-partitions.md)), its box green: a page
judges each partition before it reads it, ends before one that does not fit what it has left, and hands
a sorted partition that fits no page out part way, cut after a record of its archive or a piece of it
in memory. Its answer's position is the last sort key handed out, and the next page goes on from that
row, each record read once. [O172](../../appendix/optimizations.md#o172-a-partition-handed-out-part-way-under-an-overlay-or-fragments-reads-more-than-its-page),
found building it, is drawn under *Found on the way, any time*: a partition under a resident overlay,
or fragments, still reads more than its page. It is optional and before no gate.

**The answer in rows is built** the same day ([F143](../../features/scan-rows.md)), its box green: a
page's rows held to the scan's filter and built as its projection on the shard, in chunks of a get's
answer, and `MovieScan` generated beside each get. A page's bytes are then its rows' and its work what
it read, so a filter that keeps little ends its page at its work.

**The page on the wire is built** ([F144](../../features/scan-pages-on-the-wire.md)), its box green: a
standalone node grants scans beside streams and cancels, judges a page and refuses by name what it will
not serve, streams its answer from the buffers the page was read into with the page's draw held until
its last frame is written, and stops a cancelled page at its next step.
[Item 380](../../appendix/known-issues.md), found building it, is drawn under *Found on the way, any
time*: an answer queued to a client that never reads holds its draw until that client's connection ends.
It blocks no gate; a count of scans a principal, after M24, is the larger half of its cure.

**The client's driver is built** ([F145](../../features/scan-driver.md)), its box green:
`client.scan(MovieScan::new())` reads a table whole from a standalone node, its streams learned from
the pages' own `HostedElsewhere` refusals, its rows handed on only once a page's answer is whole and
held to its page, a shed or cut page asked again under a fresh id, a resume token over exactly the pages
handed out, and `split(n)`. Its acceptance row for P24 found
[item 381](../../appendix/resolved/empty-get-refused.md), fixed with it and drawn green under *Found on
the way*: a get that named no key was never answered at all, where the contract had called it refused.

**SHQL's `SCAN`, shoalctl's paging, the bench's client scan and the example are built**
([F146](../../features/shql-scan.md)), its box green, and with it every build step of M24. What is
left is the gate's exit: the workspace run, the reviews, the query path's before and after on a Zen1
host, and the lab exit.

**The workspace run, the counts and the review are done** since 2026-11-12, their box green. Every
check and feature build of the runbook passed. The workspace run (ws22) ran 3,200 tests: 3,198
passed, and two of `shoaladm`'s unit tests failed on an expectation F140 had made stale, `read,
write` being every privilege, corrected; nothing failed under load. The gate's changes, some 12,400
lines of Rust over `5a1c162f`, were then read by two readers given the diff and not each other's
conclusions, one for what a client, a node and a token can do to each other, one for stability and
cost, and every finding was checked against the source before anything was done with it. Neither
found a memory safety defect, a way round the `scan` privilege or a panic a frame can reach: every
count is capped before anything is sized by it, a spec and every chunk are validated before they are
read, the refusals keep their order, a cluster node grants no scans, and a get's path gained one
compare.

Four defects were found, each reproduced by a test before it was fixed, drawn green under *Found on
the way*:

- [Item 382](../../appendix/resolved/dropped-rows-stepped.md): a scan in rows held its shard for
  every row its filter dropped, since only the chunks it wrote were counted into its steps.
- [Item 383](../../appendix/resolved/stalled-page-answered.md): a page given up for making no
  progress was never answered, its `Timeout` dropped as a cancelled page's answer is.
- [Item 384](../../appendix/resolved/scan-patience-counts-its-own-wait.md): a scan's patience
  counted the time its caller took between pages, so a paused scan ended at its next shed page.
- [Item 385](../../appendix/resolved/position-search-bounded.md): a page looking for its position
  after a merge, or from a position a client changed, read the whole partition outside its work.

Four more are filed and drawn, none of them before M24: [item 386](../../appendix/resolved/scan-pages-held-to-the-bound.md), a
scan asking more pages of a connection than its node serves there, on a node of more than a few
executors, whose cure is a choice of what that bound means and is the user's; [item 387](../../appendix/resolved/reached-never-in-a-refused-tablet.md), a page that
refuses a tablet part way answering a key its own client refuses, which no standalone node produces
and which is drawn before M25; [item 388](../../appendix/resolved/scan-body-held-to-its-page.md), a
client allocating a page's body before its answer is held to the page that asked it; and [item 389](../../appendix/resolved/rows-page-shed-answers-what-it-covered.md), a page in rows shed at a chunk refused whole. Five
optimizations, [O173](../../appendix/optimizations.md#o173-a-record-is-validated-up-to-three-times-on-a-page-that-cuts-it)
to O177, are optional and before no gate. The three tests the fixes added take the count to 3,203.

**The reference cell before and after is done** since 2026-11-12, its box green
([F144's page](../../features/scan-pages-on-the-wire.md#the-reference-cell-before-and-after-m24) has
its table). The grid's reference cell on titan, the commit before M24's first against M24 as built,
eight rounds with the first side alternating: every figure within, the cell's rate 8,087 operations a
second before and 8,064 after. The same cell reading alone and the storage-free control were run
beside it, since the cell's writes wait on a sync that would hide a small cost a query; both within,
the reads-only arm over twenty-four rounds once its first eight left its mean 2.8% apart inside
overlapping ranges. M24's comparison in the read relay and its arm in the loop's dispatch cost a query
nothing these arms can see, and nothing was filed. ~~What is left of the gate is the lab exit.~~

**The exit's rounds on the lab are run** since 2026-11-12, their box green ([M24's exit](m24-exit.md)).
Eight rounds of a client's scan over the wire to a standalone node on titan, the reference cell on the
scan's own client on europa, every scan handing out every row once. Beside a scan of archived rows the
budget held, in rows, in partitions, filtered and projected. **Beside a scan of resident rows it
missed**: nothing paces a resident row, the scan filled the gigabit link, and the cell's read p99 rose
3.5 times and its insert p99 1.63 times.
[Item 390](../../appendix/resolved/resident-scan-fills-the-link.md) is that
miss, drawn between the exit and the gate: **M24 is not marked delivered until the user decides what
the budget asks of a link**. The exit also priced [item 386](../../appendix/resolved/scan-pages-held-to-the-bound.md), a page shed
for nearly every page answered at the client's defaults on five shards, and found
[O178](../../appendix/optimizations.md#o178-a-clients-connection-probes-three-in-a-hundred-of-its-small-answers-for-a-loss-that-did-not-happen),
a client's connection probing its small answers for a loss that did not happen, optional and drawn under
*Found on the way*. Its node and driver add seven tests, which takes the count to 3,210.

**Item 390's cure is built and swept** since 2026-11-13, both boxes green. The user chose that the node paces a
scan's answers to a share of its link's speed ([F147](../../features/scan-link-pace.md)). [The sweep](link-pace.md#the-sweep)
on the exit's resident arm found the pace taking off what the link added: at a quarter of the link and under,
the cell's read median is its own, its read p99 falls from 3.33 to about 2 times, and its insert p99 from 1.59 to
1.16. No share holds the budget, though. A resident scan costs the reads beside it ~~on its node~~ about 0.4 ms at p99 at
any rate, beside another client's cell too, and more beside scans that send almost nothing:
[item 391](../../appendix/resolved/paced-scan-reads-and-inserts.md),
drawn before the gate. **Decided with the user the same day**: item 390 is resolved as the link's part, its box green,
item 391 is chased next and M24 waits on it, and the share stays at 25%
([the decisions](link-pace.md#decided-with-the-user-2026-11-13)). F147's fourteen tests take the count to 3,224, every one passing in a workspace run.
Chasing item 391's insert part found its cause, [item 392](../../appendix/resolved/scan-map-reads-paced-and-read-ahead.md):
a scan's walk reads its table's archive map from the device on every page, unpaced and uncounted, even over resident
rows, beside the intent logs' fdatasyncs ([the record](link-pace.md#a-scans-walk-reads-the-map-from-the-device)), drawn
before item 391; and [O179](../../appendix/optimizations.md#o179-a-shard-holds-its-cpu-through-its-eevdf-slice-while-the-io-wq-worker-running-its-fdatasync-waits),
a shard holding its cpu through its scheduler slice while the worker running its fdatasync waits, optional and drawn
under *Found on the way*. **Item 392 is fixed** (2026-11-14), its box green: a spike that served the map from memory showed
it the insert part, and decided with the user a walk's cursors now read the map sixteen pages ahead into a scans' cache of
its own, each read of a scan's page drawn from its budget and paced by its device. ~~Item 391 waits on the lab's rounds
with the fix, and O179 is measured again after them.~~ **The lab's rounds with the fix** (2026-11-14,
[the record](link-pace.md#the-fix-on-the-lab)) held the cell's inserts at 1.01 to 1.04 times their own second by second
beside the 25% scan, and its reads, set beside the scan's twin in the same rounds, rose as a paced bulk flow of the scan's
rate with no Shoal in it raises them, within its spread ([the twin](link-pace.md#the-read-part-beside-its-twin)). What is
left before the gate is [the confirmation](link-pace.md#the-confirmation): M24's exit's resident half again at the default
on the fix's node, eight rounds with the twin in each, which item 391 is resolved or kept open on. ~~O179 is measured again
after it.~~ **The confirmation ran on 2026-11-14** ([its record](link-pace.md#the-confirmation)), and its box is green:
eight rounds, every scan whole. Beside every resident scan the cell's inserts held the budget at the median, 1.00 to
1.04 times, and its reads overlapped the twin's. The archived half held as at the exit. So
[item 391](../../appendix/resolved/paced-scan-reads-and-inserts.md) is resolved, its box green, and **M24 is
delivered** (2026-11-14), its box green, as decided with the user: the read clause against the twin, and the step clause
with its remainder apart. That remainder is
[O180](../../appendix/optimizations.md#o180-a-paced-resident-scans-encode-steps-run-past-their-bound-on-their-own-cpu-about-once-in-ninety):
a paced resident scan's encode steps pass 100 µs on their own cpu about once in ninety. It is optional, drawn after
M24, and the confirmation adds no tests.
O179's slice was measured again after the confirmation (2026-11-14, [the record](link-pace.md#the-slice-again-after-item-392)),
four rounds as decided with the user: with item 392 fixed it still took the io-wq workers' long waits away, 9.8 a second
to 0.2, but the cell's inserts held at either slice, the ranges overlapping, and a scan's steps were as many. It is
measured and not taken, and its box is green.

```reef
focus = "m24steps"
direction = "LR"
page = "../../plan.md"
```

## Finishing After M24

The *After M24* box was taken up on 2026-11-15, the user asking for every row of it, each marked done and committed as
it lands. Four things were decided with its plan:

- **Three rows are measured before they are kept**: the next cursor sent ahead, which W4 left "priced on M24's
  streams"; saved positions, W5's optional row; and a scan's connection handed to its executor, which W6 left
  "measured first". Each has a rule written before its run, on [the spikes page](spikes.md) as
  [X39](spikes.md#x39-the-next-cursor-sent-ahead-priced), [X40](spikes.md#x40-saved-positions-between-pages-priced)
  and [X41](spikes.md#x41-a-scan-answers-hop-between-shards-priced). A row that does not pay is recorded as measured
  and not taken, its change reverted and its box green, as O152, O153 and O179 were.
- **Item 386's cure is the client's**: it holds its pages in flight on a connection to what its node serves there,
  and opens more connections for a scan that wants more.
- **A count of scans a principal is a fair share when contended.** A shard holds each principal to an equal share of
  its scan class only while another principal was shed there, so a lone tenant keeps the whole class; and one
  principal's answered and unwritten pages may hold at most half its node's scan budget. A node that authenticates
  nobody has one principal.
- **The found items on these paths are fixed with them**: items 380, 386, 387, 388 and 389, each reproduced by a test
  first. Items 386 and 388 come before the cursor sent ahead, which raises a stream's pages in flight, and item 380
  lands with the count a principal, the larger half of its cure.

The steps are drawn in the order they are built. Only these arrows are needs: the cursor sent ahead needs items 386
and 388, each measured row needs its spike, and item 380's cure needs the count a principal. Items 380, 386, 388 and
389 leave *Found on the way* for their places among these steps, on this page's diagrams and the whole plan's; item
387 is drawn before M25 too, which it was filed against. The exit is a workspace run and its counts, a security
review of the changes, and the reference cell before and after on a Zen1 host, since items 380 and 386 touch a
connection's write relay.

**Item 388 is fixed** (2026-11-15), its box green ([Resolved #388](../../appendix/resolved/scan-body-held-to-its-page.md)):
a page's slot carries its page's body bound, which the client's reader holds a declared body to before it allocates
a byte of it, so a node can no longer make a client reserve more for a page than the page could have been answered
with. Its reproduction is the count's one more test. Its rest, found by the plan's review the same day, is fixed
too: an answer of another kind under a page's id is drained unassembled and refused by name, where it had been
allocated up to a gibibyte, and a page's body is reserved no further than twice its answer bytes before its bytes
arrive. Its two tests take the count to 3,248.

**Item 386 is fixed** (2026-11-15), its box green ([Resolved #386](../../appendix/resolved/scan-pages-held-to-the-bound.md)),
the user's choice of cure: the client holds its pages in flight on each connection to its node's eight. A page takes a
place on a connection with room before it is written, the reader gives the place back as it delivers the answer, so a
paused scan holds none its answers have left, and a scan's pages have connections of their own, eight unless set. A lone
scan on six executors that had 48 pages shed in 350 now has none. Its two tests took the count to 3,246.

**Item 387 is fixed** (2026-11-15), its box green ([Resolved #387](../../appendix/resolved/reached-never-in-a-refused-tablet.md)):
a page's progress goes no further than the last of its tablets it did not refuse, and a node holds every answer to its
page as its client will before it sends it, so a cluster's copy that refuses a tablet where it stands, from M25, ends its
scan by that refusal's own code. A test fault that refuses a hosted tablet reproduced it on a standalone node. Its two
tests take the count to 3,250.

**Item 389 is fixed** (2026-11-15), its box green ([Resolved #389](../../appendix/resolved/rows-page-shed-answers-what-it-covered.md)):
a page in rows marks where its body holds each partition whole, and a chunk the scan budget sheds cuts the body back to
the last mark, its draws given back, the page ending short there by its budget, refused only if it covered nothing. Its
test takes the count to 3,251.

**The count a principal is built** (2026-11-15), [F148](../../features/scan-shares.md), its box green, and with it
**item 380 is fixed** ([Resolved #380](../../appendix/resolved/unread-scan-answers-bounded.md)), its box green too.
Item 380 came in three parts: a connection owes at most eight pages until their answers are written, a cancel or its
end giving their places back; a write relay whose client takes nothing stands aside on its link after a second and
ends its connection after `scans.stall_after` (30 s), and one that ends for any reason shuts its read side, so a
connection its node ended holds nothing; and one principal's unwritten answers hold at most half the scan budget, a
page past it shed by name before it draws. A shard's scan class is shared fairly among the principals that contend
for it, a page giving its free place way to another principal waiting with no more places, the longest waiting
first, so a lone tenant keeps the whole class and three on a class of two take turns. Every part was reproduced by a
test first; the count is 3,263.

**The helper stepping through keys is built** (2026-11-15), [F149](../../features/key-steps.md), its box
green: `client.step_keys` cuts a caller's keys into steps of 64, each one get its closure makes, four in flight,
and hands each key back in the caller's order with its own rows, read in place from its step's answer. An answer
whose index does not describe its rows is refused, a refused step ends it by name, and dropping it cancels its
steps. It reads about eighty times a get a key's rate on a node in process. The count is 3,271.

**X39 is reported and the next cursor sent ahead is measured, not taken** (2026-11-15,
[the record](cursor-sent-ahead.md)), both boxes green. Its pre-check, a scan alone at one and two streams a shard,
found one stream 0.5% slower resident, the ranges disjoint, and as fast archived, so the ahead was built behind a
setting and run side against side. One stream a shard with it read as fast as two without it, both ways, but the
cell beside it read 1.06 times slower at the p99 second by second, disjoint, and at two streams it bought nothing.
Its client half is reverted. What one stream a shard is worth without it is filed as
[O182](../../appendix/optimizations.md#o182-a-clients-default-of-one-scan-stream-a-shard), optional, under
*Found on the way*.

**X40 is reported and saved positions are built** (2026-11-15, [the record](saved-positions-priced.md),
[F150](../../features/saved-positions.md)), both boxes green. At the depth a lab-sized node holds of the smallest
rows, 62,914 partitions a tablet, a planned page's pass over its tablet's resident keys held its loop up to 840 µs
and was 55% of its time on its shard, past both of the rule's bounds. A page that passes over its tablet now saves
its range's keys under the tablet's stamp, and the page after it takes its keys from them, sorting a bucket at a
time: every page after the second finds its keys in about 7 µs, and the pass is 7.8% of the pages' time. A key added
to the tablet moves its stamp. The count is 3,277.

**X41 is reported and the hand-off is measured, not taken** (2026-11-15, [the record](answer-hop-priced.md)), both
boxes green. Priced in process first, as the plan's review asked: a 4 MiB answer written after a hop to another
executor cost 1.06 times the cpu a gibibyte of one written where it was filled on europa and 1.07 times on titan,
0.07% of a Zen1 core at the default link share. The lab's spike build was not made.

**The exit is done** (2026-11-15), its box green, and with it the *After M24* box. A review of the changes by two
fresh readers, one of the client half and one of the node's, found seven defects, each reproduced by a test first and
fixed ([Resolved #394](../../appendix/resolved/scan-place-search-on-its-own.md) to
[#400](../../appendix/resolved/reused-channel-drained.md)): a paused scan held every other scan of its client from a
place, a crafted page made a saved position's search quadratic on its loop, a principal's burst passed its share of
the budget, a connection escaped the stall bound by blocking on another frame or by trickling, saved positions cost more
than their bound said, a body past its reservation grew to a power of two, and a reused channel could hand on its last
query's failure. It filed [item 401](../../appendix/known-issues.md#401-on-a-node-that-authenticates-nobody-one-slow-reader-sheds-every-clients-scans),
[item 402](../../appendix/known-issues.md#402-a-client-that-does-not-cancel-abandoned-pages-gives-a-pages-place-back-before-its-node-does)
and [O183](../../appendix/optimizations.md#o183-a-page-in-rows-shed-at-a-chunk-cuts-back-further-than-it-needs), under
*Found on the way* (items 401 and 402 among [M25's steps](#finishing-m25) since 2026-11-15). The workspace run found item 394's first fix wanting, a place found for a paused scan held by a
page never written, and a page's whole front is a task of its own since; finishing it filed
[item 403](../../appendix/resolved/torn-client-write.md),
drawn among M25's steps since 2026-11-15 and fixed as their first. The run again found F148's row asserting a share shed its premise did not need, which it changed, and F148's page
corrected its claim that a lone tenant's default scan holds a class against another: two asking as many pages take
turns without the share. Each run passed 3,283 of 3,284 ([Tests That Fail Under Load](../../appendix/flaky-tests.md#runs),
ws25 and ws26). The reference cell before and after on titan, `160742fc` against `b07a2b9d`, eight rounds of it, its
read-only twin and the ephemeral get, was within on every figure, the cell's operations 0.999 times
(`shoal-spike-scan/results/after24-ab/b07a2b9d/`). What came after it changes the client's scan pages, which none of
those arms asks, a test, and moves two of the client's bookkeeping steps into functions of their own, unchanged.

```reef
focus = "after24"
direction = "LR"
expand = ["review24"]
page = "../../plan.md"
```

## Finishing M25

M25 was taken up on 2026-11-15, its plan approved the same day, the user asking for it built, marked done and
committed. Four things were decided with its plan:

- **A refusal carries no applied index.** Q38 had a copy behind a floor refuse "with its applied index", which no
  frame has a field for: the node counts the floor waits it gave up and logs the index, and no frame moves
  ([the amendments](contract.md#amended-at-m25s-plan-2026-11-15)).
- **The bench scans through client pages by default** on a cluster that activated the scans' wire version, its
  copies named by `--scan-copies followers|leaders|voters`, and a spec from before reads back as the admin walk, as a
  spec from before F74 reads back as routing by endpoints, so every capture taken before stays comparable.
- **The found items on M25's paths are fixed with it**, each reproduced by a test first: items 393 and 401 to 403,
  and item 404, filed by the plan: a bootstrap's seeded grants commit `scan` before the activation every other grant
  waits for.
- **M25 is delivered only once its lab exit is run**, as M24 was: the fixture's rows, a workspace run and a review,
  then the budget on the lab's three nodes and its record.

**Item 403 is fixed** (2026-11-15), its box green ([Resolved #403](../../appendix/resolved/torn-client-write.md)):
a write that stops part way through its bytes, dropped or failed, tears its connection, which then writes nothing more
and is never handed out again, so the rest of a cut frame is never read from the next. Its test takes the count to
3,285.

A review of the plan by a fresh reader found six rules of Q38's record that could not be built as written, recorded
beside it as [the amendments](contract.md#amended-at-m25s-plan-2026-11-15): a page is judged against the floor held
when it was asked; a group's boundary is its copy's applied index at the page's admission; every tablet's copy is
judged before an empty one is called finished; a caller's token is held back at `Quorum` until its group is named; at
`Quorum` one page a route set until its group is named; and a floor past a copy's log refuses at once, the pages
waiting on floors bounded a shard.

The steps are drawn in the order they are built. Only these arrows are needs: the node's page needs the wire
version; item 393 shares the page's voter rule; the driver needs the node's page, the dealing, its cluster's pools and
item 403; the failure rows and the bench need the driver; the exit needs every step. Items 393 and 401 to 403 leave
*Found on the way* for their places among these steps, on this page's diagrams and the whole plan's.

```reef
focus = "m25steps"
direction = "LR"
page = "../../plan.md"
```

## The work beside the gates

None of this is needed by a gate. ~~Every piece of it is planned~~ Every piece of it but the optional
boxes and what was found on the way is planned, decided with the user on 2026-10-31: each sits after the gate whose work it builds on and where it can first be measured, or
feeds the gate that lands it ([the milestones page](milestones.md#beside-the-gates) says why each sits
where it does). The gates are the ones above, drawn left to right.

```reef
focus = "found"
page = "../../plan.md"
```

Whole partitions in answers feeds M24, since it changes what an answer carries ~~and lands with it~~; a
listing's fields kept apart feeds M26 for the same reason, decided by X19 before buckets hold much: **required**
since X19 reported (2026-11-07), a listing at the default threshold having read 58 times its entries' fields.
X19's decisions put three more boxes before M24, each required: **large sorted partitions written as records of
bounded size**, so a page verifies and folds a bounded record at a time, which cures
[item 373](../../appendix/resolved/whole-record-holds.md),
found by X19, **done** (2026-11-07, [F133](../../features/bounded-records.md)), its two boxes green: a merge
writes a sorted partition past 1 MiB as a base and pieces, a page folds a record a step, and a get reads the
pieces in place, the user's choice; **O162**, a walked record handed out from its read buffer, never copied, **done**
(2026-11-07, [F134](../../features/walked-records-uncopied.md)), its box green: every walk reads its archives into
heap buffers its page carries across threads, a record whole hands out where its read put it, and a page draws its
reads alone and copy room only for the pieces a fold over fragments makes; and **a scan budget of at
least the largest row a node admits**, so any row fits alone and waits its turn, never refused for size
([the milestones page](milestones.md#m24-scans-on-one-node) lists them), **done** (2026-11-08,
[F135](../../features/scan-budget-floor.md)), its box green: every node keeps a scan budget of at least its
request body bound and a row's slack, about 64 MiB at the defaults, the user's choice of the floor's
measure and of once the row, and the scan budget serves its waiting draws in turn, its head waiting past a
draw's quarter second for its room.
**Whole partitions in answers is done** (2026-11-06, [F132](../../features/whole-partitions-in-answers.md)),
its scope the user's: the body below the frames. The engine's partitions archive as `shoal-proto`'s
layouts, `PARTITION_LAYOUT` names them, the shard writes a page's partitions behind Q36's chunk heads with
nothing copied, and the client's reader refuses a whole page by name unless every chunk of it holds. What
is left of it is M24's, which carries the body: `.partitions()`, the request's body kind and its refusals,
and `UnknownLayout`. Two items were found on the way and wait beside the rest: 367, the partitions micro
bench, which does not compile, and 368, an archived B-tree's count that rkyv's check never compares with its
entries, which F132's reader guards and a row's own map field does not.

**Found on the way, worked through** (2026-11-05 and 06). The user asked for every open box of *Found on
the way, any time* to be done, as they stood on 2026-11-05: items 349, 352, 358, 360, 361, 362 (the access
check's) and 204 (the stream budget's), and O140, O156, O157 and O158. Each is green, fixed with its
reproduction or measured on the lab and decided by what the measurement said, the user deciding O140's
second half, which became O159 and is green too. Items 363 and 364 and O159 to O161 were found on the way
and are drawn here by rule 10: 364 is fixed, O159 applied, O161 measured and not taken, and item 363 and
O160 wait as their own work, neither blocking a gate.

**After M23, optional, worked through** (2026-11-08 and 09). The user asked for every open box of *After
M23, optional* to be worked: O145, O150, O151, O152, O153, O154 and O155, O144 being moot already. Each was
built, then measured on the lab, A B B A, against its parent (O153 against a build with every candidate
figure stubbed out, O151's modes against buffered), a result only where the two sides' ranges are
disjoint, and kept by what the measurement said. Each box is green. O150 is applied: a merge reads no
archived copy an insert replaces whole. O145 is applied as [F137](../../features/encode-steps-together.md),
small planned partitions several to an encode step. O154 is applied as
[F136](../../features/capture-allocates-nothing.md): a warmed capture allocates nothing, kept as work removed,
since its long captures' fall was not disjoint round by round. O155 is applied as a floor of four scan reads
a device. O151 is built as the opt-in [F138](../../features/wal-segments-written-ahead.md), WAL
segments written ahead and one flush a device: a shared flush doubled the cell's inserts, and it and the
`direct` mode both lost the mixed bench's write tail, so `buffered` stays the default. O152 and O153 were
measured and not taken: O152's change is reverted, and O153's two cheap fixes stand. Nothing new was found
on the way.

O149 and O147 follow M23, as the user decided on 2026-10-31. A larger segment is the larger merge block
this diagram drew until then. M23's exit found that consolidation is not needed after an hour's churn
([Q62](contract.md#questions-to-answer)). It also found that 40 MiB segments keep four times a tablet's
records together a read, so what remains is what a larger segment costs a node's foreground, memory
and start. W12's rate is read again at the segments O149 sets, since a cheaper mebibyte buys the same
share more bytes than more device time would. **O149 is applied** (2026-11-09), both its boxes green:
`replication.segment_bytes` defaults to 40 MiB, measured on the lab side against side on the cell alone and
with a node restarted under it, round 14's mixed bench, and the cell beside a scan of `Big`. No figure the
user's rule named was disjointly worse, nothing was evicted at either size, and a restarted node caught up
in five seconds at both. A scan of `Big` read 37.5 partitions a device read against 9.8, and W12's rate,
read again in those rounds, is 26.7 MiB a second of a Zen1 host's 970 EVO against 13.9 at the share's
75 ms ([the record](../../appendix/optimizations.md#o149-larger-wal-segments-keep-a-tablets-records-together-four-times-longer),
[W12](performance.md)). Its run found
[item 379](../../appendix/known-issues.md#379-a-benchs-kill-or-stop-event-times-its-nodes-catch-up-on-a-member-that-stayed-up),
drawn under *Found on the way*: a bench's kill event times its node's catch-up on a member that stayed
up, so O149 timed its restarts on the node itself. **O147 is measured** (2026-11-10), its box green. It is
built as [F139](../../features/io-workers-off-the-shards.md), `resources.io_worker_cpus`, and was measured on
the lab, `own` against `control` and `spare`. Off their cpus a Zen1 node's shards were runnable but waiting a
tenth as long, and the cell lost a quarter of its inserts, so by the user's rule **`own` stays the default**
and F139 is an opt-in. Planning it found
[O170](../../appendix/optimizations.md#o170-every-shards-blocking-thread-runs-on-the-first-shards-cpu),
every shard's blocking thread on the first shard's cpu, built with it as part of the opt-in. Its run found
[O171](../../appendix/optimizations.md#o171-a-nodes-syncs-sent-from-fewer-cpus-go-through-fewer-of-its-drives-queues),
the cause of the loss: five shards' syncs sent from two cpus go through two of the drive's per-cpu queues.
It is drawn after O147, optional and the user's to decide. O144 and O145 are optimizations of the rebuilt walk that
the exit did not take up: optional, after it. O144 is moot since
[F134](../../features/walked-records-uncopied.md) (2026-11-07): a page holds its bytes once because what it
hands out is its reads' own buffers, so no copy is left to let them go for, and its box is green. O145 is
applied (2026-11-08) and its box is green: a planned page's small partitions are encoded several to a step
([F137](../../features/encode-steps-together.md)), and a scan of `Keyword` beside the cell took 0.72
encode steps a resident partition against 1.33, its rate, its cpu and the drivers' figures unmoved. O150 and O151 were found by item 351's run (2026-11-01),
in the cell's own write path: a merge reads the archived row an insert replaces whole, and a node's
WAL syncs are bound by its 970 EVO's flushes once the drive is loaded. Both are optional, after M23,
which reads the cell as it is. O150 is applied (2026-11-08) and its box is green: a merge reads a
partition's archived copy only when its first intent needs it, and the cell overwriting archived rows
read 0.97 device reads an insert on the Zen1 hosts before and 0.03 after, its rate and tails unmoved. O151 was built as the opt-in
[F138](../../features/wal-segments-written-ahead.md) once a probe met the user's rule (2026-11-08), and
measured (2026-11-09), and its box is green: a shared flush doubled the cell's inserts, and it and the
`direct` mode both lost the mixed bench's write tail, so `buffered` stays the default. O152 was found while F130 was built (2026-11-01): a step's own cpu
read only past its bound halves what the count costs a step, and leaves every shorter step's own
cpu unread, which item 350's attribution read. Optional, after M23. It was measured on the lab
(2026-11-08) and not taken, and its box is green: built with a sample of one step in eight, it left a
scan of `Keyword` 0.13 reads a partition, about 32 µs of the cluster's cpu a second, which no figure
could show, for an estimate where the figure was exact, so every step's stop reads its cpu again.
[O153](../../appendix/optimizations.md#o153-always-on-measuring-logic-behind-a-feature-flag-measured)
was filed at the user's request while item 353's counts were planned (2026-11-01): whether every
figure a node keeps always on, F130's clock and item 353's fault counts among them, each about a
microsecond an operation, is worth compiling out of a product build behind one feature, measured
first. Optional, after M23. It was measured on the lab (2026-11-09) and not taken, and its box is green:
a build with every candidate figure stubbed out moved nothing the cell or a scan of `Big` read past its
noise, so every figure stays in every build, two of them made cheaper on the way.
[O154](../../appendix/optimizations.md#o154-a-captures-view-allocates-nothing-on-the-loop) was filed
when item 353's fix closed (2026-11-02): the view a capture takes still allocates a few times on its
shard's loop, its delta's copy, its run cursors and their merge, and the allocator's events inside
them hold the long `Big` captures left, about one in 900 on the Zen1 hosts. Optional, after M23,
decided with the user. It is applied (2026-11-08) as [F136](../../features/capture-allocates-nothing.md)
and its box is green: a warmed capture allocates nothing, and the Zen1 hosts' long `Big` captures fell from
21 in 36,428 to 1 in 35,992 over six rounds a side, not disjoint round by round, the cell and the scan
unmoved. ~~Item 353 was found by item 350's
run (2026-11-01): light steps past 100 µs on their own cpu, which on the lab's kernels holds the
interrupts their cpu takes.~~ Item 353, found by item 350's run, is M23's since 2026-11-01 and is drawn
under [Finishing M23](#finishing-m23). [Item 355](../../appendix/resolved/scan-pages-served-in-turn.md)
was found by item 350's confirmation run (2026-11-01): a scan's tablet beside the cell shed for a
minute and given up, ~~every page slot held by pages waiting on their pace. Whether it is M23's is the
user's to decide.~~ its pages starved at their device's pace for eleven seconds at a time, which their
waiter gave up. It is fixed before the rounds again (2026-11-03): a device's pages take and read in
turn, and a page given up is let go; four rounds of the cell beside a scan of `Keyword` met no page
given up and no tablet given up, a page's p99 falling from 4.6 to 13.2 s to 1.5 to 2.1. [Item 357](../../appendix/resolved/preload-sheds-waited-out.md) was found by item 353's
fix's view run (2026-11-02): a preload's inserts shed past its eight sends, which failed the run. It is
fixed (2026-11-02): a preload waits out a shed for a minute, and every node counts its write sheds by
cause, which four preloads of the rounds on the lab found none of. [Item 358](../../appendix/resolved/control-answers-waited-for.md)
was found by item 355's workspace run (2026-11-03): a fixture cluster's first initialization found no
control leader for fifteen seconds under load, both its voters reachable, the second time since
Resolved #337. It is fixed (2026-11-05), and its box is green: the control group threw away every append answer
later than its 50 ms heartbeat interval, Resolved #190's defect, which only the data groups' links had been given a
floor for ([Resolved #358](../../appendix/resolved/control-answers-waited-for.md)).
[Item 359](../../appendix/resolved/the-cells-own-climb.md)
was found by the exit's rounds again (2026-11-03): a whole scan of `Big` lifts the cell's read p99 1.14
times. The scan did not get dearer; the cell reads faster without it since `345e17dd`. Item 359 is drawn
here until the user decides whether it is M23's. It was attributed in part on 2026-11-04, by a run of
the Big capture alone and a replay of the scan's reads by fio:
- **It is not the drive's queue.** fio put a scan's reads on the Zen1 970 EVOs and left the cell's
  reads as fast as a control.
- **The slow reads are the Zen1 members' gets that reach their disk**, which carry a loaded 970 EVO's
  read tail.
- **The cell's read p99 climbs as far over the same seconds of an arm with nothing beside it.** So
  whether the 1.14 is the scan's is not yet shown.

The twin, the same day, showed it is not. The scan arm, run with its scan paced to a seventy-fifth of
the default share, read 1.11 times [1.06, 1.21] against the default's 1.14 [1.05, 1.15]. The scan's
reads of the Zen1 drives were cut eightfold, and the cell's reads did not notice. The 1.14 is the
cell's own climb at that point of the capture. Item 359 is resolved as not the scan's
([Resolved #359](../../appendix/resolved/the-cells-own-climb.md)), decided with the user, and its box is green.
[O155](../../appendix/optimizations.md#o155-two-scan-reads-a-device-collapse-a-scans-rate-ninefold)
was found by its first run: two scan reads a device collapse a scan's rate ninefold, where O146
measured four at two thirds of sixteen's. It is optional, after M23, since sixteen is the default and
nobody runs two. It was attributed and applied (2026-11-08), and its box is green: a device's share charges
each of few reads its whole latency, so a device's scan reads are never fewer than four. On the lab
(2026-11-09) a scan at two reads named, raised to four, read 1.45 times as fast alone and 1.91 times beside
the cell, the cell unmoved.
[O156](../../appendix/optimizations.md#o156-an-integer-sort-key-priced-beside-a-padded-string) was
found by W1's optional row of sort keys of any `Ord` type, built on 2026-11-05 as
[Resolved #42, #207](../../appendix/resolved/sort-keys-of-any-type.md) and
[F131](../../features/time-types.md): the bench's and TMDB's sorted tables keep the padded string keys
they were written with, and ~~what an integer key would save is unmeasured~~. It is optional, any time,
since no gate is judged on a sorted table's keys. It is measured (2026-11-06), a pair of workloads
differing in the key alone run on titan: an integer key saves a fifth of a resident get of named keys, a
tenth of a range or a page and a ninth of a partition's load from its archive. Moving the bench's or
TMDB's key is a baseline decision left to its owner, and the box is green.
[Item 360](../../appendix/resolved/lab-exit-rows.md)
was found by that change's workspace run (2026-11-05): W7's acceptance table names M23's lab exit as a
test, `a_scan_beside_the_reference_cell_holds_its_budget`, which no function is, so F122's check fails
since M23 was marked delivered. ~~How a lab exit belongs in an acceptance table is the user's to decide.~~
The user decided on 2026-11-05: the check learns rows marked `**Lab exit**`, each held to the page that
records its run rather than to a function. It is fixed the same day, and its box is green
([Resolved #360](../../appendix/resolved/lab-exit-rows.md)).
[Item 338](../../appendix/resolved/resident-floor-reserve.md) was found by X16's first quick pass
(2026-10-29): a node of 1 GiB held about 1.5 GB no figure named, so every shard evicted two fifths of its
rows at every check and the cell's reads stayed on disk. It is fixed (2026-11-05): a check past the
node's memory by more than eviction could free finds it past its floor, counts it and warns, and each
shard keeps a reserve of an eighth of its budget rather than evicting at every check; a node whose floor
at ready leaves its rows nothing is refused at its start. Its box is green.
[Item 340](../../appendix/resolved/orphan-archives-swept.md) was found while item 339 was fixed
(2026-10-30): an archive file no map names is never deleted, as a crash between an archive pass's settle
and its unlink leaves one. It is fixed (2026-11-05): a start sweeps every table's archive directory
after the rehome and before any shard opens a table, and removes only a file no map on disk names, its
log replayed and its index read. Its box is green.
[Item 361](../../appendix/known-issues.md#361-an-archive-the-counters-name-and-the-disk-lost-fails-every-archive-pass-of-its-shard)
was found by reading that fix's archive pass: an archive whose file is gone while the map's counters name
it fails every pass of its shard at its open. It waits on no gate. It is fixed (2026-11-05): a pass skips a
lost archive, counted and warned of once, and goes on, and its box is green
([Resolved #361](../../appendix/resolved/lost-archive-skipped.md)).
[Item 363](../../appendix/known-issues.md#363-a-copy-that-lost-an-archive-is-not-repaired) was found by
that fix: a cluster copy that lost an archive keeps refusing its records and is never repaired from its
peers. It waits on no gate.
[Item 370](../../appendix/resolved/scan-partition-past-its-budget.md) and
[item 371](../../appendix/resolved/scan-page-over-dead-partitions.md) were found by reading the walk while
[X19](spikes.md#x19-the-edges) was planned (2026-11-07): a partition past half the scan budget, which a
page holds twice, was shed on every ask and never refused by name, and a page over partitions every row
of which was deleted drew for each until it was shed and never moved its cursor, so either scan never
finished. The user chose to fix both inside X19, and both are fixed the same day: a page's draw is judged
by what it would hold, a partition past the budget refused by its key, and every key a page covers moves
it, dead bytes counting toward its bound. Their boxes are green.
[O162](../../appendix/optimizations.md#o162-a-walked-archived-partition-is-copied-while-its-runs-buffer-holds-it)
was found by item 370's fix: a walked archived partition is copied while its run's buffer holds it, so a
page holds it twice. It ~~is optional, any time, and X19's huge partition prices it~~ is required before M24
since X19 (2026-11-07), and has left this subgraph for its place before M24. It is done (2026-11-07,
[F134](../../features/walked-records-uncopied.md)), and its box is green.
[O163](../../appendix/optimizations.md#o163-a-partition-the-scan-budget-cannot-hold-is-read-or-encoded-before-it-is-refused)
was found by X19's huge partition: a partition the budget cannot hold is read, or encoded up to the budget,
before it is refused. It is optional, and M24's split and floor leave nothing to refuse for size~~.~~ but a
resident or overlaid row past half the budget, item 377, since the floor ([F135](../../features/scan-budget-floor.md))
holds a row once.
[Item 372](../../appendix/resolved/encode-cost-relearned.md) was found by X19's own run: one slow first encode
step could hold a table's steps at a row or two for its shard's life, since a step of fewer than eight rows
taught its cost nothing. It is fixed the same day, and its box is green.
[Item 374](../../appendix/resolved/scan-draws-held-and-waited.md) was found by X19's listing on its side
cluster: pages each read a large partition and waited for room for its copy, so eight streams over 16 MiB
objects listed one in two minutes, their reads thrown away. It is fixed the same day, a wave drawing its
partitions' copies with their reads, and its box is green.
[O164](../../appendix/optimizations.md#o164-a-partition-past-128-bounded-records-is-cut-into-128-larger-ones),
[O165](../../appendix/optimizations.md#o165-a-gets-load-of-a-partition-in-pieces-validates-every-piece-in-one-handler-call),
[O166](../../appendix/optimizations.md#o166-a-snapshot-cut-and-an-export-fold-a-partition-in-pieces-into-one-record) and
[O167](../../appendix/optimizations.md#o167-a-merge-rewrites-every-piece-of-a-large-partition-it-merges-whole)
were found building [F133](../../features/bounded-records.md) (2026-11-07): a partition past 128 records has its
records grown to fit an index page's count, a get's load still validates every piece in one call, a snapshot cut
and an export fold a partition in pieces, and a merge that rewrites a large partition rewrites every piece. Each is
optional, any time, and none is before M24.
[Item 376](../../appendix/resolved/dead-archived-bytes-counted.md) was found by a review of O162's design
(2026-11-07): since F133 a dead archived partition's bytes counted toward no bound of its page, so a page over
dead partitions read past every one of them to its range's end, ended only by its budget. It is fixed in F134's
change, and its box is green.
[O168](../../appendix/optimizations.md#o168-a-fold-over-fragments-holds-every-record-of-its-partition-until-its-last-piece-is-made)
was found by F134's lab run (2026-11-08): a partition a merge left with a fragment over its base is folded into
pieces of the page's own, holding every record until the last piece, so it still costs its page twice its bytes,
and one of 17 MiB was refused on a four-shard node. It is optional, and M24's split bounds what a page of a
sorted partition holds.
[O169](../../appendix/optimizations.md#o169-a-paced-walks-reads-into-fresh-heap-buffers-take-longer-in-flight)
was found by the same run: a paced walk that hands every row out is 2% slower on F134. Its reads spend longer in
flight, and the device-time pace charges that time. Fresh heap pages faulted in for each read are the first
suspect. It is optional, and is to be attributed first.
[Item 375](../../appendix/known-issues.md#375-a-scans-page-of-a-64-mib-partition-still-delays-a-get-on-its-shard-past-a-millisecond-about-once-a-page)
was found by F133's timing: a page of a 64 MiB partition in bounded records still delays a get on its shard past
a millisecond about once a page, at most 2.7 ms, unchanged by a smaller bound. It is to be attributed first; M24's
split ~~and O162 each cut~~ cuts the fresh memory a page holds, the first suspect. O162, built as F134, halved that
memory, and the tail stayed as it was (2026-11-08). It waits on no gate.
[Item 377](../../appendix/known-issues.md#377-a-resident-or-overlaid-row-past-half-the-scan-budget-is-refused-by-name)
and [item 378](../../appendix/known-issues.md#378-a-row-past-its-nodes-request-body-bound-can-still-be-refused-by-a-scan)
were found building [F135](../../features/scan-budget-floor.md) (2026-11-08). The floor holds a row once, the user's
choice, so a resident or overlaid row, which its page holds about twice, is still refused past half the budget, a
row past 32 MiB at the defaults. And a node floors its budget by its own request body bound, so a row an update grew
past it, or one written through a peer of a larger frame, can still be refused. Neither is named before a gate.
[Item 364](../../appendix/resolved/dropped-client-connects-quietly.md)
was found in O156's runs on titan (2026-11-05): a client dropped while its pool was opening a connection
logged an ERROR at the hand-off to its stopped proxy, in every bench run. It is fixed (2026-11-06): the
hand-off is outside `connect_to`'s logged span and refuses a dropped client's connection quietly, and its
box is green.
[Item 352](../../appendix/resolved/core-probes-off-the-loop.md), found while item 351 was planned, is fixed
(2026-11-05): a shard probed its groups' cores on its loop after every applied batch, 660 times in six seconds
of writes in the fixture; a probe is now the tick's, at most once a second, on a task of its own, and its box is
green.
[O157](../../appendix/optimizations.md#o157-a-nodes-heap-on-transparent-huge-pages) was found by its run
on europa's loopback cluster: mimalloc madvises its arenas for huge pages, about a quarter of a gibibyte
of every node's floor. It is optional, any time, and wants a before and after on the lab first. It is
measured and not taken (2026-11-06): without huge pages a node holds half as much at ready and a tenth less
loaded, and the cell's tails lose at 6 GiB and at 1 GiB, so every node keeps them; offering them to a
small node's deployment is on the todos. Its box is green.
[O158](../../appendix/optimizations.md#o158-what-openraft-holds-of-every-nodes-floor) was found by the
same run's heap profile: openraft allocates every group's engine queue to 4,096 commands, #191's shape
again, and its runtime stats about 40 MiB a node, about 0.2 GiB of a node's heap. Optional, any time. It is applied
(2026-11-05) through forks of openraft and base2histogram the workspace patches in, and measured on the
lab: a side cluster node's resident set at ready fell from 1.13 to 1.19 GiB to 283 to 289 MiB, and once
loaded from 2.2 GiB to 1.38 to 1.43, the cell's foreground unmoved. Its box is green.
[O140](../../appendix/optimizations.md#o140-a-resident-reads-p99-beside-the-cells-inserts-is-six-times-scylladbs-on-the-same-hosts),
filed from X20, is measured (2026-11-06): the cell's read p99 beside its inserts was item 352's and the
longest holds behind its p999 O159's, and with both fixed it reads at 0.77 ms where ScyllaDB's cell, its
writes synced as Shoal's are, reads at 3.11. Its box is green.
[O159](../../appendix/optimizations.md#o159-a-compacted-segments-checkpoint-and-its-evictable-marking-hold-a-shards-loop-for-milliseconds)
was found by O140's attribution (2026-11-05): with item 352 fixed, the longest things holding a Zen1
shard's loop beside the cell's inserts were a compacted segment's checkpoint write, 16 ms, and its
evictable marking, 4 ms, about every six seconds. The user decided to fix both, and they are built
(2026-11-06): the write's capture and encoding are its task's, a group a step, and the marking takes 256
partitions a step from the second inbox. On the lab both holds became steps of a quarter of a millisecond
at most, and its box is green.
[O160](../../appendix/optimizations.md#o160-a-shards-tick-and-a-sealed-segment-hold-a-zen1-shards-loop-for-up-to-two-milliseconds)
was found by its A/B: what holds a Zen1 shard's loop longest after it is the loop's tick, 0.4 ms ten times
a second, and a sealed segment, 0.5 ms. It is optional, any time, and wants a measurement first.
[O161](../../appendix/optimizations.md#o161-a-wal-batch-is-made-durable-by-a-flush-where-a-fua-write-would-do)
was found by O140's ScyllaDB run with each write synced (2026-11-06): its commit log's writes were made
durable by FUA writes and its inserts answered at 2.5 ms, where Shoal's WAL flushes each batch. Optional,
any time, a drive's FUA measured first. Measured the same day and not taken: on titan's 970 EVO a FUA write
costs twice a write and a flush, so the WAL keeps its flush. Its box is green.

The work found on the way waits on no gate. ~~Its two defects to reproduce are recorded on
[W1's page](prerequisites.md#found-on-the-way) and are filed once a test shows them; the rest are on
[Known Issues](../../appendix/known-issues.md) and [Optimizations](../../appendix/optimizations.md).~~
Its two defects to reproduce, recorded on [W1's page](prerequisites.md#found-on-the-way), were each
shown by a test on 2026-11-05: a bundle that does not validate ended its node, filed and fixed as
[item 362](../../appendix/resolved/unreadable-bundle-ends-its-connection.md), and the stream budget is
[item 204](../../appendix/resolved/stream-budget-a-nodes.md),
filed before W1 was written and fixed the same day: one bucket a node. The rest are on [Known Issues](../../appendix/known-issues.md) and
[Optimizations](../../appendix/optimizations.md).

## The whole plan

Every piece of work above in one diagram, from the contract to the ordered listing: W1's rows,
including the optional ones its own page lists, every spike, every question a gate is closed by, each
gate, M23's steps and its exit, M24's steps and its exit, [W42](scan-speed.md#the-order)'s levers, the work beside the gates,
the steps the *After M24* box is built in, M25's steps and its exit, and everything found on the way. W1's optional counters of walks are F123, M23's first step, and its
optional sort keys of any `Ord` type are Resolved #42, #207 and F131 (2026-11-05). It reads
top to bottom. A column with a title is a set of boxes with no order among them, and an arrow to its
title reaches every box in it.

```reef
root = "scans-plan"
height = "80vh"
page = "../../plan.md"
```

M23's steps are drawn by what each needed, not by the order they were committed in: F123's counters
needed nothing of the others, and the pace in device time needed the reads in runs it paces. A gate's
questions are its "closed before it" on [the milestones page](milestones.md#the-gates); X21 and X22
close Q40 and Q41, and Q37 is asked again with scan work on executors of its own. The parts' diagrams
above stay beside this one and say the same thing; a change to a box is made on both.

## Keeping it current

**Whoever finishes a piece of work turns its box green in the same change**, with a ✅ at the start of
its label and `:::done` after it, as the object storage part's boxes were turned
([its page](../../object-storage/whats-left-todo.md#keeping-it-current)). A piece of work that is added,
split or dropped is added, split or dropped here too, and an optional one says *(optional)* in its
title. **Every defect and optimization found on the way is drawn here in the same change that files
it**, where it belongs, and a gate built in steps is drawn down to them, as M23 is. **Every box is on
[the whole plan](#the-whole-plan) as well**, and changed there in the same change. The conventions are
`CLAUDE.md`'s, under *Planning Docs*.

## Related

[W1](prerequisites.md) for the rows the gates begin with; [Spikes](spikes.md) for what has to report
first; [Milestones](milestones.md) for what each gate delivers; [W13](contract.md) for the questions and
the record.
