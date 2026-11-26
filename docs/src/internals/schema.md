# The schema

| Table | Partition | Sort | Holds |
|---|---|---|---|
| `ProjectRow` | the constant `"projects"` | name | the repo's path |
| `PlanRow` | project | plan name | `current`, `pending` and `retired` revisions, the pages' `pages_current` and `pages_retired`, and a `version` |
| `NodeRow` | (project, plan, revision) | id | a `version`, status, group, written order, the spec as JSON, the body |
| `PageRow` | (project, plan, revision) | path | a `version`, and the page's markdown |
| `QueueRow` | project | rank | the plan and id of the node queued, and the `seq` it was written at |
| `WorkRow` | project | `plan/id` | a `version`, and the node's work record as JSON: its worktree, branch, base branch, start, start tree, fork and pin, who started it, and each capture's SHAs and who sent it |
| `RevisionRow` | (project, `plan/id`) | number, padded | a `version`, and the revision as JSON: its SHAs, conflicts, files without their hunks, and who sent its capture |
| `RevisionFileRow` | (project, `plan/id`, number) | path | the file's diff as JSON, its hunks and lines |
| `CommentRow` | (project, plan) | number, padded | a `version`, and the comment as JSON: its node, time, author and body, and a thread's target, trail, state, resolutions and marks, a reply's thread, or a review's state and who marked it |
| `LoopRow` | the constant `"loops"` | the pane's key | a `version`, and the loop as JSON: its project, plan, session, node, the node it acted on, its state and when it started |
| `DecisionRow` | (project, `plan/id`) | number, padded | a `version`, and the decision as JSON: its call, session, where and when it was asked and answered, what it asked, what came of it, and whose session asked it |
| `UserRow` | the constant `"users"` | name | a `version`, and the user as JSON: their password's argon2id hash, whether they are an admin, and when they were added |
| `TokenRow` | the constant `"tokens"` | the token's SHA-256 | a `version`, and what it signs in as JSON: its user, kind, label, and when it was issued and expires |
| `RemoteRow` | the constant `"remotes"` | the project's name | a `version`, and the remote as JSON: its URL, branch, clone's path and the commit last fetched |
| `EditRow` | (project, subject: `node/<plan>/<id>`, `page/<plan>/<path>` or `queue`) | `u64::MAX` less the edit's stamp, padded, then the store's random number | the edit as JSON: who made it, when, the version it left and what it changed |

`schema::tests::fingerprint_is_pinned` pins it. Shoal can't migrate a table yet, so a change to a
row needs a fresh data directory, and then a new pin.
