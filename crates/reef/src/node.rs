//! reef's node commands: a node's YAML put, got and removed through the JSON API
//!
//! ```text
//! reef node get <ID>            # print a node's YAML, after the version it was read at
//! reef node put [FILE]          # put a node's YAML, from FILE or standard input
//! reef node rm <ID>...          # remove the nodes named, as one change
//! ```
//!
//! The YAML is a node's frontmatter, with the keys a book's nodes take.
//!
//! - **The version rides in a comment.** `get` prints `# version: N` before the YAML, which
//!   YAML reads as a comment. `put` reads it back as the version its writer read, so a put made
//!   after another writer's is refused as stale. YAML with no version line is a new node, and is
//!   refused as stale if its id is taken.
//! - **A group goes in with its members.** A spec's inline `nodes:` are lifted out, each a new
//!   node of its own in the group that lists it, and put in one batch with it.
//! - **YAML only.** A put keeps a stored node's body and its place in the plan's written order.
//!   A new node goes after the plan's last.
//! - **`rm` removes exactly what it names.** A group goes only with its members named too, and
//!   a node another needs only once nothing needs it, since check refuses either alone.
//!
//! A put or removal that is stale, or that would add a problem to the plan, writes nothing and
//! fails with what was wrong: each stale node, or check's messages.

pub use reef_api::puts_for;
use reef_api::{Checked, NodeRemove, Puts, Refusal, StalePut, StoredNode, lift};
use reef_model::{NodeSpec, PlanName};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::client::Client;

/// The key of the comment `get` prints a node's version in, and `put` reads it from
const VERSION_KEY: &str = "version:";

/// Prints a node's YAML, after the version it was read at
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan the node is in
/// * `id` - The node's id
/// * `out` - Where to print it
pub async fn get<W: Write>(
    client: &Client,
    plan: &PlanName,
    id: &str,
    out: &mut W,
) -> anyhow::Result<()> {
    // the node as it stands, or the service's error naming it
    let node = client.node(plan, id).await?;
    out.write_all(printed(&node).as_bytes())?;
    Ok(())
}

/// Puts a node's YAML, its inline members with it, unless the batch is stale or breaks the plan
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan to put it in
/// * `text` - The YAML, after the version line `get` printed, if it was got
/// * `out` - Where to print each node written, at its new version
pub async fn put<W: Write>(
    client: &Client,
    plan: &PlanName,
    text: &str,
    out: &mut W,
) -> anyhow::Result<()> {
    // the version its writer read, then the spec, each member lifted out of it
    let read = read_version(text)?;
    let spec = NodeSpec::from_yaml(text)
        .map_err(|error| anyhow::anyhow!("the YAML is not a node's spec: {}", error.message))?;
    let specs = lift(spec);
    // each node keeps its place and body, and new ones go after the plan's last
    let listing = client.list(plan).await?;
    let puts = puts_for(&listing, specs, read);
    let ids: Vec<String> = puts.iter().map(|put| put.node.spec.id.clone()).collect();
    // checked against the whole plan as one change
    let checked = client.batch(plan, &Puts::new(puts)).await?;
    let versions = settled(checked, plan, "put")?;
    for (id, version) in ids.iter().zip(versions) {
        writeln!(out, "put `{id}` at version {version}")?;
    }
    Ok(())
}

/// Removes the nodes named as one change, unless one has moved on or it breaks the plan
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan to remove them from
/// * `ids` - The ids of the nodes to remove
/// * `out` - Where to print each node removed
pub async fn rm<W: Write>(
    client: &Client,
    plan: &PlanName,
    ids: &[String],
    out: &mut W,
) -> anyhow::Result<()> {
    // each node at the version it stands at, so one that moves on before the removal is kept
    let listing = client.list(plan).await?;
    let versions: HashMap<&str, u64> = listing
        .nodes
        .iter()
        .map(|node| (node.spec.id.as_str(), node.version))
        .collect();
    let mut removes = Vec::with_capacity(ids.len());
    for id in ids {
        // a node that isn't there is named in its plan before anything is sent
        let name = plan.node(id.as_str())?;
        let Some(read) = versions.get(id.as_str()) else {
            anyhow::bail!("node `{name}` is not in its plan");
        };
        removes.push(NodeRemove::new(id.clone(), *read));
    }
    // checked against the whole plan as one change
    let checked = client.batch(plan, &Puts::removing(removes)).await?;
    settled(checked, plan, "removal")?;
    for id in ids {
        writeln!(out, "removed `{id}`")?;
    }
    Ok(())
}

/// Reads the YAML to put: a file's, else standard input's
///
/// # Arguments
///
/// * `cwd` - The working directory, which a relative path is read from
/// * `file` - The file to read, or `None` or `-` for standard input
pub fn read_input(cwd: &Path, file: Option<&Path>) -> anyhow::Result<String> {
    match file {
        // a file named, read from the working directory
        Some(file) if file != Path::new("-") => {
            let path = cwd.join(file);
            std::fs::read_to_string(&path)
                .map_err(|error| anyhow::anyhow!("reading {}: {error}", path.display()))
        }
        // else whatever was piped in
        _ => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|error| anyhow::anyhow!("reading standard input: {error}"))?;
            Ok(text)
        }
    }
}

