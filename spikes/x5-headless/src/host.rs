//! The stub host: spawns `claude -p` over stream-json and answers what it asks
//!
//! Every line either side writes is logged with the time it was seen, so `report` can lay the
//! session out beside the hooks X4's stub logs.

use anyhow::{Context, Error};
use serde_json::{Map, Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::Mutex;

/// The feedback the stub rejects a plan with
const FEEDBACK: &str = "Not yet: say in the plan which test covers the new function, by its name.";

/// One headless session's options
#[derive(clap::Args)]
pub struct Drive {
    /// The repo the session runs in
    #[arg(long)]
    repo: PathBuf,
    /// The run's log, a JSON line per line seen
    #[arg(long)]
    log: PathBuf,
    /// The session's first message
    #[arg(
        long,
        default_value = "Take the next reef node: call the x4 next_task tool and follow what it says."
    )]
    kickoff: String,
    /// Seconds to hold each question before answering it
    #[arg(long, default_value_t = 0)]
    wait: u64,
    /// The model the session runs
    #[arg(long, default_value = "sonnet")]
    model: String,
    /// The session's effort
    #[arg(long, default_value = "low")]
    effort: String,
    /// The permission mode to start in, if not the account's default
    #[arg(long)]
    mode: Option<String>,
    /// Send the Agent SDK's `initialize` control request before the first message
    #[arg(long)]
    init: bool,
    /// A message sent once the first turn ends, to see whether the session carries on
    #[arg(long)]
    then: Option<String>,
    /// How many plans to reject before approving one
    #[arg(long, default_value_t = 1)]
    rejects: u32,
    /// The Claude Code binary
    #[arg(long, default_value = "claude")]
    claude: String,
}

/// The run's log, shared by every task that sees a line
#[derive(Clone)]
struct Log {
    /// The file, appended to line by line
    file: Arc<std::sync::Mutex<std::fs::File>>,
}

impl Log {
    /// Opens a fresh log
    ///
    /// # Arguments
    ///
    /// * `path` - Where the log goes
    fn create(path: &PathBuf) -> Result<Log, Error> {
        // a run starts its own log
        let file = std::fs::File::create(path).with_context(|| format!("creating {path:?}"))?;
        Ok(Log {
            file: Arc::new(std::sync::Mutex::new(file)),
        })
    }

    /// Logs one line, with when it was seen and who wrote it
    ///
    /// # Arguments
    ///
    /// * `from` - Who wrote it: `claude`, `host` or `stderr`
    /// * `msg` - What was written
    fn put(&self, from: &str, msg: Value) {
        // a line a record, flushed at once so a crash keeps everything before it
        let line = json!({ "at": now_ms(), "from": from, "msg": msg });
        let mut file = self.file.lock().expect("the log's lock holds");
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

/// The session's stdin, which the host writes answers to and closes to end the session
#[derive(Clone)]
struct Input {
    /// The pipe, gone once closed
    pipe: Arc<Mutex<Option<ChildStdin>>>,
    /// The run's log
    log: Log,
}

impl Input {
    /// Writes one JSON line to the session, and logs it
    ///
    /// # Arguments
    ///
    /// * `value` - The line
    async fn send(&self, value: Value) -> Result<(), Error> {
        // logged first, so the log shows what was sent even if the pipe is gone
        self.log.put("host", value.clone());
        let mut pipe = self.pipe.lock().await;
        let Some(stdin) = pipe.as_mut() else {
            anyhow::bail!("the session's stdin is closed");
        };
        stdin.write_all(format!("{value}\n").as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Closes the session's stdin, which ends a session with nothing left to do
    async fn close(&self) {
        // dropping the pipe closes it
        self.log.put("host", json!({ "note": "closing stdin" }));
        self.pipe.lock().await.take();
    }
}

/// What the stub user has answered so far
struct Answers {
    /// How many plans it has rejected
    rejected: AtomicU32,
    /// How many plans to reject before approving one
    rejects: u32,
    /// Seconds to hold a question before answering it
    wait: u64,
}

/// Runs one headless session in a repo, answering its prompts, until it ends
///
/// # Arguments
///
/// * `drive` - The session's options
pub async fn drive(drive: Drive) -> Result<(), Error> {
    let log = Log::create(&drive.log)?;
    // the SDK's own flags: stream-json both ways, and every prompt sent to the host
    let mut cmd = Command::new(&drive.claude);
    cmd.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
    ])
    .args(["--verbose", "--permission-prompt-tool", "stdio"])
    .args(["--model", &drive.model, "--effort", &drive.effort]);
    if let Some(mode) = &drive.mode {
        cmd.args(["--permission-mode", mode]);
    }
    // only what claude needs: no ZELLIJ_*, TMUX* or CLAUDE* from whoever runs the spike
    cmd.env_clear();
    for name in ["HOME", "USER", "PATH", "LANG", "SHELL"] {
        if let Ok(value) = std::env::var(name) {
            cmd.env(name, value);
        }
    }
    cmd.current_dir(&drive.repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    log.put(
        "host",
        json!({ "note": "spawning", "args": format!("{cmd:?}") }),
    );
    let mut child = cmd.spawn().context("spawning claude")?;
    let input = Input {
        pipe: Arc::new(Mutex::new(child.stdin.take())),
        log: log.clone(),
    };
    // stderr logged as it comes
    let stderr = child.stderr.take().context("claude's stderr")?;
    let err_log = log.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            err_log.put("stderr", Value::String(line));
        }
    });
    // the SDK's handshake, if asked for
    if drive.init {
        input
            .send(json!({
                "type": "control_request",
                "request_id": "init-1",
                "request": { "subtype": "initialize" },
            }))
            .await?;
    }
    // the first message, as the SDK writes a user's
    input.send(user(&drive.kickoff)).await?;
    let answers = Arc::new(Answers {
        rejected: AtomicU32::new(0),
        rejects: drive.rejects,
        wait: drive.wait,
    });
    let stdout = child.stdout.take().context("claude's stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut results = 0;
    while let Some(line) = lines.next_line().await? {
        // every line logged as it was seen, JSON or not
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            log.put("claude", json!({ "raw": line }));
            continue;
        };
        log.put("claude", msg.clone());
        say(&msg);
        match msg["type"].as_str().unwrap_or_default() {
            // a prompt: answered on a task of its own, so a held question blocks nothing
            "control_request" => {
                let input = input.clone();
                let answers = answers.clone();
                tokio::spawn(async move {
                    if let Err(error) = respond(&input, &answers, msg).await {
                        input
                            .log
                            .put("host", json!({ "error": format!("{error:#}") }));
                    }
                });
            }
            // the turn's end: carry on once if asked, else let the session end
            "result" => {
                results += 1;
                match (&drive.then, results) {
                    (Some(then), 1) => input.send(user(then)).await?,
                    _ => input.close().await,
                }
            }
            _ => {}
        }
    }
    // the session's end
    let status = child.wait().await?;
    log.put("host", json!({ "note": "exited", "status": status.code() }));
    println!("claude exited: {status}");
    Ok(())
}

