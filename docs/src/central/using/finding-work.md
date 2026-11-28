# Finding work

## Find your project

Run `reef` from a checkout of the repo. It finds the project from the URLs your checkout's
remotes fetch from, so any clone works, whatever its path or scheme:

```sh
reef project show                  # shop  git@github.com:acme/shop.git (`main` at 6376987)
reef plan ls
reef plan ls --long                # which plans have work going on, who is on what, and done today
```

Anywhere else, name it with `--project shop`. Where a project has several plans, the checkout
picks one: the plan that follows its branch, else the plan of the node its worktree is on. A
checkout that picks none, or a plan other than its own, is named with `--plan`.

## Find work

```sh
reef next                          # the queue's first, then what is ready, then what waits and on what
reef next --all                    # across every plan, whichever the checkout picks
reef queue add L2 --top            # take L2 first; also --bottom, --before ID, --after ID
reef queue move L2 --after L5
reef queue rm L2
reef queue ls                      # each entry ready, waiting, in progress, blocked or done, and who queued it
reef queue ls --all                # every plan's entries, in the project's order
reef queue log                     # who queued, moved or took off each entry, newest first, 200 at a time
```

Each plan has a queue of its own, and `next` follows the plan's before its plan order. The queue
commands work in the checkout's plan, as the node commands do: a bare id is that plan's, and
`--top`, `--bottom`, `--before` and `--after` place an entry among its own plan's entries alone,
so a node can't go beside another plan's. An id elsewhere in the project is `project/plan/id`.
Where nothing picks a plan, `queue ls` and `queue log` show every plan's, in the project's order.
