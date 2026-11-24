# The service

`Service<B: Backend>` is what the front doors call: the JSON API, and the app's server functions.
It reads and writes through a `Store`, and checks every write against the whole plan with
`mdbook-reef check`'s rules.

- **A write is refused if it adds a problem.** The plan is built as it stands and as the write
  would leave it, and a problem found after that wasn't found before comes back as
  `Checked::Refused`, with check's own message. A stored node's problems sit at its name
  (``reef/central/C11: needs `X`, which is not a node``), so a fix gets through on a plan that
  already has other problems.
- **Puts come in batches** (`Service::put`), each put naming the version its writer read. A
  group with no members yet is work that says neither why nor what, and a member alone names a
  group that isn't a node, so a group goes in with its members. A stale put refuses the whole
  batch as `Checked::Stale` before it is checked. A batch is written in place, not atomically:
  a write refused partway, which only another process could cause, is an error naming what was
  written.
- **A whole plan is checked too** (`Service::write_plan`). An import into an empty plan is
  refused if it has any problem, before a revision is reserved.
- **Docs come from the plan's pages, over the project's working tree.** A stored node's links
  resolve from its repo's root, so a link to a doc or heading that isn't there refuses the write,
  and a node can take its why and what from a doc. A plan keeps markdown pages of its own, which
  lie over the repo: a page hides the repo's file at its path. A link that leaves the repo
  (``doc `../x.md` leaves the repo``) refuses it too, and no doc is read through a symlink that
  leads out of the repo. A project registered by its git URL reads its docs from the service's
  clone at the commit last fetched instead ([Projects from remote
  repos](remotes.md)).
- **A page edit is checked as a node write is** (`Service::edit_page`): taking out a page a node
  links to, or the heading it names, or the section its why came from, is refused.
- **A link can name a node.** `node:C28`, or `node:M6#rows` at a heading of its body, in a page
  or in any of a node's markdown, opens that node in the sidebar (its anchor carries
  `data-goto`, which both viewers follow). One naming a node the plan lacks is a problem, so a
  node can't be removed while a page links it. `reef import` writes each link into a book's
  node file so, naming the node that heads the file, and `reef export` writes each back to the
  file its node lands in (C68).
- **The check runs under the plan's write lock**, through `Store::put_checked` and
  `Store::write_plan_checked`, so two writes that are fine alone can't break the plan together.
  It builds the plan off the async threads, since its docs are files.
- **`check` and `payload`** give a plan's problems and the viewer's payload. The payload's doc
  links are absolute paths under the plan's `docs/` (`reef_api::paths::docs`), where `doc_page`
  renders each doc as the page mdbook would write for it: a `<main>` holding the doc, each
  heading under mdbook's id, so the viewer's *Read here* cuts its section out unchanged. Only
  markdown is served, so an image a doc shows doesn't load there.
- **Each applied write is a change.** A project registered, a plan listed, a batch put, a page
  edited or a whole plan written goes to every subscriber of `Service::changes`, after the write's lock is let go,
  so two writers' changes to one plan can arrive in either order; each put names its node's new
  version. `Service::end_changes` closes every subscriber, as a server does before it stops.

- **A node's work record comes from its checkout** (`Service::start_work`). `reef start` reads
  the worktree it runs in through gix, writes its worktree as a tree through git's CLI, pins the
  start under `refs/reef/start/<plan>/<id>`, and sends what it read. The service keeps it, a
  node only if it is in its plan, and reads no checkout itself. A capture's SHAs
  (`Service::add_capture`) join the start whose pinned commit they name.
- **A node's changes come from its checkout too.** `reef node capture` diffs the node's start,
  carried over what its base branch brought in, to its worktree as a seal takes it: tracked
  files as they are, untracked ones only where the base has them. gix takes every step but the
  snapshot. The service keeps each capture as the node's next revision (`Service::add_capture`),
  unless its latest already holds it, and tells the change stream of it (`revision-captured`).
- **Comments are threads on a node's work** (`Service::start_thread`): on a line of one of its
  revisions, which must hold that line, or on the node as a whole. Replies follow in a thread
  (`Service::reply`), and a thread is marked open or resolved over the version its writer read
  (`Service::mark_thread`). A comment's body is markdown, read back rendered with raw HTML
  escaped, and each comment and mark is a change (`comment-added`, `thread-marked`).
- **A line's thread follows its line** (`Store::follow_threads`). Each capture follows the
  node's threads to its latest revision before it is told, by comparing each two revisions'
  stored hunks, and a thread keeps where its line went in each. One whose line a revision
  deleted, edited or no longer shows is outdated from it on. The threads a capture moved are a
  change of their own (`threads-followed`).
- **A change resolves a thread** (`Store::resolve_by_trailers`). `reef node capture` reads the
  `Reef-Resolves: <n>` trailers on the node's commits past its start and the base branch, and
  each thread of the plan one names is resolved and linked to the revision the capture kept, or
  the latest it matched. A commit's change, by its author, author date and summary, resolves a
  thread once, so a rebase never resolves one the user reopened. `reef comment resolve <n>`
  links a revision by hand (`Service::resolve_thread`), over the version it read. Each thread
  written is `thread-marked`.
- **A terminal loop is kept by its pane** (`Service::edit_loop`): zellij's or tmux's pane, else
  the Claude Code process, `CLAUDE_PID`, which every `/clear` keeps. A loop feeds a registered
  project, or one listed plan of it, holds one node at a time, which no other loop may take,
  and keeps the node whose end it acted on and whether it is paused. Each session that starts
  in its pane takes it over, so a clear's new session keeps it. Each loop written is
  `loop-changed`, to its project.

The store's own `put` and `write_plan` are unchecked; nothing outside the service and the tests
writes through them.
