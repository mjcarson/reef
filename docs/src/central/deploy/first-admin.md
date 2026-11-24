# The first admin

Add the first admin right away. Until a user exists, anyone with a shell on the server can add
the first one, who is an admin.

On the server, as yourself:

```sh
reef user add ada                         # over loopback; asks for a password twice
reef login --user ada                     # keeps a token in ~/.config/reef/credentials.toml
reef whoami
```

- **Only the first user comes from loopback without signing in.** After that only an admin adds
  users, from any host ([Users](../admin/users.md#users)).
- **The CLI on the server needs no config** while the plain port is the default `12180`. If you
  moved it, run it with `--conf /etc/reef/config.toml`, or set `REEF_CONFIG`.

## Open it to other hosts

Let the TLS port through the firewall, and nothing else. With ufw, for instance:

```sh
sudo ufw allow 12443/tcp
```

Then check it from another host:

```sh
curl --cacert fullchain.pem https://reef.example.com:12443/api/v1/me     # {"required":true}
```

In a browser, `https://reef.example.com:12443/` sends you to `/sign-in`, then to the projects
once you have signed in. Next:

- [Add a project and a plan](../projects/index.md).
- [Set up each user's CLI](../using/index.md).
