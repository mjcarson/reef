# Setting up a book

```sh
cargo install --path crates/mdbook-reef      # the preprocessor and CLI
mdbook-reef install path/to/book             # writes reef.js + reef.css, edits book.toml
```

`install` adds `[preprocessor.reef]` and adds `reef.js` and `reef.css` to
`output.html.additional-js` and `additional-css`. `reef.js` is a single classic script with the
wasm inlined, so the book also works when opened straight from `file://`.

`install` also writes a Claude skill, `crates/mdbook-reef/assets/skill`, to
`.claude/skills/reef/` at the root of the git repository that holds the book (skip it with
`--no-skill`). It teaches coding agents to draw new work as nodes before starting it, to mark
it done and its acceptance criteria met in the change that finishes it, to record benchmark
figures in `performance`, and to run `mdbook-reef check` after every edit. After upgrading
reef, `mdbook-reef skill [book]` refreshes it.
