# The JSON API

`api::router(service)` is a plain axum 0.8 router, the version Dioxus 0.7.10 resolves, so reef-app
can mount it beside its server functions. It takes an `Arc<Service<B>>`, or the `&'static Service`
that `Service::shared` opens. Every route is one service call, and every body and answer is a
`reef-api` type; `reef_api::paths` has every path, and builders that percent-encode ids.

| Route | Answer |
|---|---|
| `GET /api/v1/projects` | every `Project` |
| `POST /api/v1/projects` `NewProject` | `AddedProject`: 201, 409 if the name or repo is taken, or 502 if its `remote` can't be fetched |
| `GET /api/v1/find-project?dir=&remote=` | the `Project` a directory belongs to, or one of its checkout's remotes was registered by, or 404 |
| `POST /api/v1/projects/{project}/fetch` | `Fetched`: 200 whether its branch moved or not, 422 for a local project, or 502 if its remote can't be fetched |
| `GET /api/v1/projects/{project}/plans` | every `PlanInfo` |
| `POST /api/v1/projects/{project}/plans` `NewPlan` | `AddedPlan`: 201, or 200 if it was listed |
| `GET …/plans/{plan}` | its `PlanInfo`, naming the revision readers load |
| `GET …/plans/{plan}/nodes` | its `Listing` |
| `POST …/plans/{plan}/nodes` `Puts` | `Checked`: 200, 409 if stale, 422 if it would add a problem |
| `GET …/plans/{plan}/nodes/{id}` | the `StoredNode` |
| `PUT …/plans/{plan}/revision` `PlanWrite` | `Rewrite`: 200, or 422 if it would add a problem |
| `GET …/plans/{plan}/payload` | the viewer's `Payload` |
| `GET …/plans/{plan}/check` | its problems, as check prints them |
| `GET …/plans/{plan}/docs/{*path}` | a doc as its HTML page (`docs/design.html` for `docs/design.md`), the plan's page or the repo's file, or 404 |
| `GET …/plans/{plan}/pages` | its own pages: a `PageListing` |
| `POST …/plans/{plan}/pages` `PageEdit` | `PageEdited`: 200, 409 if stale, 422 if it would add a problem |
| `GET …/plans/{plan}/pages/{*path}` | the `StoredPage`, or 404 |
| `GET /api/v1/projects/{project}/work` | every `WorkRecord` of its nodes, by node |
| `GET …/plans/{plan}/work/{id}` | the node's `WorkRecord`, or 404 if it wasn't started |
| `POST …/plans/{plan}/work/{id}` `StartWork` | `Started`: 200, 409 if stale, 422 if the node is not in its plan |
| `POST …/plans/{plan}/work/{id}/captures` `AddCapture` | `Captured`: 200 if kept or unchanged, the node's threads followed to its latest revision and its trailers' threads resolved, 409 if started again since, 422 if not started |
| `GET …/plans/{plan}/work/{id}/revisions` | every `Revision` of the node, oldest first, its files without their hunks |
| `GET …/plans/{plan}/work/{id}/revisions/{n}` | the `Revision`, or 404 |
| `GET …/plans/{plan}/work/{id}/revisions/{n}/files/{*path}` | the `FileDiff`, its hunks and lines, or 404 if the revision doesn't list it |
| `GET …/plans/{plan}/comments` | every `Thread` of the plan, by number, its comments rendered, a line's with where it was `followed` and the revision it went `outdated` in |
| `GET …/plans/{plan}/nodes/{id}/comments` | every `Thread` on the node, by number |
| `POST …/plans/{plan}/nodes/{id}/comments` `NewThread` | `Commented`: 200, 422 if the node is not in its plan or the line not in its revisions |
| `GET …/plans/{plan}/comments/{n}` | the `Thread`, or 404 |
| `POST …/plans/{plan}/comments/{n}/replies` `NewReply` | `Commented`: 200, or 422 if no thread starts at `n` |
| `PUT …/plans/{plan}/comments/{n}/state` `MarkThread` | `Marked`: 200, 409 if stale, 422 if no thread starts at `n` |
| `POST …/plans/{plan}/comments/{n}/resolution` `ResolveThread` | `Resolved`: 200, 409 if stale, 422 if no thread starts at `n` or the project has no such revision |
| `GET /api/v1/loops` | every `Loop`, by pane |
| `POST /api/v1/loops` `EditLoop` | `LoopEdited`: 200 if written, 409 if the pane's loop feeds another project or plan or another loop holds the node, 422 if the pane has no loop or the node isn't the loop's or in its plan |
| `GET /api/v1/projects/{project}/loops` | every `Loop` feeding the project, by pane |
| `GET /api/v1/changes?project=&plan=` | server-sent events, one a `Change` |
| `POST /api/v1/tokens` `SignIn` | `IssuedToken`: 201, 401 if no user has that name and password, the same words for either, or 429 with `Retry-After` if too many failed lately |
| `GET /api/v1/tokens?user=` | every `TokenInfo` of the caller's, the one it sent marked `current`, or of the user an admin names; 401 or 403 otherwise |
| `DELETE /api/v1/tokens/current` | `SignedOut`: the token the caller sent revoked, or 401 if it sent none |
| `DELETE /api/v1/tokens/{id}` | `SignedOut`: the token revoked, or 404 unless it is the caller's, or any to an admin |
| `GET /api/v1/me` | `Me`: the user the caller's token signs in, and whether sign-in is required |
| `GET /api/v1/users` | every `User` to an admin, none while there are none, 401 or 403 to anyone else |
| `POST /api/v1/users` `NewUser` | `UserAdded`: 201, 409 if the name is taken, 401 or 403 unless an admin asks, or the first user is added from the service's own host |
| `DELETE /api/v1/users/{name}` | `UserRemoved`: 200 with how many tokens were revoked, 404 if no user has the name, 409 if they are the last admin, 401 or 403 unless an admin asks |
| `PUT /api/v1/users/{name}/password` `PasswordChange` | `PasswordChanged`: 200, the caller's own with its `current`, or another's by an admin, which revokes their tokens; 403 if `current` doesn't match, 404 if no user has the name, 429 if too many failed lately |
| `GET /sign-in?next=` | the browser's sign-in page, plain HTML that runs no script |
| `POST /sign-in` (a form) | the sign-in cookie set and 303 to `next`, a path of this origin, or the form again with a 401, or a 429 if too many failed lately |
| `POST /sign-out` | the cookie's token revoked, the cookie cleared, and 303 to `/sign-in` |

