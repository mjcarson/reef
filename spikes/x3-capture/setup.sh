#!/usr/bin/env bash
# Writes X3's scratch repo for the live runs: a tiny `textkit` package with no remote, the stub
# reef MCP server, Claude Code's PostToolUse and Stop hooks and git's post-commit and
# post-rewrite hooks wired in, and a worktree a node, each node started in its own.
#
#   setup.sh <repo dir> <state dir>
#
# The worktrees go beside the repo, as <repo>-a and <repo>-b. Build first:
# `cargo build --release` in spikes/x3-capture.
set -euo pipefail

repo=$(realpath -m "$1")
state=$(realpath -m "$2")
bin=$(realpath "$(dirname "$0")/target/release/x3-capture")
x3="$bin --state $state"

# a repo of its own config, so the machine's can't change what it does
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=x3 GIT_AUTHOR_EMAIL=x3@localhost
export GIT_COMMITTER_NAME=x3 GIT_COMMITTER_EMAIL=x3@localhost

# the package the stub nodes add to
mkdir -p "$repo/textkit" "$repo/.claude" "$state"
git -C "$repo" init -q -b main
printf '"""Small text helpers, one module a helper."""\n' >"$repo/textkit/__init__.py"
printf '# textkit\n\nSmall text helpers, one module a helper.\n' >"$repo/README.md"

# the stub reef server, as `reef claude install` would write it
cat >"$repo/.mcp.json" <<EOF
{
  "mcpServers": {
    "reef": { "type": "stdio", "command": "$bin", "args": ["--state", "$state", "mcp"] }
  }
}
EOF

# both Claude Code triggers, and reef making the worktree `claude -w <node>` asks for; every
# worktree gets them, since they are committed
cat >"$repo/.claude/settings.json" <<EOF
{
  "hooks": {
    "PostToolUse": [{ "matcher": "mcp__reef__.*", "hooks": [{ "type": "command", "command": "$x3 hook PostToolUse" }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "$x3 hook Stop" }] }],
    "WorktreeCreate": [{ "hooks": [{ "type": "command", "command": "$x3 hook WorktreeCreate" }] }],
    "WorktreeRemove": [{ "hooks": [{ "type": "command", "command": "$x3 hook WorktreeRemove" }] }]
  }
}
EOF
# Claude Code's own worktrees stay out of git
printf '.claude/worktrees/\n' >"$repo/.gitignore"
git -C "$repo" add -A
git -C "$repo" commit -q -m "textkit, wired for X3"

# git's triggers, in the common hooks dir every worktree runs
for event in post-commit post-rewrite; do
  printf '#!/bin/sh\nexec %s hook %s "$@"\n' "$x3" "$event" >"$repo/.git/hooks/$event"
  chmod +x "$repo/.git/hooks/$event"
done

# a worktree and a branch a node, each node started where its work happens
git -C "$repo" worktree add -q -b reef/A "$repo-a" main
git -C "$repo" worktree add -q -b reef/B "$repo-b" main
$x3 start A --dir "$repo-a"
$x3 start B --dir "$repo-b"
echo "repo:  $repo"
echo "state: $state"
