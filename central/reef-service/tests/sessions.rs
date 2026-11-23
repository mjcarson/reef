//! Sessions a runner runs, against a served service, with Claude played by this test binary
//!
//! Each case serves the in-memory twin on a loopback port of its own, registers a project for a
//! fresh repo whose plan holds ready nodes, and runs `reef loop run` in-process. Its `--claude`
//! names a script that execs this binary as [`claude_stub`], which speaks Claude Code's
//! stream-json as X5 found it. The stub takes the loop's node through reef's own `loop take`,
//! raises a prompt and waits for the answer the app sends, marks the node done, runs reef's own
//! Stop hook as Claude Code would, and ends its turn. So the runner, central reef's relay and the
//! loop are reef's own code, and only Claude is played. The app's acts are the service's own,
//! which the app's server functions call.

// with `embedded`, the shared fixtures' pool open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef_api::{
    Answer, EntryKind, LoopEdited, Prompt, PromptKind, Session, SessionAct, SessionActed,
    SessionState, Transcript,
};
use reef_service::MemoryBackend;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::git;
use common::http::Ax;

/// The variable naming what the stub plays: `permission`, `question` or `plan`
const STUB_PLAY: &str = "REEF_STUB_PLAY";

/// How long a case waits for the runner to get somewhere
const PATIENCE: Duration = Duration::from_secs(60);

