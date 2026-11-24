# Pages

```sh
reef page put design/checkout.md checkout.md   # the page's path, then the markdown to put
reef page ls
reef page get design/checkout.md               # its first line is `<!-- version: N -->`
reef page rm design/checkout.md                # refused while a node's doc link names it
```

A page is checked as a node write is: taking out a page, or a heading, that a node links to is
refused. A node links a page by its path, as it would a file of the repo.
