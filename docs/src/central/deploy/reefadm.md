# With reefadm

`reefadm` does everything in this chapter with one command. It fetches reef, shoal and glommio,
installs the toolchains they need, builds reef-serve and reef-app for the CPU of the host they'll
run on and a static CLI for any x86-64 Linux, and installs them there with a unit, a config, a
readable copy of reef's CA and the first admin. It records what it deployed, so the next upgrade
can read it.

```sh
env -u RUSTFLAGS cargo install --locked --git https://github.com/mjcarson/reef --branch CentralizedPlanner reefadm
reefadm deploy                     # a team server on this host
reefadm --host europa deploy       # the same on another host, built here for its CPU
reefadm deploy --user              # one person's server, as reef's own plan runs
reefadm --dry-run deploy           # every step it would take, taking none
```

**Install reefadm with `RUSTFLAGS` unset.** A `RUSTFLAGS="-C target-cpu=native"` exported in your
shell would build reefadm for your own CPU. reefadm copies itself to every host it deploys to,
and asks each host's CPU what it has, so it must be built to run on any x86-64. `--host`
refuses to copy a reefadm built otherwise.

**The machine you run reefadm on builds.** That machine needs Ubuntu 26.04 x86-64, and the disk
for the build: about 12 GiB for the checkouts, their builds and the toolchains, so reefadm wants
15 GiB free before it starts one. It asks before it installs packages or rustup there: git,
build-essential, `musl-tools` for the static CLI, curl and tar, then rustup's stable with its
`x86_64-unknown-linux-musl` target and the pinned nightly. The
server needs only Ubuntu 26.04, systemd and sudo, with no toolchain at all. When you run reefadm
on the server itself, the server is the machine that builds.

Run it as yourself. On a Linux other than Ubuntu 26.04 it says so and goes on, untested. It puts
`sudo` in front of only the steps that need root, and asks for sudo's password again before them
if the build outlasted it. Before the build, it asks for the first admin's password, so the build
can run unattended. `--yes` answers yes to every question.

## Built for the target's CPU

reefadm builds the server for exactly what the CPU it will run on has. It names no vendor and
no model.

- **The target is asked.** reefadm's copy there asks its CPU which instruction-set features it
  has, through the CPU itself (CPUID) and the OS's support for the wider registers. A deploy on
  the server itself asks this machine's CPU.
- **The build uses what both machines have.** The flags also reach build scripts that run on the
  machine that builds, and the built reef-serve runs there to say its schema. So a feature the
  target has and the builder lacks is left out, and reefadm names it.
- **The flags are the highest x86-64 level the CPU reaches, plus each other feature.** The level
  is `x86-64-v2`, `v3` or `v4`. Each feature beyond it that the pinned nightly knows is added as
  `+name`:

  ```
  building the server for `-C target-cpu=x86-64-v4 -C target-feature=+aes,+gfni,+sha,+vaes`
  ```

- **A CPU without AES-NI is refused,** since shoal's hashing is built with it.
- **`--target-cpu NAME`** builds for a CPU rustc names instead, such as `x86-64-v3` for a whole
  fleet at once.
- **Only reef-serve and reef-app are built for the target.** The `reef` CLI and reefadm are built
  for any x86-64, once. The CLI is a static musl build, its symbols stripped, so it runs on any
  x86-64 Linux whatever its glibc. It is stamped with reef's commit and when that commit was
  made, which `reef version` prints.
- **The build is checked.** reefadm reads back the flags cargo compiled each server with, and
  refuses a release they didn't reach. On the target, it refuses a release whose reef-serve was
  compiled with a feature that CPU lacks.
- **The release records its flags.** An upgrade builds again when the target's flags change,
  even at the same commit.

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

## From another machine

`--host` runs a reefadm command on another host over ssh, with its prompts on your terminal:

```sh
reefadm --host europa deploy               # deploy europa from here, built here
reefadm --host europa upgrade              # and upgrade it, built here again
reefadm --host ada@reef.example.com status # anything ssh reaches
```

- **It uses your `ssh`,** so `~/.ssh/config`, your agent and ProxyJump all apply. It needs to
  sign in without asking for ssh's own password, with a key or an agent. sudo's prompts and
  reefadm's reach your terminal through `ssh -t`.
- **reefadm runs there as a copy of itself,** kept in `~/.cache/reefadm/reefadm` and sent again
  whenever it differs. A host of another architecture, or one the copy won't run on, is refused,
  saying why.