/// Not a case: Claude Code, as a runner runs it, played for the cases here
///
/// The runner execs this binary naming this test alone, with what to play in [`STUB_PLAY`] and
/// the case's config in `REEF_CONFIG`. Run by the suite, with neither, it does nothing.
#[test]
fn claude_stub() {
    let Ok(play) = std::env::var(STUB_PLAY) else {
        return;
    };
    // reef's own CLI, hooks and client, on a runtime of the stub's own
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime builds");
    let mut stub = Stub {
        play,
        input: std::io::stdin().lock(),
        session: format!("stub-{}", std::process::id()),
    };
    // each message the runner sends, until it closes stdin
    while let Some(message) = stub.next_line() {
        if message["type"] != "user" {
            continue;
        }
        let text = message["message"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if let Err(error) = runtime.block_on(stub.turn(&text)) {
            eprintln!("the stub failed: {error:#}");
            std::process::exit(1);
        }
    }
    std::process::exit(0);
}

/// Claude Code, played: what it plays, its stdin, and its session's id
struct Stub {
    /// What it plays at each node: `permission`, `question` or `plan`
    play: String,
    /// Its stdin, the runner's messages and answers
    input: std::io::StdinLock<'static>,
    /// Its session's id
    session: String,
}

impl Stub {
    /// The next JSON line on stdin, or `None` once it is closed
    fn next_line(&mut self) -> Option<Value> {
        // a line that isn't JSON is skipped
        loop {
            let mut line = String::new();
            if self.input.read_line(&mut line).ok()? == 0 {
                return None;
            }
            if let Ok(value) = serde_json::from_str(&line) {
                return Some(value);
            }
        }
    }

    /// Writes one JSON line to stdout, as Claude Code does
    ///
    /// # Arguments
    ///
    /// * `value` - The line
    fn say(&self, value: Value) {
        // flushed at once, since the runner reads line by line
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{value}");
        let _ = out.flush();
    }

    /// Says some text, as an assistant message
    ///
    /// # Arguments
    ///
    /// * `text` - The text
    fn text(&self, text: &str) {
        self.say(json!({
            "type": "assistant",
            "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] },
        }));
    }

    /// Asks the runner for a tool's permission, and waits for its answer
    ///
    /// # Arguments
    ///
    /// * `id` - The request's id
    /// * `tool` - The tool's name
    /// * `input` - The tool's input
    fn ask(&mut self, id: &str, tool: &str, input: Value) -> anyhow::Result<Value> {
        self.say(json!({
            "type": "control_request",
            "request_id": id,
            "request": {
                "subtype": "can_use_tool",
                "tool_name": tool,
                "input": input,
                "tool_use_id": format!("toolu_{id}"),
            },
        }));
        // the answer naming this request, whatever else comes first
        while let Some(line) = self.next_line() {
            if line["type"] == "control_response" && line["response"]["request_id"] == id {
                return Ok(line["response"]["response"].clone());
            }
        }
        anyhow::bail!("stdin closed while `{tool}` waited for its answer")
    }

    /// Plays one turn: a node taken, asked about, done and stopped, or a reply answered
    ///
    /// # Arguments
    ///
    /// * `text` - What the runner said
    async fn turn(&mut self, text: &str) -> anyhow::Result<()> {
        // each turn starts with its session and mode
        self.say(json!({
            "type": "system",
            "subtype": "init",
            "session_id": self.session,
            "permissionMode": "default",
        }));
        if text != reef::clear::KICKOFF {
            self.text(&format!("You said: {text}"));
            return self.end();
        }
        // the node, through reef's own next_task, from the runner's pane
        let cwd = std::env::current_dir()?;
        let taken = reef(&cwd, &["loop", "take"]).await?;
        self.say(json!({
            "type": "assistant",
            "message": { "role": "assistant", "content": [
                { "type": "tool_use", "id": "toolu_take", "name": "mcp__reef__next_task", "input": {} },
            ] },
        }));
        self.say(json!({
            "type": "user",
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_take", "content": taken },
            ] },
        }));
        let var = |name: &str| std::env::var(name).ok();
        let pane = reef_api::Pane::from_env(var).expect("the runner names its pane");
        let client = reef::Client::new(&reef::Conf::load(None)?)?;
        // the node next_task said it took, since a loop holds its last node done when none is ready
        let took = taken
            .lines()
            .find_map(|line| line.strip_prefix("Took `"))
            .and_then(|rest| rest.split('`').next())
            .and_then(|name| name.parse::<reef_model::NodeName>().ok());
        let Some(node) = took else {
            self.text("Nothing is ready to take.");
            return self.end();
        };
        // what the case plays at the node
        self.play(&node.id).await?;
        // the node done, then Stop as Claude Code runs it, before the turn's end
        reef(&cwd, &["status", &node.id, "done"]).await?;
        let input = json!({ "session_id": self.session, "cwd": cwd }).to_string();
        let mut said = Vec::new();
        reef::hook::stop(
            &client,
            &cwd,
            &input,
            Some(&pane),
            None,
            |_| Ok(()),
            &mut said,
        )
        .await?;
        let said: Value = serde_json::from_slice(&said).unwrap_or(Value::Null);
        if let Some(message) = said["systemMessage"].as_str() {
            self.say(json!({
                "type": "system",
                "subtype": "informational",
                "content": format!("Stop says: {message}"),
            }));
        }
        self.end()
    }

    /// Plays what the case asks of a node: a tool's permission, a question, or a plan
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    async fn play(&mut self, id: &str) -> anyhow::Result<()> {
        match self.play.as_str() {
            // a write tool, allowed or not
            "permission" => {
                let input = json!({ "id": id, "status": "in-progress" });
                let answer = self.ask(&format!("perm-{id}"), "mcp__reef__status", input)?;
                self.text(&format!("Claude heard: {}", behaviour(&answer)));
            }
            // a question, answered with one of its options or the user's own words
            "question" => {
                let input = json!({ "questions": [{
                    "question": format!("Which greeting for {id}?"),
                    "header": "Greeting",
                    "options": [
                        { "label": "Hello", "description": "The usual" },
                        { "label": "Hi", "description": "Shorter" },
                    ],
                    "multiSelect": false,
                }] });
                let answer = self.ask(&format!("ask-{id}"), "AskUserQuestion", input)?;
                let picked =
                    &answer["updatedInput"]["answers"][format!("Which greeting for {id}?")];
                self.text(&format!(
                    "You picked {}.",
                    picked.as_str().unwrap_or("nothing")
                ));
            }
            // a plan, revised from each rejection's feedback until it is approved
            "plan" => {
                let mut plan = format!("# Plan for {id}\n\nWrite the greeting.");
                for round in 1.. {
                    let input = json!({ "plan": plan });
                    let answer = self.ask(&format!("plan-{id}-{round}"), "ExitPlanMode", input)?;
                    if answer["behavior"] == "allow" {
                        self.text(&format!("Plan {round} was approved."));
                        break;
                    }
                    let feedback = answer["message"].as_str().unwrap_or_default();
                    plan = format!("# Plan for {id}, revised\n\nWrite the greeting.\n\n{feedback}");
                }
            }
            other => anyhow::bail!("the stub plays no `{other}`"),
        }
        Ok(())
    }

    /// Ends the turn, as a `result`
    fn end(&self) -> anyhow::Result<()> {
        self.say(json!({
            "type": "result",
            "subtype": "success",
            "num_turns": 3,
            "total_cost_usd": 0.01,
            "duration_ms": 5,
        }));
        Ok(())
    }
}

