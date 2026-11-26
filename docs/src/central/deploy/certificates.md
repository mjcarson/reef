# Certificates

Every port reef serves speaks HTTPS. A browser speaks HTTP/2 only over TLS, and over HTTP/1.1 it
keeps six connections to a host across all its tabs, which two of reef's plan pages take up. The
server reads its certificates only as it starts, so restart it after a renewal.

## reef's own

With no certificate named, reef makes its own on the first start, and keeps it:

- **A CA of its own,** `ca.pem`, good for ten years, and its key, `ca-key.pem`, readable by
  the server's account alone.
- **A certificate that CA signs,** `cert.pem` and `key.pem`, for `localhost`, `127.0.0.1`,
  `::1`, the host's own name, `[api.tls] address` if it names one host, and every name in
  `[api] names`. It is good for 397 days.
- **Kept in `[api] certs`,** else `$XDG_DATA_HOME/reef/certs`, else
  `~/.local/share/reef/certs`. The server says where as it starts.
- **Made again from the same CA** when the names it must cover change, or it is within 30
  days of its end, so whatever trusts `ca.pem` keeps trusting each new one.

The loopback port always serves reef's own, so the CLI on the host calls it by name. To trust it:

- **The CLI on the host** trusts `ca.pem` from `certs` itself, when its user can read it.
  Otherwise, name a readable copy as `[api] ca`.
- **A browser** shows a warning until it trusts `ca.pem`. Import it as an authority: in
  Chrome, *Settings → Privacy and security → Security → Manage certificates → Authorities*; in
  Firefox, *Settings → Privacy & Security → Certificates → View Certificates → Authorities*.
  Or accept the warning once for the host and port.
- **Another host's CLI** names a copy of `ca.pem` as `[api] ca`
  ([Point it at the server](../using/index.md#point-it-at-the-server)).
- **Through a tunnel** to the loopback port, such as `ssh -L 11180:127.0.0.1:12180 host`, call it
  as `https://localhost:11180`: `localhost` is a name the certificate covers.

Delete `certs` to start again with a new CA, which everything has to trust afresh.

## One of your own

Name it in `[api.tls]` for other hosts, with `cert` and `key` both. The loopback port goes on
serving reef's own.

**Let's Encrypt.** Get a certificate with certbot in whatever way suits the host, then copy it
where the `reef` account can read it, after each renewal too. As a deploy hook,
`/etc/letsencrypt/renewal-hooks/deploy/reef`:

```sh
#!/bin/sh
install -m 0644 "$RENEWED_LINEAGE/fullchain.pem" /etc/reef/fullchain.pem
install -m 0640 -g reef "$RENEWED_LINEAGE/privkey.pem" /etc/reef/privkey.pem
systemctl restart reef
```

Run it once by hand for the first certificate, with
`RENEWED_LINEAGE=/etc/letsencrypt/live/reef.example.com`.

**Self-signed.** Make it a leaf. rustls refuses a CA certificate as a server's own
(`CaUsedAsEndEntity`), and `openssl req -x509` makes a CA unless it is told otherwise:

```sh
sudo openssl req -x509 -newkey rsa:3072 -nodes -days 825 \
  -keyout /etc/reef/privkey.pem -out /etc/reef/fullchain.pem \
  -subj "/CN=reef.example.com" \
  -addext "subjectAltName=DNS:reef.example.com" \
  -addext "basicConstraints=critical,CA:FALSE"
sudo chgrp reef /etc/reef/privkey.pem && sudo chmod 0640 /etc/reef/privkey.pem
```

Hand `/etc/reef/fullchain.pem` to every user, who names it as `ca` in their config. reef's own
does the same with less to do: leave `cert` and `key` out, and add the host's name to `[api]
names` if other hosts call it by one it doesn't know.

**A CA of your own.** Sign the server's certificate with it, and hand users the CA's
certificate as `ca` instead.
