# Signing in and serving other hosts

## Signing in

Sign-in is off unless reef's config asks for it with `[auth] required = true`, as reef's own unit
runs with no config at all. The commands that add the first admin, sign in, and manage users and
tokens are in [The first admin](../central/deploy/first-admin.md), [Sign
in](../central/using/sign-in.md) and [Users](../central/admin/users.md#users).

- **A password is asked for on the terminal** with echo off, never taken as an argument, or read
  from `--password-file PATH` (`-` for standard input), through `reef_api::prompt`, which the
  CLI and reef-serve share. The service keeps only its argon2id hash.
- **A login keeps a token, never the password,** in `$XDG_CONFIG_HOME/reef/credentials.toml`,
  readable by its owner alone, a token for each server by its root (`http://127.0.0.1:12180`,
  or the `https://` one `[api] url` names).
  Every request reads it again, so the MCP server, the hooks and a runner started before the
  login send it too. `--credentials PATH` names another file, `$REEF_TOKEN` gives a token
  outright, and `$REEF_CREDENTIALS` names a file, in that order, before the default.
- **A browser signs in at `/sign-in`,** which sets an HttpOnly, SameSite=Strict cookie for 30
  days, `Secure` when it signs in over TLS. The app's bar names whom it is signed in as, beside a
  Sign out, and the name opens `/account`: your password, your tokens, each revoked from there,
  and, for an admin, every user, whose password they set or whom they remove.
- **Repeated wrong passwords are slowed** (`service/throttle.rs`), in memory, by name and by
  address, each try counted as it is let through and a success taking it back. The limits are in
  [Failed sign-ins](../central/admin/users.md#failed-sign-ins).
- **A lost admin is recovered on the service's host,** from the store itself, never over the
  API: `reef-serve recover` opens the store and calls `Service::recover_admin`, so it runs only
  while the service is stopped, unless shoal is a node of its own. The steps are in [A lost
  admin](../central/admin/lost-admin.md).
- **A refused `reef` says how to sign in**, naming the service's address and the file its token
  came from.

## Serving other hosts

The loopback port stays plain HTTP, for the service's own host. Other hosts reach the JSON API,
and the app, over TLS on a second listener, `[api.tls]`, which sign-in must be required for. The
first admin is added on the service's host, over loopback. A `reef` on another host names the
service by `[api] url`, which wins over `address`, and trusts `[api] ca` beside the system's
roots and `$SSL_CERT_FILE`'s. The configs for both sides are in
[Configure](../central/deploy/config.md) and [Point it at the
server](../central/using/index.md#point-it-at-the-server).

- **The certificate is read as the server starts,** so a renewed one is served from the next
  restart. A chain or key that can't be read, or a key that isn't the certificate's, stops it,
  naming the key that set the file.
- **A `reef` refusing the certificate says so,** with every cause, rather than that the service
  isn't running.
- **A self-signed certificate must be a leaf.** `openssl req -x509` makes a CA unless told
  otherwise, and rustls refuses a CA as a server's own (`CaUsedAsEndEntity`), so add
  `-addext basicConstraints=critical,CA:FALSE`, or sign it with a CA of your own and name that
  CA as `ca`.
- **Only an admin registers what is on the service's host,** once sign-in is required: a
  directory (`--repo`), or a remote that names a path on it, `file://` or bare. A user who isn't
  one is refused before the path is looked at. `[remotes] hosts` limits the hosts a remote may
  be cloned from, for every caller.
- **Known limits:** no HSTS, since the loopback port is plain HTTP on a name the browser may
  share; no reading of a reverse proxy's `X-Forwarded-*`, so a proxy in front makes every caller
  look local; and on the plain port, with sign-in off, a page whose name a DNS rebinding points at
  loopback is the same origin as the service (C80).
