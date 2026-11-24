# Adding projects and plans

- **A project is one git repo.** Its plans' doc links lead to the repo's files, from its root,
  and reef finds the project from any checkout of it.
- **A plan is one graph of work in a project.** A project can have several plans, such as
  `backend` and `app`, and `reef next` looks across them all. It also has one queue, which
  orders work across its plans.
- **A node is one piece of work.** It holds a why, a what, its needs and its acceptance
  criteria, the same keys a book's frontmatter takes (see [Writing nodes](../../README.md#writing-nodes)).
- **A plan's pages lie over its repo.** A plan can keep markdown pages of its own, such as
  design notes, and a page hides the repo's file at the same path.

Projects and plans are added with the CLI. The app lists them and edits their nodes, but it
can't add them. Run every command below from a checkout of the repo, or name the project with
`--project`.

## Register a project

### By its git URL

On a team server, register a project by its URL. The service keeps a clone of one branch of it
and reads docs from the commit it last fetched, so it needs nobody's disk:

```sh
reef project add shop --url git@github.com:acme/shop.git              # its default branch
reef project add shop --url https://github.com/acme/shop --branch dev # or a branch of your choice
reef project ls                    # shop  git@github.com:acme/shop.git (`main` at 6376987)
reef project show                  # from a checkout: the project it belongs to
```

- **Any signed-in user can register a remote project.** With sign-in required, only an admin
  can register a remote that is a path on the server's host, `file://` or bare.
- **`[remotes] hosts`, if the config sets it,** lists the only hosts a remote may be cloned
  from.
- **A URL can't carry a user or a password,** nor be relative, nor use any transport but file,
  git, ssh, http and https.
- **Every checkout finds it.** The CLI sends the URLs its checkout's remotes fetch from, and the
  service matches them by host and path, whatever their scheme, user, port or trailing `.git`.
  So `git@github.com:acme/shop.git` and `https://github.com/acme/shop` are one project.

### Private repos

Git runs as the service's account, `reef` in [Deploying](deploy.md), and never prompts. Give
that account read access:

```sh
sudo -u reef install -d -m 0700 /var/lib/reef/.ssh
sudo -u reef ssh-keygen -t ed25519 -N '' -f /var/lib/reef/.ssh/id_ed25519
sudo cat /var/lib/reef/.ssh/id_ed25519.pub     # add it to the repo as a read-only deploy key
sudo -u reef ssh -T git@github.com             # accept the host's key once
```

A deploy key reaches one repo. For many repos, give the key to a machine user that can read
them all. Over https, use a credential helper in the `reef` account's git config instead.

### Fetching

- **On an interval:** every remote project is fetched every `[remotes] fetch_every`, five
  minutes unless the config says otherwise, and once as the server starts.
- **On request:** run `reef project fetch` from a checkout, or `reef project fetch --project
  shop` from anywhere.
- **A doc link is checked against what was fetched.** Before linking a new doc, push it to the
  project's branch, then `reef project fetch`. Until then a node linking it is refused. A doc
  that isn't merged yet can be a [page](#pages) of the plan instead.
- **The app reloads** a plan's page when its project's branch moves.

### By a directory on the server

For one person on one machine, or a repo that lives on the server's own host, register the
directory instead:

```sh
reef project add reef                         # the repo that holds the working directory
reef project add shop --repo /srv/git/shop    # or a directory named
```

The service reads its docs from that working tree as it stands, so nothing needs pushing. With
sign-in required, only an admin may do this. A checkout on another host can't find a project
registered this way, since that project has no URL to match.

## Make a plan

```sh
reef plan new launch               # an empty plan in the working directory's project
reef plan ls                       # each plan and the revision it is at
```

- **A project with one plan needs no `--plan`.** Once it has several, commands that read or
  write one plan take `--plan launch`. `reef next` looks across every plan unless `--plan`
  names one, and the project's queue holds every plan's entries.
- **A plan's name** keeps to the same rule as a project's and a user's.

## Fill it

### Write nodes

Write a node's YAML to a file outside the repo, then put it. A group goes in with its members,
inline in its `nodes:`, each of which becomes a node of its own:

