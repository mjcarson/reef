# F149. Keys a caller names, stepped through as pipelined gets

## Context

A caller holding a long list of keys has had two ways to read them: a get a key, each a round trip,
or one get naming them all, whose answer arrives whole only once every shard has answered it.
[W2](../future-work/scans/ordering.md#alternatives-rejected) weighed a third, "a client helper that
takes an iterator of keys in the caller's order and fetches them as pipelined gets", as the mechanism
for ordered scans. It was rejected *as the mechanism* on 2026-10-25, since it cannot find keys it was
not told about and is efficient only while most of them exist. A declared order is that mechanism
([W8](../future-work/scans/declared-order.md)). The plan kept the helper as a convenience, needing no
server change, for after M24, once a client scans one node. It was built with the *After M24* box
on 2026-11-15.

## What it does

`client.step_keys::<Movie>(ids, |keys| MovieGet::new(keys).into())` takes any iterator of a table's
partition keys and a closure that makes a get of a run of them. It hands each key back, in the order
it was named, with the rows that get found for it (`KeyRows`, `shoal-client/src/client/steps.rs`):

```rust
let mut steps = client.step_keys::<Movie>(ids, |keys| MovieGet::new(keys).into());
while let Some(found) = steps.next().await {
    let found = found?;
    for movie in found.rows() {
        index.add(found.key(), movie);
    }
}
```

- **Steps of keys, a window of them in flight.** The keys are cut into steps of `KEY_STEP_CHUNK`
  (64), each one get the closure makes, and `KEY_STEP_WINDOW` (4) steps are in flight at once.
  `.chunk(n)` and `.window(n)` set them, each at least one. `.options(SendOptions)` serves every
  step's get at a read level, with a deadline, and tried again while told to, as `exec_with` is. A
  step is a bundle of its own, so a pinned identity is dropped.
- **Each key in its order.** A key named twice comes back twice, its partition asked once a step. A
  key whose get found nothing, or whose rows the get's filter dropped, comes back with none. The rows
  are read in place from the step's answer, which every key of the step shares (`KeyRows::rows`, as
  `ShoalResponse::access`).
- **A get as the caller makes it.** Its filter, projection and limit are the closure's: a projection's
  rows are read with `step_keys_as::<Movie, MovieTitle>`, and a limit holds each step's get, never the
  whole list.
- **A refusal ends it by name.** A step the node refuses, or whose answer cannot be divided between
  its keys, is handed out as the error once, after every key of the steps before it. Nothing comes
  after it. The steps in flight behind it are dropped, which cancels them
  ([F75](client-cancel.md)), as is every step of a helper the caller drops.

## Design choices

- **A closure makes each step's get.** A table's get has a filter, a projection and a limit, and a
  helper that took them as arguments would copy each table's get builder. The closure keeps the
  get whatever it is, and its keys are the only thing the helper chooses.
- **Rows divided by the answer's own index.** A get's answer says which partition each run of its
  rows came from (`GetRows::groups`, [O18](../appendix/optimizations.md)). A key's rows are its
  partition's run, found by hashing the key once on the client (`get_partition_key_from_values`).
  No row is hashed again.
- **An answer is judged before it is divided.** Every row covered once, by a partition the step
  asked for, each named once (`runs_of`), or the step fails `MalformedGet`. A run past the rows would
  read what is not there, and a partition named twice or never asked for would hand a key another's
  rows. A server that answers otherwise is wrong, and the helper says so rather than guessing.
- **Steps answered in order, waited on in order.** `FuturesOrdered` holds the window, so a key comes
  back only after every key before it. A slow step holds the ones behind it, which is the price of
  the caller's order.
- **Collected sends.** Each step is `exec_with`, which answers a get that found nothing as an answer
  and a failure as an error, and retries under `SendOptions::retry`. `send_one` judges an empty get a
  failure, which is wrong here.
- **Sixty-four keys a step, four steps in flight.** A step of 64 keys is one round trip and a node's
  work split over its shards. Four of them keep the shards busy across a round trip and hold a few
  hundred keys' answers at most.

## Alternatives rejected

- **The helper as the mechanism of ordered scans.** W2's rejection stands: it cannot find keys it
  was not told about, and costs a probe for each key that does not exist.
