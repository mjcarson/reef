# With Claude Code

Once a repo is set up with `reef claude install`
([how](../projects/claude.md)), Claude Code in it has reef's
tools and skill:

- **Tools:** `mcp__reef__next`, `get`, `check` and `comments` only read, so plan mode runs them
  without asking. `put`, `status`, `accept`, `reply`, `resolve` and `next_task` write, with your
  token; a reply or a resolve is marked as Claude's.
- **The skill** tells Claude to draw new work as a node before starting it, and to mark it done,
  with its criteria, in the change that finishes it.
- **A terminal loop** feeds a session one node at a time. Type `! reef loop start` in the
  session. With zellij or tmux, reef clears the session and starts the next node itself.
  `--width 2` works two ready nodes at once, the second in a worktree and session of its own. `reef loop pause`,
  `reef loop stop` and `reef loop ls` manage it.
- **A headless loop** runs on your machine and is driven from the app:

  ```sh
  reef loop run                                # wait for the app to start it
  reef loop run --start --plan launch          # or start now, on one plan
  reef loop run --permission-mode plan -- --model sonnet
  ```

  Each node gets a fresh `claude -p` session in your checkout. The server only relays: it holds
  the transcript and the prompts waiting on you, and the app's sessions panel answers them.
  Ctrl-C ends it and lets go of its node.
- **The session you are in, shown in the app:** type `! reef loop run` in it. Nothing new is
  spawned and nothing changes in the terminal. The command attaches the session to central reef
  and returns at once, and a follower in the background streams its transcript to the app's
  sessions panel as it works, with its loop's node if one runs. Every prompt, question and plan
  is still answered in the terminal; the app shows them as history and can't answer them.

  ```sh
  ! reef loop run        # attach this session; typed again, it says it's attached already
  ! reef loop detach     # let go of it; the app shows it ended
  ```

  A `/clear` keeps it attached, followed into the new session. Exiting Claude Code ends it, and
  the app shows it ended within a second. `--start`, `--claude`, `--permission-mode` and
  arguments after `--` are a runner's, so they're refused here: run `reef loop run` outside
  Claude Code for a headless loop. A `!` command runs through your shell's own startup files, so
  the `reef` first on that shell's `PATH` is the one that attaches: an older one there starts a
  headless runner instead.
