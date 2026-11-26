# Known limits

What central reef can't do yet, and the plan's node for each where there is one.

- **A history shows its newest 200 edits.** A node's, a page's or a queue's history reads no
  further back (C88).
- **A write sent with no token names nobody.** With sign-in off, anything on the host can write
  without saying who it is.
- **An author is shown by name.** A name reused after its user was removed reads as the same
  user, though the history keeps when each was added.
- **Not every act names its author.** Registering a project or a plan, a loop's or a session's
  acts, and managing users don't say who did them (C89).
- **One disk.** Shoal keeps one copy of the data, so a lost disk is a restore from backup
  (C52).
- **No push webhook.** A pushed doc is seen at the next fetch, or after `reef project fetch`
  (C78).
- **Projects can't be removed** (C79), nor can a remote project's clone be cleaned up.
- **Plans can't be removed or renamed.** Empty one with `reef node rm` instead.
- **A project follows one branch.** Docs on other branches can't be linked until they reach it.
- **No reverse proxy.** reef reads no forwarded headers
  ([What not to do](deploy/dont.md)).
- **The loopback port trusts its host.** With sign-in off, anything on the host reaches it. A
  web page that DNS rebinding points at 127.0.0.1 is only redirected to `https://` at its own
  name, which reef's certificate doesn't cover (C80).
- **Docs in LFS or submodules.** A doc kept in LFS reads as its pointer, and a submodule's
  files aren't read.
- **The sign-in throttle forgets** on a restart.
