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
url = "https://reef.example.com:12443"          # the server's root, over TLS
ca = "/home/you/.config/reef/reef-ca.pem"       # only if the system doesn't trust its certificate
```

- **`url` wins over `address`.** `address`, plain HTTP on loopback, is only for the server's own
  host, which needs no config at the default port.
- **`ca`** is a PEM certificate to trust beside the system's roots: the server's own
  self-signed certificate, or the CA that signed it. Your admin hands it out.
- **`--conf PATH` or `$REEF_CONFIG`** names another file, such as one for a second server.
- **A certificate the CLI refuses** is reported with every cause, rather than as a server that
  isn't running.