- **`deploy`, `deploy --adopt` and `upgrade` are built here.** In order:
  1. The copy says what the host has: its CPU's features, its OS, reefadm's record there, and
     anything that would stop the command, such as a port in use. A problem stops it before
     anything is built.
  2. reefadm builds here, for that CPU.
  3. It ships the release there as one tar, into `~/.cache/reefadm/shipped`.
  4. The copy installs it there, and the shipped files are removed.
  - Only the flags about the server go there. `--src`, `--repo`, `--ref` and `--source` name this
    machine's checkouts.
- **Every other command runs there,** with every argument but `--host`, and its exit code comes
  back as yours.
- **Paths name files on that host,** such as `restore FILE` or `--cert`, except
  `--password-file`. That one is read here and sent to a file only you can read there, which
  is removed once the command is done.
- **`--dry-run`** still copies reefadm there and asks it what the host has. It then shows the
  build and the ship, and runs the install there as a dry run of its own, which changes
  nothing.

## Taking over an install made by hand

A server set up by hand, as the rest of this chapter does it, has no record of reefadm's, so
`reefadm upgrade` can't move it. `--adopt` takes it over, keeping its config and its data:

```sh
reefadm deploy --adopt                        # the team server's system unit
reefadm deploy --adopt --from-rev 70c1531     # naming the commit it was built from
reefadm deploy --user --adopt                 # your own user unit, as localhost.md makes it
```

- **It reads the install as systemd runs it,** drop-ins and all: the server the unit starts,
  the account it runs as, and the config it reads. That is `--conf`, else `REEF_CONFIG=`, else
  the default under that account's home. A team server must run as the `reef` account, whose
  home is `/var/lib/reef`, as [Building and installing](build.md) makes it.
- **It checks the schema before building,** when it can tell which one the data was written with.
  `--from-rev` names the reef commit the install was built from. Without it, a reef-serve new
  enough to say its schema says it. Otherwise reefadm says it can't tell, and relies on the
  backup it takes.
- **It keeps the install before changing anything.** Once the new release is built, it stops
  reef and backs it up to `/var/backups/reef`. It keeps the unit beside reefadm's as
  `reef.service.before-reefadm`, and copies the commands and the app to `releases/adopted`,
  which reefadm never prunes. The app's old directory, such as `/opt/reef/app`, isn't used
  any more; it's yours to remove.
- **The config is kept as it is,** unless reefadm wouldn't read it the same. A config elsewhere,
  or one that left a path such as `[shoal] data` to reef's default, is written to
  `/etc/reef/config.toml` naming that path, since reefadm's unit sets no `XDG_*` variables. Only
  the lines naming those paths are added: its other keys and its comments stay as they were.
  The one that was there is kept as `config.toml.before-reefadm`.
- **The unit's own settings are kept in a drop-in.** Anything the unit made by hand sets that
  reefadm's unit doesn't, such as `CPUAffinity=`, `AllowedCPUs=` or `MemoryMax=`, goes into
  `reef.service.d/adopted.conf`, with the comments above each, and still applies. reefadm never
  writes that drop-in again, so it's yours to change. Its `ExecStart=` and `REEF_CONFIG` are
  reefadm's own.
- **If the new release doesn't come up,** it puts the install back as it was: its unit, with the
  drop-in gone, its commands, its config, and its data from the backup. It starts it again if it was running.
- **A server that served plain HTTP,** as reef did before it served HTTPS, now serves HTTPS.
  adopt says so. Each client's `url` needs `https://`, and a client on another host needs reef's CA
  as `ca` ([Certificates](certificates.md)).

From then on it is reefadm's: `reefadm upgrade` moves it, and `reefadm status` reads it.

## Upgrading

```sh
reefadm upgrade                    # the ref it was deployed from, at its newest commit
reefadm upgrade --ref v0.2         # another branch, tag or commit
```

It runs in two halves:

1. **The admin's reefadm fetches and builds,** on the machine it runs on, for the server's CPU;
   with `--host`, it ships the release there. It fetches reef at `--ref`, which defaults to the
   ref the server was deployed from. If the commits and the CPU's flags are the running
   release's, there's nothing to do unless `--rebuild` is given. Before building, it compares the new commit's schema with
   the tables the running release recorded. A table added is fine. A table changed or dropped is
   refused, naming it, since shoal can't migrate a table ([When the store's schema
   changes](../admin/schema-changes.md)). Then it builds and stages the new release.
2. **The new release's reefadm swaps it in.** Its own unit and steps are the ones used. It stops
   reef, writes a backup to `/var/backups/reef` (skip it with `--no-backup`), makes the new
   release current, rewrites the unit if its text changed, starts it and waits for both ports to
   answer. If the new release doesn't come up, it puts the old release back, restores the backup
   over whatever the new one wrote, keeping that aside as `*.failed-<time>`, and fails saying why.

