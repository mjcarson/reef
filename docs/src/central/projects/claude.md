# Setting a repo up for Claude Code

From the repo's root:

```sh
reef claude install
git add .mcp.json .claude/settings.json .claude/skills/reef
git commit -m "Set the repo up for reef"
```

It writes:
- reef's MCP server in `.mcp.json`;
- Claude Code's hooks in `.claude/settings.json`;
- the skill that teaches Claude to keep the plan in step with the work, in
  `.claude/skills/reef`;
- git's post-commit and post-rewrite hooks;
- `.claude/settings.local.json`, which enables the server and allows its tools.

The last two stay out of git, so each user runs `reef claude install` once in their own clone.
Running it again changes nothing. See [With Claude Code](../using/claude-code.md).
