# One person on localhost

reef's own plan runs this way: reef-serve as a systemd user unit on `https://127.0.0.1:12180`,
with no config file and no sign-in. `reefadm deploy --user` sets it up in one command, with the
app ([With reefadm](reefadm.md#one-person-on-localhost)); this page does it by hand. Its data is in `~/.local/share/reef/shoal`, and the CA and
certificate it makes for itself in `~/.local/share/reef/certs` ([Certificates](certificates.md)).

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

Then [add a project and a plan](../projects/index.md). Nothing else changes: the CLI finds the
server at its default address, and trusts the CA it made. Open `https://localhost:12180` in a
browser, and import `~/.local/share/reef/certs/ca.pem` as an authority to drop its warning.

**Upgrading from a reef that served plain HTTP:** install the CLI and reef-serve together and
restart the unit. An older CLI calls `http://`, which the new server only redirects to a
certificate it doesn't trust. A login kept for `http://127.0.0.1:12180` goes on signing in to
`https://127.0.0.1:12180`.

On a machine that also runs shoal's benchmarks, reef's shards share their cores. See [Beside a
benchmark](benchmarks.md).
