//! Every route's path: the templates the router matches, and builders for clients
//!
//! Every route sits under [`PREFIX`], apart from the app's own pages and server functions, which
//! share its port once reef-app mounts the router. A builder percent-encodes each segment it
//! fills in, since an id may hold spaces or other characters a path can't.
//!
//! | Route | Template |
//! |---|---|
//! | `GET`, `POST` | [`PROJECTS`] |
//! | `GET ?dir=` | [`FIND_PROJECT`] |
//! | `GET`, `POST` | [`PLANS`] |
//! | `GET`, `POST` | [`QUEUE`] |
//! | `GET` | [`PROJECT_NEXT`] |
//! | `GET` | [`WORK`] |
//! | `GET` | [`PLAN`] |
//! | `GET`, `POST` | [`NODES`] |
//! | `GET` | [`NODE`] |
//! | `PUT` | [`REVISION`] |
//! | `GET` | [`PAYLOAD`] |
//! | `GET` | [`CHECK`] |
//! | `GET` | [`NEXT`] |
//! | `GET` | [`DOC`] |
//! | `GET`, `POST` | [`PAGES`] |
//! | `GET` | [`PAGE`] |
//! | `GET`, `POST` | [`NODE_WORK`] |
//! | `POST` | [`CAPTURES`] |
//! | `GET` | [`REVISIONS`] |
//! | `GET` | [`NODE_REVISION`] |
//! | `GET` | [`REVISION_FILE`] |
//! | `GET ?project=&plan=` | [`CHANGES`] |

use reef_model::{NodeName, PlanName};

/// What every route's path starts with
pub const PREFIX: &str = "/api/v1";

/// Every project: listed by `GET`, registered by `POST`
pub const PROJECTS: &str = "/api/v1/projects";

/// The project a directory belongs to, named by the `dir` query
pub const FIND_PROJECT: &str = "/api/v1/find-project";

/// A project's plans: listed by `GET`, added to by `POST`
pub const PLANS: &str = "/api/v1/projects/{project}/plans";

/// A project's queue: listed by `GET`, edited an entry at a time by `POST`
pub const QUEUE: &str = "/api/v1/projects/{project}/queue";

/// The work ready to start across a project's plans, in the user's order first, and what the
/// rest waits on
pub const PROJECT_NEXT: &str = "/api/v1/projects/{project}/next";

/// Every work record of a project's nodes, across its plans
pub const WORK: &str = "/api/v1/projects/{project}/work";

/// One plan, and the revision readers load
pub const PLAN: &str = "/api/v1/projects/{project}/plans/{plan}";

/// A plan's nodes: listed by `GET`, put and removed in batches by `POST`
pub const NODES: &str = "/api/v1/projects/{project}/plans/{plan}/nodes";

/// One node of a plan
pub const NODE: &str = "/api/v1/projects/{project}/plans/{plan}/nodes/{id}";

/// A plan's current revision, replaced whole by `PUT`
pub const REVISION: &str = "/api/v1/projects/{project}/plans/{plan}/revision";

/// A plan built into the viewer's payload
pub const PAYLOAD: &str = "/api/v1/projects/{project}/plans/{plan}/payload";

/// A plan's problems, as `mdbook-reef check` finds a book's
pub const CHECK: &str = "/api/v1/projects/{project}/plans/{plan}/check";

/// The work ready to start in a plan, its entries of its project's queue first, and what the
/// rest waits on, as `mdbook-reef next` says
pub const NEXT: &str = "/api/v1/projects/{project}/plans/{plan}/next";

/// One of a plan's docs, rendered as the page its links lead to
///
/// `path` is the page mdbook would write for the doc, from the root of the plan's docs:
/// `docs/design.html` for the repo's `docs/design.md`.
pub const DOC: &str = "/api/v1/projects/{project}/plans/{plan}/docs/{*path}";

/// A plan's own pages: listed by `GET`, edited a page at a time by `POST`
pub const PAGES: &str = "/api/v1/projects/{project}/plans/{plan}/pages";

/// One of a plan's own pages, by its path: `design/app.md`
pub const PAGE: &str = "/api/v1/projects/{project}/plans/{plan}/pages/{*path}";

