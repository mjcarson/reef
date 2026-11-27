//! reef's MCP server against a served service: JSON-RPC lines in, as Claude Code sends them
//!
//! Each case serves the in-memory twin on a loopback port of its own, as the CLI's suite does,
//! and runs `reef mcp`'s tools over an in-process pipe instead of standard input and output.

// with `embedded`, the shared fixtures' pool open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef::Conf;
use reef::cli::PlanPick;
use reef::mcp::{self, Tools};
use reef_api::{EditLoop, LoopEdit, Pane, Place, QueueEdit};
use reef_model::PlanName;
use reef_service::MemoryBackend;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};

use common::http::Ax;

/// The handshake's protocol version: the newest that still has an `initialize`
const PROTOCOL: &str = "2025-11-25";

/// A client of the MCP server, speaking JSON-RPC a line at a time
struct Session {
    /// The server's answers, a line each
    answers: Lines<BufReader<ReadHalf<DuplexStream>>>,
    /// Where the client's messages go
    sends: WriteHalf<DuplexStream>,
    /// The id the next request takes
    id: u64,
}

impl Session {
    /// Starts the server over a pipe, and shakes hands with it as Claude Code does
    ///
    /// # Arguments
    ///
    /// * `ax` - The served service, whose plan the tools work in
    async fn open(ax: &Ax<MemoryBackend>) -> anyhow::Result<Session> {
        Session::open_in(ax, None).await
    }

    /// Starts the server over a pipe in a session's pane, and shakes hands with it
    ///
    /// # Arguments
    ///
    /// * `ax` - The served service, whose plan the tools work in
    /// * `pane` - The pane the session runs in, whose loop next_task takes nodes for
    async fn open_in(ax: &Ax<MemoryBackend>, pane: Option<Pane>) -> anyhow::Result<Session> {
        Session::open_over(ax, client_of(ax)?, pane).await
    }

    /// Starts the server over a pipe with a client of its own, in a session's pane, and shakes
    /// hands with it
    ///
    /// # Arguments
    ///
    /// * `ax` - The served service, whose plan the tools work in
    /// * `client` - The client the tools call the service with
    /// * `pane` - The pane the session runs in, whose loop next_task takes nodes for
    async fn open_over(
        ax: &Ax<MemoryBackend>,
        client: reef::Client,
        pane: Option<Pane>,
    ) -> anyhow::Result<Session> {
        // the tools over a client of the served twin, started in the plan's repo
        let tools = Tools::new(client, ax.repo.clone(), PlanPick::default()).in_pane(pane);
        // served over one end of a pipe, the session holding the other
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        let (reader, writer) = tokio::io::split(theirs);
        tokio::spawn(mcp::serve(tools, reader, writer));
        let (answers, sends) = tokio::io::split(ours);
        let mut session = Session {
            answers: BufReader::new(answers).lines(),
            sends,
            id: 0,
        };
        // the handshake, then the notice that it is done
        let params = json!({
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": {"name": "reef-tests", "version": "0"},
        });
        let info = session.request("initialize", params).await?;
        assert_eq!(info["serverInfo"]["name"], "reef", "{info}");
        session
            .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await?;
        Ok(session)
    }

    /// Sends one message, as a line of JSON
    ///
    /// # Arguments
    ///
    /// * `message` - The message
    async fn send(&mut self, message: Value) -> anyhow::Result<()> {
        let mut line = serde_json::to_vec(&message)?;
        line.push(b'\n');
        self.sends.write_all(&line).await?;
        Ok(())
    }

