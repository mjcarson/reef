# F150. Saved positions between a scan's pages of one tablet

## Context

A planned page ([F124](scan-page-planned.md)) finds the resident partitions it may hold by looking at every resident
key of its tablet, keeping those past its start and selecting the smallest a page holds
(`PendingWalk::capture`). A tablet's keys are a hash map, in no order, so there is no cheaper way to find its
smallest, and every page of the tablet pays the pass again. [W5](../future-work/scans/tablet-reader.md#saved-positions-optional)
left saved positions between pages optional, priced on a table whose tablets take many pages; the user decided with
the *After M24* box (2026-11-15) that it is measured first. [X40](../future-work/scans/saved-positions-priced.md) did,
at the depth a lab-sized node holds of the smallest rows, 62,914 partitions a tablet: the pass held a page's loop up
to 840 µs and was 55% of its time on its shard, and at four times that depth 88%. The rule written before the run
said build it.

## What it does

- **A page that passes over its tablet keeps what it found** (`PendingWalk::capture_saving`,
  `server/tables/walk.rs`): when its tablet holds more keys in its range than it takes, every key of the range from its
  start, its own sorted at the front, is saved as the tablet's position (`Saved`, `server/tables/saved.rs`), stamped
  with the tablet's stamp at the capture.
- **The page after it goes on from there.** A planned page whose tablet has a saved position at the same stamp, from at
  or before the page's start and over the same high, takes its keys from it (`PendingWalk::capture_saved`) and never
  looks at the tablet's map. The first time the position is used, the keys past the first page's are cut into buckets
  of about a page each by key range, a counting pass and a placing pass; after that a page sorts only the bucket it
  needs, so it finds its keys in about 7 µs whatever its tablet's depth.
- **A tablet's stamp moves whenever a key may have been added to it** (`ResidentPartitions`, `server/tables/resident.rs`):
  an insert of a key it did not hold, every entry asked for, and a tablet's map made. A position at an older stamp
  serves nothing and is dropped, and the next page passes over the tablet again and saves anew.
- ~~**One position a tablet, eight a table, and 4 MiB of keys a shard's thread** (`SAVED_KEYS_BYTES`), a table's oldest
  dropped to make room~~ **One store a shard's thread for every table's positions**, one a tablet and 32 a thread,
  4 MiB of keys and buckets a thread (`SAVED_KEYS_BYTES`), its oldest dropped first whatever its table, and a position
  unused for ten seconds dropped at any table's next look (`SAVED_LIFE`), since
  [Resolved #398](../appendix/resolved/saved-position-costs.md). A position the thread has no room for is not kept,
  and its tablet's pages pass over its keys as before. A range of more keys than half the bound is never copied to be
  saved, and a tablet whose position was dropped for a moved stamp is not saved again for a second, so a tablet written
  at every page costs no copy a page.
- **No page does more than a bucket's work to find its keys** ([Resolved #395](../appendix/resolved/saved-position-bounded.md)):
  a position is cut into at most 1,024 buckets of at least 64 keys, and a page whose start is past its sorted keys goes
  straight to its start's bucket by arithmetic, whatever the page's size or its cursor.
- **Both tables, every planned page**: a scan's page and a driver's, whichever principal asked. A captured page, a
  backup archive's, saves and takes nothing.

## Design choices

- **The keys, not the rows.** A position holds partition keys alone, eight bytes each, and every key is looked up again
  when its page captures it, so what the page hands out is the partition as it is at the page's capture, as it always
  was. Nothing about a row, a filter or a principal is in it, so any page of the tablet may use any position of it.
- **Invalidated by a key added, not by a key removed.** A key removed since the position was saved is looked up and not
  found, so the page skips it; a key evicted to its archive is read from the archive's view of the page's range, which
  the page takes in the same step as ever. Only a key added could be missed, and it moves the stamp.
- **A stamp a tablet, moved conservatively.** An entry asked for may be filled with a new key or not; the stamp moves for
  every one, so a write into a tablet's partitions drops its position. A scan beside writes into the tablet it is
  reading passes over it as before; one beside writes elsewhere keeps its positions.
- **Buckets by key range, sorted one at a time.** Keys are hashes, spread evenly over their tablet's range, so equal
  ranges hold about equal counts. Sorting the whole rest at once would hold the first page's loop for the sort of every
  key; buckets spread it over the pages that need them, about a thousand keys a page.
- **Its own bound, on its thread.** A table's walk is reached through the schema's generated dispatch with no handle on
  the node's scan budget, so the positions are held to a bound of their own, counted on the shard's thread they live on
  as the capture's other scratch is.

## Alternatives rejected

- **A sorted index a tablet**, kept as keys are added: every insert would pay for an order only a scan reads.
- **A cursor kept in the scan's name**, (principal, scan id, table, tablet), as the plan sketched it: the keys are the
  tablet's and no scan's, so a key by tablet serves every scan of it and needs no identity a client could forge.
- **Positions drawn from the scan budget.** A table's walk has no handle on it, and a draw a page might be refused would
  only make the page pass over its tablet, which a bound of its own does as well.
- **Saving the page's own keys and a sorted rest on the first page.** Sorting every key of a deep tablet on one page
  would hold its loop for milliseconds; the buckets cost one linear pass, on the second page.

## Limitations

- **The first two pages of a deep tablet still hold the loop past 100 µs**: the first passes over the tablet's keys as
  before (0.79 ms at K_max, 3.1 at four times it) and the second cuts them into buckets (0.24 and 0.96 ms). A pass taken
  a step at a time would bound them.
- **Writes into the tablet being read drop its position** at every write, since an entry may add a key; such a scan
  passes over its tablet each page, as it did before.
- **Measured in process**, on one shard and rows of a key alone. The lab's own tables' tablets are one page each, where
  nothing is saved and nothing changes.
- **A page that saves a position allocates its range's keys once.** A warmed capture that saves nothing allocates
  nothing, as [F136](capture-allocates-nothing.md) holds; the saving one copies its keys, at most half the thread's
  bound of them.
- **A position counts against no budget a node reports**: its 4 MiB a shard's thread is outside the rows a node counts
  and inside its resident memory.

## Invariants to uphold

- **A saved position holds every resident key of its tablet from its start to its high, as its tablet held them at its
  stamp**, and is used only at that stamp.
- **A tablet's stamp moves for every change that may add a key to it**: never add a key to a `ResidentPartitions`
  except through `insert`, `entry` or a new map.
- **A page uses a position only from a start at or past the position's own and over the same high.**
- **Every key a position hands a page is looked up again by the page's capture**, which is what makes a removed key and
  an evicted one safe.
- **What a shard's thread's positions hold is bounded** by `SAVED_KEYS_BYTES`, ~~counted while each lives~~ every byte
  counted while each lives, its buckets' bounds with its keys, in one store a thread for every table (Resolved #398).
- **No page does more than a bucket's work to find its keys**, and a position has at most `MAX_BUCKETS` buckets
  (Resolved #395): every bucket before a start's holds only keys before it.

## Performance

X40's timing again, on this build (`shoal/tests/scan_depth.rs`, europa, release, one shard): every page after the
second finds its keys in about 7 µs, where they took 100 to 2,900 µs, and the pass is 7.8% of a tablet's pages' time on
the shard at K_max (from 55.3%) and 6.6% at four times it (from 88.2%). A page's wait at the client in the middle of a
tablet is 657 µs at K_max, from 968, and 984 at four times it, from 3,463. A tablet of one page, every lab table's,
saves nothing. A write pays one more counter's increment for each partition it reaches.

## Tests

| Test | What breaks if the feature is reverted |
| --- | --- |
| `a_deep_tablets_pages_go_on_from_where_the_last_left_its_keys` (`shoal/tests/scan_depth.rs`) | A deep tablet's later pages look at every key of it again |
| `a_key_added_to_a_tablet_mid_scan_is_never_missed` (`shoal/tests/scan_depth.rs`) | A page goes on from keys saved before a key was added and misses it, or hands out a removed key |
| `a_saved_tablets_keys_come_out_in_order_a_page_at_a_time` (`shoal-core`'s `server::tables::saved`) | A position's buckets hand a page keys out of order, twice or not at all |
| `a_page_from_a_later_start_holds_no_key_before_it` (`server::tables::saved`) | A page from a later start is handed keys before it, or another set when asked again |
| `a_position_serves_only_what_it_saved` (`server::tables::saved`) | A position serves another tablet, an earlier start, another high or an older stamp |
| `a_threads_positions_are_counted_and_bounded` (`server::tables::saved`) | A thread's positions hold more than their bound, or are not uncounted when dropped |
| `a_far_start_on_a_deep_tablet_is_found_at_once` (`server::tables::saved`) | A far start on a deep tablet is found a bucket at a time (Resolved #395) |
| `a_deep_tablets_pages_cost` (`shoal/tests/scan_depth.rs`, ignored) | X40's figures, before and after |

## Related

- [X40](../future-work/scans/saved-positions-priced.md), which priced it, and [W5](../future-work/scans/tablet-reader.md#saved-positions-optional).
- [F124](scan-page-planned.md), the planned page whose pass this saves, and [F136](capture-allocates-nothing.md), the
  capture's scratch.
- [The *After M24* box](../future-work/scans/whats-left-todo.md#finishing-after-m24).
