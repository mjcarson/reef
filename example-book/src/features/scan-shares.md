# F148. A scan's share a principal

## Context

A shard reads a few pages of its clients' scans at once, its scan class (`scans.pages_in_flight`, two unless
set), and sheds the rest `Shedding` ([F116](scan-admission.md)); the node's scan budget holds every page's
buffers while it reads and every answer's until its last frame is written ([F118](scan-budget.md),
[F144](scan-pages-on-the-wire.md)). Neither knew who asked. [W7](../future-work/scans/isolation.md#admission)
left the gap in one line: "A count a principal, so one tenant's scans cannot take every shard's class, is a
later addition that changes no frame", and placed it after M24, once a client scans one node.

Two things showed what the gap costs:

- **One client's scan fills a shard's class by default.** A scan asks two pages an executor and a shard admits
  two, so a second tenant's scan is shed at every shard ~~for as long as the first runs~~, its streams backing off
  up to a quarter second and asking again. *Measured since* (2026-11-15, the *After M24* exit): two tenants asking
  two pages at once each took turns at a class of two without the share, each as likely to ask first when a place
  freed, the second 91% to 99% of the way through the table when the first ended. What one takes is in proportion to
  the pages it asks at once: beside a tenant asking eight, a tenant asking one was 6% to 71% of the way through when
  the other ended, and with the share 62% to 100%, 35% to 100% on a loaded host.
- **One tenant's connections that read nothing can hold the whole scan budget**
  ([item 380](../appendix/resolved/unread-scan-answers-bounded.md)): every page they ask is read and its answer
  holds its draw until written, so every other tenant's pages are shed for the budget until they go.

Decided with the user on 2026-11-15, with the *After M24* box: **a count a principal is a fair share when
contended**, so a lone tenant keeps the whole class, and **one principal's answered and unwritten pages may
hold at most half the node's scan budget**. A node that authenticates nobody has one principal.

## What it does

- **A principal is who a connection authenticated as** (`ScanPrincipal`, `server/scan_shares.rs`): a principal by
  name, one anonymous principal for every connection of a node that authenticates nobody, and the node itself
  for a page it asks in process. Every shard knows every connection's principal from its start, so a page's
  host reads it there; the page carries it to its host in its call (`ScanCall::principal`).
- **A shard's scan class is shared fairly when principals contend** (`FairShare`, kept by each shard's `Walks`).
  A page that finds the class full is shed as before, and its principal is counted as waiting there for half a
  second (`FAIR_SHARE_WAIT`). A page that finds a free place takes it unless another principal waits there
  holding no more places than its own, the one waiting longest first, in which case it gives the place way: shed
  `Shedding` by name, *holds its share of this shard's scan class while another principal waits for a place*, and
  counted as waiting itself. A principal's place is given back as its page ends, where the class's is.
