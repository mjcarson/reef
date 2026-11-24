# Backups

A backup is the data directory and the clones, copied while the service is stopped:

```sh
sudo systemctl stop reef
sudo tar -C /var/lib/reef -czf /srv/backup/reef-$(date +%F).tar.gz shoal clones
sudo systemctl start reef
```

To restore one, stop the service, move the current directories aside, unpack the backup in
their place, and start it again:

```sh
sudo systemctl stop reef
sudo mv /var/lib/reef/shoal /var/lib/reef/shoal.bad && sudo mv /var/lib/reef/clones /var/lib/reef/clones.bad
sudo tar -C /var/lib/reef -xzf /srv/backup/reef-2026-11-24.tar.gz
sudo systemctl start reef
```

Everything comes back as it was at the backup: plans, pages, users, tokens, the queue, work
records, comments. Anything written after it is gone. The clones' paths are kept in the data,
so restore both to the paths they were backed up from.

Exported plans, committed in their repos as [Keep an export in
git](../projects/export.md) describes, are a second copy of the nodes and
pages that needs no downtime.
