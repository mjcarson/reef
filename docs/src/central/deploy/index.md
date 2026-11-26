# Deploying

This chapter sets up a team server: reef-app as a system service under an account of its own,
serving other hosts over HTTPS, with sign-in required. Running it for one person on one machine is
much shorter; see [One person on localhost](localhost.md).

The examples use `reef.example.com`, port `12443` for other hosts, the account `reef`, its config in
`/etc/reef` and its data in `/var/lib/reef`. Change any of them.

## Choose what to run

- **reef-app or reef-serve.** reef-app serves the web app and the JSON API on one port, so
  people can use a browser. reef-serve serves the JSON API alone, for teams that only use the
  CLI and Claude Code. Their config, data and behaviour are otherwise the same, and either can
  replace the other over the same data directory.
- **Shoal in process, or as a node.** By default the server runs shoal, reef's store, in its own
  process. An external `reef-shoal` node is a second unit to run, and is worth it only if you
  want to restart reef without restarting its store, or to choose the cores its shards take.
  See [An external shoal node](external-shoal.md), and [Beside a benchmark](benchmarks.md) for
  a machine that also runs one.
- **One process a data directory.** Never run reef-app and reef-serve over the same data at
  once. The second one refuses to start.

## What it needs

- **Linux with systemd**, and **git 2.29** or later on the server, which clones remote projects.
- **Rust:** stable for the CLI, and nightly for the server, whose store is linked in with the
  `embedded` feature. The app also needs the `wasm32-unknown-unknown` target.
- **dx 0.7.10**, Dioxus's CLI, to build the app: `cargo install dioxus-cli --version 0.7.10 --locked`.
- **Shoal and glommio beside reef:** the `central` workspace builds against shoal's
  `ScanSupport` checkout at `../shoal` and glommio at `../glommio`, next to reef's checkout.
- **A locked-memory limit** of at least 30 MiB for each shoal shard, plus 30 MiB. Below it,
  glommio runs without registered io buffers, and the server warns as it starts.
  `LimitMEMLOCK=infinity` in the unit takes care of it.
- **Build on the server, or on a machine with the same CPU.** The `central` workspace builds
  with `-Ctarget-cpu=native`, so its binaries may not run on an older CPU.