/// A user's message, as the Agent SDK writes it to a session's stdin
///
/// # Arguments
///
/// * `text` - What the user says
fn user(text: &str) -> Value {
    json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
        "parent_tool_use_id": null,
        "session_id": "",
    })
}

/// Answers one control request from the session, as the stub user would
///
/// # Arguments
///
/// * `input` - The session's stdin
/// * `answers` - What the stub user has answered so far
/// * `msg` - The control request
async fn respond(input: &Input, answers: &Answers, msg: Value) -> Result<(), Error> {
    let id = msg["request_id"].clone();
    let request = &msg["request"];
    // anything but a tool's permission is acknowledged with nothing
    if request["subtype"] != "can_use_tool" {
        return input.send(success(id, json!({}))).await;
    }
    let tool = request["tool_name"].as_str().unwrap_or_default();
    let tool_input = request["input"].clone();
    let decision = match tool {
        // a question: held as long as asked, then its first option picked
        "AskUserQuestion" => {
            tokio::time::sleep(Duration::from_secs(answers.wait)).await;
            let picked = first_options(&tool_input);
            let mut updated = tool_input.as_object().cloned().unwrap_or_default();
            updated.insert("answers".to_string(), Value::Object(picked));
            json!({ "behavior": "allow", "updatedInput": updated })
        }
        // a plan: rejected with feedback as often as asked, then approved
        "ExitPlanMode" if answers.rejected.fetch_add(1, Ordering::SeqCst) < answers.rejects => {
            json!({ "behavior": "deny", "message": FEEDBACK })
        }
        // anything else, an approved plan included, is allowed as asked
        _ => json!({ "behavior": "allow", "updatedInput": tool_input }),
    };
    input.send(success(id, decision)).await
}

/// The first option of each question, keyed by the question's text
///
/// # Arguments
///
/// * `input` - AskUserQuestion's input
fn first_options(input: &Value) -> Map<String, Value> {
    // each question to its first option's label
    let mut picked = Map::new();
    for question in input["questions"].as_array().into_iter().flatten() {
        let text = question["question"].as_str().unwrap_or_default();
        let label = question["options"][0]["label"].as_str().unwrap_or_default();
        picked.insert(text.to_string(), Value::String(label.to_string()));
    }
    picked
}

/// A control response that succeeded
///
/// # Arguments
///
/// * `id` - The request's id
/// * `response` - What answers it
fn success(id: Value, response: Value) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": id, "response": response },
    })
}

/// Prints a line a person can follow the session by
///
/// # Arguments
///
/// * `msg` - A line the session wrote
fn say(msg: &Value) {
    // the kinds worth a line, each in a few words
    let kind = msg["type"].as_str().unwrap_or_default();
    match kind {
        "system" => println!("system {}", msg["subtype"].as_str().unwrap_or_default()),
        "assistant" => {
            for block in msg["message"]["content"].as_array().into_iter().flatten() {
                match block["type"].as_str() {
                    Some("text") => {
                        println!("said: {}", block["text"].as_str().unwrap_or_default())
                    }
                    Some("tool_use") => {
                        println!("tool: {}", block["name"].as_str().unwrap_or_default())
                    }
                    _ => {}
                }
            }
        }
        "control_request" => println!(
            "asked: {} {}",
            msg["request"]["subtype"].as_str().unwrap_or_default(),
            msg["request"]["tool_name"].as_str().unwrap_or_default()
        ),
        "result" => println!(
            "result: {} ${}",
            msg["subtype"].as_str().unwrap_or_default(),
            msg["total_cost_usd"]
        ),
        _ => {}
    }
}

/// The current time in milliseconds since the Unix epoch
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}
