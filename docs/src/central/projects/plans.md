# Making a plan

```sh
reef plan new launch               # an empty plan in the working directory's project
reef plan new app --branch dev     # one that follows a branch of the repo
reef plan branch launch release    # follow another branch, unless the plan's links break there
reef plan branch launch --clear    # follow the project's own branch again
reef plan ls                       # each plan, the revision it is at, what it follows, who listed it
```

- **A project with one plan needs no `--plan`.** Once it has several, the checkout picks one:
  the plan that follows its branch, else the plan of the node its worktree is on, such as a
  group member's worktree on `reef/<id>`. Elsewhere, commands that read or write one plan take
  `--plan launch`, and the refusal lists each plan and the branch it follows. A plan is named by
  its name alone, or in full as `project/plan` in its own project.
- **`reef next` looks in the checkout's plan,** and across every plan where nothing picks one,
  or with `--all`.
- **Each plan has a queue of its own,** which its `next` follows first. An entry is placed among
  its plan's entries alone, and each plan's queue keeps its own history beside the project's.
- **A plan's name** keeps to the same rule as a project's and a user's.

## A plan follows a branch

A project can keep plans for several lines of work at once, such as `main`'s and a long-lived
`dev`'s. Each plan follows one branch of the repo, and its doc links are read and checked there.

- **A plan follows its project's own branch unless it names one:** the branch a remote project
  was registered by, or a local project's working tree as it stands.
- **A remote project's clone fetches every branch a plan follows,** beside its own, on the same
  interval and on `reef project fetch`. A push to one branch moves only the plans that follow
  it, and only their pages in the app reload. A branch deleted from the remote keeps where it
  was last fetched, and `reef project fetch` says it failed.
- **A local project's plan reads a worktree that has its branch checked out,** the main one
  first, so its uncommitted docs link as the working tree's do. With none, it reads the
  branch's last commit.
- **A branch is checked before a plan follows it,** as every write is. A branch the repo hasn't
  is refused, a remote project's clone fetches one it lacks, and a change that would break the
  plan's links is refused with the problems it would add, leaving the plan as it was.
