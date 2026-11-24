# central: reef's store and service over shoal

Centralized reef keeps projects, plans and nodes in shoal instead of in a book's markdown. This
workspace is the only part of reef that depends on shoal, so it is a workspace of its own, which
reef's stable root workspace excludes.

```
reef-service   the service and its store: every write checked against the whole plan, reef's
               shoal schema, and the only code that touches shoal; the JSON API's router, and
               `reef-serve`, which serves it alone
reef-shoal     the external node: shoal's node program over reef's schema
reef-app       the UI, a Dioxus 0.7 fullstack app on axum, which serves the JSON API beside its
               pages
```

Its docs are in reef's book:

- [The central workspace](../docs/src/internals/central.md): building and testing it, and the
  `reef` CLI's commands
- [The service](../docs/src/internals/service.md), [the JSON API](../docs/src/internals/api.md),
  [the store](../docs/src/internals/store.md) and [the schema](../docs/src/internals/schema.md)
- [reef's own plan](../docs/src/internals/own-plan.md): the user unit that keeps it, and how it
  is edited, exported and moved to a fresh data directory
- [Central reef](../docs/src/central/overview.md): deploying, using and administering a server
