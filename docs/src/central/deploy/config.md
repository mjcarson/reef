# Configuring

`/etc/reef/config.toml`, readable by everyone, since it holds no secret and the CLI on this host
reads its `[api]`:

```toml
[shoal]
data = "/var/lib/reef/shoal"        # absolute, and never on tmpfs
cores = 2                           # shards; each wants 30 MiB of locked memory
memory = "256MiB"                   # each shard's memory budget

[api]
address = "127.0.0.1:12180"         # HTTPS, loopback only: this host's CLI and first admin
certs = "/var/lib/reef/certs"       # where reef keeps the CA and certificate it makes
ca = "/etc/reef/reef-ca.pem"        # a readable copy of that CA, for the CLI on this host

[api.tls]
address = "0.0.0.0:12443"           # other hosts, over HTTPS
cert = "/etc/reef/fullchain.pem"    # the chain, as PEM, the server's own certificate first
key = "/etc/reef/privkey.pem"       # its private key, as PEM; leave both out for reef's own

[remotes]
clones = "/var/lib/reef/clones"     # absolute; never move it once projects are registered
fetch_every = "5m"                  # how often every remote project is fetched; "0s" never
hosts = ["github.com"]              # the only hosts a remote may be cloned from; default any
```

- **A misspelt key stops the server,** naming it, rather than being ignored.
- **Every caller signs in, on loopback too.** Nothing in the config turns it off. An older
  config's `[auth] required = false` stops the server, naming the key, and `reefadm upgrade`,
  or `deploy --adopt`, takes it out ([Upgrades](../admin/upgrades.md)).
  `required = true` is read and changes nothing.
- **The server checks the config before shoal starts.** It refuses a memory budget of 0, a
  relative or tmpfs data path, and a certificate or key it can't read, and says what to change.
- **Every port speaks HTTPS.** A plain-HTTP request is answered with a redirect to `https://` at
  the same host, port and path. The loopback port serves reef's own certificate, made in
  `certs` on the first start ([Certificates](certificates.md)). `[api.tls]` serves the one it
  names, else reef's own too.
- **The loopback port stays loopback.** The server refuses to listen on any other address for
  `[api] address`. Other hosts reach it only through `[api.tls]`.
- **The CLI on this host trusts reef's own CA** from `certs`, if it can read it. The `reef`
  account's data directory is closed to other accounts, so copy `ca.pem` out once the server
  has made it, and name the copy as `ca`:
  `sudo install -m 0644 /var/lib/reef/certs/ca.pem /etc/reef/reef-ca.pem`.
- **The server hands its users the `reef` its release holds.** A reefadm release's `bin/reef` is
  static and stamped with its commit, and the server finds it beside its own binary, so a team
  server's config says nothing of it. An install made by hand names its client instead, which
  must be a stamped build ([Building and installing](build.md)); anything else is said as the
  server starts, and no client is served:

  ```toml
  [client]
  reef = "/usr/local/bin/reef"        # the stamped reef handed to users; default the release's own
  update = true                       # the CLI's alone: whether a stamped reef reading this updates itself
  ```
- **`[shoal] exclude_cores` keeps the shards off cores,** listed as physical cores, both threads of
  each, such as `[0, 1, 2, 3]`; unset, they may take any cpu but cpu 0. A machine that also
  runs shoal's benchmarks wants it, and a port off theirs: see [Beside a benchmark](benchmarks.md).