    /// Sends a request and waits for its answer's result
    ///
    /// # Arguments
    ///
    /// * `method` - The request's method
    /// * `params` - Its parameters
    async fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        // a fresh id, so the answer can be told from any other message
        self.id += 1;
        let id = self.id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        loop {
            let line =
                self.answers.next_line().await?.ok_or_else(|| {
                    anyhow::anyhow!("the server closed before answering {method}")
                })?;
            let answer: Value = serde_json::from_str(&line)?;
            // a notice or a request of the server's own is passed over
            if answer["id"] != json!(id) {
                continue;
            }
            anyhow::ensure!(answer.get("error").is_none(), "{method} failed: {answer}");
            return Ok(answer["result"].clone());
        }
    }

    /// Calls a tool, returning its text and whether it was an error
    ///
    /// # Arguments
    ///
    /// * `name` - The tool's name
    /// * `arguments` - Its arguments
    async fn call(&mut self, name: &str, arguments: Value) -> anyhow::Result<(String, bool)> {
        let params = json!({"name": name, "arguments": arguments});
        let result = self.request("tools/call", params).await?;
        // a tool answers in text, and says whether it failed
        let text = result["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("{name} answered no text: {result}"))?
            .to_string();
        let failed = result["isError"].as_bool().unwrap_or(false);
        Ok((text, failed))
    }
}

/// The CLI's client of a served twin, as the MCP server holds one
///
/// # Arguments
///
/// * `ax` - The served service
fn client_of(ax: &Ax<MemoryBackend>) -> anyhow::Result<reef::Client> {
    // a config naming the twin's port and where reef's own CA is kept, and nothing else
    let mut conf = Conf::default();
    conf.api.address = ax.address;
    conf.api.certs = Some(ax.certs.clone());
    reef::Client::new(&conf)
}

/// The CLI's client of a served twin, signed in with a token kept in a credentials file of the
/// case's own, as `reef login` keeps one
///
/// # Arguments
///
/// * `ax` - The served service
/// * `user` - The user the token signs in
/// * `token` - The token
fn signed_client_of(
    ax: &Ax<MemoryBackend>,
    user: &str,
    token: &str,
) -> anyhow::Result<reef::Client> {
    // the login kept for the root the client calls, and the config naming that file
    let credentials = ax.dir.join(format!("credentials-{user}.toml"));
    let login = reef::credentials::Login {
        user: user.to_string(),
        token: token.to_string(),
    };
    reef::credentials::keep(&credentials, &format!("https://{}", ax.address), login)?;
    let mut conf = Conf::default();
    conf.api.address = ax.address;
    conf.api.certs = Some(ax.certs.clone());
    conf.credentials = Some(credentials);
    reef::Client::new(&conf)
}

/// Claude's writes through the tools name Claude, for the user the client's token signs in,
/// and the same client's own writes outside the tools name the user alone
#[tokio::test]
async fn claudes_writes_are_marked_for_its_user() -> anyhow::Result<()> {
    // a twin asking every caller to sign in, and ada, its first user, signed in
    let ax = common::Fx::memory("mcp-claudes-writes")
        .await?
        .api_signing_in()
        .await?;
    common::sign_in_cases::first_admin(&ax, "ada").await?;
    let request = reef_api::SignIn {
        name: "ada".to_string(),
        password: common::sign_in_cases::PASSWORD.to_string(),
        label: Some("a case".to_string()),
    };
    let (_, issued): (_, reef_api::IssuedToken) =
        ax.client.post(reef_api::paths::TOKENS, &request).await?;
    let mut session =
        Session::open_over(&ax, signed_client_of(&ax, "ada", &issued.token)?, None).await?;
    // a node put and set in progress through the tools
    let yaml = "id: C1\ntitle: C1 Some work\ndescription:\n  why: Without C1, something is \
                missing.\n  what: With C1, it is there.\n";
    let (put, failed) = session.call("put", json!({"yaml": yaml})).await?;
    assert!(!failed, "{put}");
    let (status, failed) = session
        .call("status", json!({"id": "C1", "status": "in-progress"}))
        .await?;
    assert!(!failed, "{status}");
    // each is Claude's, for ada, in the node's history
    let client = signed_client_of(&ax, "ada", &issued.token)?;
    let node = ax.plan.node("C1")?;
    let history = client.node_history(&node, None).await?;
    let authors: Vec<Option<String>> = history
        .edits
        .iter()
        .map(|logged| logged.author.describe())
        .collect();
    assert_eq!(
        authors,
        vec![
            Some("Claude, for ada".to_string()),
            Some("Claude, for ada".to_string())
        ]
    );
    // a thread ada starts herself, and Claude's reply to it through the tools
    let new = reef_api::NewThread {
        target: reef_api::Target::Node,
        body: "Why?".to_string(),
        by: reef_api::Writer::User,
        review: false,
    };
    let ada = ax.client.signed(&issued.token);
    let (_, commented): (_, reef_api::Commented) = ada
        .post(&reef_api::paths::node_comments(&node), &new)
        .await?;
    let reef_api::Commented::Added { thread, .. } = commented else {
        anyhow::bail!("the thread wasn't kept: {commented:?}");
    };
    let (replied, failed) = session
        .call(
            "reply",
            json!({"thread": thread.number, "body": "Because."}),
        )
        .await?;
    assert!(!failed, "{replied}");
    let thread = client
        .thread(&ax.plan, thread.number)
        .await?
        .expect("the thread is kept");
    let said: Vec<Option<String>> = thread
        .comments
        .iter()
        .map(|comment| comment.author.describe())
        .collect();
    assert_eq!(
        said,
        vec![Some("ada".to_string()), Some("Claude, for ada".to_string())]
    );
    assert!(!thread.wants_answer());
    Ok(())
}

