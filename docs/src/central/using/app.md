# The app

Open the server's root in a browser and sign in. The sign-in lasts 30 days on that browser.

- **The front page** lists every project and its plans.
- **A plan's page** draws the plan, and follows changes live. Click a box to open its panel:
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
- **The sessions panel**, from the bar, shows each session attached to central reef, live:
  - the headless sessions `reef loop run` started, whose loops it starts, pauses and stops and
    whose questions and plans it answers;
  - a Claude Code session of your own, attached with `! reef loop run` typed in it, which it
    shows with its loop's node and its transcript, to read: no buttons, since you answer it in
    its terminal. `! reef loop detach` there, or the session's end, shows it ended.

  [With Claude Code](claude-code.md) says how to attach each.
- **Your name in the bar** opens your account: your password and your tokens, and, for an
  admin, every user.
