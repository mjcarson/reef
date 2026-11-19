//! Names that hold across projects and plans: a plan is `project/plan`, and a node is
//! `project/plan/id`.
//!
//! An id is unique only within its plan, so trailers, URLs, tools and queues that span plans
//! name a node in full. Within a plan, a bare id still names one of its own nodes, so specs read
//! the same as frontmatter does.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The most characters a project or plan name may hold
pub const MAX_NAME_LEN: usize = 64;

/// Why a name was refused
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    /// A project name that is empty, too long, or holds a character names can't hold
    Project(String),
    /// A plan name that is empty, too long, or holds a character names can't hold
    Plan(String),
    /// An id that is empty or holds a `/`
    Id(String),
    /// Text that is not in the form the name takes
    Form {
        /// The text as written
        text: String,
        /// The form it should take, such as `project/plan/id`
        form: &'static str,
    },
}

impl fmt::Display for NameError {
    /// Say what was refused and the rule it broke
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // the rule project and plan names share
        let rule = format!(
            "1 to {MAX_NAME_LEN} ASCII letters, digits, `.`, `_` or `-`, starting with a letter or digit"
        );
        match self {
            NameError::Project(name) => write!(f, "project name `{name}` is not {rule}"),
            NameError::Plan(name) => write!(f, "plan name `{name}` is not {rule}"),
            NameError::Id(id) => write!(f, "id `{id}` is empty or holds a `/`"),
            NameError::Form { text, form } => write!(f, "`{text}` is not a name of the form `{form}`"),
        }
    }
}

impl std::error::Error for NameError {}

/// The form a plan's name takes
const PLAN_FORM: &str = "project/plan";

/// The form a node's name takes
const NODE_FORM: &str = "project/plan/id";

/// Whether a project or plan name keeps to the rule names share
///
/// # Arguments
///
/// * `name` - The project or plan name to check
fn is_valid_name(name: &str) -> bool {
    // a name starts with a letter or digit
    let starts_well = name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    // and holds only letters, digits, `.`, `_` and `-`
    let chars_ok = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    starts_well && chars_ok && name.len() <= MAX_NAME_LEN
}

/// Whether an id can be the last part of a node's name
///
/// # Arguments
///
/// * `id` - The id to check
fn is_valid_id(id: &str) -> bool {
    !id.trim().is_empty() && !id.contains('/')
}

/// A plan's full name: the project it belongs to and its own name within it
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PlanName {
    /// The project this plan belongs to
    pub project: String,
    /// The plan's name within its project
    pub plan: String,
}

impl PlanName {
    /// Name a plan, refusing a project or plan name that breaks the rule names share
    ///
    /// # Arguments
    ///
    /// * `project` - The project the plan belongs to
    /// * `plan` - The plan's name within its project
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_model::PlanName;
    ///
    /// let plan = PlanName::new("reef", "central").unwrap();
    /// assert_eq!(plan.to_string(), "reef/central");
    /// ```
    pub fn new<P: Into<String>, Q: Into<String>>(project: P, plan: Q) -> Result<Self, NameError> {
        // convert both parts into strings
        let project = project.into();
        let plan = plan.into();
        // refuse a part that breaks the rule
        if !is_valid_name(&project) {
            return Err(NameError::Project(project));
        }
        if !is_valid_name(&plan) {
            return Err(NameError::Plan(plan));
        }
        Ok(PlanName { project, plan })
    }

    /// Name a node of this plan
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id within this plan
    pub fn node<T: Into<String>>(&self, id: T) -> Result<NodeName, NameError> {
        NodeName::new(self.clone(), id)
    }
}

impl fmt::Display for PlanName {
    /// Write the plan's name as `project/plan`
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.project, self.plan)
    }
}

impl FromStr for PlanName {
    type Err = NameError;