/// The `history` tool prints a node's history as `reef node log` does, naming Claude's edits as
/// Claude's for the user signed in, and a cursor no page named comes back as a tool error
#[tokio::test]
async fn the_history_tool_prints_node_log() -> anyhow::Result<()> {
    // a twin asking every caller to sign in, and ada, its first user, signed in
    let ax = common::Fx::memory("mcp-history-tool")
        .await?
        .api_signing_in()
        .await?;
    common::sign_in_cases::first_admin(&ax, "ada").await?;
    let request = reef_api::SignIn {
        name: "ada".to_string(),
        password: common::sign_in_cases::PASSWORD.to_string(),
        label: Some("a case".to_string()),
    };
    let (_, issued): (_, reef_api::IssuedToken) =
        ax.client.post(reef_api::paths::TOKENS, &request).await?;
    let mut session =
        Session::open_over(&ax, signed_client_of(&ax, "ada", &issued.token)?, None).await?;
    // a node never edited says so, as the command does
    let (never, failed) = session.call("history", json!({"id": "C1"})).await?;
    assert!(!failed, "{never}");
    assert_eq!(never, "no edit of `reef/central/C1` is kept\n");
    // a node put and set in progress through the tools
    let yaml = "id: C1\ntitle: C1 Some work\ndescription:\n  why: Without C1, something is \
                missing.\n  what: With C1, it is there.\n";
    let (put, failed) = session.call("put", json!({"yaml": yaml})).await?;
    assert!(!failed, "{put}");
    let (status, failed) = session
        .call("status", json!({"id": "C1", "status": "in-progress"}))
        .await?;
    assert!(!failed, "{status}");
    // the tool's text is the command's, each edit Claude's for ada
    let (told, failed) = session.call("history", json!({"id": "C1"})).await?;
    assert!(!failed, "{told}");
    let client = signed_client_of(&ax, "ada", &issued.token)?;
    let mut printed = Vec::new();
    reef::history::node(&client, &ax.plan.node("C1")?, None, false, &mut printed).await?;
    assert_eq!(told, String::from_utf8(printed)?);
    let lines: Vec<&str> = told.lines().collect();
    assert_eq!(lines.len(), 2, "{told}");
    assert!(
        lines[0].starts_with("v2 ")
            && lines[0].ends_with("  Claude, for ada: status unset → in-progress"),
        "{told}"
    );
    assert!(
        lines[1].starts_with("v1 ") && lines[1].ends_with("  Claude, for ada: added"),
        "{told}"
    );
    // and a cursor no page named is a tool error, saying where one comes from
    let (bad, failed) = session
        .call("history", json!({"id": "C1", "older": "nope"}))
        .await?;
    assert!(failed, "{bad}");
    assert!(bad.contains("no history's cursor"), "{bad}");
    Ok(())
}

