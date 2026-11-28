# Upgrades

A server reefadm deployed upgrades with one command, `reefadm upgrade`
([With reefadm](../deploy/reefadm.md#upgrading)), which backs up first and puts the old release back
if the new one doesn't come up.

By hand, a new version is a rebuild, a reinstall and a restart. Back up first ([Backups](backups.md)), so
you can go back to the old binaries and their data.

From reef's checkout:

```sh
git pull                           # and shoal's and glommio's checkouts, if reef needs newer ones
# build as Deploying says, then:
sudo systemctl stop reef
sudo install -m 0755 target/release/reef central/target/release/reef-serve /usr/local/bin/
sudo rm -rf /opt/reef/app && sudo cp -r central/target/dx/reef-app/release/web /opt/reef/app
sudo systemctl start reef
```

- **Every caller signs in from this release on.** A server whose config sets
  `[auth] required = false` refuses to start, naming the key: take it out before the restart.
  `reefadm upgrade` does it for you. A server with no user answers nobody until its first admin
  is added from its host ([The first admin](../deploy/first-admin.md)), as `reefadm upgrade`
  does. Then each user runs `reef login` once, since their CLI, MCP server, hooks and runners
  send that login's token. What was written before keeps no author: nothing is backdated.
- **Users' `reef` follows by itself.** The server hands its users the `reef` its release holds,
  and every answer names that build, so a stamped `reef` replaces itself with the new one the
  next time it calls ([Keeping reef up to date](../using/updates.md)). A `reef` built with
  `cargo install` isn't stamped, so its user runs `reef update` once, or upgrades it by hand.
  By hand, the server hands out the client `[client] reef` names, if it is stamped
  ([Configuring](../deploy/config.md)): install the new one there before the restart.
- **When the skill changed,** users run `reef claude install` again in each repo. It changes
  only what is out of date, and the skill it writes names the tools of the `reef` that wrote
  it. Commit the changed skill.
