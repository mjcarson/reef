#!/usr/bin/env bash
# Writes X4's scratch repo: a tiny `textkit` crate with the stub reef MCP server and the hooks
# wired in, and a state dir for the loop outside the repo.
#
#   setup.sh <repo dir> <state dir>
#
# Build first: `cargo build --release` in spikes/x4-loop.
set -euo pipefail

repo=$(realpath -m "$1")
state=$(realpath -m "$2")
bin=$(realpath "$(dirname "$0")/target/release/x4-loop")
x4="$bin --dir $state"

# the crate the stub nodes add functions to
mkdir -p "$(dirname "$repo")"
cargo new --lib --vcs git --name textkit "$repo" >/dev/null
mkdir -p "$repo/.claude"

# the stub reef server, as `reef claude install` would write it
cat >"$repo/.mcp.json" <<EOF
{
  "mcpServers": {
    "x4": { "type": "stdio", "command": "$bin", "args": ["--dir", "$state", "mcp"] }
  }
}
EOF

# a hook entry that runs one of the spike's hooks
hook() {
  printf '[{%s"hooks": [{"type": "command", "command": "%s hook %s"}]}]' "${2:-}" "$x4" "$1"
}
watched='"matcher": "mcp__x4__.*|EnterPlanMode|ExitPlanMode|AskUserQuestion", '

# the hooks and the status line; Stop and SessionStart drive the loop, the rest only log
cat >"$repo/.claude/settings.json" <<EOF
{
  "hooks": {
    "Stop": $(hook Stop),
    "SessionStart": $(hook SessionStart),
    "SessionEnd": $(hook SessionEnd),
    "UserPromptSubmit": $(hook UserPromptSubmit),
    "PreCompact": $(hook PreCompact),
    "PreToolUse": $(hook PreToolUse "$watched"),
    "PostToolUse": $(hook PostToolUse "$watched"),
    "PermissionRequest": $(hook PermissionRequest "$watched")
  },
  "statusLine": { "type": "command", "command": "$x4 status-line" }
}
EOF

# allow rules in the local file don't wait for folder trust; every x4 tool is allowed, so if
# plan mode still stops one, plan mode is what stopped it
cat >"$repo/.claude/settings.local.json" <<EOF
{
  "permissions": {
    "allow": ["mcp__x4", "Edit", "Write", "Bash(cargo test:*)", "Bash(cargo build:*)", "Bash(cargo check:*)"]
  },
  "enabledMcpjsonServers": ["x4"]
}
EOF

# the local settings stay out of git, as Claude Code's own setup does
printf '.claude/settings.local.json\n' >>"$repo/.gitignore"
git -C "$repo" add -A
git -C "$repo" -c user.name=x4 -c user.email=x4@localhost commit -qm "textkit, wired for X4"

# a fresh stub plan
$x4 reset --mode inject
echo "repo:  $repo"
echo "state: $state"