/// One node's work record: read by `GET`, and recorded by `POST`, which starts the node
pub const NODE_WORK: &str = "/api/v1/projects/{project}/plans/{plan}/work/{id}";

/// A node's captures: each one sent by `POST`, kept as a revision and its SHAs added to the
/// node's record
pub const CAPTURES: &str = "/api/v1/projects/{project}/plans/{plan}/work/{id}/captures";

/// A node's revisions, oldest first, each listing its files without their hunks
pub const REVISIONS: &str = "/api/v1/projects/{project}/plans/{plan}/work/{id}/revisions";

/// One of a node's revisions, by its number
pub const NODE_REVISION: &str =
    "/api/v1/projects/{project}/plans/{plan}/work/{id}/revisions/{revision}";

/// One file of a node's revision, its hunks and all, by the path it is shown under
pub const REVISION_FILE: &str =
    "/api/v1/projects/{project}/plans/{plan}/work/{id}/revisions/{revision}/files/{*path}";

/// The change stream, as server-sent events, narrowed by the `project` and `plan` queries
pub const CHANGES: &str = "/api/v1/changes";

/// Percent-encodes one path segment or query value
///
/// Every byte but an unreserved one (letters, digits, `-`, `.`, `_` and `~`) is encoded.
///
/// # Arguments
///
/// * `text` - The segment or value to encode
///
/// # Examples
///
/// ```
/// use reef_api::paths::encode;
///
/// assert_eq!(encode("Item 346"), "Item%20346");
/// ```
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        // unreserved bytes stand for themselves, and every other byte is escaped
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The path of the project a directory belongs to
///
/// # Arguments
///
/// * `dir` - The directory, as the service's host sees it
pub fn find_project(dir: &str) -> String {
    format!("{FIND_PROJECT}?dir={}", encode(dir))
}

/// The path of a project's plans
///
/// # Arguments
///
/// * `project` - The project's name
pub fn plans(project: &str) -> String {
    format!("{PROJECTS}/{}/plans", encode(project))
}

/// The path of a project's queue
///
/// # Arguments
///
/// * `project` - The project's name
pub fn queue(project: &str) -> String {
    format!("{PROJECTS}/{}/queue", encode(project))
}

/// The path of the work ready to start across a project's plans
///
/// # Arguments
///
/// * `project` - The project's name
pub fn project_next(project: &str) -> String {
    format!("{PROJECTS}/{}/next", encode(project))
}

/// The path of every work record of a project's nodes
///
/// # Arguments
///
/// * `project` - The project's name
pub fn work(project: &str) -> String {
    format!("{PROJECTS}/{}/work", encode(project))
}

/// The path of one plan
///
/// # Arguments
///
/// * `plan` - The plan's full name
pub fn plan(plan: &PlanName) -> String {
    format!("{}/{}", plans(&plan.project), encode(&plan.plan))
}

/// The path of a plan's nodes
///
/// # Arguments
///
/// * `plan` - The plan's full name
pub fn nodes(plan: &PlanName) -> String {
    format!("{}/nodes", self::plan(plan))
}

/// The path of one node of a plan
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `id` - The node's id
///
/// # Examples
///
/// ```
/// use reef_api::paths;
/// use reef_model::PlanName;
///
/// let plan = PlanName::new("reef", "central").unwrap();
/// assert_eq!(
///     paths::node(&plan, "Item 346"),
///     "/api/v1/projects/reef/plans/central/nodes/Item%20346"
/// );
/// ```
pub fn node(plan: &PlanName, id: &str) -> String {
    format!("{}/{}", nodes(plan), encode(id))
}

/// The path of a plan's current revision
///
/// # Arguments
///
/// * `plan` - The plan's full name
pub fn revision(plan: &PlanName) -> String {
    format!("{}/revision", self::plan(plan))
}

/// The path of a plan's payload
///
/// # Arguments
///
/// * `plan` - The plan's full name
pub fn payload(plan: &PlanName) -> String {
    format!("{}/payload", self::plan(plan))
}

/// The path of a plan's problems
///
/// # Arguments
///
/// * `plan` - The plan's full name
pub fn check(plan: &PlanName) -> String {
    format!("{}/check", self::plan(plan))
}

