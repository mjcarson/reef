# Reaching shoal

reef-service starts a shoal pool in its own process, unless reef's config names an external node.
The config is TOML at `$REEF_CONFIG`, else `$XDG_CONFIG_HOME/reef/config.toml`, else
`~/.config/reef/config.toml`. With no file there, every key takes its default.

```toml
[shoal]
address = "127.0.0.1:12100"   # an external node; unset starts shoal in this process
data = "/home/me/.local/share/reef/shoal"   # the in-process pool's data; default $XDG_DATA_HOME/reef/shoal
cores = 2                     # the in-process pool's shards
memory = "256MiB"             # each shard's memory budget

[api]
address = "127.0.0.1:12180"   # where the JSON API listens over HTTPS, and `reef` calls it: loopback only
certs = "/home/me/.local/share/reef/certs"   # reef's own CA and certificate; default $XDG_DATA_HOME/reef/certs
names = ["reef.example"]      # more names reef's own certificate covers

[api.tls]                     # other hosts, beside it
address = "0.0.0.0:12443"     # any address
cert = "/etc/reef/fullchain.pem"   # the certificate chain, as PEM, the server's own first; else reef's own
key = "/etc/reef/privkey.pem"      # its private key, as PEM

[remotes]
clones = "/home/me/.local/share/reef/clones"   # remote projects' clones; default $XDG_DATA_HOME/reef/clones
fetch_every = "5m"            # how often every remote project is fetched, in s, m or h; "0s" never
hosts = ["github.com"]        # the hosts a remote may be cloned from; default any
```

Only one process can open a data directory. `Service::shared` opens the process's service once,
however often it is asked, since `dioxus::serve` reruns its setup after every hot-patch. Stopping an
in-process pool cleanly takes up to 3 s.

An external node is `reef-shoal`, over a shoal config of its own (`reef-shoal/shoal.example.yml`).
Its data paths must be absolute, and never on tmpfs.

Before shoal starts, either way, reef checks its config and names the fix for each problem. It
refuses a memory budget of 0, a relative or tmpfs data path, and a locked-memory limit under
512 KiB. It warns on a limit under 30 MiB for each shard and 30 more, below which glommio runs
without registered io buffers (`ulimit -l unlimited` lifts it).

```sh
cargo +nightly run --release -p reef-shoal -- serve --conf shoal.yml
```
