# Upgrades

A new version is a rebuild, a reinstall and a restart. Back up first ([Backups](backups.md)), so
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

- **Tell users to upgrade their `reef`.** A CLI newer than the server says the server has no
  route for what it asked.
- **When the skill changed,** users run `reef claude install` again in each repo. It changes
  only what is out of date, and the skill it writes names the tools of the `reef` that wrote
  it. Commit the changed skill.
