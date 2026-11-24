# Writing nodes

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

The keys are the ones in [Writing nodes](../../book/nodes.md):
[Why and what](../../book/why-and-what.md), [Acceptance
criteria](../../book/acceptance.md) and [Performance](../../book/performance.md).
