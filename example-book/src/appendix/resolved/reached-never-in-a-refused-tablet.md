# 387. A page that refused a tablet part way answered `reached` at that tablet's last key, which its own client refused

## Symptom

A scan's page that refused one of its tablets where it stood answered how far it had got with a key inside
that refused tablet. Its client holds every answer to its page before it reads a byte of it
(`ScanAnswer::check_against`), and refuses an answer that *reached a key in a tablet it refused*, so the scan
ended `MalformedScan`, a name that says the node sent something that cannot be right, where the tablet's own
refusal (`StaleTopology`, `Quarantined` or `Unavailable`) would have said why. On a standalone node no hosted
tablet's page is refused with those codes, so nothing produced it before a test did; a cluster's copies will,
from [M25](../../future-work/scans/milestones.md#m25-scans-across-a-cluster), which this was drawn before.

## Cause

A page reads the tablets its executor hosts in ascending order, and its progress is one key, `reached`
(`PageTask::run`, `shoal-core/src/server/shard/scan.rs`). A tablet whose page was refused where it stood was
listed among the answer's refusals and came back to the loop as a tablet read to its end (`Ok(None)`), so the
loop set `reached` to its last key. Two other places could land there too: the key of the tablet just below
the next one read (`done_before`), and, when every tablet was read, the last key of the page's last tablet.
The client's rule is that a page's progress may pass over a refused tablet, as it passes over the tablets
another executor hosts, and never land in one, since every key below `reached` that was not refused is
finished.

## Evidence

**Found by reading the source**, in M24's exit review (2026-11-12), **and reproduced** on 2026-11-15 before
the fix, through a test fault that refuses a hosted tablet's walk by a code (`faults::refuse_walk_tablet`),
as a cluster's copy may be refused where it stands. `a_page_that_refuses_a_tablet_ends_its_scan_by_that_refusal`
in `shoal/tests/scans.rs` refuses the last tablet `Quarantined`, a film in it, on a node of one executor.
Against the tree before the fix:

```text
thread 'a_page_that_refuses_a_tablet_ends_its_scan_by_that_refusal' panicked at shoal/tests/scans.rs:622:5:
the scan ended by Protocol(MalformedScan("the answer reached a key in a tablet it refused")), not by its tablet's refusal
```

## The fix

- **A tablet refused where it stood is told apart** from one read to its end: reading a tablet comes to
  `TabletRead::Done`, `Left(key)` or `Refused`, and a refused one moves nothing.
- **A page's progress goes no further than the last of its tablets it did not refuse**, in every place it is
  set: past the tablets below the next one read (`done_before`) and when every tablet is read (`covered_to`),
  both through one function, `last_not_refused`. The page keeps the tablets it refused itself apart
  (`PageTask::refused`), a few at most, from admission's refusals, which name every tablet another executor
  hosts, so the rule costs a look a refused tablet.
- **A node holds its own answer to its page before it sends it**, as its client will (`check_against`, against
  the page's own body bound, `ScanPage::body_bound`, which the client reads too). An answer that cannot be
  right is the node's defect: logged at ERROR and answered `Internal` by name, never put on the wire.

## Alternatives rejected

**Never moving `reached` past a refused tablet**, as the item first said. A page whose lowest tablets another
executor hosts is answered with progress past every one of them; a refused tablet is the same, a tablet the
page did not finish that its client asks again. Holding `reached` below it would end the page at the first
refusal for no reason. The rule is that progress never lands in one.

**Checking only in the client.** It already refused the answer, which is how the defect showed; what it could
not do was say why. The node knows the refusal's code, so the node holds itself to the rule before it sends.

**A debug assertion in the node's check.** A panic on a shard ends the executor and every connection on it, a
far worse answer to one page's defect than a refusal of that page by name.

## Invariants to uphold

- **A page's `reached` never lands in a tablet it refused.** It may pass over one.
- **Every place that sets a page's progress goes through `last_not_refused`**, or the end of a tablet read to
  its end, or the key a tablet was left at.
- **A node holds every answer to its page before it sends it** with the check its client uses, against the
  bound its client uses (`ScanPage::body_bound`).

## Still open

Nothing of this item.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_page_that_refuses_a_tablet_ends_its_scan_by_that_refusal` (`shoal/tests/scans.rs`) | The scan ends `MalformedScan`, its answer having reached a key in the tablet it refused |
| `a_pages_progress_never_lands_in_a_tablet_it_refused` (`shoal-core`'s `server::shard::scan`) | A page's progress lands in a refused tablet below its last, or past every refused one at its end |

## Related

- [F141](../../features/scan-pages-over-tablets.md), whose page this is, and [F140](../../features/scan-frames.md),
  whose check the client and now the node hold an answer to.
- [M25](../../future-work/scans/milestones.md#m25-scans-across-a-cluster), whose copies refuse tablets where they
  stand.
- [The *After M24* box](../../future-work/scans/whats-left-todo.md#finishing-after-m24), with which it was fixed.