/// Exactly the tools that only read say so, the tools say what the CLI does, and a put that
/// breaks the plan comes back as a tool error with check's message
#[tokio::test]
async fn read_only_tools_say_so_and_writes_say_why_they_fail() -> anyhow::Result<()> {
    // a twin with `reef/central` listed, served on a port of its own
    let ax = common::Fx::memory("mcp-tools").await?.api().await?;
    let mut session = Session::open(&ax).await?;
    // every tool is listed, and only get, check and next are marked read-only
    let listed = session.request("tools/list", json!({})).await?;
    let mut tools: Vec<(String, bool)> = listed["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no tools: {listed}"))?
        .iter()
        .map(|tool| {
            let name = tool["name"].as_str().unwrap_or_default().to_string();
            let read_only = tool["annotations"]["readOnlyHint"].as_bool() == Some(true);
            (name, read_only)
        })
        .collect();
    tools.sort();
    let expected = [
        ("accept", false),
        ("check", true),
        ("comments", true),
        ("get", true),
        ("history", true),
        ("next", true),
        ("next_task", false),
        ("put", false),
        ("reply", false),
        ("resolve", false),
        ("status", false),
    ];
    let expected: Vec<(String, bool)> = expected
        .iter()
        .map(|(name, read_only)| (name.to_string(), *read_only))
        .collect();
    assert_eq!(tools, expected);
    // a node put as YAML, and read back with its version line
    let yaml = "id: C1\ntitle: C1 Some work\ndescription:\n  why: Without C1, something is \
                missing.\n  what: With C1, it is there.\n";
    let (put, failed) = session.call("put", json!({"yaml": yaml})).await?;
    assert_eq!((put.as_str(), failed), ("put `C1` at version 1\n", false));
    let (got, failed) = session.call("get", json!({"id": "C1"})).await?;
    assert!(!failed, "{got}");
    assert!(got.starts_with("# version: 1\nid: C1\n"), "{got}");
    // ready to start, then started, and the plan whole
    let (next, _) = session.call("next", json!({})).await?;
    assert!(
        next.starts_with("Ready to start (1), in plan order:\n  C1"),
        "{next}"
    );
    let (status, failed) = session
        .call("status", json!({"id": "C1", "status": "in-progress"}))
        .await?;
    assert_eq!(
        (status.as_str(), failed),
        ("`C1` is in-progress, at version 2\n", false)
    );
    let (checked, failed) = session.call("check", json!({})).await?;
    assert_eq!(
        (checked.as_str(), failed),
        ("`reef/central` has no problems\n", false)
    );
    // a put that adds a problem is a tool error with check's message, and writes nothing
    let broken = "id: C2\nneeds: [nope]\ndescription: {why: Slow., what: Fast.}\n";
    let (refused, failed) = session.call("put", json!({"yaml": broken})).await?;
    assert!(failed, "{refused}");
    assert!(
        refused.contains("reef/central/C2: needs `nope`, which is not a node"),
        "{refused}"
    );
    let (missing, failed) = session.call("get", json!({"id": "C2"})).await?;
    assert!(failed && missing.contains("not in its plan"), "{missing}");
    // and a criterion it doesn't have is refused as the CLI refuses it
    let accept = json!({"id": "C1", "criterion": 1, "status": "met"});
    let (refused, failed) = session.call("accept", accept).await?;
    assert!(failed, "{refused}");
    assert!(
        refused.contains("`C1` has 0 acceptance criteria"),
        "{refused}"
    );
    Ok(())
}

