# Signing in

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
