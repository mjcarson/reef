# 399. A scan's body grown past its reservation was rounded up to a power of two

## Symptom

A client reading a page's body longer than its reservation held up to twice the room the body needed: a body of 66 MiB,
within its page's bound, ended in 128 MiB of capacity after about 126 MiB of copies. Across a scan's 64 pages in flight
a node could make a client hold about 8 GiB where [item 388](scan-body-held-to-its-page.md)'s bound meant 4.6.

## Cause

A body's reader reserves no further than twice its page's answer bytes before its bytes arrive (item 388), and each data
frame grew it with `AlignedVec::reserve`, which rounds a capacity up to the next power of two
(`read_payload_into`, `shoal-client/src/client.rs`).

## Evidence

Found by the After M24 exit's review of the client half (2026-11-15), by reading. **Reproduced** by
`a_pages_body_grows_no_further_than_its_length` (`client::page_reader_tests`), on the tree before the fix:

```text
thread 'client::page_reader_tests::a_pages_body_grows_no_further_than_its_length' panicked at shoal-client/src/client.rs:7066:9:
a body of 3146752 bytes held 4194304 bytes of room
```

## The fix

A body's assembly keeps the length its opener declared (`Assembling::declared`), and a data frame that needs more room
doubles the body's capacity, so it is copied few times, never past that length (`reserve_exact`).

## Alternatives rejected

- **Reserving the declared length at the opener**, as before item 388: a node could then make a client reserve a page's
  whole bound for every page before sending a byte.

## Invariants to uphold

- **No body grows past the length its opener declared**, and it is reserved no further than twice its page's answer
  bytes before its bytes come.

## Still open

Nothing.

## Tests

| Test | What breaks if the fix is reverted |
| --- | --- |
| `a_pages_body_grows_no_further_than_its_length` (`shoal-client`'s `client::page_reader_tests`) | A body past its reservation ends with up to twice its length's room |

## Related

- [Resolved #388](scan-body-held-to-its-page.md), whose reservation this grows from.
