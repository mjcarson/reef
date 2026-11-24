# Projects and remotes

- **Who registers what:** any signed-in user registers a project by a remote URL. Only an admin
  registers a directory on the server's host, or a remote that is a path there (`file://` or
  bare).
- **`[remotes] hosts`** limits the hosts any remote may be cloned from. Set it on a server whose
  account can read more than its users should.
- **`[remotes] fetch_every`** sets how often every remote project is fetched: `30s`, `5m`, `1h`,
  or `0s` for only when asked. Each fetch is shallow and of one branch, and git gives up after
  5 minutes.
- **Clones are never removed,** since a project can't be yet (C79). They sit in
  `[remotes] clones`, named by project and the moment each was made.
- **Never move `[remotes] clones`.** Each project keeps its clone's path, so a moved directory
  strands them.
- **Private remotes** are reached through the `reef` account's ssh keys or git credential
  helpers ([Private repos](../projects/private-repos.md)).
