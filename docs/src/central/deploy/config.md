# Configuring

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
