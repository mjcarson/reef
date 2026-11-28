# Users and tokens

## Users

The first user, added from the server's own host, is an admin. After that only an admin adds
users, from any host, and only once they have signed in.

```sh
reef user add bob                  # asks for bob's first password twice; --password-file PATH to read it
reef user add carol --admin        # an admin, who can add and remove users
reef user ls                       # every user, admins marked, and who added each
reef user passwd bob               # set bob's password, which revokes his tokens
reef user rm bob                   # remove bob and revoke his tokens; never the last admin
reef user log                      # who added or removed each user, set a password or revoked a token
```

- **A user's admin flag is set when they are added.** Otherwise, only `reef-serve recover`
  makes an existing user an admin ([A lost admin](lost-admin.md)).
- **Users change their own passwords** with `reef user passwd`, giving their current one, which
  keeps their tokens.
- **The account page** in the app, opened from your name in the bar, does the same for an
  admin: every user, who added them, and whose password you set or whom you remove.
- **Every act on a user is logged,** for admins alone: adding and removing users, setting a
  password, revoking a token, and a recovery on the server, which names nobody. `reef user log`
  prints it, newest first, 200 at a time (`--older`, `--all`), and the account page shows its
  newest 200. Signing in and out isn't logged.

## Tokens

Every login is a token. A terminal's lasts until it is revoked. A browser's lasts 30 days, or
until it signs out.

```sh
reef token ls --user bob           # bob's tokens, by id, with their labels and when they were issued
reef token revoke 3f9c0a1b2d4e5f60 # any user's token, by its id
```

A revoked token stops at once, and so does any change stream it opened. Removing a user, or
setting their password, revokes all their tokens.

## Failed sign-ins

Repeated wrong passwords are slowed, at the token route, at the browser's sign-in form and for
a user's own password change:

- **The limits:** a name may fail 5 times, and an address 20, an IPv6 address counted by its
  /64. After that each try waits, starting at a second and doubling up to 5 minutes. It is
  answered 429, with `Retry-After`, before its password is checked.
- **A success clears the name,** and failures are forgotten an hour after the last.
- **It is kept in memory,** so a restart forgets it.
