# 388. A client allocated for a page's body before its answer was held to the page that asked it

## Symptom

A node could make a client reserve far more memory for a scan's page than that page could have been
answered with. A client's reader allocated a page's body from the length its answer declared as soon as
the length was within `SCAN_BODY_BOUND`, 72 MiB, the most any page's body may be, and only held the
answer to the page that asked it once the whole body had arrived. A scan has up to 64 pages in flight on
a node, so a node that answered every page with the largest length made its client reserve 4.6 GiB of
address space, resident as far as the node then sent bytes, for pages that asked a megabyte each. A node
that declared a long body and sent none of it left the scan waiting out its 45 second ask with the
buffer reserved.

## Cause

The bound a page's answer is held to is the page's own: twice the answer bytes it asked, since a page may
pass its bound by the partition it was reading, and the largest row a node admits past that
(`settle`, `shoal-client/src/client/scans.rs`). The reader that assembles the body is the connection's,
shared by every query on it (`TcpProxy::read_frame`, the `ScanAnswer` arm in `shoal-client/src/client.rs`),
and the slot it found the page by (`Waiter`) carried nothing of the page. So the reader could only hold a
body to the bound of any page, and the page's bound came after the allocation.
[F140](../../features/scan-frames.md) said a client checks an answer against its page before it
allocates for its body; what came first was the bound of every page, not of this one.

## Evidence

**Found by reading the source**, in M24's exit review (2026-11-12), **and reproduced** on 2026-11-15
before the fix. A review of the *After M24* plan found the rest the same day, reproduced before its fix too:
an answer of another kind under a page's id was allocated up to the client's bound on a get's answer, a
gibibyte, and a body within the page's bound was reserved whole before a byte of it came. `a_body_past_its_pages_bound_is_refused_before_it_is_allocated` in `shoal/tests/scans.rs`
puts a forwarder between a client and a node, raises the body length the first streamed answer declares
to 71 MiB, past the bound of a page that asked 64 KiB of answer and under `SCAN_BODY_BOUND`, and passes
on none of that page's data frames. Against the tree before the fix the client reserved the body and
waited for it:

```text
thread 'a_body_past_its_pages_bound_is_refused_before_it_is_allocated' panicked at shoal/tests/scans.rs:535:6:
the scan waited for a body its page could never have asked: Elapsed(())
```

`an_answer_of_another_kind_to_a_page_is_refused_before_it_is_allocated` puts an opener of a get's answer
declaring 900 MiB in place of the first answer a page streams, and passes on none of its data frames. Before
the fix's rest:

```text
thread 'an_answer_of_another_kind_to_a_page_is_refused_before_it_is_allocated' panicked at shoal/tests/scans.rs:593:6:
the scan waited for a body of another kind of answer: Elapsed(())
```

`a_pages_body_is_reserved_no_further_than_it_is_answered_with`, a unit test of the reader, opens a page's
body of 60 MiB and sends none of it; with the body reserved at its declared length it fails at its bound of
256 KiB.

## The fix

A page's slot carries its page's body bound. The scan registers its ask with `Shoal::track_page`, which
puts the bound in the slot (`Waiter::page_body`); any other query's slot carries none. Both the ask and
`settle` take the bound from one function, `page_body_bound`, so the reader and the scan never disagree.

The reader holds a declared body to two bounds before it allocates a byte:

- **Any page's**, as before. A body past `SCAN_BODY_BOUND` is a frame no node of this build sends, and it
  ends the connection.
- **The page's own.** A body past it is read and dropped unassembled (`Inbound::refuse`), and the page's
  scan is handed the answer's head alone. `settle` holds the head to its page as it always did, finds its
  body longer than the page allows, and ends the scan by name, `MalformedScan`, with nothing handed out.
  A slot that asked no page has a bound of zero, so no body of a scan's is ever assembled for a get or a
  bundle.
- **No other kind's body under a page's id.** An opener of a get's answer or of an object's range under a
  page's slot is read and dropped unassembled too, and the page's ask handed that answer without its body,
  which it refuses by name, `MalformedScan`. Before, either was allocated up to what the client takes of a
  get's answer, a gibibyte unless set.
- **A body is reserved no further than a page is answered with.** The slot carries a reservation beside its
  bound, twice the answer bytes the page asked (`page_body_reserve`), and a body within the bound reserves
  no more than that before its bytes arrive and grows as they come. The bound's 64 MiB is a row's room past a
  page's bytes, which a page reaches only when one large partition carries it; reserved whole, every page a
  node declared long would hold it before sending a byte.

The connection is kept: the answer is one page's, and the other pages on the connection are answered as
they were.

## Alternatives rejected

**Ending the connection at a body past its page's bound**, as a body past any page's does. Every page on
the connection would fail `ConnectionLost`, which a scan asks again, so a node that answered every page
too long would be asked again and again for the scan's patience, never refused by name. One page's answer
that cannot be right ends that page's scan by name, which W4 asks of every such answer.

**Checking the head against the whole page in the reader**, `check_against` and all. The reader would need
each page's frame beside its slot, a copy of every page asked. The body's length is the only thing the
reader allocates by, so it is the only thing the reader needs to hold; the scan holds the rest, before it
reads a byte of the body.

**A smaller `SCAN_BODY_BOUND`.** It is the most any page's body may be, set by the largest page a client
may ask. A page that asks less is bounded by what it asked.

## Invariants to uphold

- **A scan's body is held to the bound of the page that asked it before anything is allocated for it.**
  The reader's bound and `settle`'s come from `page_body_bound` and nowhere else.
- **A page's slot assembles a scan's answer's body and no other kind's.** Every opener arm of the reader
  asks whether its slot is a page's before it allocates.
- **A page's body is reserved no further than `page_body_reserve` before its bytes arrive.**
- **A slot that asked no page assembles no body of a scan's.** A page's slot is registered with
  `track_page`; every other slot's `page_body` is `None`.
- **One page's answer that cannot be right ends that page's scan by name, and keeps its connection.** Only
  a frame no node of this build could send ends a connection.

## Still open

Nothing of this item. A scan still holds a page's answer whole in memory before it hands out a row, at
most its page's bound a stream, which [F145](../../features/scan-driver.md#limitations) records. A get's
streamed answer is reserved at its declared length, up to what the client said it takes of one, as it was
before scans: the client chose that bound, where a page's is the node's to keep to.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_body_past_its_pages_bound_is_refused_before_it_is_allocated` (`shoal/tests/scans.rs`) | The client reserves the inflated body and waits for it, and the test's ten second wait fails it by name |
| `an_answer_of_another_kind_to_a_page_is_refused_before_it_is_allocated` (`shoal/tests/scans.rs`) | A get's opener under a page's id is allocated up to a gibibyte and waited for, and the test's ten second wait fails it by name |
| `a_pages_body_is_reserved_no_further_than_it_is_answered_with` (`shoal-client`'s `client::page_reader_tests`) | A page's body is reserved at its declared length, 60 MiB, before a byte of it arrives |
| `an_answer_that_cannot_be_right_ends_its_scan_by_name` (`shoal/tests/scans.rs`) | Were the reader to end the scan's connection or let a damaged body through, the scan would not end by name with its token resuming it |

## Related

- [F145](../../features/scan-driver.md), whose reader and driver this is.
- [F140](../../features/scan-frames.md), which said the client holds an answer to its page before it
  allocates for its body.
- [The *After M24* box](../../future-work/scans/whats-left-todo.md#finishing-after-m24), whose cursor sent
  ahead gives a stream a second page in flight, which is why this was fixed first.
