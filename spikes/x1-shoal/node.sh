#!/usr/bin/env bash
# Starts X1's external node on 127.0.0.1:12100, with its data in the spike's `data/node`.
#
# shoal's data paths must be absolute (a relative one fails opening the directory with EBADF),
# so the config is written out with this checkout's path first.
set -euo pipefail
# the spike's directory, however this was started
here="$(cd "$(dirname "$0")" && pwd)"
# the node's data directory, on the same disk as the checkout
mkdir -p "$here/data/node"
# the config, with the data path filled in
sed "s|@DATA@|$here/data/node|g" "$here/node/shoal.yml" > "$here/data/shoal.yml"
# shoal's engine needs nightly
exec cargo +nightly run -q --manifest-path "$here/Cargo.toml" -p x1-node -- serve --conf "$here/data/shoal.yml"
