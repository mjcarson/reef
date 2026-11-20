#!/usr/bin/env bash
# Drives a test Claude Code session for X4's runs, standing in for the user at the keyboard.
# The session runs on a tmux server of its own (socket `x4`), with a scrubbed environment, so
# it never inherits the zellij pane or Claude session of whoever runs this script.
#
#   drive.sh start <name> <repo> [zellij]   start claude in <repo>, inside zellij if asked
#   drive.sh type <name> <text>             type a line and press Enter
#   drive.sh keys <name> <key>...           press keys (tmux key names: Enter, Down, Escape)
#   drive.sh screen <name>                  print the screen
#   drive.sh stop <name>                    end the session (and its zellij session)
#
# Watch a session read-only with `tmux -L x4 attach -r -t <name>`.
set -euo pipefail

cmd=$1
name=$2
tmux_x4() { tmux -L x4 "$@"; }

case $cmd in
start)
  repo=$3
  # only what claude, cargo and zellij need: no ZELLIJ_*, TMUX* or CLAUDE* from the caller
  clean=(env -i HOME="$HOME" USER="$USER" SHELL="$SHELL" PATH="$PATH" LANG="${LANG:-C.UTF-8}"
    TERM=xterm-256color CLAUDE_CODE_FORCE_SESSION_PERSISTENCE=1)
  if [[ ${4:-} == zellij ]]; then
    # zellij inside tmux; claude is typed into its shell, so it inherits zellij's pane vars
    "${clean[@]}" tmux -L x4 new-session -d -s "$name" -x 200 -y 55 -c "$repo" "zellij -s $name"
    sleep 3
    tmux_x4 send-keys -t "$name" -l "cd $repo && claude"
    tmux_x4 send-keys -t "$name" Enter
  else
    "${clean[@]}" tmux -L x4 new-session -d -s "$name" -x 200 -y 55 -c "$repo" "claude"
  fi
  ;;
type)
  tmux_x4 send-keys -t "$name" -l "$3"
  sleep 0.5
  tmux_x4 send-keys -t "$name" Enter
  ;;
keys)
  shift 2
  tmux_x4 send-keys -t "$name" "$@"
  ;;
screen)
  tmux_x4 capture-pane -p -t "$name"
  ;;
stop)
  tmux_x4 kill-session -t "$name" 2>/dev/null || true
  zellij delete-session --force "$name" 2>/dev/null || true
  ;;
*)
  echo "unknown command: $cmd" >&2
  exit 1
  ;;
esac
