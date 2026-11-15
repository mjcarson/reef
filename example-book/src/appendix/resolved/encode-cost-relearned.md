# 372. A table's encode steps could be held at a row or two for its shard's life by one slow first step

## Symptom

A scan's page of a large resident sorted partition took one or two rows an encode step, however cheap
its rows were, on some shards and not others, and stayed so for the shard's life.

In [X19](../../future-work/scans/spikes.md#x19-the-edges)'s run (2026-11-07, europa, a node of four shards
in process, a 256 MiB scan budget) a resident partition of 8 MiB of kibibyte rows took 7,877 steps for its
7,884 rows. One of 17 MiB of 32-byte rows took 247,580 steps for 247,580 rows. Each step held its loop 1 to
1.8 µs. The same partitions on another node's shards took 61 and 460 rows a step.

A step is a message of the loop's second inbox and a piece of the page's answer, so the page:

- took 145 ms for 8 MiB and 860 ms for 17 MiB;
- drew 35 MiB and 157 MiB of the scan budget, each piece an archive root of its own;
- was refused `ResponseTooLarge` where the budget would have held the same partition in pieces of 64 KiB.

## Cause

`EncodeCosts::learn` (`shoal-core/src/server/shard/walk.rs`) learns what a row of a table costs from each
encode step, and `rows_of` sizes the table's next steps as `ENCODE_TIME` over that cost
([item 347](encode-steps-by-rows.md)). Two rules combined:

- **The first step to teach a table sets its cost outright** (`or_insert(shown)`). Every later step only
  moves it a quarter of the way, at most four times as far. A first step on a partition its loop had not
  touched yet, 20 rows in 198 µs of its thread's cpu in X19's run, set 9.9 µs a row: five rows a step.
- **A step of fewer than `ENCODE_ROWS_LEARNED`, eight rows, teaches nothing**, so that a step's lookup of
  its key is no large part of what a row is learned to cost. Once the cost allows fewer than eight rows a
  step, every step takes fewer than eight, and none teaches it again.

The cost was then never corrected on that shard, whatever its rows cost.

## Evidence

**Established by reproducing it**, twice:

- **X19's lab run** above: in the run with a 256 MiB budget, every partition past 2 MiB on the locked
  shards took 1 to 2 rows a step, its steps' own figures showing 0.5 to 1.4 µs of cpu a row. That cost
  allows thirty to a hundred rows a step.
- **A unit test**, `a_tables_steps_are_taught_again_when_their_cost_holds_them_few` (`shard::walk::tests`).
  It teaches a table one slow first step of 20 rows in 200 µs, then 64 steps of the five rows it allows at
  half a microsecond a row. On the tree before the fix:

```text
test server::shard::walk::tests::a_tables_steps_are_taught_again_when_their_cost_holds_them_few ... FAILED
thread '...' panicked at shoal-core/src/server/shard/walk.rs:2145:9:
held at 5 rows a step
```

## The fix

**A step of fewer than eight rows teaches its table's cost when it took every row the cost let it take.**
Then the cost is what held the step to so few, and only such a step can show the cost was wrong. A step cut
short by its bytes or its copy's deadline still teaches nothing, as before, its lookup a large part of its
time.

From a slow first step the cost falls a quarter of the way at a step, and back within about ten steps to
what the rows cost. Once a step takes eight or more rows, it learns as it always did.

## Alternatives rejected

- **Cap the first step as later ones are capped.** A cost that starts from a cold step would still move
  only through steps of eight rows or more, so a table whose rows really cost more than 6 µs, or a run of
  slow steps, would lock it the same way.
- **Learn from every step.** A step of one or two rows cut short by its deadline would teach a row's cost
  as its lookup's, and hold the next steps to a few rows: the feedback the threshold was put there to stop.
- **Learn a fixed cost a step and a cost a row.** That is closer to what a step costs, and a model to fit
  where the rule above already recovers.

## Invariants to uphold

- **No table's cost can stop being taught.** Whatever the cost allows, a step that took all of it teaches
  it. Since [F137](../../features/encode-steps-together.md) a step of several small partitions that its
  rows allowance ended is such a step, however few rows it took (`EncodeTook::held_by_rows`).
- **A step cut short of its allowance by its bytes or its deadline, with fewer than eight rows, teaches
  nothing.**
- **A step's own cpu teaches it, never its wall** (item 350).

## Still open

What a resident partition's page costs once its steps take what they should is X19's E2, measured again on
this fix.

## Tests

| Test | Where | Breaks if |
| --- | --- | --- |
| `a_tables_steps_are_taught_again_when_their_cost_holds_them_few` | `shoal-core/src/server/shard/walk.rs` | A step that took every row its table's cost allowed teaches nothing again |
| `an_encode_steps_rows_are_learned_from_its_tables_steps` | `shoal-core/src/server/shard/walk.rs` | A step cut short with too few rows starts teaching, or a slow step moves the cost more than 1.75 times |

## Related

[Item 347](encode-steps-by-rows.md), which learns a row's cost; [item 350](encode-copy-deadline.md), the
copy's deadline that cuts a step short; [X19](../../future-work/scans/spikes.md#x19-the-edges), whose run found it.
