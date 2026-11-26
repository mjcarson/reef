# Using it

This chapter is for everyone who works from a server someone else runs. It covers setting up the
CLI, signing in, the everyday commands, the app, and Claude Code. To start a plan of your own,
see [Adding projects and plans](../projects/index.md).

## Install the CLI

From reef's checkout, on stable Rust:

```sh
cargo install --locked --path crates/reef
```

The one binary is the CLI, Claude Code's MCP server (`reef mcp`) and its hooks (`reef hook …`).
Keep it at the same version as the server. A `reef` newer than its server says the server has
no route for what it asked.

## Point it at the server

`~/.config/reef/config.toml`:

```toml
[api]
url = "https://reef.example.com:12443"          # the server's root
ca = "/home/you/.config/reef/reef-ca.pem"       # only if the system doesn't trust its certificate
```

- **`url` wins over `address`.** `address` is only for the server's own host, which the CLI
  calls over HTTPS, trusting the CA the server made in `[api] certs` when it can read it.
- **`ca`** is a PEM certificate to trust beside the system's roots: reef's own CA, `ca.pem`,
  if the server makes its own certificate, else the server's self-signed certificate, or the
  CA that signed it. Your admin hands it out
  ([Certificates](../deploy/certificates.md)).
- **Through a tunnel** to the server's loopback port, such as
  `ssh -L 11180:127.0.0.1:12180 reef.example.com`, set `url = "https://localhost:11180"` and
  `ca` to a copy of the server's `ca.pem`. The CLI calls it past any proxy, as it does this
  host.
- **`--conf PATH` or `$REEF_CONFIG`** names another file, such as one for a second server.
- **A certificate the CLI refuses** is reported with every cause, rather than as a server that
  isn't running.
