# Remote projects

A project can be registered by its git URL rather than a directory on the service's host, for a
service its users don't share a disk with. The service keeps a clone of one branch of it, and
reads that project's docs from its clone at the commit it last fetched. A node's changes still
come from the user's own checkout, as `reef start` and `reef node capture` read them there.
How to register one, reach a private repo and fetch it is in [By its git
URL](../central/projects/register.md#by-its-git-url).

- **The clone is bare, shallow and one branch**, made by `git init --bare` in `[remotes] clones`
  and filled by the fetch every later fetch runs: the branch's tip alone, over its own ref, so a
  force-push is followed. Its directory is named by its project and the moment it was made, so a
  registration retried after a failure never meets the clone that one left. It needs git 2.29.
- **Docs are read from git's objects at the commit the project's row keeps**, through gix, never
  a working tree. Every read of one moment sees one commit, and a fetch lands for readers only
  once its commit is written. A symlink is followed while it stays in the tree, and a submodule's
  files aren't read.
- **A fetch is asked for, or on an interval.** `reef project fetch` (`POST …/fetch`) fetches one
  project; `[remotes] fetch_every`, five minutes unless set, fetches every remote project, and
  the first round runs as the server starts. A branch that moved is `project-fetched`, which the
  app's plan page reloads on.
- **A link is checked against what was fetched.** A node linking a doc that isn't pushed and
  fetched yet is refused, so push, then `reef project fetch`, before linking a new doc.
- **A clone of the repo finds the project.** Every `reef` sends the URLs its checkout's remotes
  fetch from, users and passwords left out, and the service matches them by host and path,
  whatever their scheme, user, port, trailing `/` or `.git`. A directory in a local project's repo
  on the service's host finds that project first.
- **Git runs as the service's account.** A private remote is reached through that account's ssh
  keys or credential helpers. A URL with a password, a user over http(s), or a relative path is
  refused, as is any transport but file, git, ssh, http and https, since every caller sees a
  project's URL and `ext::` would run a command. Git never prompts, and stops after 5 minutes.
- **A path on the service's host is an admin's to name,** with sign-in required, and
  `[remotes] hosts`, if set, names the only hosts a remote may be cloned from.
- **Known limits:** a clone is never removed, since a project can't be; moving `[remotes] clones`
  leaves the paths projects keep behind; a doc kept in LFS reads as its pointer; and with sign-in
  off, anyone who can reach the loopback port can have the service clone any repo its account
  can read, or read any directory with `--repo`.
