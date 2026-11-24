# Keeping an export in git

The service is the plan's source of truth. A book exported from it can still live in the repo,
for review in pull requests, as a copy to import from, and as a book mdbook-reef builds:

```sh
reef export plan --plan launch               # once: a new book in an empty directory
reef export plan --plan launch --replace     # after each change: only the files whose text changed
```

The directory must be in the project's repo. Commit the book with the work it describes, and
never edit it by hand, since the next export writes over it. This repo keeps its own plan this
way, in `plan/`.
