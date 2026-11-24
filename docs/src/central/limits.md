# Known limits

What central reef can't do yet, and the plan's node for each where there is one.

- **No authors.** Edits, comments and work records don't say which user made them (C50).
- **One disk.** Shoal keeps one copy of the data, so a lost disk is a restore from backup
  (C52).
- **No push webhook.** A pushed doc is seen at the next fetch, or after `reef project fetch`
  (C78).
- **Projects can't be removed** (C79), nor can a remote project's clone be cleaned up.
- **Plans can't be removed or renamed.** Empty one with `reef node rm` instead.
- **A project follows one branch.** Docs on other branches can't be linked until they reach it.
- **No reverse proxy.** reef reads no forwarded headers
  ([What not to do](deploy/dont.md)).
- **The plain port trusts its host.** With sign-in off, a web page that DNS rebinding points at
  127.0.0.1 counts as the plain port's own origin (C80).
- **Docs in LFS or submodules.** A doc kept in LFS reads as its pointer, and a submodule's
  files aren't read.
- **The sign-in throttle forgets** on a restart.
