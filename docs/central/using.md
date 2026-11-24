# Using central reef

This page is for everyone who works from a server someone else runs. It covers setting up the
CLI, signing in, the everyday commands, the app, and Claude Code. To start a plan of your own,
see [Adding projects and plans](projects-and-plans.md).

## Install the CLI

From reef's checkout, on stable Rust:

```sh
cargo install --locked --path crates/reef
```

The one binary is the CLI, Claude Code's MCP server (`reef mcp`) and its hooks (`reef hook …`).
Keep it at the same version as the server. A `reef` newer than its server says the server has
no route for what it asked.

## Point it at the server

`~/.config/reef/config.toml`:

```toml
[api]
url = "https://reef.example.com:12443"          # the server's root, over TLS
ca = "/home/you/.config/reef/reef-ca.pem"       # only if the system doesn't trust its certificate
```

- **`url` wins over `address`.** `address`, plain HTTP on loopback, is only for the server's own
  host, which needs no config at the default port.
- **`ca`** is a PEM certificate to trust beside the system's roots: the server's own
  self-signed certificate, or the CA that signed it. Your admin hands it out.
- **`--conf PATH` or `$REEF_CONFIG`** names another file, such as one for a second server.
- **A certificate the CLI refuses** is reported with every cause, rather than as a server that
  isn't running.

## Sign in

```sh
reef login                         # your login name, else --user NAME; asks for the password
reef whoami                        # `ada`, at https://reef.example.com:12443
reef user passwd                   # change your password; asks for the current one first
reef token ls                      # your tokens, the one this reef sends marked
reef token revoke --others         # sign out everywhere else
reef logout                        # revoke this token and forget it
```

- **A login keeps a token, never your password,** in `~/.config/reef/credentials.toml`, readable
  only by you, a token for each server. The CLI, the MCP server and the hooks all send it, and
  read it for each request, so a Claude session started before you logged in still works.
- **A terminal's token never expires.** It lasts until you log out or revoke it, an admin sets
  your password, or you are removed. Changing your own password keeps it.
- **The password is asked for on the terminal**, with echo off, or read with `--password-file
  PATH` (`-` for standard input). It is never taken as an argument.
- **For CI**, sign in once as a user kept for it, with
  `reef login --user ci --credentials ci.toml --label ci`, then hand the job the token from that
  file as `$REEF_TOKEN`. `$REEF_CREDENTIALS` names a credentials file instead. The order is
  `--credentials`, then `$REEF_TOKEN`, then `$REEF_CREDENTIALS`, then the default file.
- **A refused `reef` says how to sign in,** naming the server and the file its token came from.

## Find your project

Run `reef` from a checkout of the repo. It finds the project from the URLs your checkout's
remotes fetch from, so any clone works, whatever its path or scheme:

```sh
reef project show                  # shop  git@github.com:acme/shop.git (`main` at 6376987)
reef plan ls
```

Anywhere else, name it with `--project shop`. Where a project has several plans, name one with
`--plan`.

## Find work

```sh
reef next                          # the queue's first, then what is ready, then what waits and on what
reef queue add L2 --top            # take L2 first; also --bottom, --before ID, --after ID
reef queue move L2 --after L5
reef queue rm L2
reef queue ls                      # each entry ready, waiting, in progress, blocked or done
```

The queue is the project's, one for all its plans, and `next` follows it before plan order. An
id elsewhere in the project is `project/plan/id`.

## Change the plan

```sh
reef node get L2 > l2.yaml         # its YAML, after a `# version:` line
reef node put l2.yaml              # checked against the whole plan; refused if stale or if it breaks it
reef node get L2 --md              # as a node file, with its body
reef status L2 in-progress         # todo | in-progress | done | blocked
reef accept L2                     # its criteria, numbered
reef accept L2 1 met --test tests/checkout.rs::paid_order_shows
reef check                         # the plan's problems
reef page get design/checkout.md   # the plan's own pages
```

Keep YAML you are editing outside the repo, so it isn't committed by mistake. See [Write
nodes](projects-and-plans.md#write-nodes) for how a node is written and checked.

## Record work

- **Start a node where its work happens:** `reef start L2` records your checkout's branch and
  start and marks the node in progress. `reef work` shows what this checkout is on.
- **Its changes are captured from your checkout,** by the git hooks `reef claude install` writes
  ([Set the repo up for Claude Code](projects-and-plans.md#set-the-repo-up-for-claude-code)),
  or by hand with `reef node capture`. Each capture is a revision of the node, which the app's
  Changes tab shows.
- **Comments are threads** on a revision's lines, or on the node:

  ```sh
  reef comment ls                  # the plan's open threads; --node L2, --all for resolved too
  reef comment reply 7 "Fixed in the next push"
  reef comment resolve 7 --reply "No change needed"
  ```

  A commit with a `Reef-Resolves: 7` trailer resolves thread 7 once it is captured.

## The app

Open the server's root in a browser and sign in. The sign-in lasts 30 days on that browser.

- **The front page** lists every project and its plans.
- **A plan's page** draws the plan, and follows changes live. Click a box to open its panel:
  - its YAML, which you can edit and save, checked as a put is;
  - **Changes**: the node's captured revisions and their diffs;
  - **Comments**: its threads; click a diff line's number to start one there;
  - **Decisions**: every plan its Claude sessions asked approval for, and every question they
    asked, with what was answered.
- **The sessions panel**, from the bar, shows headless Claude sessions that `reef loop run`
  started, and answers their questions and plans ([below](#with-claude-code)).
- **Your name in the bar** opens your account: your password and your tokens, and, for an
  admin, every user.

## With Claude Code

Once a repo is set up with `reef claude install`
([how](projects-and-plans.md#set-the-repo-up-for-claude-code)), Claude Code in it has reef's
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