Each release keeps the one before it, so the next upgrade can go back to it. Older releases
are removed, but never one a running process still uses. When it's done, upgrade each user's
`reef` too. When reef's skill changed between the two commits, upgrade says so: run `reef claude
install` again in each repo.

`--fresh` upgrades to a schema that can't read the data. It moves the data and the clones aside
and starts empty. Export every plan first, and import them again after, as [When the store's
schema changes](../admin/schema-changes.md) says.

## Uninstalling

```sh
reefadm uninstall                  # the unit, the releases, the commands and the record
reefadm uninstall --purge          # and the data, the config, the account and the checkouts
```

Without `--purge`, the config, the data, the clones, reef's certificates and the `reef` account
stay, so `reefadm deploy` later puts reef back over them. `--purge` removes those too, with the
account's home and its ssh keys, but never the backups, and it says where they are. Both ask
first.

## Administering

```sh
reefadm status                     # its release, unit, ports, data, certificate and memlock
reefadm logs -f                    # the unit's journal, followed
reefadm backup                     # its data, copied while it is stopped
reefadm restore FILE               # a backup put back, what was there moved aside
reefadm recover ada                # ada an admin again, with a new password
```

- **`status`** shows the deployed release and its ref, the unit's state, whether each port
  answers and whether it asks callers to sign in, the size of the data and the clones, the free
  disk, when reef's own certificate ends, and the unit's locked-memory limit. It exits 1 when
  the unit isn't running or a port doesn't answer, naming each, so a monitor can run it.
- **`backup`** stops reef, writes `reef-<date>T<time>-<release>.tar.gz` to `/var/backups/reef`
  (or `--to DIR`), and starts reef again. The tarball holds the data, the clones, reef's
  certificates and the `reef` account's `.ssh` and `.gitconfig`, readable by root alone, with
  a manifest naming the release that wrote them. Each path is kept absolute, since every
  project keeps its clone's path.
- **`restore`** asks first. If the backup was written by a schema the running release can't read,
  or with another shard count, it says so in the question. It stops reef, moves the data aside as
  `*.before-restore-<time>`, unpacks the backup where it was taken from, gives it to the account,
  and starts reef again.
- **`recover`** asks for the new password, or reads `--password-file`, then stops reef and runs
  `reef-serve recover` as the account with the password on its input ([A lost
  admin](../admin/lost-admin.md)). Then it starts reef again, whatever recover did.

Each takes `--user` for one person's user unit; without it, reefadm picks the team server, else
the user unit if that's all there is.

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

On the machine that builds:

| | |
|---|---|
| the checkouts | `~/.local/share/reefadm/src`, or `--src` |
| each CPU's build of the servers | `src/reef/central/target/cpu-<level>-<hash>`, the three built last kept |
| a release staged to ship | `~/.local/share/reefadm/out` |

On a host deployed with `--host`, a shipped release waits in `~/.cache/reefadm/shipped` until
it is installed, and reefadm's copy is `~/.cache/reefadm/reefadm`.

- **A release holds the binaries and the app.** Each build becomes a release named for its
  reef commit and the time it was built. reefadm copies it in whole, then points `current` at it.
  The unit runs the server under `current`.
- **The unit says reefadm wrote it.** Every upgrade writes it again, so keep your changes in a
  drop-in (`sudo systemctl edit reef`).
- **The checkouts sit side by side,** as central's build needs. reef is fetched at `--ref` from
  `--repo`. shoal and glommio are fetched at the commits that reef commit's
  `central/sources.toml` pins, and `--source shoal=URL` fetches one from elsewhere. A pinned
  commit its branch no longer holds, as when the branch's history was rewritten, is fetched by
  itself, and reefadm says so, since the pin is due to move. The toolchains are the ones the
  same file names.

## When it stops

reefadm checks each step's state before acting on it. A deploy that stopped partway, for example
on a build error or a server that didn't come up, can simply be run again: it keeps what it did
and does the rest. reefadm writes its record last, so once it exists, `reefadm deploy` refuses
and names `reefadm upgrade` instead.

Before changing anything, it refuses in these cases, and says what to change:
- systemd doesn't answer;
- the ports are taken;
- the disk of the machine that builds is too small for the build;
- a `reef.service` exists that reefadm didn't write, such as one set up by hand from this chapter.

When the server doesn't answer, reefadm prints the last lines of the unit's journal.