    /// Read a plan's name written as `project/plan`
    ///
    /// # Arguments
    ///
    /// * `text` - The name to read
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        // a plan's name has exactly two parts
        let form_error = || NameError::Form { text: text.to_string(), form: PLAN_FORM };
        let Some((project, plan)) = text.split_once('/') else {
            return Err(form_error());
        };
        if plan.contains('/') {
            return Err(form_error());
        }
        PlanName::new(project, plan)
    }
}

impl TryFrom<String> for PlanName {
    type Error = NameError;

    /// Read a plan's name from serialized text
    ///
    /// # Arguments
    ///
    /// * `text` - The name to read
    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<PlanName> for String {
    /// Write a plan's name as text to serialize
    ///
    /// # Arguments
    ///
    /// * `name` - The name to write
    fn from(name: PlanName) -> String {
        name.to_string()
    }
}

/// A node's full name: the plan it belongs to and its id within that plan
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NodeName {
    /// The plan this node belongs to
    pub plan: PlanName,
    /// The node's id, unique within its plan
    pub id: String,
}

impl NodeName {
    /// Name a node of a plan, refusing an id that is empty or holds a `/`
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan the node belongs to
    /// * `id` - The node's id within that plan
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_model::{NodeName, PlanName};
    ///
    /// let plan = PlanName::new("reef", "central").unwrap();
    /// let name = NodeName::new(plan, "C1").unwrap();
    /// assert_eq!(name.to_string(), "reef/central/C1");
    /// ```
    pub fn new<T: Into<String>>(plan: PlanName, id: T) -> Result<Self, NameError> {
        // convert the id into a string
        let id = id.into();
        // refuse an id that can't be the last part of a name
        if !is_valid_id(&id) {
            return Err(NameError::Id(id));
        }
        Ok(NodeName { plan, id })
    }

    /// Resolve a node named in a plan: a bare id names a node of that plan, and anything
    /// else must be a full `project/plan/id`
    ///
    /// # Arguments
    ///
    /// * `text` - The bare id or full name as written
    /// * `within` - The plan it was written in
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_model::{NodeName, PlanName};
    ///
    /// let central = PlanName::new("reef", "central").unwrap();
    /// assert_eq!(NodeName::resolve("C3", &central).unwrap().to_string(), "reef/central/C3");
    /// assert_eq!(NodeName::resolve("shoal/scans/m23", &central).unwrap().plan.project, "shoal");
    /// ```
    pub fn resolve(text: &str, within: &PlanName) -> Result<Self, NameError> {
        // a bare id names a node of the plan it was written in
        if !text.contains('/') {
            return NodeName::new(within.clone(), text);
        }
        // anything else is a full name
        text.parse()
    }

    /// Name another node of the same plan
    ///
    /// # Arguments
    ///
    /// * `id` - The other node's id
    pub fn sibling<T: Into<String>>(&self, id: T) -> Result<NodeName, NameError> {
        NodeName::new(self.plan.clone(), id)
    }
}

impl fmt::Display for NodeName {
    /// Write the node's name as `project/plan/id`
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.plan, self.id)
    }
}

impl FromStr for NodeName {
    type Err = NameError;

    /// Read a node's name written as `project/plan/id`
    ///
    /// # Arguments
    ///
    /// * `text` - The name to read
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        // a node's name has exactly three parts
        let parts: Vec<&str> = text.split('/').collect();
        let [project, plan, id] = parts.as_slice() else {
            return Err(NameError::Form { text: text.to_string(), form: NODE_FORM });
        };
        // check each part in turn
        NodeName::new(PlanName::new(*project, *plan)?, *id)
    }
}

impl TryFrom<String> for NodeName {
    type Error = NameError;

