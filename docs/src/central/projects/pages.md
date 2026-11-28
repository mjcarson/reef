# Pages

```sh
reef page put design/checkout.md checkout.md   # the page's path, then the markdown to put
reef page ls
reef page get design/checkout.md               # its first line is `<!-- version: N -->`
reef page rm design/checkout.md                # refused while a node's doc link names it
```

A page is checked as a node write is: taking out a page, or a heading, that a node links to is
refused. A node links a page by its path, as it would a file of the repo.

A page may be a `SUMMARY.md` under a folder, such as `docs/SUMMARY.md`, which orders that folder
when a plan reads it as its docs ([A plan's docs](plans.md#a-plans-docs)). Only the root's
`SUMMARY.md` is refused, since an export writes its own there, and a nested one exported into a
book imports back. Taking out a chapter that the plan's summary lists, or the summary itself, is
refused, as taking out a page a node links to is.
