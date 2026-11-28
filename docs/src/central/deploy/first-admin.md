# The first admin

Every central reef asks its callers to sign in, on loopback too, so a server with no user
answers nobody but the routes that sign one in. `reefadm deploy`, and `reefadm upgrade` or
`deploy --adopt` of a server with no user, add the first admin and sign them in, unless given
`--no-admin` ([With reefadm](reefadm.md)). By hand, add the first admin right away. Until a user
exists, anyone with a shell on the server can add the first one, who is an admin.

On the server, as yourself:

```sh
reef user add ada                         # over loopback, HTTPS; asks for a password twice
reef login --user ada                     # keeps a token in ~/.config/reef/credentials.toml
reef whoami
```

- **Only the first user comes from loopback without signing in.** After that only an admin adds
  users, from any host ([Users](../admin/users.md#users)).
- **The CLI on the server reads `/etc/reef/config.toml`:** run it with `--conf`, or set
  `REEF_CONFIG`. Its `ca` names the copy of reef's own CA the CLI trusts there
  ([Configuring](config.md)), since the `reef` account's data directory is closed to you.

## Open it to other hosts

Let the `[api.tls]` port through the firewall, and nothing else. With ufw, for instance:

```sh
sudo ufw allow 12443/tcp
```

Then check it from another host:

```sh
curl --cacert fullchain.pem https://reef.example.com:12443/api/v1/me     # {"required":true}
curl --cacert reef-ca.pem https://reef.example.com:12443/api/v1/me       # with reef's own
```

In a browser, `https://reef.example.com:12443/` sends you to `/sign-in`, then to the projects
once you have signed in. Next:

- [Add a project and a plan](../projects/index.md).
- [Set up each user's CLI](../using/index.md).