/// The path of the work ready to start in a plan, and what the rest waits on
///
/// # Arguments
///
/// * `plan` - The plan's full name
pub fn next(plan: &PlanName) -> String {
    format!("{}/next", self::plan(plan))
}

/// The path every page of a plan's docs is under, ending in `/`
///
/// A payload's links start from it, as a book's start from its root.
///
/// # Arguments
///
/// * `plan` - The plan's full name
///
/// # Examples
///
/// ```
/// use reef_api::paths;
/// use reef_model::PlanName;
///
/// let plan = PlanName::new("reef", "central").unwrap();
/// assert_eq!(
///     format!("{}design/app.html", paths::docs(&plan)),
///     "/api/v1/projects/reef/plans/central/docs/design/app.html"
/// );
/// ```
pub fn docs(plan: &PlanName) -> String {
    format!("{}/docs/", self::plan(plan))
}

/// The path of a plan's own pages
///
/// # Arguments
///
/// * `plan` - The plan's full name
pub fn pages(plan: &PlanName) -> String {
    format!("{}/pages", self::plan(plan))
}

/// The path of one of a plan's own pages
///
/// Each part of the page's path is encoded on its own, and they stay apart.
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `path` - The page's path
///
/// # Examples
///
/// ```
/// use reef_api::paths;
/// use reef_model::PlanName;
///
/// let plan = PlanName::new("reef", "central").unwrap();
/// assert_eq!(
///     paths::page(&plan, "design/the app.md"),
///     "/api/v1/projects/reef/plans/central/pages/design/the%20app.md"
/// );
/// ```
pub fn page(plan: &PlanName, path: &str) -> String {
    // each part is encoded, and the slashes between them kept
    let parts: Vec<String> = path.split('/').map(encode).collect();
    format!("{}/{}", pages(plan), parts.join("/"))
}

/// The path of one node's work record
///
/// # Arguments
///
/// * `node` - The node's full name
pub fn node_work(node: &NodeName) -> String {
    format!("{}/work/{}", self::plan(&node.plan), encode(&node.id))
}

/// The path a node's captures are sent to
///
/// # Arguments
///
/// * `node` - The node's full name
pub fn captures(node: &NodeName) -> String {
    format!("{}/captures", node_work(node))
}

/// The path of a node's revisions
///
/// # Arguments
///
/// * `node` - The node's full name
pub fn revisions(node: &NodeName) -> String {
    format!("{}/revisions", node_work(node))
}

/// The path of one of a node's revisions
///
/// # Arguments
///
/// * `node` - The node's full name
/// * `revision` - The revision's number
pub fn node_revision(node: &NodeName, revision: u32) -> String {
    format!("{}/{revision}", revisions(node))
}

/// The path of one file of a node's revision
///
/// Each part of the file's path is encoded on its own, and they stay apart, as a page's are.
///
/// # Arguments
///
/// * `node` - The node's full name
/// * `revision` - The revision's number
/// * `path` - The path the file is shown under
///
/// # Examples
///
/// ```
/// use reef_api::paths;
///
/// let node = "reef/central/C32".parse().unwrap();
/// assert_eq!(
///     paths::revision_file(&node, 2, "src/a b.rs"),
///     "/api/v1/projects/reef/plans/central/work/C32/revisions/2/files/src/a%20b.rs"
/// );
/// ```
pub fn revision_file(node: &NodeName, revision: u32, path: &str) -> String {
    // each part is encoded, and the slashes between them kept
    let parts: Vec<String> = path.split('/').map(encode).collect();
    format!(
        "{}/files/{}",
        node_revision(node, revision),
        parts.join("/")
    )
}

/// The path of the change stream, narrowed to a project or to one plan
///
/// # Arguments
///
/// * `project` - Only this project's changes, or every change
/// * `plan` - Only this plan's changes within that project, or all of the project's
pub fn changes(project: Option<&str>, plan: Option<&str>) -> String {
    // each narrowing given becomes a query
    let mut queries = Vec::new();
    if let Some(project) = project {
        queries.push(format!("project={}", encode(project)));
    }
    if let Some(plan) = plan {
        queries.push(format!("plan={}", encode(plan)));
    }
    match queries.is_empty() {
        true => CHANGES.to_string(),
        false => format!("{CHANGES}?{}", queries.join("&")),
    }
}