/// What an answer let the tool do, in a word
///
/// # Arguments
///
/// * `answer` - The runner's answer
fn behaviour(answer: &Value) -> String {
    match answer["behavior"].as_str() {
        Some("allow") => "allowed".to_string(),
        _ => format!("denied: {}", answer["message"].as_str().unwrap_or_default()),
    }
}

/// Runs one `reef` command in-process, with the config `REEF_CONFIG` names, returning what it
/// printed
///
/// # Arguments
///
/// * `cwd` - The working directory to run it from
/// * `args` - The command line after `reef`
async fn reef(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    let line = std::iter::once("reef").chain(args.iter().copied());
    let cli = reef::Cli::from_args(line)?;
    let mut out = Vec::new();
    reef::run(cli, cwd, &mut out).await?;
    Ok(String::from_utf8(out)?)
}

/// A served twin, a config naming its address, a repo whose plan holds ready nodes, and the
/// script the runner runs as Claude
struct Sx {
    /// The served service, and the case's own directory
    ax: Ax<MemoryBackend>,
    /// The config file naming the server's address
    conf: PathBuf,
    /// The repo of project `runs`, whose plan `docs` holds the case's nodes
    repo: PathBuf,
    /// The script that execs this binary as the stub
    claude: PathBuf,
}

impl Sx {
    /// Serves a fresh twin, registers project `runs` for a fresh repo with plan `docs` holding
    /// the nodes named, and writes the stub's script, playing what is asked
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its directory
    /// * `ids` - The ids of the plan's nodes, each a piece of work
    /// * `play` - What the stub plays at each node
    async fn new(case: &str, ids: &[&str], play: &str) -> anyhow::Result<Sx> {
        use std::os::unix::fs::PermissionsExt;
        // a twin served on a port of its own, and the config the CLI reads its address from
        let ax = common::Fx::memory(&format!("sessions-{case}"))
            .await?
            .api()
            .await?;
        let conf = ax.dir.join("reef.toml");
        std::fs::write(&conf, format!("[api]\naddress = \"{}\"\n", ax.address))?;
        let repo = git::repo(&ax.dir.join("runs"))?;
        // Claude, played by this binary, whatever the runner passes it
        let claude = ax.dir.join("claude.sh");
        let script = format!(
            "#!/bin/sh\n{STUB_PLAY}='{play}' exec '{}' claude_stub --exact --nocapture --quiet\n",
            std::env::current_exe()?.display()
        );
        std::fs::write(&claude, script)?;
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755))?;
        let sx = Sx {
            ax,
            conf,
            repo,
            claude,
        };
        // the project, its plan and its nodes
        sx.cli(&["project", "add", "runs"]).await?;
        sx.cli(&["plan", "new", "docs"]).await?;
        for id in ids {
            let file = sx.ax.dir.join(format!("{id}.yml"));
            let yaml = format!(
                "id: {id}\ntitle: {id} Some work\ndescription:\n  why: Without {id}, something \
                 is missing.\n  what: With {id}, it is there.\n"
            );
            std::fs::write(&file, yaml)?;
            let file = file.to_str().expect("the case's dir is utf-8");
            sx.cli(&["node", "put", file]).await?;
        }
        Ok(sx)
    }

    /// Runs one `reef` command from the repo with the case's config, returning what it printed
    ///
    /// # Arguments
    ///
    /// * `args` - The command line after `reef --conf <config>`
    async fn cli(&self, args: &[&str]) -> anyhow::Result<String> {
        let conf = self.conf.to_str().expect("the config is a utf-8 path");
        let line = ["reef", "--conf", conf]
            .into_iter()
            .chain(args.iter().copied());
        let cli = reef::Cli::from_args(line)?;
        let mut out = Vec::new();
        reef::run(cli, &self.repo, &mut out).await?;
        Ok(String::from_utf8(out)?)
    }

    /// Starts `reef loop run` in the repo, on a task of its own, until the sender is dropped or
    /// sent to, returning what it printed
    fn runner(
        &self,
    ) -> (
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<anyhow::Result<String>>,
    ) {
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let (conf, repo, claude) = (self.conf.clone(), self.repo.clone(), self.claude.clone());
        let task = tokio::spawn(async move {
            // as `reef loop run --claude <script>` runs it, with the case's config
            let client = reef::Client::new(&reef::Conf::load(Some(&conf))?)?;
            let runs = reef::run::Runs {
                claude: claude.display().to_string(),
                start: false,
                mode: None,
                args: Vec::new(),
                conf: Some(conf),
            };
            let pick = reef::cli::PlanPick::default();
            let mut out = Vec::new();
            let stop = async {
                let _ = stopped.await;
            };
            reef::run::run_until(&client, &repo, &runs, &pick, &mut out, stop).await?;
            Ok(String::from_utf8(out)?)
        });
        (stop, task)
    }

    /// Waits until the project's one session passes a test, returning it
    ///
    /// # Arguments
    ///
    /// * `what` - What is waited for, for the failure's message
    /// * `test` - Whether the session is as waited for
    async fn session_when(
        &self,
        what: &str,
        test: impl Fn(&Session) -> bool,
    ) -> anyhow::Result<Session> {
        let start = tokio::time::Instant::now();
        loop {
            // the project's sessions as the app reads them
            let sessions = self.ax.service.project_sessions("runs").await?;
            if let Some(session) = sessions.into_iter().find(|session| test(session)) {
                return Ok(session);
            }
            if start.elapsed() > PATIENCE {
                let transcript = self.transcript().await.ok();
                anyhow::bail!("waited {PATIENCE:?} for {what}: {transcript:#?}");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The project's one session's whole transcript
    async fn transcript(&self) -> anyhow::Result<Transcript> {
        let sessions = self.ax.service.project_sessions("runs").await?;
        let session = sessions
            .first()
            .ok_or_else(|| anyhow::anyhow!("no session"))?;
        self.ax.service.session_transcript(&session.id, 0).await
    }

    /// The first prompt a session waits on, of the kind tested for
    ///
    /// # Arguments
    ///
    /// * `what` - What is waited for, for the failure's message
    /// * `test` - Whether the prompt is the one waited for
    async fn prompt_when(
        &self,
        what: &str,
        test: impl Fn(&Prompt) -> bool,
    ) -> anyhow::Result<(Session, Prompt)> {
        let session = self
            .session_when(what, |session| session.prompts.iter().any(&test))
            .await?;
        let prompt = session
            .prompts
            .iter()
            .find(|prompt| test(prompt))
            .cloned()
            .expect("the session holds the prompt it was found by");
        Ok((session, prompt))
    }

    /// Whether a node of plan `docs` is done
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    async fn done(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self
            .cli(&["node", "get", id])
            .await?
            .contains("status: done"))
    }

    /// Asks something of a session as the app does, returning what came of it
    ///
    /// # Arguments
    ///
    /// * `session` - The session
    /// * `act` - What is asked
    async fn act(&self, session: &Session, act: SessionAct) -> anyhow::Result<SessionActed> {
        self.ax.service.act_on_session(&session.id, act).await
    }
}

/// The texts of the entries of one kind in a transcript
///
/// # Arguments
///
/// * `transcript` - The transcript
fn texts(transcript: &Transcript) -> Vec<String> {
    transcript
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Text { text } | EntryKind::Note { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// A loop started from the app runs a fresh session a node, its transcript and prompts relayed:
/// each answer reaches the session, a done node moves on, a pause stops after the node, a stop
/// ends the session, and the runner lets go once stopped
#[tokio::test(flavor = "multi_thread")]
async fn a_loop_started_from_the_app_runs_a_session_a_node() -> anyhow::Result<()> {
    let sx = Sx::new("runs", &["A", "B"], "permission").await?;
    let (stop, runner) = sx.runner();
    // attached, idle, with no loop until the app starts one
    let session = sx
        .session_when("the runner to attach", |session| {
            session.state == SessionState::Idle
        })
        .await?;
    assert!(session.pane.runs(), "{:?}", session.pane);
    let started = sx.act(&session, SessionAct::Start).await?;
    assert!(
        matches!(&started, SessionActed::Looped { edited, .. } if matches!(**edited, LoopEdited::Started { .. })),
        "{started:?}"
    );
    // A taken in a session, its tool waiting for the app, which allows it
    let is_permission = |prompt: &Prompt| matches!(prompt.kind, PromptKind::Permission { .. });
    let (session, prompt) = sx.prompt_when("A's prompt", is_permission).await?;
    assert_eq!(prompt.id, "perm-A");
    let answer = SessionAct::Answer {
        prompt: prompt.id.clone(),
        answer: Answer::Allow,
    };
    let acted = sx.act(&session, answer.clone()).await?;
    assert!(matches!(acted, SessionActed::Ordered { .. }), "{acted:?}");
    // an answer is sent once
    let again = sx.act(&session, answer).await?;
    assert!(matches!(again, SessionActed::NoPrompt { .. }), "{again:?}");
    // A done, and B taken in a fresh session, its own tool waiting
    let (session, prompt) = sx
        .prompt_when("B's prompt", |prompt| prompt.id == "perm-B")
        .await?;
    assert!(sx.done("A").await?);
    let transcript = sx.transcript().await?;
    let said = texts(&transcript);
    assert!(
        said.contains(&"Claude heard: allowed".to_string()),
        "{said:#?}"
    );
    assert!(
        said.iter()
            .any(|text| text == "`runs/docs/A` is done, so the next node starts in a fresh session"),
        "{said:#?}"
    );
    assert!(
        said.iter()
            .any(|text| text.contains("reef's runner starts it in a fresh session")),
        "Stop's message for a runner: {said:#?}"
    );
    let sessions = transcript
        .entries
        .iter()
        .filter(|entry| matches!(entry.kind, EntryKind::Session { .. }))
        .count();
    assert_eq!(sessions, 2, "a session a node");
    // paused from the app, then B's tool denied: B done, the loop paused, the session ended
    let paused = sx.act(&session, SessionAct::Pause).await?;
    assert!(matches!(paused, SessionActed::Looped { .. }), "{paused:?}");
    let deny = SessionAct::Answer {
        prompt: prompt.id,
        answer: Answer::Deny {
            message: "Not now.".to_string(),
        },
    };
    sx.act(&session, deny).await?;
    let session = sx
        .session_when("the pause", |session| session.state == SessionState::Paused)
        .await?;
    assert!(sx.done("B").await?);
    let said = texts(&sx.transcript().await?);
    assert!(
        said.contains(&"Claude heard: denied: Not now.".to_string()),
        "{said:#?}"
    );
    // stopped from the app: the loop gone, the runner idle and still attached
    let stopped = sx.act(&session, SessionAct::Stop).await?;
    assert!(
        matches!(&stopped, SessionActed::Looped { edited, .. } if matches!(**edited, LoopEdited::Stopped { .. })),
        "{stopped:?}"
    );
    sx.session_when("the stop", |session| session.state == SessionState::Idle)
        .await?;
    // and the runner, stopped, detached
    let _ = stop.send(());
    let printed = runner.await??;
    assert!(printed.contains("attached reef's runner"), "{printed}");
    let session = sx
        .session_when("the runner to let go", |session| {
            session.state == SessionState::Detached
        })
        .await?;
    let gone = sx.act(&session, SessionAct::Start).await?;
    assert!(matches!(gone, SessionActed::Gone { .. }), "{gone:?}");
    Ok(())
}

/// A runner's loop works one node at a time, so a wider start on its pane is refused
#[tokio::test(flavor = "multi_thread")]
async fn a_runners_loop_is_one_node_wide() -> anyhow::Result<()> {
    let sx = Sx::new("wide", &["A"], "permission").await?;
    // a start two wide on a runner's pane
    let wide = sx
        .cli(&["loop", "start", "--pane", "reef:k3x9-4242", "--width", "2"])
        .await;
    let error = wide.expect_err("a runner's loop is one node wide");
    assert!(
        format!("{error:#}").contains("works one node at a time"),
        "{error:#}"
    );
    Ok(())
}

/// A session this service never heard of, as after it restarted, is no error to its runner's
/// client but a cue to attach again, and the app is told it isn't there
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_session_is_told_so() -> anyhow::Result<()> {
    let sx = Sx::new("unknown", &["A"], "permission").await?;
    let client = reef::Client::new(&reef::Conf::load(Some(&sx.conf))?)?;
    // a report or an ask for orders comes back as none, so the runner attaches again
    let report = reef_api::Report {
        entries: Vec::new(),
        state: SessionState::Idle,
        claude: None,
        prompts: Vec::new(),
    };
    assert_eq!(client.report_session("gone-1", &report).await?, None);
    let orders = client
        .session_orders("gone-1", 0, Duration::from_millis(10))
        .await?;
    assert_eq!(orders, None);
    // and what the app asks of it is a 404 saying how a runner attaches
    let error = client
        .act_on_session("gone-1", &SessionAct::Start)
        .await
        .expect_err("no session goes by that id");
    assert!(
        format!("{error:#}").contains("`reef loop run`"),
        "{error:#}"
    );
    Ok(())
}

/// An answer picked in the app reaches the session, which carries on with it: the question
/// shows with its choices as Claude offered them, the label picked comes back to Claude keyed by
/// its question, and the node is done before the next is taken
#[tokio::test(flavor = "multi_thread")]
async fn an_answer_picked_in_the_app_reaches_the_session() -> anyhow::Result<()> {
    let sx = Sx::new("question", &["A", "B"], "question").await?;
    let (stop, runner) = sx.runner();
    let session = sx
        .session_when("the runner to attach", |session| {
            session.state == SessionState::Idle
        })
        .await?;
    sx.act(&session, SessionAct::Start).await?;
    // A's question, its header and choices as Claude offered them
    let is_question = |prompt: &Prompt| matches!(prompt.kind, PromptKind::Question { .. });
    let (session, prompt) = sx.prompt_when("A's question", is_question).await?;
    let PromptKind::Question { questions } = &prompt.kind else {
        unreachable!("found by its kind");
    };
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0].question, "Which greeting for A?");
    assert_eq!(questions[0].header, "Greeting");
    let labels: Vec<&str> = questions[0]
        .options
        .iter()
        .map(|choice| choice.label.as_str())
        .collect();
    assert_eq!(labels, ["Hello", "Hi"]);
    assert!(!questions[0].multi_select);
    // the app's pick, as its question card sends it
    let answers =
        std::collections::BTreeMap::from([("Which greeting for A?".to_string(), "Hi".to_string())]);
    let picked = SessionAct::Answer {
        prompt: prompt.id.clone(),
        answer: Answer::Answers { answers },
    };
    let acted = sx.act(&session, picked).await?;
    assert!(matches!(acted, SessionActed::Ordered { .. }), "{acted:?}");
    // the session carried on with it: A done, and B's question next, in a fresh session
    sx.prompt_when("B's question", |prompt| prompt.id == "ask-B")
        .await?;
    assert!(sx.done("A").await?);
    let transcript = sx.transcript().await?;
    let said = texts(&transcript);
    assert!(said.contains(&"You picked Hi.".to_string()), "{said:#?}");
    let answered = transcript.entries.iter().any(|entry| {
        matches!(&entry.kind, EntryKind::Answered { prompt, answer: Answer::Answers { answers } }
            if prompt == "ask-A" && answers["Which greeting for A?"] == "Hi")
    });
    assert!(answered, "the answer is in the transcript");
    let _ = stop.send(());
    runner.await??;
    Ok(())
}

/// A plan rejected in the app with feedback comes back revised from it, as a prompt of its own,
/// and once approved the session carries on to the node's end; each plan is rendered, and kept
/// in the transcript in turn
#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_plan_comes_back_revised() -> anyhow::Result<()> {
    let sx = Sx::new("plan", &["A"], "plan").await?;
    let (stop, runner) = sx.runner();
    let session = sx
        .session_when("the runner to attach", |session| {
            session.state == SessionState::Idle
        })
        .await?;
    sx.act(&session, SessionAct::Start).await?;
    // the first plan, rendered for the app
    let (session, first) = sx
        .prompt_when("the first plan", |prompt| prompt.id == "plan-A-1")
        .await?;
    let PromptKind::Plan { plan, html } = &first.kind else {
        panic!("a plan: {first:?}");
    };
    assert!(plan.starts_with("# Plan for A\n"), "{plan}");
    assert!(html.contains("<h1"), "{html}");
    // rejected with feedback, which the plan comes back revised from
    let feedback = "Name the test that covers it.";
    let reject = SessionAct::Answer {
        prompt: first.id.clone(),
        answer: Answer::Reject {
            feedback: feedback.to_string(),
        },
    };
    let acted = sx.act(&session, reject).await?;
    assert!(matches!(acted, SessionActed::Ordered { .. }), "{acted:?}");
    let (session, second) = sx
        .prompt_when("the revised plan", |prompt| prompt.id == "plan-A-2")
        .await?;
    let PromptKind::Plan { plan, .. } = &second.kind else {
        panic!("a plan: {second:?}");
    };
    assert!(
        plan.contains("revised") && plan.contains(feedback),
        "{plan}"
    );
    // approved, the session carries on to the node's end
    let approve = SessionAct::Answer {
        prompt: second.id.clone(),
        answer: Answer::Approve,
    };
    sx.act(&session, approve).await?;
    sx.session_when("the node's end", |session| {
        session.state == SessionState::Idle
    })
    .await?;
    assert!(sx.done("A").await?);
    let transcript = sx.transcript().await?;
    assert!(texts(&transcript).contains(&"Plan 2 was approved.".to_string()));
    // each version kept in turn, each answer beside it
    let plans: Vec<&str> = transcript
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Asked { prompt } => match &prompt.kind {
                PromptKind::Plan { plan, .. } => Some(plan.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(plans.len(), 2, "{plans:?}");
    let answers: Vec<&Answer> = transcript
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Answered { answer, .. } => Some(answer),
            _ => None,
        })
        .collect();
    assert_eq!(
        answers,
        [
            &Answer::Reject {
                feedback: feedback.to_string()
            },
            &Answer::Approve
        ]
    );
    let _ = stop.send(());
    runner.await??;
    Ok(())
}
