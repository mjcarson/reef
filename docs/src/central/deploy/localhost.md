# One person on localhost

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

Then [add a project and a plan](../projects/index.md). Nothing else changes: the CLI finds the
server at its default address.
