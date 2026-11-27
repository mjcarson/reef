# With reefadm

`reefadm` does everything in this chapter with one command. It fetches reef, shoal and glommio,
installs the toolchains they need, builds the CLI, reef-serve and reef-app, and installs them
with a unit, a config, a readable copy of reef's CA and the first admin. It records what it
deployed, so the next upgrade can read it.

```sh
cargo install --locked --git https://github.com/mjcarson/reef --branch CentralizedPlanner reefadm
reefadm deploy                     # a team server on this host
reefadm deploy --user              # one person's server, as reef's own plan runs
reefadm --dry-run deploy           # every step it would take, taking none
```

Run it as yourself, on a Linux host with systemd. It puts `sudo` in front of only the steps
that need root, and asks for sudo's password again before them if the build outlasted it. The
first build takes a while, and about 12 GiB of disk for the checkouts, their builds and the
toolchains; reefadm wants 15 GiB free before it starts one. Before that, it asks for the first
admin's password, so the build can run unattended. It also asks before it installs packages
or rustup. `--yes` answers yes to every question.

## A team server

`reefadm deploy` sets up what the rest of this chapter describes. The pages after this one
are the same steps done by hand.

- **reef-app as the system unit `reef`**, under the account `reef`, with sign-in required.
  `--api-only` runs reef-serve instead.
- **HTTPS for other hosts on `--port`**, 12443 by default, using reef's own certificate. That
  certificate covers each `--name` (repeat the flag for more names, which default to the host's
  own). To serve a certificate of your own instead, give `--cert` and `--key`.
- **`/etc/reef/config.toml`**, written from the flags, including `--cores` and `--memory`. A
  later deploy keeps the file as it is, and refuses a flag that disagrees with it.
- **The CA copied out** to `/etc/reef/reef-ca.pem`, for this host's CLI and for you to hand
  to users.
- **The first admin**, `--first-admin` (default: you), added and signed in from this host.
  `--password-file` reads the password from a file rather than asking twice. `--no-admin`
  leaves the first user to [The first admin](first-admin.md).
- **The firewall**: if ufw or firewalld is on, reefadm asks to open the port.

It ends by saying where reef is and what to hand users:

```
reef is up at https://reef.example.com:12443 for other hosts, and https://127.0.0.1:12180 on this one, from release 992226c4a1b2-20261127072258
  hand each user its CA, /etc/reef/reef-ca.pem, to name as `ca` in their config
  this host's CLI: `export REEF_CONFIG=/etc/reef/config.toml`
  next, from a project's repo: `reef project add NAME`
```

## One person on localhost

`reefadm deploy --user` sets up [One person on localhost](localhost.md): a user unit on
`https://127.0.0.1:12180`, with no config file, no sign-in and no sudo. It turns lingering on,
so reef keeps running after you log out. `--cores` and `--memory` write a config to
`~/.config/reef/config.toml`, and nothing else does.

## What goes where

| | team | user |
|---|---|---|
| releases | `/opt/reef/releases/<commit>-<time>` | `~/.local/share/reef/releases/…` |
| the one in use | `/opt/reef/current`, a link | `~/.local/share/reef/current` |
| `reef`, `reef-serve`, `reefadm` | `/usr/local/bin`, linked to `current/bin` | `~/.cargo/bin`, the same |
| unit | `/etc/systemd/system/reef.service` | `~/.config/systemd/user/reef.service` |
| config | `/etc/reef/config.toml` | none, unless a flag needs one |
| data, clones, certificates | `/var/lib/reef/{shoal,clones,certs}` | `~/.local/share/reef/{shoal,clones,certs}` |
| reefadm's record | `/etc/reef/reefadm.toml` | `~/.local/share/reef/reefadm.toml` |
| the checkouts it builds | `~/.local/share/reefadm/src`, or `--src` | the same |

- **A release holds the binaries and the app.** Each build becomes a release named for its
  reef commit and the time it was built. reefadm copies it in whole, then points `current` at it.
  The unit runs the server under `current`.
- **The unit says reefadm wrote it.** Every upgrade writes it again, so keep your changes in a
  drop-in (`sudo systemctl edit reef`).
- **The checkouts sit side by side,** as central's build needs. reef is fetched at `--ref` from
  `--repo`. shoal and glommio are fetched at the commits that reef commit's
  `central/sources.toml` pins, and `--source shoal=URL` fetches one from elsewhere. The
  toolchains are the ones the same file names.

## When it stops

reefadm checks each step's state before acting on it. A deploy that stopped partway, for example
on a build error or a server that didn't come up, can simply be run again: it keeps what it did
and does the rest. reefadm writes its record last, so once it exists, `reefadm deploy` refuses
and names `reefadm upgrade` instead.

Before changing anything, it refuses in these cases, and says what to change:
- systemd doesn't answer;
- the ports are taken;
- the disk is too small for the build;
- a `reef.service` exists that reefadm didn't write, such as one set up by hand from this chapter.

When the server doesn't answer, reefadm prints the last lines of the unit's journal.
