#!/usr/bin/env bash
# Drives X3's live runs against the repo setup.sh wrote.
#
#   drive.sh run <worktree> <node> <task> <out> [order]
#                                one headless session on one node, in its worktree; order is
#                                `either` (the default) or `done-first`, this repo's own
#   drive.sh run-w <repo> <node> <task> <out>
#                                one headless session started with `claude -w <node>`, whose
#                                worktree reef's WorktreeCreate hook makes
#   drive.sh squash <repo> <state> <node>...
#                                squash nodes' branches into main and prune everything git had
#                                of them
#
# Each session runs under `env -i`, so this session's own CLAUDE_* variables, hooks and pane
# don't leak into it, as X4 found they would.
set -euo pipefail

bin=$(realpath "$(dirname "$0")/target/release/x3-capture")

# the session's instructions: the node's task, then how to finish it
prompt() {
  local node=$1 task=$2 order=${3:-either}
  local finish="When the work is finished, mark node $node done with the reef MCP server's \
status tool, and commit your work with git."
  if [ "$order" = done-first ]; then
    finish="When the work is finished, first mark node $node done with the reef MCP server's \
status tool, then commit your work with git, in that order."
  fi
  echo "You are working on node $node of this repo's plan. $task Keep it small: one new \
module, no tests, nothing else changed. $finish Do not ask any questions."
}

# a headless session with a clean environment and only the tools a node needs
session() {
  env -i HOME="$HOME" PATH="$PATH" USER="$USER" LANG="${LANG:-C.UTF-8}" \
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
    GIT_AUTHOR_NAME=x3 GIT_AUTHOR_EMAIL=x3@localhost \
    GIT_COMMITTER_NAME=x3 GIT_COMMITTER_EMAIL=x3@localhost \
    claude -p --output-format stream-json --verbose \
    --allowedTools "Read Edit Write Glob Grep Bash(python3:*) Bash(git add:*) Bash(git commit:*) Bash(git status:*) Bash(git diff:*) Bash(git log:*) mcp__reef__status mcp__reef__get" \
    --max-budget-usd 1 "$@"
}

case "$1" in
run)
  worktree=$(realpath "$2")
  out=$(realpath -m "$5")
  cd "$worktree"
  session --mcp-config "$worktree/.mcp.json" --strict-mcp-config "$(prompt "$3" "$4" "${6:-either}")" >"$out"
  ;;
run-w)
  repo=$(realpath "$2")
  out=$(realpath -m "$5")
  cd "$repo"
  session --mcp-config "$repo/.mcp.json" --strict-mcp-config -w "$3" "$(prompt "$3" "$4")" >"$out"
  ;;
squash)
  repo=$(realpath "$2")
  state=$(realpath "$3")
  shift 3
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
  export GIT_AUTHOR_NAME=x3 GIT_AUTHOR_EMAIL=x3@localhost
  export GIT_COMMITTER_NAME=x3 GIT_COMMITTER_EMAIL=x3@localhost
  # squash each node's branch into main, as a PR's squash would, with no hooks capturing it
  for node in "$@"; do
    worktree=$(jq -r --arg node "$node" '.work[$node].worktree' "$state/state.json")
    git -C "$repo" -c core.hooksPath=/dev/null merge -q --squash "reef/$node"
    git -C "$repo" -c core.hooksPath=/dev/null commit -q -m "$node, squashed"
    git -C "$repo" worktree remove --force "$worktree"
    git -C "$repo" branch -q -D "reef/$node"
    git -C "$repo" update-ref -d "refs/reef/start/$node"
  done
  # then everything unreachable goes
  git -C "$repo" reflog expire --expire=now --all
  git -C "$repo" gc -q --prune=now
  # every head the captures read is gone from git
  for head in $(jq -r '.captures[][] | .head // empty' "$state/state.json" | sort -u); do
    if git -C "$repo" cat-file -e "$head" 2>/dev/null; then
      echo "still in git: $head"
    else
      echo "gone from git: $head"
    fi
  done
  ;;
*)
  echo "usage: drive.sh run|squash ..." >&2
  exit 2
  ;;
esac
