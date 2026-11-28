# Making a plan

```sh
reef plan new launch               # an empty plan in the working directory's project
reef plan new app --branch dev     # one that follows a branch of the repo
reef plan branch launch release    # follow another branch, unless the plan's links break there
reef plan branch launch --clear    # follow the project's own branch again
reef plan branch launch --merged   # follow the branch its branch was merged into
reef plan ls                       # each plan, the revision it is at, what it follows, its docs, who listed it
reef plan ls --long                # and each summed up: its state, nodes done, last update, who works on what
reef plan docs launch docs/        # read a folder as the plan's docs, unless it isn't there
reef plan docs launch docs/src/SUMMARY.md   # or a summary in mdbook's format
reef plan docs launch              # what it reads, and its chapters as an outline
reef plan docs launch --clear      # name none: its book is its root node, then its pages
```

- **A project with one plan needs no `--plan`.** Once it has several, the checkout picks one:
  the plan that follows its branch, else the plan of the node its worktree is on, such as a
  group member's worktree on `reef/<id>`. Elsewhere, commands that read or write one plan take
  `--plan launch`, and the refusal lists each plan and the branch it follows. A plan is named by
  its name alone, or in full as `project/plan` in its own project.
- **`reef next` looks in the checkout's plan,** and across every plan where nothing picks one,
  or with `--all`.
- **Nodes move between plans,** a subtree of needs or a group at a time, with their work,
  comments and history ([Moving nodes](../using/changing.md#moving-nodes-to-another-plan)).
- **Each plan has a queue of its own,** which its `next` follows first. An entry is placed among
  its plan's entries alone, and each plan's queue keeps its own history beside the project's.
- **A plan's name** keeps to the same rule as a project's and a user's.
- **`--long` sums each plan up** beneath its listing, as the app's front page does. A plan is
  *active* while a node of it is in progress, a session on it is live or a test run on it builds
  or runs, else *idle*. Its state is *testing* while a run builds or runs, else *sessions* while a
  session is live, else *no sessions*. It counts its nodes done, groups left out, and those that
  went done in the last 24 hours and are done still, and names its last update and who made it.
  Then each node in progress names who started its work, and each live session and run its node
  and who attached or started it. A session or run that names no plan is its project's only
  plan's, or no plan's in a project of several.

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
- **A plan migrates with its branch.** Once a plan's branch is merged, `--merged`, run from a
  checkout, follows the branch whose history now holds the old branch's last commit: the
  project's own branch first, then the checkout's. It still works after the old branch is
  deleted, from the commit the service last fetched of it or the plan's captures. A squash or a
  rebase leaves no trace git can follow, so it says to name the branch instead. The queue and
  the loops held to the plan go with it, since they are the plan's. A plan remembers the
  branches it followed before, so a checkout still on one keeps working in it, and `next` and
  `next_task` there say where the plan went.
- **A branch is checked before a plan follows it,** as every write is. A branch the repo hasn't
  is refused, a remote project's clone fetches one it lacks, and a change that would break the
  plan's links is refused with the problems it would add, leaving the plan as it was.

## A plan's docs

A plan names the docs that say what it is for, its goals and its requirements, and reads them as
a book of chapters, which `GET …/plans/{plan}/book` answers and the app draws.

- **A folder or a summary.** A path ending `.md` is a summary in mdbook's format, and any other a
  folder; a trailing `/` is dropped, and `.` is the root. Either is read where the plan's links
  are: its pages over its repo's files, at the branch it follows.
- **A folder is read by name.** Its `README.md` or `index.md` comes first, as its own page, then
  its other markdown files by name, then its folders by name. Each folder is a section headed by
  its own page, else titled by its name, and a folder that holds no markdown is left out. Each
  chapter is titled by its first heading, else its file's name.
- **A summary orders a folder.** A folder that holds a `SUMMARY.md`, at any depth, is read by it
  as mdbook reads one: a title, prefix chapters, numbered chapters with nested lists as
  sub-chapters and `[Draft]()` drafts, `#` part titles, `---` separators and suffix chapters. Its
  numbered chapters are numbered as mdbook numbers them, running on across parts, and its links
  lead from its own folder. A summary may be a page, anywhere but the root
  ([Pages](pages.md)).
- **A plan that names none** reads its first node in written order that is in no group, then
  each of its pages by path under *Pages*, as an export's fresh summary lists them.
- **Checked as links are.** Naming docs that aren't there, or that leave the repo, is refused,
  and so is a page edit or a branch change that takes away a chapter a summary lists, the
  summary itself, or a named folder's last doc. Taking a page out of a folder just shrinks the
  book. A fetch or the working tree can't be refused, so a chapter one took away is a problem
  `reef check` lists, and the book reads it as a draft.
- **Kept in the plan's history** under `plan/<plan>`, as a branch change is, and named in `reef
  plan ls`: ``revision 4, reads its project's working tree, docs `docs/` ``.
