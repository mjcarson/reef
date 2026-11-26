# When the store's schema changes

Shoal can't migrate a table yet. A version that adds a table reads the old data as it is. A
version that changes an existing row needs a fresh data directory. Before upgrading, read what
changed in `central/reef-service/src/schema.rs` since the running version. reef's own history of
which changes needed one is in [reef's own plan](../../internals/own-plan.md).

Only a plan's nodes and pages carry over to a fresh data directory, through an export and an
import:

1. **Export every plan** with the old server still running, from a checkout of its project's
   repo: `reef export DIR --plan P`, or `--replace` over the book you keep in git. Note
   `reef user ls` and `reef project ls`.
2. **Stop it, and move the data aside:**
   `sudo systemctl stop reef && sudo mv /var/lib/reef/shoal /var/lib/reef/shoal.old && sudo mv
   /var/lib/reef/clones /var/lib/reef/clones.old`. Then recreate both directories, owned by
   `reef`, as [Install](../deploy/build.md#install) does.
3. **Install the new version and start it.** Add the first admin from the host, then every
   user again, each with a new password.
4. **Register each project again**, by the same URL, then `reef import DIR --plan P` each plan.

**What doesn't carry over:** users and their tokens, the queue, work records, captured
revisions, comment threads, loops, the plans and questions kept from Claude sessions, and every
node's, page's and queue's history of who edited it. Nodes keep their status and their criteria,
since those are part of each node.
