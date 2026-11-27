# A systemd unit

`reefadm deploy` writes this unit, with ExecStart on its current release, and starts it
([With reefadm](reefadm.md)). By hand, `/etc/systemd/system/reef.service`:

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

It says where it listens, where reef's own CA is, and, with no user yet:

```
reef-app: reef's own certificate is signed by its CA in /var/lib/reef/certs/ca.pem (made now): trust that file in a browser, or name it as `[api] ca` on another host
reef-app: callers must sign in, and reef has no user yet: add the first with `reef user add NAME` from this host
```

Copy that CA where the CLI on this host can read it, as [Configuring](config.md) says.