- **One get naming every key.** Its answer arrives whole once every shard has answered it, holds
  every row at once, and has the bound of one answer. The helper holds a window.
- **Steps unordered.** The keys would come back as their steps were answered, a little sooner.
  W2 asked for the caller's order, and `stream_unordered` already serves a caller that does not care.
- **Owned rows.** Deserializing every row would cost a copy a row for a caller that only reads them.
  The rows are read in place, as every get's are, and a caller deserializes what it keeps.
- **Spreading a step's keys by tablet**, so each step reaches one shard. A get of keys on several
  shards is split by its node already, and grouping keys by tablet would break the caller's order
  inside a step.

## Limitations

- **A step is held by its slowest key.** Its answer comes once every shard owning one of its keys
  has answered.
- **No retry by default.** A step refused `Shedding` or `NotLeader` ends the helper unless the
  options give it a retry budget, as a collected send does.
- **The keys must be `Send` and `Clone`.** Each is kept until its step is handed out, and a
  repeated key is cloned once.
- **A key with no rows and a key that does not exist look the same.** Both come back with none, as a
  get answers them.
- **Hash collisions share a partition.** Two keys that hash to one partition key are one partition to
  the engine, and both come back with its rows.

## Invariants to uphold

- **Every key is handed back once, in the order it was named**, with the rows its step's answer holds
  for its partition and no other's.
- **An answer is divided only once its index describes it**: never in part, never by a guess.
- **A step's failure is handed out once and ends the helper**, after every key of the steps before
  it. Dropping the helper drops every step in flight, which cancels them.
- **What it holds is bounded by its window**: `window` steps' keys and answers, and the keys of the
  one being handed out.
- **The helper is the client's alone**: no frame, no server path. `shoal-client-check` builds it
  with no engine.

## Performance

On europa, in release, against a node in process of two shards (an ignored timing,
`the_helpers_cost_beside_a_get_a_key`, never a benchmark): 10,000 films asked in a scattered order,
five rounds. One get a key, each awaited in turn, read **21,500 to 22,300 keys a second**. The helper
at its defaults read **1.64 to 1.96 million**, about eighty times as many. One get naming all 10,000
read **1.08 to 1.16 million**, since its one answer is built and sent whole before any of it is read,
while the helper's steps overlap the node's work with the client's. The helper adds nothing to a
node's query path: every step is an ordinary get.

## Tests

| Test | What breaks if the feature is reverted |
| --- | --- |
| `keys_come_back_in_the_order_they_were_named_with_their_rows` (`shoal/tests/key_steps.rs`) | Keys come back out of their order, a repeated key once, a missing one with another's row, or a step's bundle left tracked |
| `a_sorted_partitions_rows_come_back_with_its_key` (`shoal/tests/key_steps.rs`) | A sorted partition's rows are split or handed to another key |
| `a_filter_and_a_projection_are_kept` (`shoal/tests/key_steps.rs`) | A step's get loses its filter or its projection |
| `a_refused_step_ends_the_helper_by_its_refusal` (`shoal/tests/key_steps.rs`) | A refused step hands out keys after it, another error, or nothing, or leaves its steps in flight |
| `a_dropped_helper_leaves_nothing_tracked` (`shoal/tests/key_steps.rs`) | A dropped helper leaves its steps' bundles tracked while the node holds their answers |
| `an_answers_runs_are_where_its_index_says` (`shoal-client`'s `client::steps`) | A key's rows are read from another run |
| `an_index_that_does_not_describe_its_answer_is_refused` (`client::steps`) | An answer naming a partition not asked for, twice, or past its rows is divided |
| `a_client_steps_through_keys_without_an_engine` (`shoal-client-check/tests/client_half.rs`) | The helper needs the engine to compile |

## Related

- [W2](../future-work/scans/ordering.md#alternatives-rejected), which kept it as a helper and not as
  the mechanism.
- [The client](../api/client.md#keys-a-caller-names), where it sits beside a get and a scan.
- [F75](client-cancel.md), what dropping a step does.
- [The *After M24* box](../future-work/scans/whats-left-todo.md#finishing-after-m24).
