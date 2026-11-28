# The app

Open the server's root in a browser and sign in. The sign-in lasts 30 days on that browser.

- **The front page** lists every project, who registered it, how many plans it has and how many
  are active, then its plans: the active ones first, then the rest by their last update. A plan is
  active while a node of it is in progress, a session on it is live, or a test run on it builds
  or runs. Each plan shows:
  - the branch it follows and the commit last fetched of it, or "working tree" for a local
    project's plan that follows none, and who listed it;
  - its state of work: *testing* while a test run builds or runs, else *sessions* while a session
    is live, else *no sessions*;
  - how many of its nodes are done, how many went done in the last 24 hours (hover for which),
    and when it was last updated and by whom;
  - who is working on what: each node in progress with who started it, each live session with
    whose it is and its loop's node, and each test run with who started it and how it stands.

  It changes as the work does, with no reload, over one change stream for the whole page. Times
  are relative; hover over one for the exact time.
- **A plan's page** draws the plan, and follows changes live. Its bar lists the project's plans,
  each with its branch, to move between them, and picks the branch the plan follows from the
  repo's branches: a pick is checked as `reef plan branch` is, and one that would break a link
  says which. The queue beside it is the plan's own, and a fetch
  reloads it only when the branch it follows moved. Click a box to open its panel:
  - its YAML, which you can edit and save, checked as a put is;
  - **Changes**: the node's captured revisions and their diffs. Each Rust file's changes are
    grouped by what they change: a struct, enum or trait with its definition and the methods
    of its impl blocks, then its loose functions, then its other lines, such as its imports,
    each tagged **new**, **changed** or **removed** with its counts. Click a group to see its
    lines. **Classic**, at the tab's top right, draws every file as git does instead, and the
    browser keeps the pick. Other files, Rust that doesn't parse, and revisions captured before
    reef outlined files show their plain diff;
  - **Comments**: its threads; click a diff line's number to start one there;
  - **Decisions**: every plan its Claude sessions asked approval for, and every question they
    asked, with what was answered;
  - **History**: who made each edit of the node, what it changed and when, newest first, 200
    at a time: **Older edits** shows the next 200. Each start is there too.
  - **Timeline**: everything that happened to the node, newest first, in one list: its edits,
    each start, each capture, and each comment, thread mark and resolution. A capture's line
    opens it in **Changes**, and a comment's opens **Comments**. A comment pending in a review
    isn't there until the review is submitted.

  Each comment says who wrote it, "Claude, for ada" where Claude wrote it with ada's login, and
  each thread who marked it last. A revision says who captured it, a decision whose session
  asked it, and a queue entry who placed it.

  Right-click a box, or a group, for its menu: **Queue first** and **Queue last** put it at
  either end of the plan's queue, or move it there, and **Off the queue** takes a queued node
  off. **Status** sets a node's to do, in progress, blocked or done, as `reef status` does; a
  group has none, since its status is its members'. **Open** shows the node's panel at any of
  its tabs, and **Copy id** copies its id. Escape, or a click anywhere outside it, closes the
  menu.
- **A plan's docs**, from **Docs** in its bar, read as a book at
  `/plans/{project}/{plan}/docs/{path}`: the docs it names with `reef plan docs`, or else its
  root node then its pages. The chapter bar on the left lists the chapters as mdbook does. A
  section folds and unfolds with its ❱, and the one holding the open chapter starts unfolded.
  The chapter shows in mdbook's column, with links to the chapters before and after it. A
  ```` ```reef ```` fence draws as a live view of the plan, as on the plan's page. A link to
  another chapter opens it here, at the heading its anchor names, and the browser's back button
  goes back. A link naming a node opens the plan's page with that node shown. A page edited, or
  a fetch that moves the plan's branch, shows with no reload. **Graph** goes back to the plan.
- **The sessions panel**, from the bar, shows each session attached to central reef, live:
  - the headless sessions `reef loop run` started, whose loops it starts, pauses and stops and
    whose questions and plans it answers;
  - a Claude Code session of your own, attached with `! reef loop run` typed in it, which it
    shows with its loop's node and its transcript, to read: no buttons, since you answer it in
    its terminal. `! reef loop detach` there, or the session's end, shows it ended.

  Each row says who attached its session, who started its loop, and who changed the loop last,
  where that was someone else. [With Claude Code](claude-code.md) says how to attach each.
- **Your name in the bar** opens your account: your password and your tokens, and, for an
  admin, every user, who added each, and the log of acts on users.
