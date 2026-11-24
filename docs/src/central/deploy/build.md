# Building and installing

## Build

From reef's checkout:

```sh
cargo build --release -p reef                     # the CLI: target/release/reef
cd central                                        # its .cargo/config.toml must apply
cargo +nightly build --release -p reef-service --features embedded --bin reef-serve
cd reef-app
env -u RUSTFLAGS RUSTUP_TOOLCHAIN=nightly \
  dx build --release @server --platform server --features embedded
```

- **Run the server builds from `central`**, not from reef's root. cargo reads
  `central/.cargo/config.toml`, which shoal's hashing needs, only from there.
- **Unset `RUSTFLAGS` for dx.** dx passes it to the wasm build too, where `-Ctarget-cpu=native`
  breaks it.
- **`--release` goes before `@server`.** After it, only the server is a release build, and the
  browser gets a debug client.
- **The app is two things:** `central/target/dx/reef-app/release/web/server`, the binary, and
  the `public/` directory beside it, the client it serves. Copy both together. The server finds
  `public/` next to itself, else at `$DIOXUS_PUBLIC_PATH`.

## Install

From reef's root again:

```sh
sudo useradd --system --home-dir /var/lib/reef --create-home --shell /usr/sbin/nologin reef
sudo install -d -o reef -g reef -m 0750 /var/lib/reef/shoal /var/lib/reef/clones
sudo install -d -m 0755 /etc/reef /opt/reef
sudo install -m 0755 target/release/reef central/target/release/reef-serve /usr/local/bin/
sudo rm -rf /opt/reef/app
sudo cp -r central/target/dx/reef-app/release/web /opt/reef/app           # server and public/
```

- **The `reef` account owns the data.** Shoal's data and the clones live under its home, and git
  runs as it, so its ssh keys reach private remotes ([Private
  repos](../projects/private-repos.md)).
- **Install reef-serve even if you run the app.** `reef-serve recover` is how a lost admin is
  restored ([A lost admin](../admin/lost-admin.md)).
- **Install the CLI on the server too.** The first admin is added from it.
