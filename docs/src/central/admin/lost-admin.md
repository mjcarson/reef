# A lost admin

If no admin can sign in, recover one on the server itself, from the store. Never do this over
the API. The service holds its data directory, so stop it first:

```sh
sudo systemctl stop reef
sudo -u reef reef-serve recover ada --conf /etc/reef/config.toml   # a new password, asked twice
sudo systemctl start reef
```

- **What it does:** it sets ada's password, makes ada an admin and revokes ada's tokens. If no
  user has the name, it adds ada as an admin.
- **`--password-file PATH`** reads the password from a file instead.
- **It runs as `reef`,** the account that owns the data.
- **A running service refuses it,** saying the store is locked.
- **With an [external shoal node](../deploy/external-shoal.md)**, it runs while the
  service does. The service then trusts a revoked token for up to a minute.
