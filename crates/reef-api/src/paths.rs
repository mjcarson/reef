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
//! | `GET` | [`PLAN_COMMENTS`] |
//! | `GET`, `POST` | [`NODE_COMMENTS`] |
//! | `GET`, `PUT` | [`NODE_REVIEW`] |
//! | `GET`, `POST` | [`NODE_DECISIONS`] |
//! | `GET` | [`THREAD`] |
//! | `POST` | [`REPLIES`] |
//! | `PUT` | [`THREAD_STATE`] |
//! | `POST` | [`SESSIONS`] |
//! | `GET ?after=`, `POST`, `DELETE` | [`SESSION`] |
//! | `GET ?after=&wait=`, `POST` | [`SESSION_ORDERS`] |
//! | `GET` | [`PROJECT_SESSIONS`] |
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

/// A plan's comment threads, on every node of it, by their numbers
pub const PLAN_COMMENTS: &str = "/api/v1/projects/{project}/plans/{plan}/comments";

/// One node's comment threads, read by `GET`, and a new one started by `POST`
pub const NODE_COMMENTS: &str = "/api/v1/projects/{project}/plans/{plan}/nodes/{id}/comments";

/// A node's open review: read by `GET`, submitted or discarded by `PUT`
pub const NODE_REVIEW: &str = "/api/v1/projects/{project}/plans/{plan}/nodes/{id}/review";

/// A node's decisions: listed by `GET`, those a hook read from a session's transcript kept by
/// `POST`
pub const NODE_DECISIONS: &str = "/api/v1/projects/{project}/plans/{plan}/nodes/{id}/decisions";

/// The query a thread's routes take to serve threads pending in a review too, which they leave
/// out otherwise
pub const PENDING_QUERY: &str = "pending=true";

/// One comment thread, by its number in its plan
pub const THREAD: &str = "/api/v1/projects/{project}/plans/{plan}/comments/{thread}";

/// A thread's replies, each one sent by `POST`
pub const REPLIES: &str = "/api/v1/projects/{project}/plans/{plan}/comments/{thread}/replies";

/// A thread's state, marked open or resolved by `PUT`
pub const THREAD_STATE: &str = "/api/v1/projects/{project}/plans/{plan}/comments/{thread}/state";

/// A thread resolved by hand by `POST`, a revision linked to it as the one that answered it
pub const THREAD_RESOLUTION: &str =
    "/api/v1/projects/{project}/plans/{plan}/comments/{thread}/resolution";

/// Every loop, read by `GET`, and a change to a pane's loop sent by `POST`
pub const LOOPS: &str = "/api/v1/loops";

/// One project's loops
pub const PROJECT_LOOPS: &str = "/api/v1/projects/{project}/loops";

/// Runners' sessions: a runner attaches by `POST`
pub const SESSIONS: &str = "/api/v1/sessions";

/// One runner's session: its transcript after the `after` query's entry, read by `GET`, the
/// runner's report sent by `POST`, and its letting go by `DELETE`
pub const SESSION: &str = "/api/v1/sessions/{session}";

/// One session's orders: those after the `after` query's number, waited for up to the `wait`
/// query's milliseconds by `GET`, and the app's act on the session sent by `POST`
pub const SESSION_ORDERS: &str = "/api/v1/sessions/{session}/orders";

/// One project's runners' sessions
pub const PROJECT_SESSIONS: &str = "/api/v1/projects/{project}/sessions";

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

/// The path of a plan's comment threads
///
/// # Arguments
///
/// * `plan` - The plan's full name
pub fn plan_comments(plan: &PlanName) -> String {
    format!("{}/comments", self::plan(plan))
}

