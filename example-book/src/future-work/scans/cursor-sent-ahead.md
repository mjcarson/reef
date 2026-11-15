# X39. The next cursor sent ahead, priced

**Reported** (run 2026-11-15). This is the record of spike [X39](spikes.md#x39-the-next-cursor-sent-ahead-priced):
whether a stream that asks its next page as soon as its last page's head arrives, before that page's body, reads a scan
faster, or lets one stream a shard read as fast as two do today. [W4](cursor-and-pages.md#alternatives-rejected) left
it as a later gain "priced on M24's streams"; the user decided on 2026-11-15, with the *After M24* box, that it is
measured before it is kept.

**The verdict: measured, not taken.** With the ahead, one stream a shard reads a scan as fast as two without it, alone,
resident or archived, but the cell beside it reads 1.06 times slower at the p99, second by second, the ranges
disjoint. At today's two streams a shard the ahead buys nothing. Its client half
([147c6f68](#how-it-is-run)) is reverted, and what one stream a shard is worth without it is filed apart as
[O182](../../appendix/optimizations.md#o182-a-clients-default-of-one-scan-stream-a-shard).

## The question

**What would change the design**, written before any run ([the spike's row](spikes.md#x39-the-next-cursor-sent-ahead-priced)):
the ahead is kept, on by default, if one of these holds and the cell's read and insert p99 beside it are no worse by
a disjoint range:

- a scan of resident rows or of archived rows at the client's default streams, two a shard, is faster by a disjoint
  range; or
- one stream a shard with the ahead reads as fast as two without it, the ranges overlapping. In this case the
  client's default may drop to one stream a shard, which asks half its node's scan class.

Otherwise it is reverted and recorded as measured, not taken. A p99 is read over the scan's window and second by
second, as [M24's exit](m24-exit.md) and [the link pace's confirmation](link-pace.md) read
[W7](isolation.md#the-budget)'s read clause: each range held to overlap.

## How it is run

Both halves on [M24's exit harness](m24-exit.md) (`shoal-spike-scan/results/m24-exit-lab.sh`): titan's standalone node
of five shards at 6 GiB, a principal required, its 1,000,000 tagged rows of about a kilobyte; europa's client over the
lab's 1 GbE; both under the `performance` governor, titan's tmdb node, timers and governors held and put back, both
hosts identical to their snapshots after each run. Every scan is paced as it ships: a link share of 25% and a device
share of 75 ms a second.

**The pre-check first.** The ahead keeps at most two pages of a stream in flight, so a stream with it asks no more of
its node at once than two streams without it, and what it buys is bounded by what a second stream a shard buys. So
the method was amended before any run to price that first (`STREAMS_ONLY=1`, 219a1c56): a scan alone at one and two
streams a shard, resident straight after a fresh load and archived after a restart, six rounds, the order alternating.
Had one stream read as fast as two on both, the ahead would not have been built.

**Then the A/B** (`AHEAD_ONLY=1`, a build of 147c6f68): the client's driver with the ahead behind
`ScanBuilder::ahead`, off and on in one build. A settled stream whose page's head holds to its page, refused nothing
and stopped at a bound of its own asks its next page from where the head says it stops, at most one ahead; the reader
hands the head on as its opener arrives (`ClientMsg::ScanHead`), the body still assembled and the page's place on its
connection kept; a page asked ahead waits for its predecessor before anything of it is taken in, so its rows go after
its predecessor's and a checkpoint covers exactly the pages handed out, and a predecessor asked again aborts it. Four
rounds, a node made fresh and loaded each: a resident scan alone at one and two streams a shard each way; the cell
beside a resident scan as the client is today (two streams, no ahead), at two streams with it and at one with it; and
an archived scan alone at each count each way, restarted before each. The sides' order reversed by round. The cell is
M24's reference cell at depth 16, warmed 100 s, its scan window the scan's own.

The predictions were written first, both times (`shoal-spike-scan/results/x39/predictions.md`); the captures and the
reports are under `shoal-spike-scan/results/x39/` (`precheck/`, `ab/`), read by `x39-report.py`.

## The pre-check

| Scan | Streams a shard | Scan, s | MiB/s of bodies | Asked again |
| --- | --- | --- | --- | --- |
| Resident | 1 | 33.34 [33.33, 33.34] | 29.8 | 0 |
| Resident | 2 | 33.17 [33.15, 33.30] | 29.9 | 0 |
| Archived | 1 | 62.6 [62.0, 63.0] | 15.9 | 0 |
| Archived | 2 | 62.7 [61.7, 65.4] | 15.8 | 0 |

- **Archived, one stream a shard reads as fast as two**, 0.999 times the time, the ranges overlapping: the device's
  share binds, as at M24's exit.
- **Resident, one stream a shard is 0.5% slower**, 1.005 times the time, the ranges disjoint by 23 ms at their edges.
  By the rule's letter the ahead was built.

## The A/B

| Scan | Streams a shard | Ahead | Scan, s | Pages asked ahead | Given up | Asked again |
| --- | --- | --- | --- | --- | --- | --- |
| Resident | 1 | off | 33.34 [33.34, 33.34] | 0 | 0 | 0 |
| Resident | 1 | on | 33.24 [32.85, 33.29] | 989 | 0 | 0 |
| Resident | 2 | off | 33.29 [33.20, 33.31] | 0 | 0 | 0 |
| Resident | 2 | on | 33.19 [33.18, 33.22] | 985 | 0 | 0 |
| Archived | 1 | off | 63.25 [62.97, 63.37] | 0 | 0 | 0 |
| Archived | 1 | on | 63.15 [62.83, 63.18] | 989 | 0 | 0 |
| Archived | 2 | off | 62.62 [60.93, 63.43] | 0 | 0 | 0 |
| Archived | 2 | on | 62.26 [60.74, 63.04] | 985 | 0 | 0 |

Almost every page was asked ahead, all but each stream's first and last and the pages that learned the node's
executors; none was given up and none asked again. By the source, a page's head is written only once its reads have
ended and given back their place in the shard's class, so the page asked ahead arrives at a class with room: a class
of two admitted a stream's two pages, and two streams' four, without shedding.

- **At two streams a shard the ahead buys nothing**: 0.997 times the time resident and 0.994 archived, the ranges
  overlapping. The first clause fails.
- **At one stream a shard it closes what one stream loses**: 0.997 times one stream's time without it, disjoint, and
  0.998 times two streams' without it resident, 1.009 archived, both overlapping. The scan half of the second clause
  holds. One stream without it was again disjointly slower than two resident, 1.0015 times the time.

The cell beside a resident scan, over the scan's window:

| The cell beside | Read p99, µs | Read p99 second by second, µs | Insert p99, µs | Insert p99 second by second, µs | Ops/s |
| --- | --- | --- | --- | --- | --- |
| Two streams, as today | 1,055 [978, 1,140] | 974 [963, 999] | 41,023 [39,775, 42,719] | 38,623 [37,823, 39,903] | 1,364 |
| Two streams, ahead | 1,129 [1,029, 1,421] | 1,029 [999, 1,317] | 40,479 [39,007, 42,687] | 37,151 [35,615, 38,655] | 1,404 |
| One stream, ahead | 1,174 [1,036, 1,329] | 1,033 [1,009, 1,273] | 40,303 [39,679, 41,503] | 38,879 [38,559, 39,391] | 1,362 |

- **Beside one stream a shard with the ahead the cell's reads are slower**: their p99 1.11 times today's over the
  window, overlapping, and **1.06 times second by second, the ranges disjoint**. The inserts are the same both ways.
  The cell half of the second clause fails, so the ahead is not kept.
- Beside two streams with the ahead the reads are 1.07 and 1.06 times, both overlapping, and the inserts the same: no
  clause the ahead could meet at two streams asks for this side, which only shows the ahead costs the cell's reads at
  either count.

## Against the predictions

| Prediction | Held? |
| --- | --- |
| Pre-check 1: archived, one stream as fast as two | **Held**: 0.999 times, overlapping |
| Pre-check 2: resident, the same within 3% | **Held in size, failed by the letter**: 0.5% slower, disjoint by 23 ms |
| Pre-check 3: no page asked again | **Held** |
| A/B 1: resident, one stream with the ahead as fast as two without | **Held**: 0.998 times, overlapping |
| A/B 2: resident, two streams with the ahead no faster, pages asked again | **Held in part**: no faster, but nothing was asked again; the ahead page waits for its predecessor's reads to end |
| A/B 3: archived, every side the same | **Held**: within 1%, every pair overlapping |
| A/B 4: the cell's p99s the same at one stream with the ahead | **Failed**: its reads 1.06 times second by second, disjoint |

## What it says for the design

- **A stream's gap between pages is small at the lab's link and devices.** What one stream a shard loses to two is
  0.15% to 0.5% of a resident scan and nothing of an archived one, since a paced scan is bound by its link's share or
  its device's, and while one stream is between pages the others take its part of the share.
- **The ahead fills that gap and costs the cell's reads more than it buys.** It overlaps a page's reads with its
  predecessor's write, a little more of the shard's time spent beside the cell's reads; where that costs them was not
  looked for, since the gain is a few tenths of a percent.
- **One stream a shard without the ahead is filed apart** as
  [O182](../../appendix/optimizations.md#o182-a-clients-default-of-one-scan-stream-a-shard): it costs a resident scan
  alone up to half a percent and would hold half a node's scan class a scan, room
  [F148](../../features/scan-shares.md) shares with other principals. Nobody measured the cell beside it, which is what
  taking it would need.
- **W4's alternative stands as it was**: a later gain, now priced, not taken.

## Limitations

- **One node, one link.** A client farther from its node, or one whose node is several, has longer gaps between a
  stream's pages; the ahead could buy more there. M25's scans across a cluster are where that would show.
- **The cell's read rise is not explained.** Four rounds put it 6% above today's second by second, disjoint by 10 µs
  at the ranges' edges; more rounds could narrow it. The rule reads it as it stands.
- **The pre-check's bound is an argument, not a measurement**: the ahead was built and measured anyway, since the
  pre-check did not excuse it.

## Found on the way

- **[O182](../../appendix/optimizations.md#o182-a-clients-default-of-one-scan-stream-a-shard), a client's default of
  one scan stream a shard.** Optional, after M24: what it costs a scan alone is measured here, what it costs the cell
  is not.

## Related

- [X39 on the spikes page](spikes.md#x39-the-next-cursor-sent-ahead-priced), and
  [W4](cursor-and-pages.md#alternatives-rejected), which left the ahead as a later gain.
- [M24's exit](m24-exit.md) and [the link pace](link-pace.md), whose harness this ran on and whose reading of a p99
  it holds to.
- [F145](../../features/scan-driver.md), the client's driver the ahead was built into and taken out of.
- [The *After M24* box](whats-left-todo.md#finishing-after-m24).
