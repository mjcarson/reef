# Logs and health

On a server reefadm deployed, `reefadm status` says all of this at once and exits 1 when reef
isn't serving, and `reefadm logs` shows the journal ([With reefadm](../deploy/reefadm.md#administering)).
By hand:

```sh
journalctl -u reef -b              # where it listens, sign-in, and any warning about its config
curl https://reef.example.com:12443/api/v1/me      # {"required":true}: up, and asking callers to sign in
curl --cacert /etc/reef/reef-ca.pem https://127.0.0.1:12180/api/v1/me   # its loopback port, on the host
```

`/api/v1/me` answers without signing in, so it can serve as a health check.

As it starts, the server checks its config and refuses or warns, naming the fix, for:
- a memory budget of 0;
- a data path that is relative or on tmpfs;
- a certificate it can't read;
- a locked-memory limit too low for its shards.
