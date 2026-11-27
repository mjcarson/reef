//! reef's JSON API: the service's data types, and the requests, answers and changes its routes
//! carry
//!
//! `reef-service` keeps plans in shoal and checks every write against the whole plan. Its JSON
//! API is a plain axum router the CLI, the MCP server and the hooks call, so none of them depends
//! on the app's server functions or on shoal. These are the types both sides speak, and they are
//! the service's own: a [`NodeWrite`] given to the service is the one a client sends.
//!
//! - **Nodes** ([`node`]): a node as read, as written, as put and as removed, what came of a
//!   batch or a whole-plan write, a plan's listing, and a spec's inline members lifted out.
//! - **Projects and plans** ([`project`]): a project and the repo its plans describe, a plan and
//!   the revision readers load, and the requests that add them.
//! - **Pages** ([`page`]): a plan's own markdown docs, as read and as written, an edit of one,
//!   what came of it, and the paths a page can have.
//! - **The queue** ([`queue`]): a project's order of work, where an edit puts an entry, and
//!   what came of one.
//! - **Work records** ([`work`]): where a node's work happens and what it started from, as
//!   `reef start` read it from the node's checkout, and the SHAs each capture since read.
//! - **Revisions** ([`revision`]): each capture of a node's changes, its files, hunks and lines,
//!   and the anchor a comment names a line by.
//! - **Comments** ([`comment`]): threads on a line of a node's revision or on the node as a
//!   whole, each open or resolved, where a line's thread followed it to in later revisions, the
//!   changes that resolved it, and what came of a comment, a mark or a resolve.
//! - **Loops** ([`loops`]): the terminal loop's state: the pane a session feeds a project in,
//!   the node it holds, the node whose end it acted on, and whether it is paused.
//! - **Decisions** ([`decision`]): each plan a node's sessions asked approval for and its
//!   verdict, and each question and the answer picked, read from the sessions' transcripts.
//! - **Sessions** ([`session`]): the sessions a runner the user started runs headless, as
//!   central reef relays them: each one's transcript and prompts, what the app asks of it, and the
//!   orders the runner takes.
//! - **Users** ([`user`]): reef's users, the tokens they sign in with, and the requests that add
//!   and remove them, sign them in and out, change their passwords and list their tokens.
//! - **History** ([`history`]): who made each edit of a node, a page or a queue, what it changed
//!   and when.
//! - **Changes** ([`change`]): what the change stream tells of each write.
//! - **Errors** ([`error`]): what a route answers when it can't do what was asked.
//! - **Paths** ([`paths`]): every route's path, for the router and for clients to agree on.
//! - **Prompts** (`prompt`, with the `prompt` feature): asking for a password on the terminal,
//!   or reading one from a file, for the CLI and `reef-serve`.
//! - **Config** ([`config`]): where reef's config file is, which the service and its clients read.
//!
//! The service serves HTTPS to localhost, at [`DEFAULT_ADDRESS`] unless reef's config names
//! another, with a certificate of its own, and other hosts where its `[api.tls]` says.

pub mod change;
pub mod comment;
pub mod config;
pub mod decision;
pub mod error;
pub mod history;
pub mod loops;
pub mod node;
pub mod page;
pub mod paths;
pub mod project;
#[cfg(feature = "prompt")]
pub mod prompt;
pub mod queue;
pub mod revision;
pub mod session;
pub mod user;
pub mod work;

pub use change::{Change, FollowedThread, PutNode, PutPage};
pub use comment::{
    COMMENT_BODY_MAX, Comment, Commented, MarkReview, MarkThread, Marked, NewReply, NewThread,
    RESOLVES_TRAILER, Resolution, ResolveThread, Resolved, Resolves, ResolvingCommit, Review,
    ReviewState, Reviewed, Target, Thread, ThreadMark, ThreadState, TrailerOutcome, TrailerRead,
    Writer,
};
pub use decision::{
    Decided, Decision, DecisionKind, DecisionsRecorded, Outcome, RecordDecisions, Via,
};
pub use error::{ApiError, ErrorCode};
pub use history::{Edited, HISTORY_MAX, HistoryPage, HistoryQuery, Logged, LoopAct, UserAct};
pub use loops::{EditLoop, Loop, LoopEdit, LoopEdited, LoopState, Member, Pane};
pub use node::{
    Checked, Listing, MoveNodes, Moved, NodePut, NodeRemove, NodeWrite, PlanWrite, Puts, Refusal,
    Renumbered, Rewrite, Rewritten, StalePut, StoredNode, lift, puts_for, records,
};
pub use page::{
    PLAN_PAGES_MAX_BYTES, PageEdit, PageEdited, PageListing, PageWrite, StoredPage, WRITE_BODY_MAX,
    check_page_path,
};
pub use project::{
    AddedPlan, AddedProject, BranchFetched, Branched, Fetched, NewPlan, NewProject, NewRemote,
    PlanBranch, PlanInfo, Project, Registered, Remote, Tip,
};
pub use queue::{Listed, Place, Placed, QueueEdit, QueueListing, Queued, Ranked};
pub use revision::{
    Anchor, FileDiff, FileStatus, FileSummary, Hunk, Line, LineKind, Revision, Side,
};
pub use session::{
    Answer, Attach, Choice, Entry, EntryKind, Order, Ordered, Prompt, PromptKind, Question, Report,
    Session, SessionAct, SessionActed, SessionState, Transcript,
};
pub use user::{
    Author, IssuedToken, Me, NOBODY, NewUser, PasswordChange, PasswordChanged, SignIn, SignedOut,
    TokenInfo, TokenKind, User, UserAdded, UserRemoved, WRITER_HEADER,
};
pub use work::{
    AddCapture, CaptureRead, CaptureShas, Captured, StartWork, Started, WorkRecord, WorkStart,
};

/// Where the service listens when reef's config names no address: localhost only
pub const DEFAULT_ADDRESS: &str = "127.0.0.1:12180";
