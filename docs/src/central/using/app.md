# The app

Open the server's root in a browser and sign in. The sign-in lasts 30 days on that browser.

- **The front page** lists every project and its plans, each with the branch it follows and the
  commit last fetched of it, or "working tree" for a local project's plan that follows none.
- **A plan's page** draws the plan, and follows changes live. Its bar lists the project's plans,
  each with its branch, to move between them, and picks the branch the plan follows from the
  repo's branches: a pick is checked as `reef plan branch` is, and one that would break a link
  says which. The queue beside it is the plan's own, and a fetch
  reloads it only when the branch it follows moved. Click a box to open its panel:
  - its YAML, which you can edit and save, checked as a put is;
  - **Changes**: the node's captured revisions and their diffs;
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
- **The projects' page** says who registered each project; a plan's link, who listed it.