- **One principal's unwritten answers hold at most half the scan budget** (`PrincipalHolds`, the node's, and
  `HeldBytes`, an answer's). Each answer's draws are counted against its principal by a guard its body holds
  beside them, from its answer until its last frame is written or it is dropped. A page whose principal holds
  half the budget or more is shed by name at its coordinator before it draws, *its scan answers waiting to be
  written hold its share of this node's scan budget already*. A connection takes its principal's count once, so a
  page takes no lock. Since [Resolved #396](../appendix/resolved/principal-share-reserved-at-admission.md) a page
  reserves what it will first draw against its principal as it is admitted, by one compare and swap that refuses it
  past the share, so pages being read count as answers waiting to be written do and a burst cannot pass the share.
- **The figures say which**: `ScanPageFigures::shed_share` and `shed_held` count the pages shed for each, beside
  `shed`, which counts every shed page as before.

## Design choices

- **A share when contended, never a fixed count.** A fixed count a principal would hold a lone tenant to part of
  every shard's class: half, at the default, which halves a scan nobody else wants the class for. A share asked
  only while another principal waits costs a lone tenant nothing.
- **Yield to a waiter holding no more, the longest waiting first.** A principal whose page ends asks again within
  a round trip, where a shed one waits up to a quarter second before it asks again; a freed place left to whoever
  asked first would go back to its holder almost every time, and a third tenant on a class of two would never
  get in. Giving way to the one waiting with fewer, ties to the one waiting longest, makes the tenants take turns.
  ~~Almost every time~~: measured at the exit, a freed place went to whichever principal asked first, which beside
  another asking as many pages at once is either about as often; the share is what holds a principal asking more
  pages at once to its half.
- **Half a second's wait.** Past the longest a client waits before it asks a shed page again and a round trip, so
  a principal still asking is still counted as waiting, and one that stopped is forgotten within half a second;
  a place may idle that long for a principal that went away.
- **The budget's half, counted by what answers hold, not by pages.** What starves another tenant is the budget,
  and an answer holds what its page drew; a count of pages would let a principal of large pages hold more than
  one of small ones. Half leaves every other principal the other half; it binds a lone tenant only when its
  unwritten answers pass half the budget, which a reading client's do not (a stream has one answer in flight).
- **Checked at the coordinator, before the draw.** A page shed there holds nothing; one shed after it drew would
  have taken budget to be refused.
- **One anonymous principal for an unauthenticated node.** Nothing tells its connections' tenants apart, and a
  count a connection would let one tenant take every place by opening more connections.

## Alternatives rejected

- **A fixed count a principal, off unless set.** An operator would have to know to set it, and every node not set
  would keep the gap.
- **A fixed count a principal by default**, half a node's class: simple, and it halves a lone tenant's scan at
  default settings.
- **A count a connection.** A tenant opens connections; a connection is already held to eight pages
  ([item 386](../appendix/resolved/scan-pages-held-to-the-bound.md), item 380), which bounds what one connection
  queues, not what one tenant takes.
- **Weighted shares, a principal's weight in its grants.** It needs a field in the grants every node reads, which
  older builds refuse, and nothing yet asks for unequal tenants.
- **A queue of waiting pages, served in turn.** The node holds nothing between pages, and a client asks a shed
  page again; holding pages in a queue would hold their draws and their connections' places while they wait.

## Limitations

- **On a node that authenticates nobody, one slow reader sheds every client's scans** for up to the stall bound, since
  every connection is the one anonymous principal ([item 401](../appendix/known-issues.md#401-on-a-node-that-authenticates-nobody-one-slow-reader-sheds-every-clients-scans)).
- **An operator's walk of the scan class is attributed to nobody.** An admin walk asked as a scan's page (the
  bench's cluster driver) takes a place in its shard's class and carries no principal past its connection's
  start; it neither yields nor is yielded to. It is an admin's.
- **A place may idle up to half a second** for a principal that was shed and then stopped asking.
- **A principal is waiting only once it was shed**, so a class whose pages end together can pass to the principal
  that asks first, whole, and back again: on a loaded host two principals asking two pages each held the class in
  turns, both places at a time, and nothing was shed for a share. The turns are fair over time, since each is as
  likely to ask first; it is a principal asking more pages at once that the share holds back.
- **A split scan's parts are one principal's**, sharing its share, as they should.
- **No weights.** Every principal's share is equal.
- **A cluster answers no client's scan before M25**, so a share across a cluster's nodes is M25's: each node shares
  its own, which is what a page reaches.
- **No view shows the figures yet.** `shed_share`, `shed_held` and `stalled` are in a node's `ScanPageFigures`
  beside F144's, which shoaladm's stats view does not read either: it reads a cluster's members, which answer no
  client's scan before M25, and a standalone node answers no stats remotely. They are read in process today, and
  the stats view takes the client page figures with M25.

## Invariants to uphold

- **A principal's place in a shard's class is taken where the page's place is and given back where the page's is**,
  on every way out: `handle_scanned` gives it back, and a page whose task never started gives it back as it is
  refused.
- **A principal yields only to one that waits**, shed at that shard within `FAIR_SHARE_WAIT`; a principal nobody
  contends with is never shed for its share.
- **What an answer holds is counted against its principal by a guard its body holds** (`HeldBytes` in
  `AnswerHolds`), never by a pair of calls, so an answer dropped unwritten gives its bytes back too.
- **The held share is judged before a page draws**, at its coordinator, ~~by what answers hold~~ by what answers and
  the pages being read hold, each page's draw reserved against its principal as it is admitted (Resolved #396).

## Performance

A scan's page takes a hash lookup of its connection's principal and an atomic load at its coordinator, and a
hash lookup and an insert at its host's admission; its answer an atomic add and, when written, a subtract. A get
and a write take nothing of it. Two principals' scans of one shard side by side, one asking eight pages at once and
the other one, each hand out every row, the first's pages shed for its share 24 to 76 times in a scan of about sixty
pages each, which `two_principals_share_a_shards_scan_class` counts; what it costs a scan's rate beside another
tenant's is the share it gives up, by design.

## Tests

| Test | What breaks if the feature is reverted |
| --- | --- |
| `a_principals_unwritten_answers_hold_at_most_its_share_of_the_budget` (`shoal/tests/scan_frames.rs`) | One principal's unread answers run the scan budget dry, and its next page is shed for the budget, not its share |
| `two_principals_share_a_shards_scan_class` (`shoal/tests/scans.rs`) | A principal asking eight pages at once beside one asking one is never held to its share, five runs of five with the yield turned off; a lone one is still never shed for it |
| `a_lone_principal_takes_every_place` (`shoal-core`'s `server::scan_shares`) | A lone principal is held back by its own sheds |
| `a_holder_gives_way_to_a_principal_waiting_with_fewer` (`server::scan_shares`) | A freed place goes back to its holder while another principal waits |
| `three_principals_on_a_class_of_two_take_turns` (`server::scan_shares`) | A third principal on a class of two starves |
| `a_principal_that_stops_asking_waits_no_more` (`server::scan_shares`) | A principal that went away keeps a place idle for ever |
| `a_principals_held_bytes_are_counted_until_its_answers_go` (`server::scan_shares`) | A principal's count drifts from what its answers hold, or a principal nothing holds is never forgotten |

## Related

- [W7](../future-work/scans/isolation.md#admission), whose later addition this is.
- [F116](scan-admission.md), the scan class, and [F118](scan-budget.md), the budget, which this shares out.
- [Resolved #380](../appendix/resolved/unread-scan-answers-bounded.md), whose larger half this is.
- [Resolved #386](../appendix/resolved/scan-pages-held-to-the-bound.md), a connection's own bound.
- [The *After M24* box](../future-work/scans/whats-left-todo.md#finishing-after-m24).
