# Private repos

Git runs as the service's account, `reef` in [Deploying](../deploy/index.md), and never prompts. Give
that account read access:

```sh
sudo -u reef install -d -m 0700 /var/lib/reef/.ssh
sudo -u reef ssh-keygen -t ed25519 -N '' -f /var/lib/reef/.ssh/id_ed25519
sudo cat /var/lib/reef/.ssh/id_ed25519.pub     # add it to the repo as a read-only deploy key
sudo -u reef ssh -T git@github.com             # accept the host's key once
```

A deploy key reaches one repo. For many repos, give the key to a machine user that can read
them all. Over https, use a credential helper in the `reef` account's git config instead.