#[cfg(test)]
mod tests {
    use super::{PLAN, encode};
    use reef_model::PlanName;

    /// Each builder fills in the template the router matches
    #[test]
    fn builders_fill_in_their_templates() {
        // a plan's path is its template, filled in
        let plan = PlanName::new("reef", "central").unwrap();
        let filled = PLAN
            .replace("{project}", "reef")
            .replace("{plan}", "central");
        assert_eq!(super::plan(&plan), filled);
        // and the routes under it add their own segment
        assert_eq!(super::nodes(&plan), format!("{filled}/nodes"));
        assert_eq!(super::node(&plan, "C53"), format!("{filled}/nodes/C53"));
        assert_eq!(super::revision(&plan), format!("{filled}/revision"));
        assert_eq!(super::payload(&plan), format!("{filled}/payload"));
        assert_eq!(super::check(&plan), format!("{filled}/check"));
        assert_eq!(super::next(&plan), format!("{filled}/next"));
        assert_eq!(super::pages(&plan), format!("{filled}/pages"));
        // a page's path fills in the page's template, its slashes kept
        let page = super::PAGE
            .replace("{project}", "reef")
            .replace("{plan}", "central")
            .replace("{*path}", "design/app.md");
        assert_eq!(super::page(&plan, "design/app.md"), page);
        // and a project's queue and its next fill in theirs
        assert_eq!(
            super::queue("reef"),
            super::QUEUE.replace("{project}", "reef")
        );
        assert_eq!(
            super::project_next("reef"),
            super::PROJECT_NEXT.replace("{project}", "reef")
        );
        // and so do a project's work records, and one node's with its captures
        assert_eq!(
            super::work("reef"),
            super::WORK.replace("{project}", "reef")
        );
        let node = plan.node("C30").unwrap();
        let work = super::NODE_WORK
            .replace("{project}", "reef")
            .replace("{plan}", "central")
            .replace("{id}", "C30");
        assert_eq!(super::node_work(&node), work);
        assert_eq!(super::captures(&node), format!("{work}/captures"));
        // and its revisions, one of them, and one of that one's files
        let revisions = super::REVISIONS
            .replace("{project}", "reef")
            .replace("{plan}", "central")
            .replace("{id}", "C30");
        assert_eq!(super::revisions(&node), revisions);
        let revision = super::NODE_REVISION
            .replace("{project}", "reef")
            .replace("{plan}", "central")
            .replace("{id}", "C30")
            .replace("{revision}", "3");
        assert_eq!(super::node_revision(&node, 3), revision);
        let file = super::REVISION_FILE
            .replace("{project}", "reef")
            .replace("{plan}", "central")
            .replace("{id}", "C30")
            .replace("{revision}", "3")
            .replace("{*path}", "src/lib.rs");
        assert_eq!(super::revision_file(&node, 3, "src/lib.rs"), file);
        // and a doc's page fills in the rest of the docs' template
        let page = super::DOC
            .replace("{project}", "reef")
            .replace("{plan}", "central")
            .replace("{*path}", "design/app.html");
        assert_eq!(format!("{}design/app.html", super::docs(&plan)), page);
    }

    /// Anything a segment can't hold is escaped, and nothing else
    #[test]
    fn encode_escapes_what_a_segment_cant_hold() {
        // unreserved bytes are left as they are
        assert_eq!(encode("C53.a_b-c~"), "C53.a_b-c~");
        // spaces, slashes, queries and percent signs are escaped, and so is utf-8, byte by byte
        assert_eq!(encode("a b/c?d%"), "a%20b%2Fc%3Fd%25");
        assert_eq!(encode("é"), "%C3%A9");
    }

    /// The change stream's queries are only those given
    #[test]
    fn changes_takes_only_the_narrowings_given() {
        assert_eq!(super::changes(None, None), "/api/v1/changes");
        assert_eq!(
            super::changes(Some("reef"), Some("central")),
            "/api/v1/changes?project=reef&plan=central"
        );
        assert_eq!(
            super::find_project("/src/my repo"),
            "/api/v1/find-project?dir=%2Fsrc%2Fmy%20repo"
        );
    }
}
