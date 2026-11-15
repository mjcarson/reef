# 391. Beside a paced scan of resident rows the foreground's reads waited in its host's network path, and its inserts on the node

## Symptom

[F147](../../features/scan-link-pace.md), the cure of [item 390](resident-scan-fills-the-link.md), holds a scan's
answers to a share of the link they leave by. Its sweep on the lab (2026-11-13,
[the record](../../future-work/scans/link-pace.md#the-sweep)) found that at a quarter of titan's gigabit, the
default, a scan of a million resident rows still moved the reference cell beside it:

- its read p99 from 0.46 ms to about 0.95, 2.07 [1.99, 2.15] times, where
  [W7's budget](../../future-work/scans/isolation.md#the-budget) allows 0.56 ms;
- its insert p99 1.16 [1.13, 1.28] times, where the budget allows 1.10.

Both ranges were disjoint from the cell alone's. The cell's median did not move, and its read p99 was raised the same
every second. Another client's cell, driven from hyperion, rose as the scanner's own did. No share held the budget,
an eighth of the link included, so M24 stayed open behind it.

## Cause

Two causes, and only the second was Shoal's.

- **The reads waited in titan's network path.** Any steady flow out of titan's NIC makes the packets behind it wait
  about half a millisecond at the tail. The NIC is a Realtek r8169 with no scatter-gather or segmentation offload, and
  beside the paced scan its queue took bursts of up to 65 KB, about half a millisecond of the gigabit. A paced flow
  with no Shoal in it costs the cell's reads as much as the scan, and a ping answered in titan's kernel rises with
  both.
- **The inserts waited for the intent logs' fdatasyncs, which the scan's reads of its map lengthened**:
  [item 392](scan-map-reads-paced-and-read-ahead.md).
  - A scan's walk read its table's archive map from the device on every page. That held even over resident rows,
    since every partition a table ever archived is in the map's runs.
  - No pace, budget or figure saw those reads. Beside the 25% scan they were a few hundred small reads a second on
    titan's NVMe, against twenty without it.
  - They shared the drive with the intent logs' fdatasyncs. The kernel's fdatasync p99 rose from 23.4 to 34.7 ms,
    and every insert waits for its sync.

## Evidence

**Established by measurement on the lab**, a run at a time, each run's predictions written before its rounds. The
records are [item 391 placed](../../future-work/scans/link-pace.md#item-391-placed) and the runs after it on the same
page. The insert part was **reproduced by a test**, item 392's.

**The read part is the host's, not the node's** (2026-11-13):

- The node's own get p99, from a frame's arrival to its answer's write, rose from 0.27 to 0.31 ms, while the
  client's rose from 0.47 to 0.94.
- A ping from hyperion, answered in titan's kernel and never by the node, rose at p99 from 0.13 to 0.69 ms beside
  the scan, and not at all beside the cell alone.
- Two TCP connections from titan to europa with no Shoal in them, paced together to the scan's rate by the same
  `SO_MAX_PACING_RATE`, cost the cell's reads 1.93 [1.85, 2.64] times, against the scan's 2.03 [2.00, 2.17]. At an
  eighth of the link they still cost 1.50 times.
- None of these moved it:
  - the allocator never purging;
  - io_uring's workers on spare cpus;
  - pages four times larger;
  - the root qdisc `fq` in place of `fq_codel`.

  Ethernet flow control is off at both ends.

**The insert part was the scan's** (2026-11-13 and 2026-11-14):

- The same bulk flow left the cell's inserts at 1.00 [0.99, 1.02] times, where the scan raised them 1.27
  [1.19, 1.29], the ranges disjoint. Sent from a shard's own cpu, the flow still left them at 1.02.
- A `stage-profile` node put the rise in a write's wait for its sync, not in its write.
- Three readings were ruled out:
  - the shard syncing more often, against the sweep's flush counts;
  - the executor's polling of its ring
    ([the polling test](../../future-work/scans/link-pace.md#not-the-executors-polling));
  - the workers' wait for a busy shard's cpu ([O179](../optimizations.md#o179-a-shard-holds-its-cpu-through-its-eevdf-slice-while-the-io-wq-worker-running-its-fdatasync-waits)).
    A shorter slice took those waits away but not the rise in every round.
- A kernel trace placed the time in the fdatasync itself.
- titan's NVMe counters then showed the scan reading the drive, which its own figures said it did not.
- A spike that served the map from memory took the reads away, and the insert rise with them: 0.90 [0.87, 0.95]
  times against 1.30 [1.12, 1.42]
  ([the map in memory](../../future-work/scans/link-pace.md#the-map-in-memory-the-cause-shown)).
- Item 392's test, `a_walks_pages_read_each_index_page_once`, reproduced the map's reads against the unfixed tree:
  120 reads of a 42-page run by 120 walk pages.

**With item 392's fix** (2026-11-14, three rounds,
[the record](../../future-work/scans/link-pace.md#the-fix-on-the-lab)):

- the cell's inserts held 1.01 to 1.04 times their own, second by second;
- titan's NVMe read 42 times a second beside the scan.

**Beside its twin in the same rounds** (2026-11-14, three rounds,
[the record](../../future-work/scans/link-pace.md#the-read-part-beside-its-twin)), the cell's read p99, second by
second:

| Beside | Read p99 ratio |
| --- | --- |
| The scan | 2.07 [2.03, 2.25] |
| The same scan, the cell on another client | 2.09 [2.06, 2.18] |
| A paced bulk flow at the scan's rate | 1.88 [1.83, 2.58] |

The ranges overlap.

**The confirmation** (2026-11-14, eight rounds, 04:36 to 08:38 UTC,
[the record](../../future-work/scans/link-pace.md#the-confirmation)) ran M24's exit's resident half again, at the
default, on the fix's node, with the twin in every round. Its predictions were written before any round. Every scan
handed out every row once, and titan's NVMe read 14 to 43 times a second beside the scans, against the control's 22.

| The cell, eight rounds | Read p99 ratio | Against the twin | Insert p99 ratio | Against the control |
| --- | --- | --- | --- | --- |
| Alone (the control) | 1.02 [0.96, 1.04] | - | 0.99 [0.90, 1.11] | - |
| Beside a resident scan in rows | 2.20 [2.02, 2.69] | overlap | 1.00 [0.91, 1.07] | overlap |
| In partitions | 2.22 [1.85, 3.05] | overlap | 1.01 [0.88, 1.10] | overlap |
| On another client, beside the same scan | 2.27 [2.00, 2.56] | overlap | 1.04 [0.92, 1.07] | overlap |
| Beside the twin | 2.35 [1.83, 2.77] | - | 0.99 [0.95, 1.03] | overlap |

The medians' quotient against the twin is 0.94 to 0.96 over the window and 1.03 to 1.06 second by second. In
rounds 1 to 4 the archived half held as at the exit: reads 0.69 and inserts 1.05 against the archived control.

## The fix

- **The insert part is cured by [Resolved #392](scan-map-reads-paced-and-read-ahead.md).** A walk's cursors read the
  map sixteen pages ahead into a scans' cache of its own, so a walk reads each of the map's pages once. Every read
  of a scan's page is drawn from its budget, paced by its device and counted.
- **The read part is judged against its twin**, decided with the user on 2026-11-13. The twin is the cell alone
  beside a paced bulk flow of the scan's rate from the node's host, with no Shoal in it, in the same rounds. A rise
  is the scan's only where its range is disjoint from the twin's and above it. M23's read clause was judged against
  a twin of its arm in the same way ([Resolved #359](the-cells-own-climb.md)).
- **Nothing else in Shoal changed for the item.** The confirmation's harness runs the twin in every round
  (`CONFIRM_BULK`, `m24-exit-lab.sh`), and the report sets each resident arm beside it.

## Alternatives rejected

- **A smaller default share.** At an eighth of the link the cell's reads still rose 1.88 times, and the bulk flow at
  an eighth cost them 1.50. No share takes titan's network path away. A slower scan only takes longer, and its steps
  pass 100 µs more often.
- **Group commit by the sync for a standalone node's intent log**, chosen first on a run's reading of more, smaller
  syncs. The sweep's four rounds showed the syncs came later, not smaller, and nothing was built.
- **A shard polling its ring while scan work runs.** The glommio fork's diagnostic switch left the rise: 1.27 times
  against 1.40, the ranges overlapping.
- **A shorter scheduler slice for the node's threads** (O179). It took away the io-wq workers' waits, but not the
  rise in every round. The spike then showed those waits cost the fdatasyncs nothing, so O179 stays optional ~~and is
  measured again after item 392~~. Measured again with item 392 fixed (2026-11-14), it bought the inserts nothing: 0.95
  [0.94, 1.10] times at 100 µs against 0.99 [0.93, 1.05] at the default, and it is not taken
  ([the record](../../future-work/scans/link-pace.md#the-slice-again-after-item-392)).
- **Reading W7's read clause against the bare budget here.** No paced flow out of the lab's NICs at a quarter of
  their link holds it, Shoal's or not. Read that way, the clause would judge titan's NIC and not the scan.
- **The scans reading their map through the point lookups' cache**, or skipping the map over resident tablets.
  These are item 392's alternatives, rejected on its page.

## Invariants to uphold

- **W7's read clause beside a resident scan is judged against its twin, run in the same rounds**: never against a
  twin borrowed from another run, a host or a day.
- **A twin is a flow at the arm's own rate, from the node's own host, with nothing of Shoal in it.** A twin slower
  than its arm, or sent from another host, hides the arm's part.
- **Every read a scan's page makes of its device, of its map as of its records, is drawn and paced**, as
  Resolved #392 has it. A resident scan reads its device only for its map's read-ahead: titan's NVMe at the cell
  alone's rate, under 60 reads a second.
- **The insert clause is read as defined**, against the cell alone, never against the twin. The twin costs the
  inserts nothing, so a rise beside the scan is the scan's.

## Still open

- **The read part is titan's**, not cured. On a NIC with segmentation offload, or a faster link, the twin may cost
  less. The clause is judged against the twin wherever it runs. A node on 2.5 GbE, with gigabit clients, is in
  [the todos](../todos.md#a-clients-own-asked-pace-for-its-scans).
- ~~**O179**, the slice, is measured again after item 392, as decided with the user.~~ Measured (2026-11-14) and not
  taken: the waits it removes cost the inserts nothing with item 392 fixed
  ([the record](../../future-work/scans/link-pace.md#the-slice-again-after-item-392)).
- **Rare clusters of 130 to 210 ms on the node's own insert wait** come in a few arms of every run, scans and controls
  alike. They line up with titan's crash-looping kubelet and etcd more often than not, and they are not the scan's.
- **The step clause's remainder beside the same scans** is M24's, not this item's. Encode steps pass 100 µs on their
  own cpu about once in ninety, and they are kept apart as
  [O180](../optimizations.md#o180-a-paced-resident-scans-encode-steps-run-past-their-bound-on-their-own-cpu-about-once-in-ninety).

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_walks_pages_read_each_index_page_once` (shoal-core, `index`) | A walk of 120 pages reads its 42-page run 120 times, and a resident scan reads its device a few hundred times a second beside the intent logs' syncs |
| `a_scan_pages_reads_of_its_map_are_drawn_paced_and_counted` (shoal-core, `walk::reads`) | A scan page's map reads skip its budget, its device's pace or its read slots, or go uncounted |
| `a_walk_reads_its_tables_map_ahead_and_counts_it` (`shoal/tests/tablet_walk.rs`) | A node's walk in small pages reads its map a page at a time, or its figures do not count it |
| `a_clients_scan_beside_the_reference_cell_holds_its_budget` ([W7](../../future-work/scans/isolation.md#acceptance-tests)) | The lab's exit: the cell beside a resident scan over the insert clause, or its reads above its twin's. An exit, not a unit test |

## Related

- [Item 390](resident-scan-fills-the-link.md), the link's part, which F147 cured and this lay beneath.
- [Item 392](scan-map-reads-paced-and-read-ahead.md), the insert part's cause and cure.
- [The lab's record](../../future-work/scans/link-pace.md), from the sweep to the confirmation.
- [M24's exit](../../future-work/scans/m24-exit.md), and [M24](../../future-work/scans/milestones.md#m24-scans-on-one-node).
- [Item 359](the-cells-own-climb.md), whose twin set M23's read clause beside its arm the same way.
- [O179](../optimizations.md#o179-a-shard-holds-its-cpu-through-its-eevdf-slice-while-the-io-wq-worker-running-its-fdatasync-waits),
  the slice the chase found.
