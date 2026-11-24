# Writing nodes

```markdown
---
id: m23                        # unique across the book; the only required key
title: M23 The walk rebuilt    # default: the file's first heading, then the id
status: done                   # todo | in-progress | done | blocked   (default: todo)
optional: true                 # drawn dashed
group: scans                   # the group this belongs to (alias: parent)
needs: [m22, w1]               # solid arrows from each; naming a group needs all of it
helps: [o171]                  # dashed arrows: helps, does not gate
docs: [milestones.md#m23-the-walk-rebuilt-for-load, ../../features/f125.md]
collapsed: false               # a group that starts open (groups start collapsed)
description:                   # why it is needed and what finishing it will do; see below
  why: A scan's archive reads wait on each other, one page at a time.
  what: A page's archive reads go out together, in disk order.
  details: Longer markdown, shown below them.
decisions:                     # the sidebar's design choices (alias: design; default: from the docs)
  - Runs cut in disk order, issued in the merge's order.
acceptance:                    # how it will be known to be done (alias: criteria); see below
  - unit: An empty key list answers at once
performance:                   # how the work changed what was measured (alias: perf); see below
  metrics:
    - { name: "inserts a second", better: higher, before: 3523, after: [6837, 6783, 6915] }
nodes:                         # members with no file of their own
  - id: i346
    title: "Item 346: paced reads go to their device together"
    status: done
    needs: [f125]
    body: Markdown for the sidebar.
---
# M23 The walk rebuilt

The body is what the sidebar shows. If this file is in SUMMARY.md, the sidebar also links to it.
```

- Any markdown file under `src` whose frontmatter has an `id` is a node. To look in only some
  directories, set `[preprocessor.reef] roots = ["plans"]`.
- Keys reef does not know, such as an Obsidian `tags:` list, are ignored.
- Links in a body and in `docs` are relative to the file, just like ordinary markdown links. A doc
  link's label is the heading it points to.
- reef strips YAML frontmatter from every chapter, since mdbook would otherwise render it as text.
