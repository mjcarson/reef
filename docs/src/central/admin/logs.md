# Logs and health

```sh
journalctl -u reef -b              # where it listens, sign-in, and any warning about its config
curl https://reef.example.com:12443/api/v1/me      # {"required":true}: up, and asking callers to sign in
```

`/api/v1/me` answers without signing in, so it can serve as a health check.

As it starts, the server checks its config and refuses or warns, naming the fix, for:
- a memory budget of 0;
- a data path that is relative or on tmpfs;
- a certificate it can't read;
- a locked-memory limit too low for its shards.
