# Administering central reef

This page is for the admins of a server set up as [Deploying](deploy.md) describes: the system
unit `reef`, the account `reef`, its config in `/etc/reef/config.toml` and its data in
`/var/lib/reef`. On a user unit, drop the `sudo` and add `--user` to `systemctl` and
`journalctl`.

## Users

The first user, added from the server's own host, is an admin. After that only an admin adds
users, from any host, and only once they have signed in.

```sh
reef user add bob                  # asks for bob's first password twice; --password-file PATH to read it
reef user add carol --admin        # an admin, who can add and remove users
reef user ls                       # every user, admins marked
reef user passwd bob               # set bob's password, which revokes his tokens
reef user rm bob                   # remove bob and revoke his tokens; never the last admin
```

- **A user's admin flag is set when they are added.** Otherwise, only `reef-serve recover`
  makes an existing user an admin ([below](#a-lost-admin)).
- **Users change their own passwords** with `reef user passwd`, giving their current one, which
  keeps their tokens.
- **The account page** in the app, opened from your name in the bar, does the same for an
  admin: every user, whose password you set or whom you remove.
- **Users can be added before sign-in is required,** so they are ready when `[auth] required`
  is turned on.

## Tokens

Every login is a token. A terminal's lasts until it is revoked. A browser's lasts 30 days, or
until it signs out.

```sh
reef token ls --user bob           # bob's tokens, by id, with their labels and when they were issued
reef token revoke 3f9c0a1b2d4e5f60 # any user's token, by its id
```

A revoked token stops at once, and so does any change stream it opened. Removing a user, or
setting their password, revokes all their tokens.

## Failed sign-ins

Repeated wrong passwords are slowed, at the token route, at the browser's sign-in form and for
a user's own password change:

- **The limits:** a name may fail 5 times, and an address 20, an IPv6 address counted by its
  /64. After that each try waits, starting at a second and doubling up to 5 minutes. It is
  answered 429, with `Retry-After`, before its password is checked.
- **A success clears the name,** and failures are forgotten an hour after the last.
- **It is kept in memory,** so a restart forgets it.

## A lost admin

If no admin can sign in, recover one on the server itself, from the store. Never do this over
the API. The service holds its data directory, so stop it first:

```sh
sudo systemctl stop reef
sudo -u reef reef-serve recover ada --conf /etc/reef/config.toml   # a new password, asked twice
sudo systemctl start reef
```

- **What it does:** it sets ada's password, makes ada an admin and revokes ada's tokens. If no
  user has the name, it adds ada as an admin.
- **`--password-file PATH`** reads the password from a file instead.
- **It runs as `reef`,** the account that owns the data.
- **A running service refuses it,** saying the store is locked.
- **With an [external shoal node](deploy.md#an-external-shoal-node)**, it runs while the
  service does. The service then trusts a revoked token for up to a minute.

## Projects and remotes

- **Who registers what:** any signed-in user registers a project by a remote URL. Only an admin
  registers a directory on the server's host, or a remote that is a path there (`file://` or
  bare).
- **`[remotes] hosts`** limits the hosts any remote may be cloned from. Set it on a server whose
  account can read more than its users should.
- **`[remotes] fetch_every`** sets how often every remote project is fetched: `30s`, `5m`, `1h`,
  or `0s` for only when asked. Each fetch is shallow and of one branch, and git gives up after
  5 minutes.
- **Clones are never removed,** since a project can't be yet (C79). They sit in
  `[remotes] clones`, named by project and the moment each was made.
- **Never move `[remotes] clones`.** Each project keeps its clone's path, so a moved directory
  strands them.
- **Private remotes** are reached through the `reef` account's ssh keys or git credential
  helpers ([Private repos](projects-and-plans.md#private-repos)).

## Logs and health

```sh
journalctl -u reef -b              # where it listens, sign-in, and any warning about its config
curl https://reef.example.com:12443/api/v1/me      # {"required":true}: up, and asking callers to sign in
```

`/api/v1/me` answers without signing in, so it can serve as a health check.

As it starts, the server checks its config and refuses or warns, naming the fix, for:
- a memory budget of 0;
- a data path that is relative or on tmpfs;
- a certificate it can't read;
- a locked-memory limit too low for its shards.

## Upgrades

A new version is a rebuild, a reinstall and a restart. Back up first ([Backups](#backups)), so
you can go back to the old binaries and their data.

From reef's checkout:

```sh
git pull                           # and shoal's and glommio's checkouts, if reef needs newer ones
# build as Deploying says, then:
sudo systemctl stop reef
sudo install -m 0755 target/release/reef central/target/release/reef-serve /usr/local/bin/
sudo rm -rf /opt/reef/app && sudo cp -r central/target/dx/reef-app/release/web /opt/reef/app
sudo systemctl start reef
```

- **Tell users to upgrade their `reef`.** A CLI newer than the server says the server has no
  route for what it asked.
- **When the skill changed,** users run `reef claude install` again in each repo. It changes
  only what is out of date, and the skill it writes names the tools of the `reef` that wrote
  it. Commit the changed skill.

### When the store's schema changed

Shoal can't migrate a table yet. A version that adds a table reads the old data as it is. A
version that changes an existing row needs a fresh data directory. Before upgrading, read what
changed in `central/reef-service/src/schema.rs` since the running version. reef's own history of
which changes needed one is in [Running it for real](../../central/README.md#running-it-for-real).

Only a plan's nodes and pages carry over to a fresh data directory, through an export and an
import:

1. **Export every plan** with the old server still running, from a checkout of its project's
   repo: `reef export DIR --plan P`, or `--replace` over the book you keep in git. Note
   `reef user ls` and `reef project ls`.
2. **Stop it, and move the data aside:**
   `sudo systemctl stop reef && sudo mv /var/lib/reef/shoal /var/lib/reef/shoal.old && sudo mv
   /var/lib/reef/clones /var/lib/reef/clones.old`. Then recreate both directories, owned by
   `reef`, as [Install](deploy.md#install) does.
3. **Install the new version and start it.** Add the first admin from the host, then every
   user again, each with a new password.
4. **Register each project again**, by the same URL, then `reef import DIR --plan P` each plan.

**What doesn't carry over:** users and their tokens, the queue, work records, captured
revisions, comment threads, loops, and the plans and questions kept from Claude sessions. Nodes
keep their status and their criteria, since those are part of each node.

## Backups

A backup is the data directory and the clones, copied while the service is stopped:

```sh
sudo systemctl stop reef
sudo tar -C /var/lib/reef -czf /srv/backup/reef-$(date +%F).tar.gz shoal clones
sudo systemctl start reef
```

To restore one, stop the service, move the current directories aside, unpack the backup in
their place, and start it again:

```sh
sudo systemctl stop reef
sudo mv /var/lib/reef/shoal /var/lib/reef/shoal.bad && sudo mv /var/lib/reef/clones /var/lib/reef/clones.bad
sudo tar -C /var/lib/reef -xzf /srv/backup/reef-2026-11-24.tar.gz
sudo systemctl start reef
```

Everything comes back as it was at the backup: plans, pages, users, tokens, the queue, work
records, comments. Anything written after it is gone. The clones' paths are kept in the data,
so restore both to the paths they were backed up from.

Exported plans, committed in their repos as [Keep an export in
git](projects-and-plans.md#keep-an-export-in-git) describes, are a second copy of the nodes and
pages that needs no downtime.

## What it can't do yet

- **No authors.** Edits, comments and work records don't say which user made them (C50).
- **One disk.** Shoal keeps one copy of the data, so a lost disk is a restore from backup
  (C52).
- **No push webhook.** A pushed doc is seen at the next fetch, or after `reef project fetch`
  (C78).
- **Projects can't be removed** (C79), nor can their clones be cleaned up, and plans can't be
  removed either.
- **No reverse proxy.** reef reads no forwarded headers
  ([Don't](deploy.md#dont)).
- **The plain port trusts its host.** With sign-in off, a web page that DNS rebinding points at
  127.0.0.1 counts as the plain port's own origin (C80).
- **Docs in LFS or submodules.** A doc kept in LFS reads as its pointer, and a submodule's
  files aren't read.
- **The sign-in throttle forgets** on a restart.