/// next_task takes the queue's first ready node and starts it, passing over a waiting entry and
/// saying why; hands the same node out again until it is done; never hands two loops one node;
/// and says what everything waits on once nothing is left to take
#[tokio::test]
async fn next_task_skips_a_waiting_queued_node_and_says_why() -> anyhow::Result<()> {
    // a twin with `reef/central` listed, and a session in a Claude Code process of its own
    let ax = common::Fx::memory("mcp-next-task").await?.api().await?;
    let client = client_of(&ax)?;
    let here = Pane::Claude { pid: 4242 };
    let there = Pane::Claude { pid: 4243 };
    let mut first = Session::open_in(&ax, Some(here.clone())).await?;
    // A, then B that needs A, then C
    for (id, needs) in [("A", ""), ("B", "needs: [A]\n"), ("C", "")] {
        let yaml = format!(
            "id: {id}\ntitle: {id} Some work\n{needs}description:\n  why: Without {id}, \
             something is missing.\n  what: With {id}, it is there.\n"
        );
        let (put, failed) = first.call("put", json!({"yaml": yaml})).await?;
        assert!(!failed, "{put}");
    }
    // B queued ahead of C, and A not at all
    let plan = PlanName::new("reef", "central")?;
    for id in ["B", "C"] {
        let add = QueueEdit::Add {
            node: plan.node(id)?,
            at: Place::Bottom,
        };
        client.edit_queue("reef", &add).await?;
    }
    // with no loop in the pane, it is refused, saying how to start one
    let (refused, failed) = first.call("next_task", json!({})).await?;
    assert!(failed, "{refused}");
    assert!(refused.contains("`! reef loop start`"), "{refused}");
    // a loop in each of two panes, over every plan of `reef`
    for pane in [&here, &there] {
        let start = EditLoop {
            pane: pane.clone(),
            edit: LoopEdit::Start {
                project: "reef".to_string(),
                plan: None,
                session: None,
                width: None,
                opens: None,
                pid: None,
            },
        };
        client.edit_loop(&start).await?;
    }
    // the first takes C, the queue's first ready entry, passing over B and saying why
    let (took, failed) = first.call("next_task", json!({})).await?;
    assert!(!failed, "{took}");
    assert!(
        took.starts_with("Took `reef/central/C` for the loop in Claude Code process 4242."),
        "{took}"
    );
    assert!(took.contains("\n# version: 2\nid: C\n"), "{took}");
    assert!(
        took.contains(
            "Skipped in the queue (1):\n  reef/central/B  B Some work\n      needs A (todo)\n"
        ),
        "{took}"
    );
    // and starts it here, in progress
    assert!(took.contains("started `C` in "), "{took}");
    assert!(took.contains("`C` is in-progress, at version 2"), "{took}");
    assert!(client.work_of(&plan.node("C")?).await?.is_some());
    // a second call before it is done hands out C again, starting nothing
    let (again, failed) = first.call("next_task", json!({})).await?;
    assert!(!failed, "{again}");
    assert!(
        again.starts_with("`reef/central/C` is still this loop's, in-progress: carry on"),
        "{again}"
    );
    assert!(!again.contains("started `C`"), "{again}");
    // the other loop never takes C, and takes A, the rest of the ready work in plan order
    let mut second = Session::open_in(&ax, Some(there.clone())).await?;
    let (other, failed) = second.call("next_task", json!({})).await?;
    assert!(!failed, "{other}");
    assert!(
        other.starts_with("Took `reef/central/A` for the loop in Claude Code process 4243."),
        "{other}"
    );
    // with C done, the first has nothing to take: A is the other loop's, and B waits on it
    let (done, failed) = first
        .call("status", json!({"id": "C", "status": "done"}))
        .await?;
    assert!(!failed, "{done}");
    let (nothing, failed) = first.call("next_task", json!({})).await?;
    assert!(!failed, "{nothing}");
    assert!(
        nothing.starts_with(
            "Nothing is ready to take for the loop in Claude Code process 4242.\n\nHeld by \
             other loops (1):\n  reef/central/A  by the loop in Claude Code process 4243\n"
        ),
        "{nothing}"
    );
    assert!(nothing.contains("Skipped in the queue (1):"), "{nothing}");
    assert!(nothing.contains("needs A (in-progress)"), "{nothing}");
    // and the loops say which node each holds
    let loops = client.project_loops("reef").await?;
    let held: Vec<(Pane, Option<String>)> = loops
        .into_iter()
        .map(|feeding| (feeding.pane, feeding.node.map(|node| node.id)))
        .collect();
    assert_eq!(
        held,
        [
            (here, Some("C".to_string())),
            (there, Some("A".to_string()))
        ]
    );
    Ok(())
}

