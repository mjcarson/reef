# reef

## This repo's plan lives in central reef

reef's own plan is `reef/central`, held by central reef: a `reef-serve` running as the systemd
user unit `reef` (`central/README.md`, "Running it for real"). `plan/` is that plan's export.
It is generated, so never edit it by hand: the next export writes over it.

The reef skill's rules still apply to this plan (why and what, acceptance written first,
`in-progress` when the code starts, `done` and each criterion `met` or `unmet` in the change
that finishes). Only the mechanics differ, until the skill for central reef exists (C38):

- **Find work** with `reef next`, and read a node with `reef node get <ID>`.
- **Change a node** by putting back what `get` printed, edited, keeping its `# version:` line:
  `reef node get C29 > <scratch>/C29.yml`, edit, `reef node put <scratch>/C29.yml`, with the
  file kept out of the repo. YAML without a version line is a new node. A group's members go in its `nodes:`, or as nodes of their own naming it in `group:`.
- **Set a status or a criterion** with `reef status <ID> <STATUS>` and
  `reef accept <ID> <N> met|unmet [--test TEST]` (`reef accept <ID>` lists them, from 1).
- **Change a design page** with `reef page get <PATH>` and `reef page put <PATH> <FILE>`, keeping
  its `<!-- version: N -->` line.
- **Check** with `reef check`. Every write is checked against the whole plan anyway, and one
  that would add a problem is refused with check's messages.
- **Then write the book**: `reef export plan --replace`, and commit `plan/` in the same change
  as the work. `cargo run -q --release -p mdbook-reef -- check plan` still checks it.
- **A node's body** (the markdown after a file's frontmatter) can't be put through reef yet
  (C69): edit it in `plan/`, run `reef import plan --plan central --replace`, then export.

Run `reef` from the repo's root. If it says nothing answers, start the server with
`systemctl --user start reef`.