/// A node as `get` prints it: the version it was read at, as a comment, then its spec's YAML
///
/// # Arguments
///
/// * `node` - The node as it was read
///
/// # Examples
///
/// ```
/// use reef::node::printed;
/// use reef_api::StoredNode;
/// use reef_model::NodeSpec;
///
/// let spec = NodeSpec { id: "C17".into(), ..NodeSpec::default() };
/// let node = StoredNode { spec, body: String::new(), order: 0, version: 3 };
/// assert_eq!(printed(&node), "# version: 3\nid: C17\n");
/// ```
pub fn printed(node: &StoredNode) -> String {
    // the version first, as a comment YAML skips, then the canonical frontmatter
    format!("# {VERSION_KEY} {}\n{}", node.version, node.spec.to_yaml())
}

/// Reads the version a node was got at from the comments before its YAML, or `None` for a new
/// node
///
/// Only the comments and blank lines before the YAML's first key are read, and a `---` opening
/// the document is passed over.
///
/// # Arguments
///
/// * `text` - The YAML, as `get` printed it or as written by hand
///
/// # Examples
///
/// ```
/// use reef::node::read_version;
///
/// assert_eq!(read_version("# version: 3\nid: C17\n").unwrap(), Some(3));
/// assert_eq!(read_version("id: C17\n").unwrap(), None);
/// ```
pub fn read_version(text: &str) -> anyhow::Result<Option<u64>> {
    let mut version = None;
    for line in text.lines() {
        let line = line.trim();
        // blank lines and a document's opening `---` come before the YAML too
        if line.is_empty() || line == "---" {
            continue;
        }
        // the first line that isn't a comment is the YAML's own
        let Some(comment) = line.strip_prefix('#') else {
            break;
        };
        // a comment that isn't the version is the writer's own
        let Some(value) = comment.trim().strip_prefix(VERSION_KEY) else {
            continue;
        };
        // the version is a number, given once
        let value = value.trim();
        let read: u64 = value.parse().map_err(|_| {
            anyhow::anyhow!(
                "`# {VERSION_KEY}` takes the number `reef node get` printed, not `{value}`"
            )
        })?;
        if version.replace(read).is_some() {
            anyhow::bail!("the YAML has two `# {VERSION_KEY}` lines, and a put reads one");
        }
    }
    Ok(version)
}

/// The versions an applied batch wrote, or an error saying why it wrote nothing
///
/// # Arguments
///
/// * `checked` - What came of the batch
/// * `plan` - The plan it was written to
/// * `action` - What the batch was, such as `put`, for the message
pub(crate) fn settled(checked: Checked, plan: &PlanName, action: &str) -> anyhow::Result<Vec<u64>> {
    match checked {
        Checked::Applied { versions } => Ok(versions),
        // each node that wasn't as its writer read it, and what reads it again
        Checked::Stale(stale) => {
            let lines: Vec<String> = stale.iter().map(stale_line).collect();
            anyhow::bail!(
                "the {action} was refused, since nodes of `{plan}` are not as they were read:\n{}",
                lines.join("\n")
            )
        }
        // check's own messages, one a line
        Checked::Refused(problems) => {
            let lines: Vec<String> = problems
                .iter()
                .map(|problem| format!("  {problem}"))
                .collect();
            anyhow::bail!(
                "the {action} was refused, since it would add these problems to `{plan}`:\n{}",
                lines.join("\n")
            )
        }
    }
}

/// Says how one stale node differs from what its writer read, and what to do about it
///
/// # Arguments
///
/// * `stale` - The stale node, and how it differs
fn stale_line(stale: &StalePut) -> String {
    let id = &stale.id;
    // what reads it again, or puts it as new
    let hint = match stale.refusal {
        Refusal::Moved => format!("`reef node get {id}` prints it as it stands"),
        Refusal::Exists => format!(
            "to change it, put what `reef node get {id}` prints, keeping its `# {VERSION_KEY}` \
             line"
        ),
        Refusal::Missing => format!("leave out the `# {VERSION_KEY}` line to put it as new"),
    };
    format!("  `{id}`: {}: {hint}", stale.refusal)
}

#[cfg(test)]
mod tests {
    use super::{printed, puts_for, read_version, settled, stale_line};
    use reef_api::{Checked, Listing, Refusal, StalePut, StoredNode, lift};
    use reef_model::{Diagnostic, NodeSpec, PlanName};

    /// A stored node of an id, at a place and a version, with a body
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    /// * `order` - Where it falls in the plan's written order
    /// * `version` - The version it is at
    fn stored(id: &str, order: u64, version: u64) -> StoredNode {
        StoredNode {
            spec: NodeSpec {
                id: id.to_string(),
                ..NodeSpec::default()
            },
            body: format!("# {id}\n"),
            order,
            version,
        }
    }

