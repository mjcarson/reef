# Keeping reef up to date

A server reefadm deployed hands its users the `reef` its release was built with: a static binary
for any x86-64 Linux, stamped with the commit it was built from and when that commit was made.
Fetch it once, and it keeps up with the server by itself.

```sh
curl --cacert ~/.config/reef/reef-ca.pem -o ~/.local/bin/reef \
  https://reef.example.com:12443/api/v1/client/reef
chmod +x ~/.local/bin/reef
reef version                      # reef 0.1.0 dc5e68cd25c9 (2026-11-27)
```

- **No sign-in is needed to fetch it.** `GET /api/v1/client/reef` and `GET /api/v1/client`, its
  build as JSON, answer anyone, so a new machine fetches `reef` before it has a token. Leave out
  `--cacert` if the system trusts the server's certificate.
- **`reef version`** prints the build, and `reef version --json` the `version`, `commit`,
  `committed` (seconds since the epoch) and `target` the server and the updater read. A `reef`
  that `cargo install` built says `(not stamped)`.

## It updates itself

Every answer the server gives names the build it hands out, in a `reef-client` header. When a
stamped `reef` sees one whose commit was made after its own, it starts `reef update --auto` in
the background, once a process, and goes on with what it was asked. The next `reef` you run is
the new one.

- **Newer only.** Builds are ordered by when their commits were made. A server that went back to
  an older release leaves a newer `reef` alone; `reef update --force` takes the server's anyway.
- **One updater at a time.** It holds `update.lock` in reef's state directory
  (`$XDG_STATE_HOME/reef`, else `~/.local/state/reef`), so any other that starts meanwhile
  leaves. What it did or failed at goes to `update.log` there.
- **A failed install waits an hour.** It keeps the commit in `update.toml`, so a `reef` that
  can't replace itself, such as one in a directory you can't write, doesn't try again on every
  command.
- **The old binary stays until the new one checks out.** The updater streams the server's binary
  into a file beside yours, makes it executable, syncs it, runs it to check it says the commit
  the server named, and only then renames it over yours. A link is followed, so the file it leads
  to is replaced and the link kept.
- **A `reef` running while it is replaced** goes on as it was, such as Claude Code's `reef mcp`,
  and whatever it starts from then on, a follower, an injector or an updater, runs the new
  binary.
- **Turn it off** with `[client] update = false` in reef's config. `reef update` still works by
  hand.

## By hand

```sh
reef update --check               # say whether the server hands out a newer build
reef update                       # install it
reef update --force               # install the server's build even if it isn't newer
```

- **A build `cargo install` made never updates itself.** It isn't stamped, so nothing orders it
  against the server's, but `reef update` typed by hand swaps it for the server's build.
- **A server older than these updates hands out no client,** and `reef update` says so. An old
  server's answers name no build, so nothing updates by itself either.
- **Another architecture or OS is refused.** The server hands out an x86-64 Linux build.

## On the server's host

The server's own `/usr/local/bin/reef` links into its release, as reefadm installed it. That one
is never replaced by an update: `reefadm upgrade` moves it with the server, and `reef update`
says to run that instead.

## The skill

A repo set up with `reef claude install` holds the skill of the `reef` that wrote it. After an
update, a Claude Code session's SessionStart hook says when the repo's skill isn't the one this
`reef` writes: run `reef claude install` again, and commit what it changes.
