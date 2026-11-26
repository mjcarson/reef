# Registering a project

## By its git URL

On a team server, register a project by its URL. The service keeps a clone of one branch of it
and reads docs from the commit it last fetched, so it needs nobody's disk:

```sh
reef project add shop --url git@github.com:acme/shop.git              # its default branch
reef project add shop --url https://github.com/acme/shop --branch dev # or a branch of your choice
reef project ls                    # shop  git@github.com:acme/shop.git (`main` at 6376987), registered by ada
reef project show                  # from a checkout: the project it belongs to
```

- **Any signed-in user can register a remote project.** With sign-in required, only an admin
  can register a remote that is a path on the server's host, `file://` or bare.
- **`[remotes] hosts`, if the config sets it,** lists the only hosts a remote may be cloned
  from.
- **A URL can't carry a user or a password,** nor be relative, nor use any transport but file,
  git, ssh, http and https.
- **Every checkout finds it.** The CLI sends the URLs its checkout's remotes fetch from, and the
  service matches them by host and path, whatever their scheme, user, port or trailing `.git`.
  So `git@github.com:acme/shop.git` and `https://github.com/acme/shop` are one project.

## Fetching

- **On an interval:** every remote project is fetched every `[remotes] fetch_every`, five
  minutes unless the config says otherwise, and once as the server starts.
- **On request:** run `reef project fetch` from a checkout, or `reef project fetch --project
  shop` from anywhere.
- **A doc link is checked against what was fetched.** Before linking a new doc, push it to the
  project's branch, then `reef project fetch`. Until then a node linking it is refused. A doc
  that isn't merged yet can be a [page](pages.md) of the plan instead.
- **The app reloads** a plan's page when its project's branch moves.

## By a directory on the server

For one person on one machine, or a repo that lives on the server's own host, register the
directory instead:

```sh
reef project add reef                         # the repo that holds the working directory
reef project add shop --repo /srv/git/shop    # or a directory named
```

The service reads its docs from that working tree as it stands, so nothing needs pushing. With
sign-in required, only an admin may do this. A checkout on another host can't find a project
registered this way, since that project has no URL to match.
