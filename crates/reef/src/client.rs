//! A client of reef's JSON API, speaking `reef-api`'s types
//!
//! Each method is one route, its path built by `reef_api::paths` and its answer one of
//! `reef-api`'s types. A route that can't do what was asked answers with an [`ApiError`], which
//! comes back as the error, so a caller can match on its code. A refusal, such as a project name
//! that is taken, is an answer, not an error. When nothing answers at the service's address, the
//! error says where the CLI looked, which key set it, and how to start the service.
//!
//! The service is called at `[api] url` if the config names one, trusting the system's roots and
//! `[api] ca`'s, else at `[api] address` on this host, over HTTPS, trusting the CA the service
//! made and keeps in `[api] certs`, if it is there. A service on this host, or through a tunnel
//! to another host's loopback port, is called past any proxy a shell has set.
//!
//! Every request sends the token kept for the service, if one is, as `Authorization: Bearer`,
//! read from where [`credentials`](crate::credentials) says for each request. A service that asks
//! the caller to sign in answers 401, which comes back as an `unauthorized` error saying to run
//! `reef login`.
//!
//! Every answer's `reef-client` header, the build the service hands its users, is read too, and a
//! newer one than this stamped build starts its updater in the background
//! ([`update`](crate::update)).

use reef_api::HistoryPage;
use reef_api::{
    AddCapture, AddedPlan, AddedProject, ApiError, Attach, Branched, Captured, Checked, Commented,
    Decision, DecisionsRecorded, EditLoop, ErrorCode, Fetched, FileDiff, IssuedToken, Listing,
    Loop, LoopEdited, Me, NewPlan, NewProject, NewReply, NewUser, Ordered, PageEdit, PageEdited,
    PageListing, PasswordChange, PasswordChanged, PlanBranch, PlanInfo, PlanWrite, Project, Puts,
    QueueEdit, QueueListing, Queued, RecordDecisions, Report, ResolveThread, Resolved, Revision,
    Rewrite, Session, SessionAct, SessionActed, SignIn, SignedOut, StartWork, Started, StoredNode,
    StoredPage, Thread, TokenInfo, Transcript, User, UserAdded, UserRemoved, WRITER_HEADER,
    WorkRecord, Writer, paths,
};
use reef_api::{
    Build, DocsNamed, MoveNodes, Moved, OpenRun, PlanDocs, PlanSummary, RunReport, TestRun,
};
use reef_model::{Book, Diagnostic, Next, NodeName, PlanName};
use serde::de::DeserializeOwned;
use std::path::Path;
use std::time::Duration;

use crate::conf::Conf;
use crate::credentials::{self, Held};
use crate::update::Watch;

/// How long to wait for the service to take a connection
const CONNECT_WAIT: Duration = Duration::from_secs(5);

/// A client of reef's JSON API
pub struct Client {
    /// The HTTP client, which keeps its connection open between requests
    http: reqwest::Client,
    /// The service's root, such as `https://127.0.0.1:12180`
    base: String,
    /// Where that root came from, for a message that says what to change
    origin: String,
    /// Where the token it signs in with comes from
    held: Held,
    /// Whether it writes as the user, or as Claude for them, which every request it sends says
    writer: Writer,
    /// What it needs to start this `reef`'s updater when an answer names a newer build
    watch: Watch,
}

impl Client {
    /// A client of the service at the address reef's config names
    ///
    /// # Arguments
    ///
    /// * `conf` - The CLI's part of reef's config
    pub fn new(conf: &Conf) -> anyhow::Result<Client> {
        // the root `url` names, else this host's address over HTTPS, which is all it speaks
        let (base, key) = match &conf.api.url {
            Some(url) => {
                let base = root(url).map_err(|error| {
                    anyhow::anyhow!("{} is `{url}`, {error}", conf.key("api.url"))
                })?;
                (base, "api.url")
            }
            None => (format!("https://{}", conf.api.address), "api.address"),
        };
        // a config that set the root is named, else the key that would change the default
        let origin = match conf.source {
            Some(_) => format!("set by {}", conf.key(key)),
            None => format!("reef's default; set {} to change it", conf.key(key)),
        };
        let mut http = reqwest::Client::builder().connect_timeout(CONNECT_WAIT);
        // a service on this host, or through a tunnel to its loopback, is reached past any proxy
        // a shell has set
        if conf.api.url.is_none() || is_loopback_root(&base) {
            http = http.no_proxy();
        }
        // the CA the service on this host made, if it has, for the address it is called at
        let own = conf.api.url.is_none().then(|| conf.own_ca()).flatten();
        if let Some(own) = &own {
            let roots = roots(own).map_err(|error| {
                anyhow::anyhow!(
                    "{}, which reef's service made, is unreadable: {error}",
                    own.display()
                )
            })?;
            for root in roots {
                http = http.add_root_certificate(root);
            }
        }
        // the roots its certificate is trusted by, beside the system's own
        if let Some(ca) = &conf.api.ca {
            let roots = roots(ca).map_err(|error| {
                anyhow::anyhow!("{} is {}: {error}", conf.key("api.ca"), ca.display())
            })?;
            for root in roots {
                http = http.add_root_certificate(root);
            }
        }
        Ok(Client {
            http: http.build()?,
            base,
            origin,
            held: credentials::held(conf.credentials.as_deref()),
            writer: Writer::User,
            watch: Watch {
                enabled: conf.client.update,
                conf: conf.source.clone(),
                credentials: conf.credentials.clone(),
            },
        })
    }

    /// The same client, saying on every request it sends that it writes as Claude, for the user
    /// its token signs in, or as the user themselves
    ///
    /// reef's MCP server writes as Claude, so the service names Claude, for the user, as the
    /// author of what it writes. The header can only ever add "Claude, for" to the token's user.
    ///
    /// # Arguments
    ///
    /// * `writer` - Who it writes as
    pub fn writing_as(self, writer: Writer) -> Client {
        Client { writer, ..self }
    }

    /// Whether it writes as the user, or as Claude for them
    pub fn writer(&self) -> Writer {
        self.writer
    }

