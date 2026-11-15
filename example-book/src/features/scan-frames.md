# F140. The scan frames, the resume token and the `scan` privilege's name

A table was scanned only through the admin walk: a JSON request, one tablet a page, an answer of keys
and counts with no row on the wire, an administrator's alone. Nothing in the protocol could carry a
client's page of a scan, say how far one got, or name a privilege for it.

Now `shoal-proto` holds a scan's page and its answer as bytes both ends agree on, `ScanPage` and
`ScanAnswer`, the resume token a client keeps between them, three refusal codes, the capability bit
that says a client reads them, and `scan` as a third privilege. Nothing sends one yet: this is the
first step of [M24](../future-work/scans/milestones.md#m24-scans-on-one-node), and every later step
speaks these frames.

## Context

[Q36](../future-work/scans/contract.md#q36-the-cursor-and-the-frames-2026-11-06) fixed the frames byte
by byte on 2026-11-06, on [W4](../future-work/scans/cursor-and-pages.md), and
[F132](whole-partitions-in-answers.md) built the body an answer carries below them: the chunk head and
the client's reader of a body in partitions. What was left was the frames themselves.

M24's plan was agreed with the user on 2026-11-10
([Finishing M24](../future-work/scans/whats-left-todo.md#finishing-m24)). Two of its decisions land
here:

1. **M24 is built in steps, a feature page and a commit each**, and the frames come first.
2. **`all` includes `scan`**, which settles [Q39](../future-work/scans/contract.md#questions-to-answer)
   ahead of M25, since M24 reads the privilege from a standalone node's file and has to say what `all`
   means there.

Since [X19](../future-work/scans/the-edges.md#what-it-says-for-the-design) M24 splits a sorted partition
too large for a page across pages, so the sort-key position Q36 reserved beside each cursor is filled:
these frames carry positions, where W4's first draft held them at zero.

## What it does

### `ScanPage`, message type 32

A fixed little-endian head of 96 bytes and its sections, which fill the frame exactly
(`shoal-proto/src/shared/protocol/scan/frames.rs`). The head is the page's id and the scan's, both v7
UUIDs; the table's id and its own fingerprint; the partition layout the client reads; the target's
kind, the read level, the body it asks for and the tablet bits it cuts by; a flag asking for its
tablets' groups; how many cursors and floors follow; its bounds on rows, answer bytes, work and time;
and its spec's length. Then a bitmap of the tablets the page may read (512 bytes), a cursor of 16 bytes
for each tablet the scan left part way, its groups' floors at 16 bytes each, the cursors' positions
padded to 16, and the table's archived scan spec at an offset that is a multiple of 16.

A bound a page names none of takes its default: 1,000 rows, 1 MiB of answer bytes, 4 MiB of work.

### `ScanAnswer`, message type 33

The page's id, then a fixed head of 80 bytes: why the page ended (`ScanEnded`: its rows, its answer
bytes, its work, the end of its tablets, its deadline, the scan budget), the last key it covered
(`reached`), what its body holds, and what it read. Then the groups it read and each one's boundary,
the tablets it refused and why, a map from each requested tablet to its group when the page asked for
one, and the position `reached` stands at inside a partition the page handed out part way. The body, a
run of F132's chunks, follows in [F73](bodies-across-frames.md)'s `Data` frames under the page's id,
and the answer's header carries `STREAMED` exactly when it has one.

**A page's progress is one key.** `ScanAnswer::finished` says which of a page's tablets the answer
finished: every requested tablet below `reached`'s that was not refused, and `reached`'s own when
`reached` is its last key and the page stopped inside no partition. `ScanAnswer::cursor` is what the
page leaves in the tablet it stopped in.

### The resume token

`ScanToken`: `SHSC`, a version, what the scan is (`ScanIdentity`: its target, level, body, cluster,
fingerprint, layout and the CRC-64/NVME of its spec), the tablets not finished, the cursors and floors
as a page carries them, and a CRC-64/NVME of every byte before it. 600 bytes, and 16 more for each
tablet left part way and each group read. `decode` refuses bytes that are not a token by name
(`TokenError::Damaged`, `UnknownVersion`), and `resumes` refuses one taken from another scan
(`WrongCluster`, `WrongFingerprint`, `WrongSpec`).

### Codes, the capability and the privilege

- `HostedElsewhere` (58), a tablet another shard of the node hosts, answered inside a page's answer;
  `WrongFingerprint` (59), a scan naming a fingerprint that is not its table's; `UnknownLayout` (75), a
  scan asking for partitions in a layout the node does not write.
- `CLIENT_CAP_SCANS`, bit 5 of the hello's capability byte. No client asks for it and no server grants
  it yet.
- `ProtocolError::MalformedScan`, a frame that cannot be right, which ends its connection.
- `Privileges::SCAN`, written `scan`. **`read` never implies it.** `all` is every privilege, scanning
  with it; `Privileges::READ_WRITE` is what `all` meant before.
- `tablet_of_bits`, the tablet of a key at any count of bits, which the test of a cursor surviving a
  split of its tablet is written against.

## Design choices

**Heads are fixed bytes, never archives**, as an object operation's are
([F80](object-wire.md)): a build reads one with no schema in hand, and a field's place is held by a
test of its bytes, not by a derive.

**Malformed or refused are told apart at the decoder.** `ScanPage::decode` fails only for a frame
that cannot be right. What a well formed page can ask that no node of this build serves is
`ScanPage::refusal`: other tablet bits, a listing, a layout this build does not write, a bound past its
most, each with the code it is answered by. So a client that asks too much is told so by name and keeps
its connection.

**A page of other tablet bits is read no further than its head.** The bitmap's length follows the
bits, so a build that cuts by twelve cannot size another count's sections. The head alone is enough to
refuse the page by name, and nothing after it is read.

**Every count is capped before anything is sized by it**: 64 cursors, 255 floors, 64 KiB of spec,
1,024 bytes a position. Past the caps, the sections must fill the frame exactly, so no length is taken
on the peer's word alone.

**Cursors ascend by key and floors by group.** A cursor in a tablet twice, or a group named twice, is
then one comparison with its neighbour. W4 asked that each group be there once; ascending order is how
that is held, and it makes every frame canonical.

**A client's check of an answer is in `shoal-proto`** (`ScanAnswer::check_against`), beside the
decoder, so the rule a client holds an answer to before it allocates for the body is one function with
tests of its own, not a driver's detail.

**Every privilege is written `all`.** A build from before scans refuses a list naming `scan`. A deploy
tool of this build grants a loader everything on a cluster that may still run that build, so the full
set is written as the one name both read (`Privileges::written`), and any narrower set as its names.

**A cluster commits no `scan` yet.** A committed grant is read by every member, and a member built
before scans cannot read the name. So the control leader takes `scan` out of a grant or a revoke before
it proposes it, and refuses one that names nothing else, until the scans' wire version is activated at
[M25](../future-work/scans/milestones.md#m25-scans-across-a-cluster). A cluster answers no scan before
then either.

## Alternatives rejected

**An archived head.** rkyv would hold the layout for us, and would tie both frames to a derive's
output and to `CLIENT_WIRE_VERSION`. A fixed head moves only when someone moves it.

**A signed token.** Everything in a token is what a page could ask anyway, and a node judges the grant,
the table, its fingerprint and every bound on every page. A key to sign with would be shared by every
node and rotated, and would stop a token resuming against a restored cluster
([W4](../future-work/scans/cursor-and-pages.md#the-resume-token)). The check is for a token damaged
where it was kept.

**Positions as row counts.** A count of rows handed out of a partition is cheaper to carry than a sort
key and wrong the moment a row is written or deleted below it. A sort key is a position whatever is
written around it.

**`all` left as reading and writing.** It was the narrower reading, and the user chose the other on
2026-11-10: `all` should mean everything.

## Limitations

- **Nothing sends or answers a frame.** The server's half is M24's fourth step and the client's its
  fifth.
- **A position is at most 1,024 bytes.** A page never stops after a row whose sort key encodes past
  that; it goes on to the next. A partition whose every sort key is longer cannot be split and is
  handed out whole.
- **A page's frame is at most about 134 KiB**, not the 70 KiB W4 named before positions were filled:
  64 positions of 1,024 bytes are the difference.
- **`all` granted on a cluster before M25 reaches no scan.** It is committed as reading and writing. A
  grant of `all` made before scans existed was committed as those two names and stays so; only a
  standalone node's file, read at every start, takes `all` as every privilege.
- **A token's cluster is whatever its client's route table names.** Nothing here checks it against a
  node.
- **`tablet_of_bits` is a rule nothing uses but its test.** Nothing splits tablets.

## Invariants to uphold

- **Neither head is an archive, and no field moves.** A new field takes reserved bytes, which are zero
  until a build reads them and refused when set before.
- **A frame that cannot be right ends its connection; a page the node will not serve is answered by
  name.** Keep the two apart: `decode` for the first, `refusal` and the server for the second.
- **Every count has a cap judged before anything is sized by it, and sections fill their frame
  exactly.**
- **A client checks an answer against its page before it allocates for its body.** ~~As built the
  reader holds a body to the most any page's may be, `SCAN_BODY_BOUND`, before it allocates, and to
  its own page only once the scan settles it ([item 388](../appendix/known-issues.md)).~~ Since
  [Resolved #388](../appendix/resolved/scan-body-held-to-its-page.md) the reader holds a body to its own
  page's bound before it allocates, and the scan holds the rest of the answer to its page before it
  reads a byte of the body.
- **`read` never implies `scan`.**
- **A set that holds every privilege is written `all`, never as its names**, until every build in
  service reads `scan`.
- **A cluster's committed grants hold no `scan` before its scans' wire version is activated.**
- **A token names positions, never places**, and is refused for any scan but its own.

## Performance

Nothing on a query's path changed: no frame is read or written yet, and a privilege check is the one
mask it was. Encoding and decoding a page or an answer copies its sections once; a page's spec is at
most 64 KiB.

## Tests

| Test | Where | Breaks if |
| --- | --- | --- |
| `the_scan_frames_heads_are_pinned_by_their_bytes` | `shoal-proto`, `scan/frame_tests.rs` | A field of either head, a section or the token moves, widens or changes its byte order |
| `scan_frames_round_trip` | the same | A page, an answer or a token does not read back as written, positions included |
| `a_page_naming_no_bound_takes_the_defaults` | the same | A bound's default moves |
| `a_page_this_build_does_not_serve_is_refused_by_name` | the same | A bound past its most, a listing, another layout or other tablet bits ends a connection instead of being answered by name |
| `a_page_that_cannot_be_right_is_malformed` | the same | A reserved byte, a count past its cap, sections that do not fill, a cursor out of order or outside its tablets, or an id that is not v7 is read |
| `an_answer_that_cannot_be_right_is_malformed` | the same | The same for an answer: its flags, counts, map, refusals and chunks |
| `an_answer_is_checked_against_its_page` | the same | A client accepts an answer its page could not have had |
| `an_answers_progress_is_one_key` | the same | A tablet is called finished that was refused, or left part way, or stopped inside a partition |
| `a_resume_token_that_does_not_check_is_refused_by_name` | the same | A damaged token, or one of another version, cluster, table or spec, resumes a scan |
| `every_byte_of_a_scan_frame_flipped_never_panics` | the same | Any flipped or missing byte of a page or an answer panics its reader |
| `a_tablet_set_is_its_bitmap` | the same | A tablet's bit moves, or tablets are read out of order |
| `a_position_survives_a_split_of_its_tablet` | `shoal-proto`, `placement.rs` | A key's tablet at one more bit is not a half of its tablet today |
| `scan_is_a_privilege_of_its_own` | `shoal-proto`, `grants.rs` | `read` or `write` reaches a scan, or `all` does not |
| `every_privilege_is_written_as_all` | the same | The full set is written as names a build before scans cannot read |
| `every_message_type_round_trips_through_its_discriminant`, `every_error_code_round_trips_through_its_discriminant` | `shoal-proto`, `protocol/tests.rs` | A message type or a code is renumbered |

## Related

- [W4, the cursor and the pages](../future-work/scans/cursor-and-pages.md), which these bytes are
- [F132](whole-partitions-in-answers.md), the body an answer carries
- [F80](object-wire.md), the object frames these are shaped like, and [F73](bodies-across-frames.md),
  the data frames a body rides in
- [F75](client-cancel.md), whose `Cancel` names a page's id
- [Finishing M24](../future-work/scans/whats-left-todo.md#finishing-m24), the steps after this one