    /// What `get` prints, `put` reads back: the version, and the same spec
    #[test]
    fn printed_node_reads_back() {
        // a node with a few keys
        let mut node = stored("C17", 4, 3);
        node.spec.title = Some("C17 Nodes put and got as YAML".to_string());
        node.spec.needs = vec!["C16".to_string(), "C4".to_string()];
        let text = printed(&node);
        assert!(text.starts_with("# version: 3\nid: C17\n"), "{text}");
        // reads back as the version it was got at and the spec it was
        assert_eq!(read_version(&text).unwrap(), Some(3));
        assert_eq!(NodeSpec::from_yaml(&text).unwrap(), node.spec);
    }

    /// Only the comments before the YAML are read, once
    #[test]
    fn version_is_read_from_the_leading_comments() {
        // none at all is a new node
        assert_eq!(read_version("id: a\n").unwrap(), None);
        // a document's opening, blank lines and other comments are passed over
        let text = "---\n\n# my note\n#version:7\nid: a\n";
        assert_eq!(read_version(text).unwrap(), Some(7));
        // a version comment after the YAML has begun is the YAML's own
        assert_eq!(read_version("id: a\n# version: 2\n").unwrap(), None);
        // a version that isn't a number, or two of them, is refused
        let error = read_version("# version: three\nid: a\n").unwrap_err();
        assert!(error.to_string().contains("not `three`"), "{error}");
        let error = read_version("# version: 1\n# version: 2\nid: a\n").unwrap_err();
        assert!(error.to_string().contains("two"), "{error}");
    }

    /// A stored node keeps its place, body and version read, and new ones go after the last
    #[test]
    fn puts_keep_stored_places_and_add_new_ones_after() {
        // a plan of two nodes, the last at place 5
        let listing = Listing {
            revision: 1,
            nodes: vec![stored("A", 1, 2), stored("G", 5, 4)],
        };
        // G got at version 4, with two new members inline
        let member = |id: &str| NodeSpec {
            id: id.to_string(),
            ..NodeSpec::default()
        };
        let group = NodeSpec {
            id: "G".to_string(),
            nodes: vec![member("m1"), member("m2")],
            ..NodeSpec::default()
        };
        let puts = puts_for(&listing, lift(group), Some(4));
        // G keeps its place and body, at the version read
        assert_eq!(puts[0].node.order, 5);
        assert_eq!(puts[0].node.body, "# G\n");
        assert_eq!(puts[0].read, Some(4));
        // each member is new, after the plan's last node, in the group that lists it
        let members: Vec<(&str, u64, Option<u64>)> = puts[1..]
            .iter()
            .map(|put| (put.node.spec.id.as_str(), put.node.order, put.read))
            .collect();
        assert_eq!(members, [("m1", 6, None), ("m2", 7, None)]);
        assert!(puts[1..].iter().all(|put| put.node.body.is_empty()));
        assert_eq!(puts[1].node.spec.group.as_deref(), Some("G"));
        // and an empty plan starts at place 0
        let empty = Listing {
            revision: 1,
            nodes: Vec::new(),
        };
        assert_eq!(puts_for(&empty, vec![member("a")], None)[0].node.order, 0);
    }

    /// A refused batch says check's messages, and a stale one each node and what to do
    #[test]
    fn refusals_say_what_was_wrong() {
        let plan = PlanName::new("reef", "central").unwrap();
        // an applied batch is its versions
        let applied = Checked::Applied {
            versions: vec![2, 1],
        };
        assert_eq!(settled(applied, &plan, "put").unwrap(), [2, 1]);
        // a refused one carries check's message as check prints it
        let refused = Checked::Refused(vec![Diagnostic {
            source: "reef/central/C99".to_string(),
            line: None,
            message: "needs `nope`, which is not a node".to_string(),
        }]);
        let error = settled(refused, &plan, "put").unwrap_err().to_string();
        assert!(error.starts_with("the put was refused"), "{error}");
        assert!(
            error.contains("reef/central/C99: needs `nope`, which is not a node"),
            "{error}"
        );
        // a stale one names each node, how it differs, and what to do
        let stale = Checked::Stale(vec![StalePut {
            id: "C17".to_string(),
            refusal: Refusal::Moved,
        }]);
        let error = settled(stale, &plan, "removal").unwrap_err().to_string();
        assert!(error.starts_with("the removal was refused"), "{error}");
        assert!(
            error.contains("`C17`: it changed since it was read"),
            "{error}"
        );
        assert!(error.contains("`reef node get C17`"), "{error}");
        // a new node over one that exists says to keep the version line
        let exists = stale_line(&StalePut {
            id: "C17".to_string(),
            refusal: Refusal::Exists,
        });
        assert!(exists.contains("it already exists"), "{exists}");
        assert!(exists.contains("`# version:`"), "{exists}");
    }
}