    /// The service's root, such as `https://127.0.0.1:12180`, which a login is kept by
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Where the token this client signs in with comes from
    pub fn held(&self) -> &Held {
        &self.held
    }

    /// The token this client signs in with now, if it has one for its service
    ///
    /// Read for each request, so a process started before `reef login` sends the token once
    /// there is one.
    pub fn token(&self) -> anyhow::Result<Option<String>> {
        match &self.held {
            Held::Token(token) => Ok(Some(token.clone())),
            Held::File(path) => {
                Ok(credentials::login_for(path, &self.base)?.map(|login| login.token))
            }
            Held::Nowhere => Ok(None),
        }
    }

    /// Trades a user's name and password for a token, or fails `unauthorized` if they don't
    /// match a user
    ///
    /// # Arguments
    ///
    /// * `sign_in` - The name, the password and what the token is for
    pub async fn sign_in(&self, sign_in: &SignIn) -> anyhow::Result<IssuedToken> {
        // a 401 here is a wrong name or password, not a want of signing in
        let request = self.http.post(self.url(paths::TOKENS)).json(sign_in);
        self.send_as(request, paths::TOKENS, true).await
    }

    /// Revokes the token this client sends
    pub async fn sign_out(&self) -> anyhow::Result<SignedOut> {
        let request = self.http.delete(self.url(paths::CURRENT_TOKEN));
        self.send(request, paths::CURRENT_TOKEN).await
    }

    /// Who the service takes this client for, and whether it asks callers to sign in
    pub async fn me(&self) -> anyhow::Result<Me> {
        self.send(self.http.get(self.url(paths::ME)), paths::ME)
            .await
    }

    /// Lists every user, by name, which only an admin may
    pub async fn users(&self) -> anyhow::Result<Vec<User>> {
        self.send(self.http.get(self.url(paths::USERS)), paths::USERS)
            .await
    }

