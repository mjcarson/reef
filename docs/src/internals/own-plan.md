# reef's own plan

reef's own plan lives in a reef-serve that runs as a systemd user unit, its data in the default
`~/.local/share/reef/shoal`, with no config file. Every caller signs in, on loopback too, so its
first user is you, added once from this host. `plan/` is the plan's export, which git keeps:
it is the copy a fresh data directory is imported from, and the book `mdbook-reef` still builds.
This page is reef's own setup. A server for a team, or one for yourself, is set up as
[Deploying](../central/deploy/index.md) says.

From `central`:

```sh
cargo install --locked --path ../crates/reef --target-dir ../target          # the terminal's client
cargo +nightly install --locked --path reef-service --features embedded --bin reef-serve --target-dir target
```

`~/.config/systemd/user/reef.service`:

```ini
[Unit]
Description=reef's central planner

[Service]
ExecStart=%h/.cargo/bin/reef-serve
Restart=on-failure
TimeoutStopSec=10

[Install]
WantedBy=default.target
```

A locked-memory limit of at least 30 MiB a shard, and 30 more, keeps glommio's registered io
buffers, so a session whose `ulimit -l` is lower needs `LimitMEMLOCK=` in the unit too.

```sh
systemctl --user daemon-reload && systemctl --user enable --now reef
journalctl --user -u reef                  # what it said starting, where its CA is, and any warning
reef user add "$USER"                      # its first user, an admin, once
reef login --user "$USER"                  # a token the CLI, the MCP server and the hooks send
reef project add reef                      # from the repo's root, once
reef import plan --plan central            # the plan, from its book
reef claude install                        # the MCP entry, the hooks and the skill, for the loop
```

- **An edit goes through reef, then out to the book.** `reef node get|put`, `reef status`,
  `reef accept` and `reef page get|put` change the plan, each checked. `reef export plan --replace`
  then writes it back over `plan/`, only the files whose text changed, and the book is committed
  in the same change. `plan/` is never edited by hand: the next export writes over it.
- **A node's body goes through reef too.** `reef node get <ID> --md` prints a node as a node
  file, its `# version:` line inside the frontmatter, and `reef node put` of that file writes its
  body as well. YAML alone keeps the body stored. A body's links lead from the repo's root, as
  the plan keeps them (C69).
- **One command sets a repo up for the loop.** `reef claude install` writes `.mcp.json`, running
  `reef mcp`, and `.claude/settings.json`, running `reef hook …` on SessionStart, Stop, SessionEnd,
  PostToolUse after reef's status tool, WorktreeCreate and WorktreeRemove. Both are committed, so
  every worktree checks them out. `.claude/settings.local.json` enables the server, allows its tools
  and offers a clear at a plan's approval. It stays out of git, and WorktreeCreate copies it into
  each worktree it makes. git's post-commit and post-rewrite go where git runs hooks,
  `core.hooksPath`'s dir if set, a shell hook already there keeping its own lines. Then trusting the
  folder is the only prompt. Running it again changes nothing, so run it after a reinstall too.
- **The skill comes from the binary.** `reef skill`, and `reef claude install`, write
  `crates/reef/assets/skill` to `.claude/skills/reef`, over the mdbook skill, so it names the
  tools and commands of the reef that wrote it. Run it again after a reinstall that changed the
  skill. `mdbook-reef install` and `mdbook-reef skill` write the mdbook skill there instead, so
  pass `--no-skill` to the first, and skip the second.
- **A new binary is a reinstall and a restart.** Install both again, then
  `systemctl --user restart reef`. Stop the unit before running reef-app over the same data. A
  `reef` newer than the service it calls says the service has no route for what it asked.
  `cargo install` builds an unstamped `reef`, which never updates itself, so this repo's stays
  as installed by hand; and this unit's `reef-serve`, in `~/.cargo/bin`, finds no stamped client
  beside it to hand out ([Keeping reef up to date](../central/using/updates.md)).
- **A changed row needs a fresh data directory**, since shoal can't migrate a table: stop the
  unit, move `~/.local/share/reef/shoal` aside, start it, then `reef project add reef` and
  `reef import plan --plan central` again. Export first if the book is behind. What carries over
  and what doesn't is in [When the store's schema
  changed](../central/admin/schema-changes.md). A table added
  needed none when C30 added `WorkRow`: its binary opened a data directory the one before it
  wrote, read the imported plan back, and kept work records beside it. C32's two tables and
  C34's `CommentRow` needed none either, nor did C37's resolutions, which a thread's row keeps
  only once it has one, so a row kept before reads back with none. C39's `LoopRow` moved the
  schema's pin but needed none: its binary read back the nodes and comments C37's wrote. Nor did
  C48's `UserRow` and `TokenRow`: its binary read back a plan the one before it wrote. Nor did
  C49's `RemoteRow`, which keeps a remote project's URL beside its `ProjectRow` rather than in
  it, nor C92's `PlanBranchRow`, which keeps a plan's branch beside its `PlanRow`: its binary
  read back a plan, a node and a queue entry the one before C87 wrote.