/// The path of one node's comment threads, which a new one is sent to
///
/// # Arguments
///
/// * `node` - The node's full name
///
/// # Examples
///
/// ```
/// use reef_api::paths;
///
/// let node = "reef/central/Item 346".parse().unwrap();
/// assert_eq!(
///     paths::node_comments(&node),
///     "/api/v1/projects/reef/plans/central/nodes/Item%20346/comments"
/// );
/// ```
pub fn node_comments(node: &NodeName) -> String {
    format!("{}/comments", self::node(&node.plan, &node.id))
}

/// The path of a node's open review
///
/// # Arguments
///
/// * `node` - The node's full name
pub fn node_review(node: &NodeName) -> String {
    format!("{}/review", self::node(&node.plan, &node.id))
}

/// The path of a node's decisions
///
/// # Arguments
///
/// * `node` - The node's full name
pub fn node_decisions(node: &NodeName) -> String {
    format!("{}/decisions", self::node(&node.plan, &node.id))
}

/// The path of one comment thread
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `thread` - The thread's number
pub fn thread(plan: &PlanName, thread: u32) -> String {
    format!("{}/{thread}", plan_comments(plan))
}

/// The path a thread's replies are sent to
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `thread` - The thread's number
pub fn replies(plan: &PlanName, thread: u32) -> String {
    format!("{}/replies", self::thread(plan, thread))
}

/// The path a thread is marked open or resolved at
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `thread` - The thread's number
pub fn thread_state(plan: &PlanName, thread: u32) -> String {
    format!("{}/state", self::thread(plan, thread))
}

/// The path a thread is resolved by hand at, a revision linked to it
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `thread` - The thread's number
pub fn thread_resolution(plan: &PlanName, thread: u32) -> String {
    format!("{}/resolution", self::thread(plan, thread))
}

/// The path of one project's loops
///
/// # Arguments
///
/// * `project` - The project's name
pub fn project_loops(project: &str) -> String {
    format!("{PROJECTS}/{}/loops", encode(project))
}

/// The path of one runner's session
///
/// # Arguments
///
/// * `session` - The session's id
pub fn session(session: &str) -> String {
    format!("{SESSIONS}/{}", encode(session))
}

/// The path of one session's orders
///
/// # Arguments
///
/// * `session` - The session's id
pub fn session_orders(session: &str) -> String {
    format!("{}/orders", self::session(session))
}

/// The path of one project's runners' sessions
///
/// # Arguments
///
/// * `project` - The project's name
pub fn project_sessions(project: &str) -> String {
    format!("{PROJECTS}/{}/sessions", encode(project))
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
        // and the plan's comment threads, a node's, one thread, and its replies, state and resolution
        let fill = |template: &str| {
            template
                .replace("{project}", "reef")
                .replace("{plan}", "central")
                .replace("{id}", "C30")
                .replace("{thread}", "7")
                .replace("{session}", "k3x9-4242")
        };
        assert_eq!(super::plan_comments(&plan), fill(super::PLAN_COMMENTS));
        assert_eq!(super::node_comments(&node), fill(super::NODE_COMMENTS));
        assert_eq!(super::node_review(&node), fill(super::NODE_REVIEW));
        assert_eq!(super::node_decisions(&node), fill(super::NODE_DECISIONS));
        assert_eq!(super::thread(&plan, 7), fill(super::THREAD));
        assert_eq!(super::replies(&plan, 7), fill(super::REPLIES));
        assert_eq!(super::thread_state(&plan, 7), fill(super::THREAD_STATE));
        assert_eq!(
            super::thread_resolution(&plan, 7),
            fill(super::THREAD_RESOLUTION)
        );
        // and a project's loops
        assert_eq!(super::project_loops("reef"), fill(super::PROJECT_LOOPS));
        // and a runner's session, its orders and a project's sessions
        assert_eq!(super::session("k3x9-4242"), fill(super::SESSION));
        assert_eq!(
            super::session_orders("k3x9-4242"),
            fill(super::SESSION_ORDERS)
        );
        assert_eq!(
            super::project_sessions("reef"),
            fill(super::PROJECT_SESSIONS)
        );
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