    /// Reads a page of the log of acts on reef's users, newest first, which only an admin may
    ///
    /// # Arguments
    ///
    /// * `older` - The cursor a page named, to read the one older than it, or `None` for the newest
    pub async fn user_history(&self, older: Option<&str>) -> anyhow::Result<HistoryPage> {
        let path = paths::history_page(paths::USER_HISTORY, older);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Reads a page of a project's loops' and sessions' history, newest first
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `older` - The cursor a page named, to read the one older than it, or `None` for the newest
    pub async fn loop_history(
        &self,
        project: &str,
        older: Option<&str>,
    ) -> anyhow::Result<HistoryPage> {
        let path = paths::history_page(&paths::loop_history(project), older);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Adds a user, which an admin may, or anyone on the service's host while it has none
    ///
    /// # Arguments
    ///
    /// * `new` - The new user, with their password
    pub async fn add_user(&self, new: &NewUser) -> anyhow::Result<UserAdded> {
        let request = self.http.post(self.url(paths::USERS)).json(new);
        self.send(request, paths::USERS).await
    }

    /// Removes a user, which only an admin may, revoking their tokens
    ///
    /// # Arguments
    ///
    /// * `name` - The user's name
    pub async fn remove_user(&self, name: &str) -> anyhow::Result<UserRemoved> {
        let path = paths::user(name);
        self.send(self.http.delete(self.url(&path)), &path).await
    }

    /// Changes a user's password: this client's own user's with their current one, or another's
    /// as an admin
    ///
    /// # Arguments
    ///
    /// * `name` - The user's name
    /// * `change` - The current password, if it is their own, and the new one
    pub async fn change_password(
        &self,
        name: &str,
        change: &PasswordChange,
    ) -> anyhow::Result<PasswordChanged> {
        let path = paths::user_password(name);
        let request = self.http.put(self.url(&path)).json(change);
        self.send(request, &path).await
    }

    /// Lists this client's user's tokens, or another user's as an admin
    ///
    /// # Arguments
    ///
    /// * `user` - The user whose tokens to list, if not this client's own
    pub async fn tokens(&self, user: Option<&str>) -> anyhow::Result<Vec<TokenInfo>> {
        let path = match user {
            Some(user) => paths::tokens_of(user),
            None => paths::TOKENS.to_string(),
        };
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Revokes a token by its id: one of this client's user's, or anyone's as an admin
    ///
    /// # Arguments
    ///
    /// * `id` - The token's id, as a listing shows it
    pub async fn revoke_token(&self, id: &str) -> anyhow::Result<SignedOut> {
        let path = paths::token(id);
        self.send(self.http.delete(self.url(&path)), &path).await
    }

    /// Lists every project, by name
    pub async fn projects(&self) -> anyhow::Result<Vec<Project>> {
        self.send(self.http.get(self.url(paths::PROJECTS)), paths::PROJECTS)
            .await
    }

    /// Registers a project for the repo that holds a directory
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `repo` - A directory in its repo, absolute
    pub async fn add_project(&self, name: &str, repo: &Path) -> anyhow::Result<AddedProject> {
        // the service reads the repo as this host sees it
        let new = NewProject::dir(name, repo);
        let request = self.http.post(self.url(paths::PROJECTS)).json(&new);
        self.send(request, paths::PROJECTS).await
    }

    /// Registers a project by its git remote, which the service clones and fetches
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `url` - The URL the service fetches from, with no password in it
    /// * `branch` - The branch whose docs it reads, or `None` for the remote's default
    pub async fn add_remote_project(
        &self,
        name: &str,
        url: &str,
        branch: Option<&str>,
    ) -> anyhow::Result<AddedProject> {
        // the service clones it before it answers
        let new = NewProject::remote(name, url, branch.map(str::to_string));
        let request = self.http.post(self.url(paths::PROJECTS)).json(&new);
        self.send(request, paths::PROJECTS).await
    }

    /// Fetches a remote project's branch, so the service reads its docs at its tip
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn fetch_project(&self, project: &str) -> anyhow::Result<Fetched> {
        let path = paths::fetch(project);
        self.send(self.http.post(self.url(&path)), &path).await
    }

    /// Finds the project a directory belongs to, or `None` if no project is registered for it
    ///
    /// A directory in a linked worktree belongs to its main repo's project, and one in a clone
    /// of a remote project's repo belongs to that project, found by its checkout's remotes.
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory, absolute
    pub async fn find_project(&self, dir: &Path) -> anyhow::Result<Option<Project>> {
        // a query carries text, so a path that isn't utf-8 can't be sent
        let text = dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("{} is not a utf-8 path", dir.display()))?;
        // with the URLs its checkout fetches from, if it is in one
        let remotes = crate::git::Checkout::open(dir)
            .map(|checkout| checkout.remote_urls())
            .unwrap_or_default();
        let path = paths::find_project(text, &remotes);
        // a directory no project holds is an answer here, not an error
        match self.send(self.http.get(self.url(&path)), &path).await {
            Ok(project) => Ok(Some(project)),
            Err(error) if is_code(&error, ErrorCode::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Lists a project's plans, by name
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn plans(&self, project: &str) -> anyhow::Result<Vec<PlanInfo>> {
        let path = paths::plans(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Lists each of a project's plans summed up: its state of work, last update, nodes done in
    /// the last 24 hours and who works on what
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn plan_summaries(&self, project: &str) -> anyhow::Result<Vec<PlanSummary>> {
        let path = paths::plan_summaries(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Lists a plan in its project, leaving one already listed as it is
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn add_plan(&self, plan: &PlanName) -> anyhow::Result<AddedPlan> {
        // the plan's name within its project, posted to the project's plans
        let path = paths::plans(&plan.project);
        let new = NewPlan::new(plan.plan.clone());
        let request = self.http.post(self.url(&path)).json(&new);
        self.send(request, &path).await
    }

    /// Moves nodes of a plan to another plan of its project, everything of theirs with them
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan the nodes leave
    /// * `request` - The nodes, the plan they join, and the group a node joins there whose own
    ///   stays behind
    pub async fn move_nodes(&self, plan: &PlanName, request: &MoveNodes) -> anyhow::Result<Moved> {
        // posted to the plan they leave, which an older service has no route for
        let path = paths::move_nodes(plan);
        let request = self.http.post(self.url(&path)).json(request);
        self.send(request, &path).await
    }

    /// Sets the branch a plan follows, or drops it so the plan follows its project's
    ///
    /// A service older than plans that follow branches has no route for it, and says so.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `branch` - The branch to follow, or `None` for its project's own
    pub async fn set_plan_branch(
        &self,
        plan: &PlanName,
        branch: Option<String>,
    ) -> anyhow::Result<Branched> {
        // put to the plan's own branch route, which an older service doesn't have
        let path = paths::plan_branch(plan);
        let request = self.http.put(self.url(&path)).json(&PlanBranch { branch });
        self.send(request, &path).await
    }

    /// Names the docs a plan reads as its book, or clears them so its book is its root node,
    /// then its pages
    ///
    /// A service older than plans that name docs has no route for it, and says so.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `docs` - The folder or summary to name, or `None` for none
    pub async fn set_plan_docs(
        &self,
        plan: &PlanName,
        docs: Option<String>,
    ) -> anyhow::Result<DocsNamed> {
        // put the docs to the plan's book route, which a service older than C122 doesn't have
        let path = paths::book(plan);
        let request = self.http.put(self.url(&path)).json(&PlanDocs { docs });
        self.send(request, &path).await
    }

    /// Reads a plan's docs as a book of chapters, each with the page its doc is served as
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn book(&self, plan: &PlanName) -> anyhow::Result<Book> {
        // get the plan's chapters from its book route
        let path = paths::book(plan);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Lists a plan's current revision, each node at the version a put of it names
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn list(&self, plan: &PlanName) -> anyhow::Result<Listing> {
        let path = paths::nodes(plan);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Gets one node of a plan, at the version a put of it names
    ///
    /// A node that isn't in its plan comes back as a `not-found` error naming it.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `id` - The node's id
    pub async fn node(&self, plan: &PlanName, id: &str) -> anyhow::Result<StoredNode> {
        let path = paths::node(plan, id);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Puts and removes a batch of nodes in a plan, checked against the whole plan as one change
    ///
    /// A stale or refused batch is an answer, not an error, and writes nothing.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `batch` - The nodes to write and to remove, each with the version its writer read
    pub async fn batch(&self, plan: &PlanName, batch: &Puts) -> anyhow::Result<Checked> {
        let path = paths::nodes(plan);
        let request = self.http.post(self.url(&path)).json(batch);
        self.send(request, &path).await
    }

    /// Writes a whole plan as a new revision, made current at once, unless it would add a
    /// problem to the plan
    ///
    /// Every node the plan had and the write leaves out is gone from the revision it makes, and
    /// so is every page, if the write carries pages. A refused write is an answer, not an error,
    /// and writes nothing.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `write` - Every node of the plan, each a node of its own, and its pages if they change
    pub async fn write_plan(&self, plan: &PlanName, write: &PlanWrite) -> anyhow::Result<Rewrite> {
        let path = paths::revision(plan);
        let request = self.http.put(self.url(&path)).json(write);
        self.send(request, &path).await
    }

    /// Lists a plan's own pages, by path, each at the version an edit of it names
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn pages(&self, plan: &PlanName) -> anyhow::Result<PageListing> {
        let path = paths::pages(plan);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Gets one of a plan's own pages, at the version an edit of it names
    ///
    /// A page the plan doesn't keep comes back as a `not-found` error naming it.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `page` - The page's path
    pub async fn page(&self, plan: &PlanName, page: &str) -> anyhow::Result<StoredPage> {
        let path = paths::page(plan, page);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Writes or takes out one of a plan's own pages, checked against the whole plan
    ///
    /// A stale or refused edit is an answer, not an error, and writes nothing.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `edit` - The page to write or take out, with the version its writer read
    pub async fn edit_page(&self, plan: &PlanName, edit: &PageEdit) -> anyhow::Result<PageEdited> {
        let path = paths::pages(plan);
        let request = self.http.post(self.url(&path)).json(edit);
        self.send(request, &path).await
    }

    /// Finds a plan's problems, as `mdbook-reef check` finds a book's
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn check(&self, plan: &PlanName) -> anyhow::Result<Vec<Diagnostic>> {
        let path = paths::check(plan);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Says what work in a plan is ready to start, its entries of the queue first, then in plan
    /// order, and what the rest waits on
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn next(&self, plan: &PlanName) -> anyhow::Result<Next> {
        let path = paths::next(plan);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Says what work across a project's plans is ready to start, the queue's first, then each
    /// plan's in plan order, and what the rest waits on
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn project_next(&self, project: &str) -> anyhow::Result<Next> {
        let path = paths::project_next(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Lists a project's queue: each entry with its rank, and whether next takes it
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn queue(&self, project: &str) -> anyhow::Result<QueueListing> {
        let path = paths::queue(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Lists a plan's own queue: its entries, in their order, and what next makes of each
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn plan_queue(&self, plan: &PlanName) -> anyhow::Result<QueueListing> {
        let path = paths::plan_queue(plan);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Reads a page of a plan's queue's history: who queued, moved or took off each of its
    /// entries, newest first
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `older` - The cursor a page named, to read the one older than it, or `None` for the newest
    pub async fn plan_queue_history(
        &self,
        plan: &PlanName,
        older: Option<&str>,
    ) -> anyhow::Result<HistoryPage> {
        let path = paths::history_page(&paths::plan_queue_history(plan), older);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Reads a page of a project's queue's history: who queued, moved or took off each entry,
    /// newest first
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `older` - The cursor a page named, to read the one older than it, or `None` for the newest
    pub async fn queue_history(
        &self,
        project: &str,
        older: Option<&str>,
    ) -> anyhow::Result<HistoryPage> {
        let path = paths::history_page(&paths::queue_history(project), older);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Reads a page of a node's history: who made each edit, what it changed and when, newest
    /// first
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `older` - The cursor a page named, to read the one older than it, or `None` for the newest
    pub async fn node_history(
        &self,
        node: &NodeName,
        older: Option<&str>,
    ) -> anyhow::Result<HistoryPage> {
        let path = paths::history_page(&paths::node_history(node), older);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Reads a page of one of a plan's own pages' history, newest first
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `path` - The page's path
    /// * `older` - The cursor a page named, to read the one older than it, or `None` for the newest
    pub async fn page_history(
        &self,
        plan: &PlanName,
        path: &str,
        older: Option<&str>,
    ) -> anyhow::Result<HistoryPage> {
        let path = paths::history_page(&paths::page_history(plan, path), older);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Edits a project's queue: puts a node in it, moves one, or takes one off
    ///
    /// A node that isn't queued, or isn't in its plan, is an answer, not an error, and changes
    /// nothing.
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `edit` - The edit
    pub async fn edit_queue(&self, project: &str, edit: &QueueEdit) -> anyhow::Result<Queued> {
        let path = paths::queue(project);
        let request = self.http.post(self.url(&path)).json(edit);
        self.send(request, &path).await
    }

    /// Lists the work records of a project's nodes, by node, across its plans
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn work(&self, project: &str) -> anyhow::Result<Vec<WorkRecord>> {
        let path = paths::work(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Gets one node's work record, or `None` if it hasn't been started
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    pub async fn work_of(&self, node: &NodeName) -> anyhow::Result<Option<WorkRecord>> {
        let path = paths::node_work(node);
        // a node not started is an answer here, not an error
        match self.send(self.http.get(self.url(&path)), &path).await {
            Ok(record) => Ok(Some(record)),
            Err(error) if is_code(&error, ErrorCode::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Records a node's start, naming the version of the record it replaces, if any
    ///
    /// A stale start, or one of a node its plan doesn't have, is an answer, not an error, and
    /// writes nothing.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `start` - What the start read in the node's checkout
    pub async fn start_work(&self, node: &NodeName, start: &StartWork) -> anyhow::Result<Started> {
        let path = paths::node_work(node);
        let request = self.http.post(self.url(&path)).json(start);
        self.send(request, &path).await
    }

    /// Sends a capture of a node's changes, to be kept as a revision with its SHAs added to the
    /// node's record
    ///
    /// A node never started, or started again since the capture's start, is an answer, not an
    /// error, and keeps nothing. So is a capture its node's latest revision already holds.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `add` - The start the capture was taken from, what it read, and its files
    pub async fn add_capture(&self, node: &NodeName, add: &AddCapture) -> anyhow::Result<Captured> {
        let path = paths::captures(node);
        let request = self.http.post(self.url(&path)).json(add);
        self.send(request, &path).await
    }

    /// Lists a node's revisions, oldest first, each with its files but not their hunks
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    pub async fn revisions(&self, node: &NodeName) -> anyhow::Result<Vec<Revision>> {
        let path = paths::revisions(node);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Gets one of a node's revisions, or `None` if it has no such revision
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `revision` - The revision's number
    pub async fn revision(
        &self,
        node: &NodeName,
        revision: u32,
    ) -> anyhow::Result<Option<Revision>> {
        let path = paths::node_revision(node, revision);
        // a revision that isn't there is an answer here, not an error
        match self.send(self.http.get(self.url(&path)), &path).await {
            Ok(revision) => Ok(Some(revision)),
            Err(error) if is_code(&error, ErrorCode::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Gets one file of a node's revision, its hunks and all, or `None` if the revision doesn't
    /// list it
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `revision` - The revision's number
    /// * `file` - The path the file is shown under
    pub async fn revision_file(
        &self,
        node: &NodeName,
        revision: u32,
        file: &str,
    ) -> anyhow::Result<Option<FileDiff>> {
        let path = paths::revision_file(node, revision, file);
        // a file the revision doesn't list is an answer here, not an error
        match self.send(self.http.get(self.url(&path)), &path).await {
            Ok(file) => Ok(Some(file)),
            Err(error) if is_code(&error, ErrorCode::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Gets one comment thread of a plan, or `None` if no thread starts at that number
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    pub async fn thread(&self, plan: &PlanName, thread: u32) -> anyhow::Result<Option<Thread>> {
        let path = paths::thread(plan, thread);
        // a thread that isn't there is an answer here, not an error
        match self.send(self.http.get(self.url(&path)), &path).await {
            Ok(thread) => Ok(Some(thread)),
            Err(error) if is_code(&error, ErrorCode::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Lists every comment thread of a plan, by number
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn plan_threads(&self, plan: &PlanName) -> anyhow::Result<Vec<Thread>> {
        let path = paths::plan_comments(plan);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Lists the comment threads on one node, by number
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    pub async fn node_threads(&self, node: &NodeName) -> anyhow::Result<Vec<Thread>> {
        let path = paths::node_comments(node);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Replies in a thread of a plan
    ///
    /// A thread that isn't there is an answer, not an error.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    /// * `reply` - The reply's body, and who writes it
    pub async fn reply(
        &self,
        plan: &PlanName,
        thread: u32,
        reply: &NewReply,
    ) -> anyhow::Result<Commented> {
        let path = paths::replies(plan, thread);
        let request = self.http.post(self.url(&path)).json(reply);
        self.send(request, &path).await
    }

    /// Resolves a thread by hand, a revision linked to it, over the version read
    ///
    /// A thread changed since, or a revision that isn't stored, is an answer, not an error.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    /// * `resolve` - The revision that answered it, and the version it was resolved against
    pub async fn resolve_thread(
        &self,
        plan: &PlanName,
        thread: u32,
        resolve: &ResolveThread,
    ) -> anyhow::Result<Resolved> {
        let path = paths::thread_resolution(plan, thread);
        let request = self.http.post(self.url(&path)).json(resolve);
        self.send(request, &path).await
    }

    /// Lists every loop, by pane
    pub async fn loops(&self) -> anyhow::Result<Vec<Loop>> {
        let path = paths::LOOPS;
        self.send(self.http.get(self.url(path)), path).await
    }

    /// Lists the loops feeding one project, by pane
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn project_loops(&self, project: &str) -> anyhow::Result<Vec<Loop>> {
        let path = paths::project_loops(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Changes a pane's loop
    ///
    /// A pane with no loop, a busy one, or a node held by another loop is an answer, not an
    /// error.
    ///
    /// # Arguments
    ///
    /// * `edit` - The pane, and the change to its loop
    pub async fn edit_loop(&self, edit: &EditLoop) -> anyhow::Result<LoopEdited> {
        let path = paths::LOOPS;
        let request = self.http.post(self.url(path)).json(edit);
        self.send(request, path).await
    }

    /// Lists a node's decisions, by number, each plan rendered
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    pub async fn decisions(&self, node: &NodeName) -> anyhow::Result<Vec<Decision>> {
        let path = paths::node_decisions(node);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Keeps a node's decisions, each by the call that asked
    ///
    /// A node that isn't in its plan is an answer, not an error.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `record` - The decisions, as read from a session's transcript
    pub async fn record_decisions(
        &self,
        node: &NodeName,
        record: &RecordDecisions,
    ) -> anyhow::Result<DecisionsRecorded> {
        let path = paths::node_decisions(node);
        let request = self.http.post(self.url(&path)).json(record);
        self.send(request, &path).await
    }

    /// Attaches a runner's session, or attaches it again with what it holds
    ///
    /// # Arguments
    ///
    /// * `attach` - The runner's pane, project, host and checkout, and what it holds already
    pub async fn attach_session(&self, attach: &Attach) -> anyhow::Result<Session> {
        let path = paths::SESSIONS;
        let request = self.http.post(self.url(path)).json(attach);
        self.send(request, path).await
    }

    /// Reports a runner's entries and where it stands, or `None` if the service doesn't know
    /// its session, as after the service restarted, so the runner attaches again
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `report` - Its entries since its last report, and where it stands now
    pub async fn report_session(
        &self,
        id: &str,
        report: &Report,
    ) -> anyhow::Result<Option<Session>> {
        let path = paths::session(id);
        let request = self.http.post(self.url(&path)).json(report);
        // a session the service has never heard of is an answer here, not an error
        match self.send(request, &path).await {
            Ok(session) => Ok(Some(session)),
            Err(error) if is_code(&error, ErrorCode::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Lets go of a runner's session
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    pub async fn detach_session(&self, id: &str) -> anyhow::Result<Session> {
        let path = paths::session(id);
        self.send(self.http.delete(self.url(&path)), &path).await
    }

    /// The orders for a runner after the last it read, waiting up to a while for one, or
    /// `None` if the service doesn't know its session
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `after` - The number of the last order read, or 0
    /// * `wait` - How long the service may wait for one
    pub async fn session_orders(
        &self,
        id: &str,
        after: u64,
        wait: Duration,
    ) -> anyhow::Result<Option<Vec<Ordered>>> {
        let path = format!(
            "{}?after={after}&wait={}",
            paths::session_orders(id),
            wait.as_millis()
        );
        // a session the service has never heard of is an answer here, not an error
        match self.send(self.http.get(self.url(&path)), &path).await {
            Ok(orders) => Ok(Some(orders)),
            Err(error) if is_code(&error, ErrorCode::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// A session's transcript after the entries read already
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `after` - How many of its entries were read already
    pub async fn session_transcript(&self, id: &str, after: u64) -> anyhow::Result<Transcript> {
        let path = format!("{}?after={after}", paths::session(id));
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Asks something of a runner's session as the app does: changes its loop, or orders an
    /// answer or a reply
    ///
    /// A prompt that doesn't wait, a Claude that isn't waiting, or a runner that is gone is an
    /// answer, not an error.
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `act` - What is asked of it
    pub async fn act_on_session(&self, id: &str, act: &SessionAct) -> anyhow::Result<SessionActed> {
        let path = paths::session_orders(id);
        let request = self.http.post(self.url(&path)).json(act);
        self.send(request, &path).await
    }

    /// Lists the sessions of runners taking nodes from a project
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn project_sessions(&self, project: &str) -> anyhow::Result<Vec<Session>> {
        let path = paths::project_sessions(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Opens a test run, or opens it again with what its client holds
    ///
    /// # Arguments
    ///
    /// * `open` - The run's id, project, node, host, checkout and command, and what it holds
    pub async fn open_test_run(&self, open: &OpenRun) -> anyhow::Result<TestRun> {
        let path = paths::TEST_RUNS;
        let request = self.http.post(self.url(path)).json(open);
        self.send(request, path).await
    }

    /// Reports where a test run and its binaries stand, or `None` if the service doesn't hold
    /// the run, as after the service restarted, so its client opens it again
    ///
    /// # Arguments
    ///
    /// * `id` - The run's id
    /// * `report` - Where the run and its binaries stand, and the failures since its last report
    pub async fn report_test_run(
        &self,
        id: &str,
        report: &RunReport,
    ) -> anyhow::Result<Option<TestRun>> {
        let path = paths::test_run(id);
        let request = self.http.post(self.url(&path)).json(report);
        // a run the service doesn't hold is an answer here, not an error
        match self.send(request, &path).await {
            Ok(run) => Ok(Some(run)),
            Err(error) if is_code(&error, ErrorCode::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// A test run as it stands, each failure with its output
    ///
    /// # Arguments
    ///
    /// * `id` - The run's id
    pub async fn test_run(&self, id: &str) -> anyhow::Result<TestRun> {
        let path = paths::test_run(id);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Lists a project's test runs, newest first, each failure's output left out
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn project_test_runs(&self, project: &str) -> anyhow::Result<Vec<TestRun>> {
        let path = paths::project_test_runs(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// The build of the `reef` the service hands its users, or `None` if it hands them none
    ///
    /// A service older than reef's updates has no such route: it answers 404, or reef-app's page
    /// for a path it doesn't know, or 401 where it asks every caller to sign in. Each is no client.
    pub async fn served_client(&self) -> anyhow::Result<Option<Build>> {
        let path = paths::CLIENT;
        let (request, _) = self.signed(self.http.get(self.url(path)))?;
        let response = request
            .send()
            .await
            .map_err(|error| self.unreachable(error))?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|error| anyhow::anyhow!("reading {path}'s answer: {error}"))?;
        match status {
            // a build, or a page that isn't one, as an app older than the route serves
            200 => Ok(serde_json::from_str(&text).ok()),
            // no client to hand out, or no route at all
            401 | 404 => Ok(None),
            _ => decode::<Build>(status, path, &text).map(Some),
        }
    }

    /// Opens the `reef` the service hands its users, its bytes to read as they come
    pub async fn client_binary(&self) -> anyhow::Result<reqwest::Response> {
        let path = paths::CLIENT_BINARY;
        let (request, _) = self.signed(self.http.get(self.url(path)))?;
        let response = request
            .send()
            .await
            .map_err(|error| self.unreachable(error))?;
        // anything but the bytes is read as the error it is
        let status = response.status().as_u16();
        if status != 200 {
            let text = response.text().await.unwrap_or_default();
            return Err(decode::<serde_json::Value>(status, path, &text)
                .err()
                .unwrap_or_else(|| anyhow::anyhow!("{status} from {path}: {text}")));
        }
        Ok(response)
    }

    /// The full URL of a route's path
    ///
    /// # Arguments
    ///
    /// * `path` - The route's path, from `reef_api::paths`
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Sends a request and reads its answer as a route's type, or as the error it answered with
    ///
    /// # Arguments
    ///
    /// * `request` - The request, ready to send
    /// * `path` - The route's path, for messages
    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        path: &str,
    ) -> anyhow::Result<T> {
        self.send_as(request, path, false).await
    }

    /// Sends a request and reads its answer, a 401 read as the route's own error when the
    /// request signs in, and as a want of signing in otherwise
    ///
    /// # Arguments
    ///
    /// * `request` - The request, ready to send
    /// * `path` - The route's path, for messages
    /// * `signs_in` - Whether the request trades a name and password for a token
    async fn send_as<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        path: &str,
        signs_in: bool,
    ) -> anyhow::Result<T> {
        // the token kept for the service, if one is, and who writes
        let (request, token) = self.signed(request)?;
        // a service that isn't there is said plainly, with how to start it
        let response = request
            .send()
            .await
            .map_err(|error| self.unreachable(error))?;
        // a newer `reef` the service hands out starts this one's updater
        self.watch.notice(response.headers());
        let status = response.status().as_u16();
        // the whole body, then read as whichever answer its status calls for
        let text = response
            .text()
            .await
            .map_err(|error| anyhow::anyhow!("reading {path}'s answer: {error}"))?;
        // a service asking to sign in says how, but a wrong name or password says that
        if status == 401 && !signs_in {
            let held = token.is_some().then_some(&self.held);
            return Err(signed_out(&self.base, held).into());
        }
        decode(status, path, &text)
    }

    /// A request with the token kept for the service, if one is, and Claude's word that it
    /// writes for the token's user, if it does, returned with the token it sends
    ///
    /// # Arguments
    ///
    /// * `request` - The request, ready to send but for these
    fn signed(
        &self,
        request: reqwest::RequestBuilder,
    ) -> anyhow::Result<(reqwest::RequestBuilder, Option<String>)> {
        // the token, read now, so a login since this client was made is sent
        let token = self.token()?;
        let request = match &token {
            Some(token) => request.bearer_auth(token),
            None => request,
        };
        // and who writes
        let request = match self.writer {
            Writer::Claude => request.header(WRITER_HEADER, Writer::Claude.as_str()),
            Writer::User => request,
        };
        Ok((request, token))
    }

    /// Says why a request got no answer at all
    ///
    /// # Arguments
    ///
    /// * `error` - What the HTTP client failed with
    fn unreachable(&self, error: reqwest::Error) -> anyhow::Error {
        // every cause, since the outermost says only that the request failed
        let causes =
            std::iter::successors(std::error::Error::source(&error), |cause| cause.source())
                .collect::<Vec<_>>();
        // a connection refused is nothing listening: the service not running, which only
        // starting it fixes
        let refused = causes.iter().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionRefused)
        });
        if error.is_connect() && refused {
            return anyhow::anyhow!(
                "reef's service isn't running at {} ({}): start it with `reef-serve`",
                self.base,
                self.origin
            );
        }
        // anything else, such as a certificate refused, a name that resolves nowhere or a
        // connection dropped partway, with every cause
        let causes = causes.iter().map(ToString::to_string).collect::<Vec<_>>();
        let said = match causes.is_empty() {
            true => format!("calling reef's service at {}: {error}", self.base),
            false => format!(
                "calling reef's service at {}: {error}: {}",
                self.base,
                causes.join(": ")
            ),
        };
        // a certificate refused says what to trust
        match causes.iter().any(|cause| cause.contains("certificate")) {
            true => anyhow::anyhow!(
                "{said}: trust the CA its certificate is signed by, its `ca.pem` if it is reef's \
                 own, with `[api] ca`, or on its host with `[api] certs`"
            ),
            false => anyhow::anyhow!("{said}"),
        }
    }
}

/// A service's root as `[api] url` names it: http or https, a host and maybe a port, and nothing
/// after, with no `/` at its end, so one root is kept by one spelling of it
///
/// # Arguments
///
/// * `url` - The URL the config names
fn root(url: &str) -> Result<String, String> {
    // a URL at all, its host lowercased and a scheme's own port dropped
    let parsed =
        reqwest::Url::parse(url).map_err(|error| format!("which is not a URL: {error}"))?;
    // the two schemes the service speaks
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "whose scheme `{}` is not http or https",
            parsed.scheme()
        ));
    }
    // somewhere to call
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err("which names no host".to_string());
    }
    // a login, not a URL, says who is calling
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("which carries a user: sign in with `reef login` instead".to_string());
    }
    // the root alone, since every route's path is added to it
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("which is not a root: name the scheme, host and port alone".to_string());
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

/// Whether a root names this host, by `localhost` or a loopback address, as a tunnel to another
/// host's loopback port does
///
/// # Arguments
///
/// * `base` - The root, such as `https://localhost:11180`
fn is_loopback_root(base: &str) -> bool {
    // the host it names, an IPv6 address out of its brackets
    let Ok(url) = reqwest::Url::parse(base) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    // this host by name, or a loopback address
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The roots a PEM file holds, which the service's certificate is trusted by
///
/// # Arguments
///
/// * `path` - The PEM file
fn roots(path: &Path) -> anyhow::Result<Vec<reqwest::Certificate>> {
    // every certificate in it, and at least one
    let pem = std::fs::read(path)?;
    let roots = reqwest::Certificate::from_pem_bundle(&pem)?;
    anyhow::ensure!(!roots.is_empty(), "it holds no PEM certificate");
    Ok(roots)
}

/// Reads an answer's body by its status: a success as the route's type, a failure as an
/// [`ApiError`], else as the route's own refusal, such as a taken name
///
/// # Arguments
///
/// * `status` - The answer's HTTP status
/// * `path` - The route's path, for messages
/// * `text` - The answer's body
pub fn decode<T: DeserializeOwned>(status: u16, path: &str, text: &str) -> anyhow::Result<T> {
    // a success is the route's answer
    if (200..300).contains(&status) {
        return serde_json::from_str(text).map_err(|error| {
            anyhow::anyhow!("{status} from {path} is not the answer expected ({error}): {text}")
        });
    }
    // a route the service doesn't have answers with nothing: it is older than this client
    if status == 404 && text.trim().is_empty() {
        anyhow::bail!(
            "reef's service has no route {path}, so it is older than this `reef`: install \
             `reef-serve` again and restart it"
        );
    }
    // an error names its kind, and its message is the service's
    if let Ok(error) = serde_json::from_str::<ApiError>(text) {
        return Err(error.into());
    }
    // else a refusal the route answers with its outcome, or something no route says
    serde_json::from_str(text).map_err(|_| anyhow::anyhow!("{status} from {path}: {text}"))
}

/// The error a 401 comes back as: the service asks the caller to sign in, and how to
///
/// # Arguments
///
/// * `address` - Where the service is called, its root
/// * `held` - Where the token sent came from, or `None` if none was sent
pub fn signed_out(address: &str, held: Option<&Held>) -> ApiError {
    // no token is a login to make, and a refused one a login to make again
    let message = match held {
        None => format!(
            "reef's service at {address} asks you to sign in, and this `reef` has no token for \
             it: run `reef login`"
        ),
        Some(Held::Token(_)) => format!(
            "reef's service at {address} refused the token in `${}`: set it to a token the \
             service issued, or unset it and run `reef login`",
            credentials::TOKEN_VAR
        ),
        Some(held) => format!(
            "reef's service at {address} refused the token kept in {}: run `reef login` again",
            held.describe()
        ),
    };
    ApiError::new(ErrorCode::Unauthorized, message)
}

/// Whether an error is the API's error of one kind
///
/// # Arguments
///
/// * `error` - The error a call returned
/// * `code` - The kind to look for
pub fn is_code(error: &anyhow::Error, code: ErrorCode) -> bool {
    error
        .downcast_ref::<ApiError>()
        .is_some_and(|error| error.code == code)
}

#[cfg(test)]
mod tests {
    use super::{Client, decode, is_code, root, signed_out};
    use crate::conf::Conf;
    use crate::credentials::Held;
    use reef_api::{AddedProject, ErrorCode, Project};
    use std::path::PathBuf;

    /// A 401 says to log in, naming the address and where a refused token came from
    #[test]
    fn unauthorized_says_to_log_in() {
        let address = "http://127.0.0.1:12180";
        // no token sent: log in
        let none = signed_out(address, None);
        assert_eq!(none.code, ErrorCode::Unauthorized);
        assert!(none.message.contains("127.0.0.1:12180"), "{}", none.message);
        assert!(
            none.message.contains("run `reef login`"),
            "{}",
            none.message
        );
        // a kept token refused: log in again, naming the file
        let file = Held::File(PathBuf::from("/home/ada/.config/reef/credentials.toml"));
        let refused = signed_out(address, Some(&file));
        assert!(
            refused.message.contains("credentials.toml"),
            "{}",
            refused.message
        );
        assert!(refused.message.contains("again"), "{}", refused.message);
        // a token given outright: the variable, never the token
        let outright = signed_out(address, Some(&Held::Token("reef_secret".into())));
        assert!(
            outright.message.contains("$REEF_TOKEN"),
            "{}",
            outright.message
        );
        assert!(
            !outright.message.contains("reef_secret"),
            "{}",
            outright.message
        );
    }

    /// A success reads as the route's answer
    #[test]
    fn success_reads_as_the_answer() {
        // a listing of one project
        let text = r#"[{"name": "reef", "repo": "/src/reef"}]"#;
        let projects: Vec<Project> = decode(200, "/p", text).expect("a listing reads");
        assert_eq!(projects[0].name, "reef");
        assert_eq!(projects[0].repo, PathBuf::from("/src/reef"));
        // and one that isn't the answer expected says what it was
        let wrong = decode::<Vec<Project>>(200, "/p", "{}").expect_err("an object read as a list");
        assert!(wrong.to_string().contains("200 from /p"), "{wrong}");
    }

    /// An error answers as the API's error, with its code, and a refusal as its outcome
    #[test]
    fn errors_and_refusals_read_as_themselves() {
        // a 404 naming its kind comes back as the error, which a caller can match on
        let text = r#"{"code": "not-found", "message": "no project is registered for /x"}"#;
        let error = decode::<Project>(404, "/f", text).expect_err("an error read as a project");
        assert!(is_code(&error, ErrorCode::NotFound));
        assert!(!is_code(&error, ErrorCode::Internal));
        assert_eq!(error.to_string(), "no project is registered for /x");
        // a taken name is the route's answer, not an error
        let taken: AddedProject = decode(409, "/p", r#""name-taken""#).expect("a refusal reads");
        assert_eq!(taken, AddedProject::NameTaken);
        // and a body no route sends names its status
        let odd = decode::<AddedProject>(502, "/p", "bad gateway").expect_err("text read");
        assert_eq!(odd.to_string(), "502 from /p: bad gateway");
        // a route a service older than this client lacks says to restart it
        let old = decode::<AddedProject>(404, "/w", "").expect_err("nothing read");
        assert!(old.to_string().contains("no route /w"), "{old}");
        assert!(old.to_string().contains("restart it"), "{old}");
    }

    /// A root is read as one spelling of it, and a URL that isn't one is refused
    #[test]
    fn url_names_a_root() {
        // a trailing `/`, a capital, or https's own port are one root
        for url in ["https://Reef.Example:12443/", "https://reef.example:12443"] {
            assert_eq!(root(url).unwrap(), "https://reef.example:12443");
        }
        assert_eq!(
            root("https://reef.example:443").unwrap(),
            "https://reef.example"
        );
        assert_eq!(
            root("http://127.0.0.1:12180").unwrap(),
            "http://127.0.0.1:12180"
        );
        // another scheme, a path, a query, a user or no URL at all is refused
        for bad in [
            "ftp://reef.example",
            "https://reef.example/api",
            "https://reef.example/?a=1",
            "https://ada:pw@reef.example",
            "reef.example:12443",
        ] {
            assert!(root(bad).is_err(), "`{bad}` was read as a root");
        }
    }

    /// A client named by `url` calls it, and one whose `ca` can't be read says which key
    #[test]
    fn url_and_ca_build_the_client() {
        // the root `url` names wins over the address
        let mut conf = Conf::default();
        conf.api.url = Some("https://reef.example:12443/".to_string());
        let client = Client::new(&conf).unwrap();
        assert_eq!(client.base(), "https://reef.example:12443");
        // a URL that isn't a root names the key
        conf.api.url = Some("https://reef.example/api".to_string());
        let error = Client::new(&conf)
            .err()
            .expect("a path was taken for a root");
        assert!(error.to_string().contains("`api.url`"), "{error}");
        // and a root file that isn't there, or holds no certificate, names its key
        conf.api.url = None;
        conf.api.ca = Some(PathBuf::from("/nowhere/reef-ca.pem"));
        let error = Client::new(&conf).err().expect("a missing root was read");
        assert!(error.to_string().contains("`api.ca`"), "{error}");
        let empty = std::env::temp_dir().join(format!("reef-ca-{}.pem", std::process::id()));
        std::fs::write(&empty, "not a certificate\n").unwrap();
        conf.api.ca = Some(empty.clone());
        let error = Client::new(&conf).err().expect("an empty root was read");
        assert!(error.to_string().contains("no PEM certificate"), "{error}");
        std::fs::remove_file(&empty).unwrap();
    }

    /// With nothing listening, the error names the address, where it came from, and the fix
    #[tokio::test]
    async fn unreachable_service_says_how_to_start_it() {
        // a loopback port that was free a moment ago, and is closed now
        let address = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let mut conf = Conf::default();
        conf.api.address = address;
        let client = Client::new(&conf).unwrap();
        // any call says the service isn't running, where it looked, and how to start it
        let error = client.projects().await.expect_err("a closed port answered");
        let message = error.to_string();
        assert!(message.contains("isn't running"), "{message}");
        assert!(message.contains(&address.to_string()), "{message}");
        assert!(message.contains("`api.address`"), "{message}");
        assert!(message.contains("`reef-serve`"), "{message}");
    }

    /// This host's service is called over HTTPS at its address, and a root names this host by
    /// `localhost` or a loopback address alone
    #[test]
    fn this_hosts_service_is_called_over_https() {
        // the default address, over HTTPS, its root's spelling the one a login is kept by
        let client = Client::new(&Conf::default()).unwrap();
        assert_eq!(client.base(), "https://127.0.0.1:12180");
        // this host, however it is named, and through a tunnel to another's loopback port
        for root in [
            "https://localhost:11180",
            "https://127.0.0.1:12180",
            "https://[::1]:12180",
        ] {
            assert!(super::is_loopback_root(root), "{root}");
        }
        // and not another host
        for root in ["https://reef.example:12443", "https://10.0.0.5:12443"] {
            assert!(!super::is_loopback_root(root), "{root}");
        }
    }
}
