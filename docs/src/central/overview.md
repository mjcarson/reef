# What central reef is

Central reef keeps plans on a server instead of in a book's markdown. A team shares one server:
every project's plans live there, every write is checked against the whole plan, and the CLI,
Claude Code and a browser all read and write the same nodes.

This part of the book is for the people who run it and the people who use it. The service's
internals, its JSON API and its store are in [Internals](../internals/central.md).

| Chapter | For | What it covers |
|---|---|---|
| [Deploying](deploy/index.md) | whoever runs the server | building it, its config, TLS, a systemd unit, the first admin |
| [Projects and plans](projects/index.md) | anyone who starts a plan | registering a repo, making a plan, writing or importing its nodes |
| [Using it](using/index.md) | everyone | the CLI from another host, signing in, the app, Claude Code |
| [Administering](admin/index.md) | admins | users and tokens, upgrades, backups, what it can't do yet |

## How it fits together

```
 a user's host                                     the server's host
┌──────────────────────────┐                      ┌─────────────────────────────────────┐
│ reef (CLI)               │                      │ reef-app  or  reef-serve            │
│ reef mcp  (Claude Code)  │── HTTPS, a token ───▶│   [api.tls]  e.g. 0.0.0.0:12443     │
│ reef hook (git, Claude)  │                      │   [api]      127.0.0.1:12180, HTTPS │
│ browser                  │── HTTPS, a cookie ──▶│        │                            │
│ a checkout of the repo   │                      │        ▼                            │
└──────────────────────────┘                      │  shoal, in process or a node        │
                                                  │  a clone of each remote project     │
                                                  └─────────────────────────────────────┘
```

- **`reef-app`** serves the web app and the JSON API on one port. Run it when people will use a
  browser, which is usually.
- **`reef-serve`** serves the JSON API alone. It also has `reef-serve recover`, which restores
  a lost admin, so install it beside reef-app as well.
- **`reef-shoal`** is an external store node. Leave it out unless you want shoal in a process of
  its own: by default the server runs shoal itself.
- **`reef`** is the CLI. It is also Claude Code's MCP server (`reef mcp`) and its hooks
  (`reef hook …`), and every user installs it.
- **Only one process opens a data directory,** so run reef-app or reef-serve, never both over
  the same data.

## Two ways to run it

- **One person on one machine:** reef-serve as a systemd user unit on loopback, over HTTPS with
  a certificate of its own, with no config and no sign-in. This is how reef's own plan runs. See
  [One person on localhost](deploy/localhost.md).
- **A team server:** reef-app as a system service under an account of its own, serving other
  hosts too, with sign-in required and projects registered by their git URLs. [Deploying](deploy/index.md)
  walks through it.

## Where things are

| | Default | Set by |
|---|---|---|
| Config | `$XDG_CONFIG_HOME/reef/config.toml`, else `~/.config/reef/config.toml` | `--conf` (reef-serve, the CLI), `$REEF_CONFIG` |
| Loopback port, HTTPS | `127.0.0.1:12180` | `[api] address` |
| Port for other hosts, HTTPS | none | `[api.tls] address` |
| reef's own CA and certificate | `$XDG_DATA_HOME/reef/certs`, else `~/.local/share/reef/certs` | `[api] certs` |
| Shoal's data | `$XDG_DATA_HOME/reef/shoal`, else `~/.local/share/reef/shoal` | `[shoal] data` |
| Remote projects' clones | `$XDG_DATA_HOME/reef/clones`, else `~/.local/share/reef/clones` | `[remotes] clones` |
| A user's tokens | `$XDG_CONFIG_HOME/reef/credentials.toml` | `--credentials`, `$REEF_TOKEN`, `$REEF_CREDENTIALS` |

The server and the CLI read the same config file. The server reads all of it. The CLI reads
only `[api]`'s `address`, `url`, `ca` and `certs`, which is how it finds the server and what it
trusts. Every port speaks HTTPS, and answers plain HTTP with a redirect to it
([Certificates](deploy/certificates.md)).