- **A refusal is an answer**, its outcome the body whatever the status, such as
  ``{"refused":[{"source":"reef/central/C11","line":null,"message":"needs `X`, which is not a node"}]}``.
- **An error is an `ApiError`**, `{"code":…,"message":…}`: `not-registered`, `not-listed` and
  `not-found` are 404, `bad-name` and `bad-request` 400, `unauthorized` 401, `forbidden` 403,
  `too-many-tries` 429, with `retry_after` in seconds, `internal` 500, and `fetch-failed` 502, a
  remote git couldn't fetch. A `repo` a client sends
  must be an absolute directory on the service's host. A `dir` must be absolute, and is matched
  against local projects only if the service's host has it, so a checkout on another host finds
  its project by its `remote`s alone.
- **A whole plan or a page is a big body.** `PUT …/revision` and `POST …/pages` take up to
  48 MiB, past axum's default of 2 MiB, which the example book's pages alone exceed.
- **The change stream** sends each change as an event named by its kind (`project-added`,
  `project-fetched`, `plan-added`, `nodes-put`, `pages-put`, `plan-written`, `queue-changed`,
  `work-recorded`), its data the change as JSON. It is live only: a
  client reads what it needs, then follows. One that falls behind by more than 256 changes is
  sent `missed`, with how many, and should read again. `plan` narrows it within a `project`, and
  a project's own changes reach it only if they concern every plan, as `project-fetched` does.
- **One layer signs callers in.** `api::serve` wraps the router in `auth::guard`, as reef-app
  wraps its whole router. It reads a token from `Authorization: Bearer`, else the `reef_sign_in`
  cookie, and hands each route its `Caller`. With `[auth] required` off, the default, it lets
  everything through. On, everything but signing in and the app's static files needs a user: a
  page a browser opens is sent to `/sign-in?next=…`, and anything else is answered 401 with
  `WWW-Authenticate: Bearer`. A change stream opened with a token ends once the token signs
  nobody in: revoked, expired, or its user removed or given a new password by an admin.
- **A write from a page of another origin is refused,** 403, with sign-in on or off, as Go's
  `CrossOriginProtection` judges it (`origin::judge`): a `Sec-Fetch-Site` other than
  `same-origin` or `none`, or without one, an `Origin` that isn't the scheme, host and port the
  request was sent to. A request with neither, as the CLI, the MCP server and the hooks send,
  goes on, and so does every read.
- **The first user comes from the service's own host**, over loopback, and is an admin; after
  that only an admin adds users, whether sign-in is required or not, so users can be added before
  it is turned on.
- **Plain HTTP on localhost, TLS beside it.** `api::bind` refuses any address but a loopback one
  for plain HTTP, and `tls::bind` adds a listener over TLS on any address where `[api.tls]` names
  one, refused unless sign-in is required. `api::serve_router` serves one router on both, and
  each request knows its `Peer`: where it came from, and whether over TLS.

`reef-serve` serves it alone, until reef-app mounts it. It listens before it opens shoal, so a
taken port, an address that isn't local or a certificate that can't be read stops it first, and
on SIGINT or SIGTERM it ends the change stream, waits for open requests and closes the service.

```sh
cargo +nightly run --release -p reef-service --features embedded --bin reef-serve   # in-process
cargo +stable run -p reef-service --bin reef-serve -- --conf reef.toml               # external node
```

`examples/payload_bench.rs` times the example book's 202-node payload, built in process and fetched
over loopback.