/// The comments tool lists a node's open threads whole, each comment with who wrote it, and
/// reads; the reply and resolve tools write as Claude, resolve linking the thread's own node's
/// latest revision, and a thread that isn't there is a tool error saying so
#[tokio::test]
async fn comment_tools_list_threads_and_answer_as_claude() -> anyhow::Result<()> {
    // a twin with `reef/central` listed, and C1 started and captured once
    let ax = common::Fx::memory("mcp-comments").await?.api().await?;
    let mut session = Session::open(&ax).await?;
    let yaml = "id: C1\ntitle: C1 Some work\ndescription:\n  why: Without C1, something is \
                missing.\n  what: With C1, it is there.\n";
    let (put, failed) = session.call("put", json!({"yaml": yaml})).await?;
    assert!(!failed, "{put}");
    let node = ax.plan.node("C1")?;
    let start = reef_api::StartWork {
        start: common::work_cases::start_in(&ax.repo, "C1", 'a'),
        replaces: None,
    };
    let (_, started): (_, reef_api::Started) = ax
        .client
        .post(&reef_api::paths::node_work(&node), &start)
        .await?;
    let reef_api::Started::Recorded { record, .. } = started else {
        anyhow::bail!("the start wasn't recorded: {started:?}");
    };
    let add = reef_api::AddCapture {
        pinned: record.start.pinned.clone(),
        shas: reef_api::CaptureShas {
            head: "d".repeat(40),
            tree: "e".repeat(40),
            merge_base: None,
            base: record.start.start_tree.clone(),
        },
        conflicts: Vec::new(),
        files: vec![common::work_cases::added_file("src/a.rs", "one\ntwo\n")],
        resolves: Vec::new(),
        commits: Vec::new(),
        draft: false,
    };
    let (_, _): (_, reef_api::Captured) = ax
        .client
        .post(&reef_api::paths::captures(&node), &add)
        .await?;
    // none open yet, then two threads the user started
    let (listed, failed) = session.call("comments", json!({})).await?;
    assert_eq!(
        (listed.as_str(), failed),
        ("`reef/central` has no open comment threads\n", false)
    );
    let mut numbers = Vec::new();
    for body in ["Why one?", "And two?"] {
        let new = reef_api::NewThread {
            target: reef_api::Target::Node,
            body: body.to_string(),
            by: reef_api::Writer::User,
            review: false,
        };
        let (_, commented): (_, reef_api::Commented) = ax
            .client
            .post(&reef_api::paths::node_comments(&node), &new)
            .await?;
        let reef_api::Commented::Added { thread, .. } = commented else {
            anyhow::bail!("the thread wasn't kept: {commented:?}");
        };
        numbers.push(thread.number);
    }
    // the node's open threads, whole
    let (listed, failed) = session.call("comments", json!({"id": "C1"})).await?;
    assert!(!failed, "{listed}");
    assert!(
        listed.starts_with("`reef/central/C1` has 2 open comment threads:"),
        "{listed}"
    );
    assert!(
        listed.contains(&format!(
            "#{} on `reef/central/C1`, the node as a whole: open\n    #{} by the user:\n        \
             Why one?",
            numbers[0], numbers[0]
        )),
        "{listed}"
    );
    // a question back leaves the first open, its last word Claude's
    let (replied, failed) = session
        .call("reply", json!({"thread": numbers[0], "body": "Which one?"}))
        .await?;
    assert!(!failed, "{replied}");
    let client = client_of(&ax)?;
    let first = client.thread(&ax.plan, numbers[0]).await?.unwrap();
    assert!(first.is_open() && !first.wants_answer());
    assert_eq!(first.comments[1].author.by, reef_api::Writer::Claude);
    // the second resolved with no change, linking C1's latest revision, after Claude's reply
    let (resolved, failed) = session
        .call(
            "resolve",
            json!({"thread": numbers[1], "reply": "Two is the count."}),
        )
        .await?;
    assert!(!failed, "{resolved}");
    assert!(
        resolved.ends_with(&format!(
            "resolved #{} of `reef/central`, answered by revision 1 of `reef/central/C1`\n",
            numbers[1]
        )),
        "{resolved}"
    );
    let second = client.thread(&ax.plan, numbers[1]).await?.unwrap();
    assert!(!second.is_open());
    assert_eq!(second.resolved_by().map(|by| by.revision), Some(1));
    assert_eq!(second.comments[1].author.by, reef_api::Writer::Claude);
    // listing every thread shows the resolved one too
    let (all, _) = session.call("comments", json!({"all": true})).await?;
    assert!(all.contains("the node as a whole: resolved"), "{all}");
    // and a thread that isn't there is an error the tool says
    let (missing, failed) = session.call("resolve", json!({"thread": 99})).await?;
    assert!(failed);
    assert!(missing.contains("has no comment thread 99"), "{missing}");
    Ok(())
}

