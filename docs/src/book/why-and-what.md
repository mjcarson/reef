# Why and what

Every piece of work says why it is needed and what finishing it will do, so that reading the
plan answers both without opening each doc. A node's `description` holds them:

```yaml
description:
  why: A caller holding a long key list waits for every shard before it sees any row.
  what: Callers step through named keys as pipelined gets, each key handed back in order.
  details: |                   # optional: a longer explanation, shown below them
    Markdown.
```

- **why:** the problem it solves or the gap it fills: what is slow, wrong, missing or unknown
  without it.
- **what:** what will be true, or possible, once it is done: the outcome, not the steps.
- A plain string, `description: Some text.`, is `details` alone.

A part that is not written is taken at build time from the node's first doc that has it, and
the sidebar says where from (*from Context*):

- **why:** the first paragraph or list of a subsection with one of the `why-sections` names,
  or the second cell of a table row (`| **Impact** | ... |`) with one of them. A list gives
  each item's bold lead (`- **Probed once a second**, off the loop`), or the whole item where
  it has none.
- **what:** the same with the `what-sections` names.

A doc's opening paragraph is never taken for either: it is more often a status line ("Reported
2026-10-29.") than a plain why or what.

A node that is not a group and has no why or no what, written or taken, fails the build with
`file:line`.

The design choices are written in `decisions` or taken from the subsection with one of the
`design-sections` names. A list whose items open with a bold lead shows each choice folded,
with its lead as the summary.

Struck-through text is left out of what is taken from docs, since it says what is true now.
The default names:

```toml
[preprocessor.reef]
why-sections = ["Why", "Context", "Motivation", "Problem", "Symptom", "Question", "The question", "Impact"]
what-sections = ["What it does", "What", "Summary", "Overview", "The fix"]  # was description-sections
design-sections = ["Design choices", "Design", "Decisions", "The fix"]
performance-sections = ["Performance", "Results", "Benchmark", "Benchmarks", "Measurements"]
```
