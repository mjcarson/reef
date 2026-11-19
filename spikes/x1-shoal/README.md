# X1: shoal in reef's build and processes

A scratch Dioxus 0.7.10 fullstack server that runs shoal in-process by default, and connects to an
external shoal node when one is configured. It answers `GET /x1/roundtrip`, a plain JSON route on
the app's own port, and its page makes the same round trip through a server function. Each round
trip reads the note the last one left (`prior`), writes its own, and reads it back.

It needs shoal's `ScanSupport` checkout at `../../../shoal` and glommio at `../../../glommio`,
beside reef. It is a workspace of its own, which reef's root workspace excludes.

## Running it

`.cargo/config.toml` sets `-Ctarget-cpu=native` for the host alone. **Unset an exported
`RUSTFLAGS` first:** dx passes it to the wasm client too, where it breaks wasm-bindgen.

```sh
unset RUSTFLAGS
cd app

# in-process, the default: nightly, with shoal's engine in the server build only
dx serve @server --platform server --features embedded

# external: start the node, then the same build with its address
../node.sh &
X1_SHOAL_ADDR=127.0.0.1:12100 dx serve @server --platform server --features embedded

# external-only: no engine, so it builds on stable
X1_SHOAL_ADDR=127.0.0.1:12100 RUSTUP_TOOLCHAIN=stable dx serve
```

`embedded` can't be a default feature, because dx keeps default features on in the wasm build too.

## Timing restarts

`./restart.sh in-process|external|external-only [rounds]` runs `dx serve` in a process group of its
own. Each round it changes `BUILD` in `app/src/main.rs` and times until a round trip answers from
the new build.

| Mode | `dx serve` restart, edit to a round trip (median, lo, hi of five) |
|---|---|
| In-process | 1.76 s (1.74 to 1.77) |
| External node, same build | 1.76 s (1.73 to 1.77) |
| External-only, stable | 1.49 s (1.29 to 1.52) |

The in-process pool is ready 35 to 45 ms after it starts. That is lost in dx's rebuild and relaunch.
Measured on a 32-thread machine with dx 0.7.10 and nightly 1.100 (stable 1.98 for external-only),
in debug builds without debuginfo.

## What it found

The plan records these in `plan/src/design/architecture.md`, under *What X1 found*.

- **Data survives restarts.** A note survives dx killing its server, whether by SIGKILL or SIGTERM.
  It also survives the node stopping and starting again. The data directory's lock is a file lock,
  so it is released when the process ends.
- **Killing dx alone orphans its server.** The orphan keeps the lock, and the next server fails
  with `StorageDirectoryLocked`.
- **`dioxus::serve` runs its closure again after a hot-patch,** so the store opens only once.
  `--hot-patch` itself failed to build the wasm client ("exceptions proposal not enabled").
- **Shoal's data paths must be absolute.** A relative one fails with EBADF, and `SHOAL_*`
  variables can't set a nested key. That is why `node.sh` writes the node's config out first.
- **A get that finds nothing is an error.** It comes back as `QueryDidNotSucceed`, shoal's known
  issue 55.