    /// Read a node's name from serialized text
    ///
    /// # Arguments
    ///
    /// * `text` - The name to read
    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<NodeName> for String {
    /// Write a node's name as text to serialize
    ///
    /// # Arguments
    ///
    /// * `name` - The name to write
    fn from(name: NodeName) -> String {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        // names print as they were written, and read back to the same name
        for text in ["reef/central/C1", "shoal/scans/Item 346", "a.b/x_y-2/m23"] {
            let name: NodeName = text.parse().unwrap();
            assert_eq!(name.to_string(), text);
            assert_eq!(name.to_string().parse::<NodeName>().unwrap(), name);
        }
        let plan: PlanName = "reef/central".parse().unwrap();
        assert_eq!((plan.project.as_str(), plan.plan.as_str()), ("reef", "central"));
        assert_eq!(plan.to_string(), "reef/central");
        // serde reads and writes them as strings
        let name: NodeName = "reef/central/C1".parse().unwrap();
        let json = serde_json::to_string(&name).unwrap();
        assert_eq!(json, "\"reef/central/C1\"");
        assert_eq!(serde_json::from_str::<NodeName>(&json).unwrap(), name);
        assert_eq!(serde_json::to_string(&plan).unwrap(), "\"reef/central\"");
        assert!(serde_json::from_str::<NodeName>("\"reef/central\"").is_err());
    }

    #[test]
    fn bad_names_refused() {
        // a project or plan name breaking the rule is refused, naming which part broke it
        let long = "a".repeat(MAX_NAME_LEN + 1);
        for bad in ["", ".hidden", "-dash", "has space", "tab\t", "café", long.as_str()] {
            assert_eq!(PlanName::new(bad, "plan"), Err(NameError::Project(bad.to_string())), "{bad:?}");
            assert_eq!(PlanName::new("proj", bad), Err(NameError::Plan(bad.to_string())), "{bad:?}");
        }
        assert!(PlanName::new("a".repeat(MAX_NAME_LEN), "p").is_ok());
        // a `/` can't sit inside a part
        assert!(matches!("a/b/c".parse::<PlanName>(), Err(NameError::Form { .. })));
        assert!(matches!("a/b/c/d".parse::<NodeName>(), Err(NameError::Form { .. })));
        assert!(matches!("a/b".parse::<NodeName>(), Err(NameError::Form { .. })));
        assert!(matches!("x y/b/c".parse::<NodeName>(), Err(NameError::Project(_))));
        assert!(matches!("a/b c/c".parse::<NodeName>(), Err(NameError::Plan(_))));
        // an id is not empty and holds no `/`
        let plan = PlanName::new("a", "b").unwrap();
        assert_eq!(plan.node(" "), Err(NameError::Id(" ".into())));
        assert_eq!(plan.node("x/y"), Err(NameError::Id("x/y".into())));
        assert!(matches!("a/b/".parse::<NodeName>(), Err(NameError::Id(_))));
        // each refusal says the rule it broke
        let msg = PlanName::new("has space", "p").unwrap_err().to_string();
        assert!(msg.contains("project name `has space`") && msg.contains("starting with a letter or digit"), "{msg}");
        assert!("a/b".parse::<NodeName>().unwrap_err().to_string().contains("`project/plan/id`"));
        assert!("a".parse::<PlanName>().unwrap_err().to_string().contains("of the form `project/plan`"));
        assert!("a/b/c".parse::<PlanName>().unwrap_err().to_string().contains("of the form `project/plan`"));
    }

    #[test]
    fn bare_ids_resolve_within_their_plan() {
        let central = PlanName::new("reef", "central").unwrap();
        // a bare id names a node of the plan it was written in
        let c3 = NodeName::resolve("C3", &central).unwrap();
        assert_eq!(c3, central.node("C3").unwrap());
        assert_eq!(c3.sibling("C1").unwrap().to_string(), "reef/central/C1");
        // a full name names its own plan's node
        let other = NodeName::resolve("shoal/scans/m23", &central).unwrap();
        assert_eq!(other.plan, PlanName::new("shoal", "scans").unwrap());
        // anything with a `/` must be a full name
        assert!(matches!(NodeName::resolve("scans/m23", &central), Err(NameError::Form { .. })));
        assert!(matches!(NodeName::resolve("", &central), Err(NameError::Id(_))));
    }
}
