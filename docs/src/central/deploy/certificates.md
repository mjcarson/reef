# Certificates

The server reads its certificate only as it starts, so restart it after a renewal.

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

Hand `/etc/reef/fullchain.pem` to every user, who names it as `ca` in their config
([Point it at the server](../using/index.md#point-it-at-the-server)).

**A CA of your own.** Sign the server's certificate with it, and hand users the CA's
certificate as `ca` instead.
