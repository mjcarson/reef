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
reef node log L2                   # who made each edit of it, what it changed and when
reef node log L2 --all             # every edit, not only the newest 200
reef page log design/checkout.md   # the same for a page
```

Every change names you, from your login, and Claude, for you, when Claude makes it through
reef's tools. A history keeps each edit since the server kept histories, newest first. A log
prints the newest 200, then the cursor onto the rest, which `--older <cursor>` prints; `--all`
prints them all.

## Moving nodes to another plan

```sh
reef node move L7 --to hotfix                  # a node to another plan of the project
reef node move L7 L8 L9 --to hotfix            # a subtree of needs, all at once
reef node move G2 --to hotfix --group H1       # a group with its members; one whose group
                                               # stays behind joins `--group`, else none
```

- **Everything of a node's moves with it:** its work record, its captured revisions, its
  comments, numbered anew after the other plan's last, its decisions, its history and its queue
  entry. Its id stays the same, so the other plan can't have one of that id already.
- **A move is checked on both plans,** each at the branch it follows. Needs only work within a
  plan, so moving a node that others left behind need, or that needs nodes left behind, is
  refused with the problems it would add: name the whole subtree, or its group, to move it at
  once.
- **A node a loop holds doesn't move** until the loop lets it go.
- **A move cut off partway is finished by running it again.**

Keep YAML you are editing outside the repo, so it isn't committed by mistake. See [Write
nodes](../projects/nodes.md) for how a node is written and checked.
