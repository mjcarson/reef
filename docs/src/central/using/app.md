# The app

Open the server's root in a browser and sign in. The sign-in lasts 30 days on that browser.

- **The front page** lists every project and its plans.
- **A plan's page** draws the plan, and follows changes live. Click a box to open its panel:
  - its YAML, which you can edit and save, checked as a put is;
  - **Changes**: the node's captured revisions and their diffs;
  - **Comments**: its threads; click a diff line's number to start one there;
  - **Decisions**: every plan its Claude sessions asked approval for, and every question they
    asked, with what was answered.
- **The sessions panel**, from the bar, shows headless Claude sessions that `reef loop run`
  started, and answers their questions and plans ([With Claude Code](claude-code.md)).
- **Your name in the bar** opens your account: your password and your tokens, and, for an
  admin, every user.
