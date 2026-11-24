# Making a plan

```sh
reef plan new launch               # an empty plan in the working directory's project
reef plan ls                       # each plan and the revision it is at
```

- **A project with one plan needs no `--plan`.** Once it has several, commands that read or
  write one plan take `--plan launch`. `reef next` looks across every plan unless `--plan`
  names one, and the project's queue holds every plan's entries.
- **A plan's name** keeps to the same rule as a project's and a user's.