```yaml
# launch.yaml
id: L
title: L The launch
nodes:
  - id: L1
    title: L1 Sign-up
    description:
      why: Visitors can't make an account, so nobody can try the shop.
      what: A visitor signs up with an email and a password, and is signed in.
    acceptance:
      - integration: A new account can sign in
  - id: L2
    title: L2 Checkout
    needs: [L1]
    docs: [docs/checkout.md]
    description:
      why: A signed-in customer can fill a cart but not pay for it.
      what: A customer pays for their cart by card and gets a receipt.
    acceptance:
      - feature: A paid order shows on the customer's account
```

```sh
reef node put launch.yaml          # checked against the whole plan, or refused with why
reef check                         # the plan's problems, if any
reef next                          # L1 is ready; L2 waits on L1
```

- **Every write is checked.** A put that would add a problem, such as a need that isn't a node,
  a cycle, a doc link to a file or heading that isn't there, or work with no why or what, writes
  nothing and prints check's messages.
- **Doc links lead from the repo's root**, so `docs/checkout.md` is that file in the repo, or a
  page at that path.
- **To change a node, get it, edit it and put it back:**

  ```sh
  reef node get L2 > l2.yaml       # its first line is `# version: N`
  $EDITOR l2.yaml
  reef node put l2.yaml            # refused as stale if someone else changed it since
  ```

  YAML without a `# version:` line is a new node. A stale put writes nothing: get it again and
  redo the edit.
- **A node's body** is markdown the sidebar shows. `reef node get L2 --md` prints the node as a
  file, frontmatter then body, and `reef node put` of that file writes both. YAML alone keeps
  the body as it is.
- **Where the work stands** is set without a put: `reef status L1 in-progress`, and `reef accept
  L1 1 met --test tests/signup.rs::new_account_signs_in` for its first criterion. A group's
  status is derived from its members.
- **Removing nodes** is `reef node rm L2`. A group goes only with its members, and a node only
  once nothing needs it.

The keys are the ones in [Writing nodes](../../README.md#writing-nodes):
[Why and what](../../README.md#why-and-what), [Acceptance
criteria](../../README.md#acceptance-criteria) and [Performance](../../README.md#performance).

### Import a book

A plan already kept as an mdbook-reef book comes in whole, its nodes and its pages:

```sh
reef import docs --plan launch     # the book's directory, which holds its book.toml
```

- **It must be a reef book:** its `book.toml` has a `[preprocessor.reef]` table, which
  `mdbook-reef install` adds.
- **The book is checked first,** and a book with any problem imports nothing.
- **A plan that doesn't exist yet** is made by the import, so `plan new` isn't needed.
- **A plan that has nodes** is replaced only with `--replace`, which drops every node and page
  the book doesn't have.
- **The book's other markdown becomes the plan's pages,** at their paths from the book's `src`.
  The book can go once it is imported.
- **A mermaid flowchart** becomes a book first: `mdbook-reef import plan.md --out
  docs/src/plan --root launch` writes its boxes as node files in a book
  ([Commands](../../README.md#commands)), and `reef import` takes it from there.

### Pages

```sh
reef page put design/checkout.md checkout.md   # the page's path, then the markdown to put
reef page ls
reef page get design/checkout.md               # its first line is `<!-- version: N -->`
reef page rm design/checkout.md                # refused while a node's doc link names it
```

A page is checked as a node write is: taking out a page, or a heading, that a node links to is
refused. A node links a page by its path, as it would a file of the repo.

## Keep an export in git

The service is the plan's source of truth. A book exported from it can still live in the repo,
for review in pull requests, as a copy to import from, and as a book mdbook-reef builds:

```sh
reef export plan --plan launch               # once: a new book in an empty directory
reef export plan --plan launch --replace     # after each change: only the files whose text changed
```

The directory must be in the project's repo. Commit the book with the work it describes, and
never edit it by hand, since the next export writes over it. This repo keeps its own plan this
way, in `plan/`.

## Set the repo up for Claude Code

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
Running it again changes nothing. See [With Claude Code](using.md#with-claude-code).

## What can't be done yet

- **A project can't be removed** (C79), nor can a remote project's clone be cleaned up.
- **A plan can't be removed or renamed.** Empty it with `reef node rm` instead.
- **A project follows one branch.** Docs on other branches can't be linked until they reach it.
- **A doc kept in LFS reads as its pointer,** and a submodule's files aren't read.
