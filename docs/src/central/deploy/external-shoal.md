# An external shoal node

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
