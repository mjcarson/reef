# Finding work

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
reef queue ls                      # each entry ready, waiting, in progress, blocked or done, and who queued it
reef queue log                     # who queued, moved or took off each entry, newest first, 200 at a time
```

The queue is the project's, one for all its plans, and `next` follows it before plan order. An
id elsewhere in the project is `project/plan/id`.
