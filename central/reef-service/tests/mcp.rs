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
        // the tools over a client of the served twin, started in the plan's repo
        let mut conf = Conf::default();
        conf.api.address = ax.address;
        let client = reef::Client::new(&conf)?;
        let tools = Tools::new(client, ax.repo.clone(), PlanPick::default());
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
        ("get", true),
        ("next", true),
        ("put", false),
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
