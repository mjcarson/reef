# Changing the plan

```sh
reef node get L2 > l2.yaml         # its YAML, after a `# version:` line
reef node put l2.yaml              # checked against the whole plan; refused if stale or if it breaks it
reef node get L2 --md              # as a node file, with its body
reef status L2 in-progress         # todo | in-progress | done | blocked
reef accept L2                     # its criteria, numbered
reef accept L2 1 met --test tests/checkout.rs::paid_order_shows
reef check                         # the plan's problems
reef page get design/checkout.md   # the plan's own pages
```

Keep YAML you are editing outside the repo, so it isn't committed by mistake. See [Write
nodes](../projects/nodes.md) for how a node is written and checked.