/// In a project of several plans, a session's tools take a bare id in the plan its pane's loop
/// is held to, a plan named in full in its own project, and a session with no loop is told
/// which form `plan` takes
#[tokio::test]
async fn tools_take_a_bare_id_in_their_loops_plan() -> anyhow::Result<()> {
    // a twin with `reef/central` listed, a second plan beside it, and a node in that plan
    let ax = common::Fx::memory("mcp-loops-plan").await?.api().await?;
    let client = client_of(&ax)?;
    let other = PlanName::new("reef", "other")?;
    client.add_plan(&other).await?;
    let mut loose = Session::open(&ax).await?;
    let yaml = "id: X\ntitle: X Some work\ndescription:\n  why: Without X, something is \
                missing.\n  what: With X, it is there.\n";
    let (put, failed) = loose
        .call("put", json!({"yaml": yaml, "plan": "other"}))
        .await?;
    assert!(!failed, "{put}");
    // with no loop, a bare id is refused, saying to name the plan by its name alone
    let (refused, failed) = loose.call("get", json!({"id": "X"})).await?;
    assert!(failed, "{refused}");
    assert!(
        refused.contains("by its name alone, such as `central`"),
        "{refused}"
    );
    // the plan named in full, in its own project, is taken, and one of another isn't
    let (got, failed) = loose
        .call("get", json!({"id": "X", "plan": "reef/other"}))
        .await?;
    assert!(!failed, "{got}");
    assert!(got.contains("id: X"), "{got}");
    let (refused, failed) = loose
        .call("get", json!({"id": "X", "plan": "shoal/other"}))
        .await?;
    assert!(failed, "{refused}");
    // a session in a pane whose loop is held to `other` takes the bare id there
    let pane = Pane::Claude { pid: 5150 };
    let start = EditLoop {
        pane: pane.clone(),
        edit: LoopEdit::Start {
            project: "reef".to_string(),
            plan: Some("other".to_string()),
            session: None,
            width: None,
            opens: None,
            pid: None,
        },
    };
    client.edit_loop(&start).await?;
    let mut held = Session::open_in(&ax, Some(pane)).await?;
    let (got, failed) = held.call("get", json!({"id": "X"})).await?;
    assert!(!failed, "{got}");
    assert!(got.contains("id: X"), "{got}");
    let (status, failed) = held
        .call("status", json!({"id": "X", "status": "in-progress"}))
        .await?;
    assert!(!failed, "{status}");
    Ok(())
}
