#!/usr/bin/env bash
# Times `dx serve` restarts of X1's app.
#
# Each round changes `BUILD` in app/src/main.rs, then times until `/x1/roundtrip` answers from the
# new build with a note written and read back: dx's rebuild, the server's restart, and shoal's
# start (in-process) or a connect (external).
#
#   ./restart.sh in-process [rounds]
#   ./restart.sh external [rounds]        # with ./node.sh already serving on 127.0.0.1:12100
#   ./restart.sh external-only [rounds]   # the same, built on stable without shoal's engine
set -euo pipefail

# the spike's directory, however this was started
here="$(cd "$(dirname "$0")" && pwd)"
# which mode, and how many rounds
mode="${1:?usage: restart.sh in-process|external|external-only [rounds]}"
rounds="${2:-5}"
# the port dx serves on
port="${X1_PORT:-8790}"
# where dx's output goes
log="${X1_LOG:-$here/data/dx-$mode.log}"
main="$here/app/src/main.rs"

# an exported RUSTFLAGS reaches the wasm client too, where `-Ctarget-cpu=native` drops
# reference-types and wasm-bindgen can't find its externref table; the host-only flag comes
# from .cargo/config.toml instead
unset RUSTFLAGS
# in-process and external are the same nightly binary, differing only in whether a node is
# named; external-only leaves the engine out, so it builds on stable
server=(@server --platform server --features embedded)
case "$mode" in
  in-process) unset X1_SHOAL_ADDR ;;
  external) export X1_SHOAL_ADDR="${X1_SHOAL_ADDR:-127.0.0.1:12100}" ;;
  external-only)
    export X1_SHOAL_ADDR="${X1_SHOAL_ADDR:-127.0.0.1:12100}"
    export RUSTUP_TOOLCHAIN=stable
    server=()
    ;;
  *) echo "unknown mode $mode" >&2; exit 2 ;;
esac

# start from build 0, and put it back however this ends
set_build() { sed -i "s/^pub const BUILD: &str = \".*\";/pub const BUILD: \&str = \"$1\";/" "$main"; }
set_build 0
mkdir -p "$(dirname "$log")"
cd "$here/app"
# in a process group of its own: killing dx alone leaves its server running, holding shoal's
# data directory locked against the next one
setsid dx serve --interactive false --open false --port "$port" "${server[@]}" > "$log" 2>&1 &
dx_pid=$!
trap 'set_build 0; kill -TERM -- "-$dx_pid" 2>/dev/null || true; wait "$dx_pid" 2>/dev/null || true' EXIT

# wait until a round trip answers from the given build, or give up after five minutes
wait_for() {
  local deadline=$((SECONDS + 300))
  until curl -sf --max-time 2 "http://127.0.0.1:$port/x1/roundtrip" 2>/dev/null | grep -q "\"build\":\"$1\""; do
    if ((SECONDS > deadline)); then echo "build $1 never answered; see $log" >&2; exit 1; fi
    sleep 0.05
  done
}

# the first build is a cold start, not a restart
wait_for 0
echo "mode=$mode: serving build 0"
curl -s "http://127.0.0.1:$port/x1/roundtrip"; echo
for round in $(seq 1 "$rounds"); do
  # change the server's code, as an edit would
  start=$(date +%s.%N)
  set_build "$round"
  # and wait for the restarted server to round-trip a note
  wait_for "$round"
  end=$(date +%s.%N)
  printf 'round %d: %.2f s\n' "$round" "$(echo "$end - $start" | bc)"
done
# what the last round trip read back, which shows data surviving each restart
curl -s "http://127.0.0.1:$port/x1/roundtrip"; echo
