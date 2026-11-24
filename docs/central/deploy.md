# Deploying central reef

This page sets up a team server: reef-app as a system service under an account of its own,
serving other hosts over TLS, with sign-in required. Running it for one person on one machine is
much shorter; see [One person on localhost](#one-person-on-localhost).

The examples use `reef.example.com`, port `12443` for TLS, the account `reef`, its config in
`/etc/reef` and its data in `/var/lib/reef`. Change any of them.

## Choose what to run

- **reef-app or reef-serve.** reef-app serves the web app and the JSON API on one port, so
  people can use a browser. reef-serve serves the JSON API alone, for teams that only use the
  CLI and Claude Code. Their config, data and behaviour are otherwise the same, and either can
  replace the other over the same data directory.
- **Shoal in process, or as a node.** By default the server runs shoal, reef's store, in its own
  process. An external `reef-shoal` node is a second unit to run, and is worth it only if you
  want to restart reef without restarting its store. See [An external shoal
  node](#an-external-shoal-node).
- **One process a data directory.** Never run reef-app and reef-serve over the same data at
  once. The second one refuses to start.

## What it needs

- **Linux with systemd**, and **git 2.29** or later on the server, which clones remote projects.
- **Rust:** stable for the CLI, and nightly for the server, whose store is linked in with the
  `embedded` feature. The app also needs the `wasm32-unknown-unknown` target.
- **dx 0.7.10**, Dioxus's CLI, to build the app: `cargo install dioxus-cli --version 0.7.10 --locked`.
- **Shoal and glommio beside reef:** the `central` workspace builds against shoal's
  `ScanSupport` checkout at `../shoal` and glommio at `../glommio`, next to reef's checkout.
- **A locked-memory limit** of at least 30 MiB for each shoal shard, plus 30 MiB. Below it,
  glommio runs without registered io buffers, and the server warns as it starts.
  `LimitMEMLOCK=infinity` in the unit takes care of it.
- **Build on the server, or on a machine with the same CPU.** The `central` workspace builds
  with `-Ctarget-cpu=native`, so its binaries may not run on an older CPU.

## Build

From reef's checkout:

```sh
cargo build --release -p reef                     # the CLI: target/release/reef
cd central                                        # its .cargo/config.toml must apply
cargo +nightly build --release -p reef-service --features embedded --bin reef-serve
cd reef-app
env -u RUSTFLAGS RUSTUP_TOOLCHAIN=nightly \
  dx build --release @server --platform server --features embedded
```

- **Run the server builds from `central`**, not from reef's root. cargo reads
  `central/.cargo/config.toml`, which shoal's hashing needs, only from there.
- **Unset `RUSTFLAGS` for dx.** dx passes it to the wasm build too, where `-Ctarget-cpu=native`
  breaks it.
- **`--release` goes before `@server`.** After it, only the server is a release build, and the
  browser gets a debug client.
- **The app is two things:** `central/target/dx/reef-app/release/web/server`, the binary, and
  the `public/` directory beside it, the client it serves. Copy both together. The server finds
  `public/` next to itself, else at `$DIOXUS_PUBLIC_PATH`.

## Install

From reef's root again:

```sh
sudo useradd --system --home-dir /var/lib/reef --create-home --shell /usr/sbin/nologin reef
sudo install -d -o reef -g reef -m 0750 /var/lib/reef/shoal /var/lib/reef/clones
sudo install -d -m 0755 /etc/reef /opt/reef
sudo install -m 0755 target/release/reef central/target/release/reef-serve /usr/local/bin/
sudo rm -rf /opt/reef/app
sudo cp -r central/target/dx/reef-app/release/web /opt/reef/app           # server and public/
```

- **The `reef` account owns the data.** Shoal's data and the clones live under its home, and git
  runs as it, so its ssh keys reach private remotes ([Private
  repos](projects-and-plans.md#private-repos)).
- **Install reef-serve even if you run the app.** `reef-serve recover` is how a lost admin is
  restored ([A lost admin](administering.md#a-lost-admin)).
- **Install the CLI on the server too.** The first admin is added from it.

## Configure

`/etc/reef/config.toml`, readable by everyone, since it holds no secret and the CLI on this host
reads its `[api]`:

```toml
[shoal]
data = "/var/lib/reef/shoal"        # absolute, and never on tmpfs
cores = 2                           # shards; each wants 30 MiB of locked memory
memory = "256MiB"                   # each shard's memory budget

[api]
address = "127.0.0.1:12180"         # plain HTTP, loopback only: this host's CLI and first admin

[api.tls]
address = "0.0.0.0:12443"           # other hosts, over TLS
cert = "/etc/reef/fullchain.pem"    # the chain, as PEM, the server's own certificate first
key = "/etc/reef/privkey.pem"       # its private key, as PEM

[auth]
required = true                     # every caller signs in; TLS won't start without it

[remotes]
clones = "/var/lib/reef/clones"     # absolute; never move it once projects are registered
fetch_every = "5m"                  # how often every remote project is fetched; "0s" never
hosts = ["github.com"]              # the only hosts a remote may be cloned from; default any
```

- **A misspelt key stops the server,** naming it, rather than being ignored.
- **The server checks the config before shoal starts.** It refuses a memory budget of 0, a
  relative or tmpfs data path, and a certificate or key it can't read, and says what to change.
- **The plain port stays loopback.** The server refuses to listen for plain HTTP on any other
  address. Other hosts reach it only through `[api.tls]`.

## A certificate

The server reads its certificate only as it starts, so restart it after a renewal.

**Let's Encrypt.** Get a certificate with certbot in whatever way suits the host, then copy it
where the `reef` account can read it, after each renewal too. As a deploy hook,
`/etc/letsencrypt/renewal-hooks/deploy/reef`:

```sh
#!/bin/sh
install -m 0644 "$RENEWED_LINEAGE/fullchain.pem" /etc/reef/fullchain.pem
install -m 0640 -g reef "$RENEWED_LINEAGE/privkey.pem" /etc/reef/privkey.pem
systemctl restart reef
```

Run it once by hand for the first certificate, with
`RENEWED_LINEAGE=/etc/letsencrypt/live/reef.example.com`.

**Self-signed.** Make it a leaf. rustls refuses a CA certificate as a server's own
(`CaUsedAsEndEntity`), and `openssl req -x509` makes a CA unless it is told otherwise:

```sh
sudo openssl req -x509 -newkey rsa:3072 -nodes -days 825 \
  -keyout /etc/reef/privkey.pem -out /etc/reef/fullchain.pem \
  -subj "/CN=reef.example.com" \
  -addext "subjectAltName=DNS:reef.example.com" \
  -addext "basicConstraints=critical,CA:FALSE"
sudo chgrp reef /etc/reef/privkey.pem && sudo chmod 0640 /etc/reef/privkey.pem
```

Hand `/etc/reef/fullchain.pem` to every user, who names it as `ca` in their config
([Point it at the server](using.md#point-it-at-the-server)).

**A CA of your own.** Sign the server's certificate with it, and hand users the CA's
certificate as `ca` instead.

## A systemd unit

`/etc/systemd/system/reef.service`:

```ini
[Unit]
Description=reef's central planner
Wants=network-online.target
After=network-online.target

[Service]
User=reef
Group=reef
Environment=REEF_CONFIG=/etc/reef/config.toml
ExecStart=/opt/reef/app/server
Restart=on-failure
TimeoutStopSec=10
LimitMEMLOCK=infinity

[Install]
WantedBy=multi-user.target
```

- **reef-app reads `$REEF_CONFIG`,** else the default path, and takes no `--conf`. For
  reef-serve, `ExecStart=/usr/local/bin/reef-serve --conf /etc/reef/config.toml` does the same.
- **Never set `PORT`.** reef-app takes it to mean dx is running it, and listens there instead.
- **A port under 1024**, such as 443, needs `AmbientCapabilities=CAP_NET_BIND_SERVICE`.
- **Stopping takes a few seconds.** On SIGTERM the server ends every change stream, waits for
  open requests and closes shoal, which can take up to 3 s.

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now reef
journalctl -u reef -b                     # where it listens, and that it has no user yet
```

It says where it listens and, with no user yet:

```
reef-app: callers must sign in, and reef has no user yet: add the first with `reef user add NAME` from this host
```

## The first admin

Add the first admin right away. Until a user exists, anyone with a shell on the server can add
the first one, who is an admin.

On the server, as yourself:

```sh
reef user add ada                         # over loopback; asks for a password twice
reef login --user ada                     # keeps a token in ~/.config/reef/credentials.toml
reef whoami
```

- **Only the first user comes from loopback without signing in.** After that only an admin adds
  users, from any host ([Users](administering.md#users)).
- **The CLI on the server needs no config** while the plain port is the default `12180`. If you
  moved it, run it with `--conf /etc/reef/config.toml`, or set `REEF_CONFIG`.

## Open it to other hosts

Let the TLS port through the firewall, and nothing else. With ufw, for instance:

```sh
sudo ufw allow 12443/tcp
```

Then check it from another host:

```sh
curl --cacert fullchain.pem https://reef.example.com:12443/api/v1/me     # {"required":true}
```

In a browser, `https://reef.example.com:12443/` sends you to `/sign-in`, then to the projects
once you have signed in. Next:

- [Add a project and a plan](projects-and-plans.md).
- [Set up each user's CLI](using.md).

## An external shoal node

To run shoal in a process of its own, build `reef-shoal` from `central` and give it a config
from `central/reef-shoal/shoal.example.yml`, with absolute data paths, never on tmpfs:

```sh
(cd central && cargo +nightly build --release -p reef-shoal)
sudo install -m 0755 central/target/release/reef-shoal /usr/local/bin/
```

Run it as a unit of its own, `reef-shoal.service`, with
`ExecStart=/usr/local/bin/reef-shoal serve --conf /etc/reef/shoal.yml`, `User=reef` and
`LimitMEMLOCK=infinity`. Then point reef at it, and order the two units:

```toml
[shoal]
address = "127.0.0.1:12100"   # the node's interface and port in shoal.yml
```

```ini
# in reef.service's [Unit]
Requires=reef-shoal.service
After=reef-shoal.service
```

With an external node, `[shoal]`'s `data`, `cores` and `memory` are ignored, since shoal.yml
sets them. `reef-serve recover` then runs while the service does.

## Don't

- **Don't put a reverse proxy in front.** reef reads no `X-Forwarded-*` header, so behind a
  proxy every caller looks like the server's own host. That lets anyone add the first user, and
  it puts every caller under the one address the sign-in throttle counts. Serve TLS from reef
  itself.
- **Don't expect HSTS.** reef sends none, since the plain loopback port shares the browser's
  idea of the host.
- **Don't leave sign-in off on a shared host.** With sign-in off, anyone who reaches the
  loopback port can write, have the service clone any repo its account can read, or register
  any directory. A web page whose name a DNS rebinding points at 127.0.0.1 counts as the plain
  port's own origin too (C80).

## One person on localhost

reef's own plan runs this way: reef-serve as a systemd user unit on `127.0.0.1:12180`, with no
config file, no TLS and no sign-in. Its data is in `~/.local/share/reef/shoal`.

```sh
cargo install --locked --path crates/reef                     # the CLI, into ~/.cargo/bin
cd central
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

```sh
systemctl --user daemon-reload && systemctl --user enable --now reef
journalctl --user -u reef                 # where it listens, and any warning
loginctl enable-linger "$USER"            # keep it running while you are logged out
```

A user unit can't raise its locked-memory limit past your login's. If the journal warns that
the limit is too low, add a `memlock` line for your user in `/etc/security/limits.conf` and
log in again.

Then [add a project and a plan](projects-and-plans.md). Nothing else changes: the CLI finds the
server at its default address.
