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
        std::fs::write(&conf, ax.cli_config())?;
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
                claude: Some(claude.display().to_string()),
                start: false,
                mode: None,
                args: Vec::new(),
                conf: Some(conf),
                credentials: None,
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
        self.ax
            .service
            .act_on_session(&session.id, act, &reef_api::NOBODY)
            .await
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

/// A Claude Code session in a terminal, as these cases play it: the variables Claude Code hands
/// its `!` commands, with the case's own state and config directories, and its transcript's file
struct Terminal {
    /// Claude Code's id for the session
    claude: String,
    /// The Claude Code process it runs in, `CLAUDE_PID`
    pid: u32,
    /// The case's `XDG_STATE_HOME`, where the follower's inbox lives
    state: PathBuf,
    /// The case's `CLAUDE_CONFIG_DIR`, where Claude Code writes transcripts
    config: PathBuf,
}

impl Terminal {
    /// A session in a terminal of a case, its project's transcript directory made
    ///
    /// # Arguments
    ///
    /// * `sx` - The case
    /// * `claude` - Claude Code's id for the session
    /// * `pid` - The process it runs in, which must run while the case follows it
    fn new(sx: &Sx, claude: &str, pid: u32) -> anyhow::Result<Terminal> {
        let terminal = Terminal {
            claude: claude.to_string(),
            pid,
            state: sx.ax.dir.join("state"),
            config: sx.ax.dir.join("claude"),
        };
        std::fs::create_dir_all(terminal.config.join("projects").join("-runs"))?;
        Ok(terminal)
    }

    /// The variables Claude Code hands a `!` command typed in the session, with no multiplexer
    fn env(&self) -> impl Fn(&str) -> Option<String> + Clone + use<> {
        let (claude, pid) = (self.claude.clone(), self.pid.to_string());
        let (state, config) = (
            self.state.display().to_string(),
            self.config.display().to_string(),
        );
        move |name: &str| match name {
            "CLAUDE_CODE_SESSION_ID" => Some(claude.clone()),
            "CLAUDE_PID" => Some(pid.clone()),
            "XDG_STATE_HOME" => Some(state.clone()),
            "CLAUDE_CONFIG_DIR" => Some(config.clone()),
            _ => None,
        }
    }

    /// The transcript Claude Code writes for a session of this terminal
    ///
    /// # Arguments
    ///
    /// * `claude` - Claude Code's id for the session
    fn transcript(&self, claude: &str) -> PathBuf {
        self.config
            .join("projects")
            .join("-runs")
            .join(format!("{claude}.jsonl"))
    }

    /// The session as `reef::follow` finds it from the variables
    fn here(&self) -> anyhow::Result<reef::follow::Here> {
        reef::follow::in_claude(self.env())?.ok_or_else(|| anyhow::anyhow!("no session found"))
    }
}

/// One line of Claude saying something, as Claude Code writes it on disk
///
/// # Arguments
///
/// * `uuid` - The record's id
/// * `text` - What Claude said
fn said(uuid: &str, text: &str) -> String {
    let record = json!({
        "type": "assistant",
        "uuid": uuid,
        "isSidechain": false,
        "timestamp": "2026-11-25T10:00:00.000Z",
        "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] },
    });
    format!("{record}\n")
}

/// Appends text to a file, making it if it isn't there
///
/// # Arguments
///
/// * `path` - The file
/// * `text` - What to append
fn append(path: &Path, text: &str) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    Ok(())
}

impl Sx {
    /// A client of the case's served twin, as the CLI makes one from its config
    fn client(&self) -> anyhow::Result<reef::Client> {
        reef::Client::new(&reef::Conf::load(Some(&self.conf))?)
    }

    /// Attaches a terminal's session as `! reef loop run` does, its follower's start recorded
    /// rather than spawned, returning what it would follow
    ///
    /// # Arguments
    ///
    /// * `terminal` - The session
    async fn attach_terminal(&self, terminal: &Terminal) -> anyhow::Result<reef::follow::Follow> {
        // a follower that reads as alive, since this process holds its inbox's start
        let spawn = |_: &reef::follow::Follow| Ok(std::process::id());
        let mut out = Vec::new();
        let follow = reef::follow::attach(
            &self.client()?,
            &self.repo,
            &terminal.here()?,
            &reef::cli::PlanPick::default(),
            spawn,
            &mut out,
        )
        .await?;
        follow.ok_or_else(|| anyhow::anyhow!("not attached: {}", String::from_utf8_lossy(&out)))
    }

    /// Runs a follower in-process, on a task of its own, until it ends or the sender is sent to
    ///
    /// # Arguments
    ///
    /// * `follow` - What it follows
    fn follower(
        &self,
        follow: reef::follow::Follow,
    ) -> (
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<anyhow::Result<reef::follow::Ended>>,
    ) {
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let conf = self.conf.clone();
        let task = tokio::spawn(async move {
            // as `reef hook follow` runs it, with the case's config
            let client = reef::Client::new(&reef::Conf::load(Some(&conf))?)?;
            let mut out = Vec::new();
            let stop = async {
                let _ = stopped.await;
            };
            reef::follow::run_until(&client, &follow, &mut out, stop).await
        });
        (stop, task)
    }

    /// Waits until a session's transcript passes a test, returning it
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `what` - What is waited for, for the failure's message
    /// * `test` - Whether the transcript is as waited for
    async fn transcript_when(
        &self,
        id: &str,
        what: &str,
        test: impl Fn(&Transcript) -> bool,
    ) -> anyhow::Result<Transcript> {
        let start = tokio::time::Instant::now();
        loop {
            // the transcript as the app reads it
            let transcript = self.ax.service.session_transcript(id, 0).await?;
            if test(&transcript) {
                return Ok(transcript);
            }
            if start.elapsed() > PATIENCE {
                anyhow::bail!("waited {PATIENCE:?} for {what}: {transcript:#?}");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// Each entry of a transcript in a line: what Claude said, a session started, or a note
///
/// # Arguments
///
/// * `transcript` - The transcript
fn lines_of(transcript: &Transcript) -> Vec<String> {
    transcript
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Text { text } => Some(text.clone()),
            EntryKind::Session { id, mode } => Some(format!("session {id} in {mode}")),
            EntryKind::Note { text } => Some(format!("note: {text}")),
            _ => None,
        })
        .collect()
}

/// `reef loop run` typed in a Claude Code session attaches that session, on its terminal's pane,
/// and returns at once, starting its follower and no Claude of its own; typed again it says it
/// is attached already, and a runner's flags or a runner's own session are refused
#[tokio::test(flavor = "multi_thread")]
async fn a_run_typed_in_claude_code_attaches_that_session() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let sx = Sx::new("terminal", &["A"], "permission").await?;
    let terminal = Terminal::new(&sx, "s1", std::process::id())?;
    // a Claude that leaves a mark if anything runs it
    let ran = sx.ax.dir.join("claude-ran");
    let marker = sx.ax.dir.join("marker.sh");
    std::fs::write(&marker, format!("#!/bin/sh\ntouch '{}'\n", ran.display()))?;
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o755))?;
    // `! reef loop run`, as the CLI runs it, each follower's start recorded
    let client = sx.client()?;
    let runs = reef::run::Runs {
        conf: Some(sx.conf.clone()),
        ..Default::default()
    };
    let pick = reef::cli::PlanPick::default();
    let mut started = Vec::new();
    let spawn = |follow: &reef::follow::Follow| {
        started.push(follow.args());
        Ok(std::process::id())
    };
    let mut out = Vec::new();
    let run = reef::cli::loop_run(
        &client,
        &sx.repo,
        terminal.env(),
        &runs,
        &pick,
        spawn,
        &mut out,
    );
    tokio::time::timeout(Duration::from_secs(10), run).await??;
    let printed = String::from_utf8(out)?;
    assert!(
        printed.contains("attached this Claude Code session to `runs`"),
        "{printed}"
    );
    // the follower started, and nothing else
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0][..2], ["hook", "follow"]);
    assert!(started[0].contains(&"s1".to_string()), "{:?}", started[0]);
    // the session, on the terminal's pane, working, Claude Code's id beside it
    let sessions = sx.ax.service.project_sessions("runs").await?;
    assert_eq!(sessions.len(), 1, "{sessions:#?}");
    let session = &sessions[0];
    assert_eq!(session.pane, reef_api::Pane::Claude { pid: terminal.pid });
    assert!(!session.pane.runs());
    assert_eq!(session.claude.as_deref(), Some("s1"));
    assert_eq!(session.state, SessionState::Working);
    assert!(printed.contains(&session.id), "{printed}");
    // its inbox, held by the follower started
    let here = terminal.here()?;
    let held = reef::follow::holder(&here.inbox);
    assert_eq!(held, Some((std::process::id(), session.id.clone())));
    // typed again, it is attached already, and no second follower starts
    let mut again = Vec::new();
    let no_spawn = |_: &reef::follow::Follow| -> anyhow::Result<u32> {
        anyhow::bail!("no second follower starts")
    };
    reef::cli::loop_run(
        &client,
        &sx.repo,
        terminal.env(),
        &runs,
        &pick,
        no_spawn,
        &mut again,
    )
    .await?;
    assert!(
        String::from_utf8(again)?.contains("attached to central reef already"),
        "a session is attached once"
    );
    // a runner's flags are refused in a Claude Code session, and its Claude never runs
    let flagged = reef::run::Runs {
        claude: Some(marker.display().to_string()),
        ..runs.clone()
    };
    let refused = reef::cli::loop_run(
        &client,
        &sx.repo,
        terminal.env(),
        &flagged,
        &pick,
        |_: &reef::follow::Follow| Ok(0),
        &mut Vec::new(),
    )
    .await
    .expect_err("a runner's flags are refused");
    assert!(
        format!("{refused:#}").contains("outside Claude Code"),
        "{refused:#}"
    );
    assert!(!ran.exists(), "the --claude given never ran");
    // and a runner's own session is shown already
    let env = terminal.env();
    let runners = move |name: &str| match name {
        "REEF_PANE" => Some("reef:k3x9-7".to_string()),
        other => env(other),
    };
    let refused = reef::cli::loop_run(
        &client,
        &sx.repo,
        runners,
        &runs,
        &pick,
        |_: &reef::follow::Follow| Ok(0),
        &mut Vec::new(),
    )
    .await
    .expect_err("a runner's session is refused");
    assert!(format!("{refused:#}").contains("runner's"), "{refused:#}");
    assert_eq!(sx.ax.service.project_sessions("runs").await?.len(), 1);
    Ok(())
}

/// An attached session's follower streams its transcript as it is written, each entry once and
/// in order, partial lines waited out; follows a clear to the new session's file once the old is
/// read to its end, noting the new session in the mode its file names; and once detached in its
/// terminal, the session reads as detached
#[tokio::test(flavor = "multi_thread")]
async fn an_attached_sessions_transcript_streams_once_and_in_order() -> anyhow::Result<()> {
    let sx = Sx::new("follow", &["A"], "permission").await?;
    let terminal = Terminal::new(&sx, "s1", std::process::id())?;
    let first = terminal.transcript("s1");
    append(&first, &said("c0", "line 0"))?;
    let follow = sx.attach_terminal(&terminal).await?;
    let id = follow.id().to_string();
    let (_stop, follower) = sx.follower(follow);
    // ten more lines, written in chunks that split lines across them
    let written: String = (1..10)
        .map(|k| said(&format!("c{k}"), &format!("line {k}")))
        .collect();
    for chunk in written.as_bytes().chunks(37) {
        append(&first, std::str::from_utf8(chunk)?)?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sx.transcript_when(&id, "the first ten lines", |transcript| {
        lines_of(transcript).len() == 10
    })
    .await?;
    // two more, and at once a clear's new session, which is followed once they are read
    append(&first, &said("c10", "line 10"))?;
    append(&first, &said("c11", "line 11"))?;
    // the new session's file names its mode first, as Claude Code writes it
    let second = terminal.transcript("s2");
    let more: String = (0..3)
        .map(|k| said(&format!("d{k}"), &format!("more {k}")))
        .collect();
    let mode = json!({ "type": "permission-mode", "permissionMode": "plan", "sessionId": "s2" });
    append(&second, &format!("{mode}\n{more}"))?;
    let here = terminal.here()?;
    let moved = reef::follow::Event::Moved {
        claude: "s2".to_string(),
        transcript: second.clone(),
    };
    reef::follow::post(&here.inbox, &moved)?;
    sx.transcript_when(&id, "the new session's lines", |transcript| {
        lines_of(transcript).iter().any(|line| line == "more 2")
    })
    .await?;
    // detached as `! reef loop detach` detaches it
    let mut out = Vec::new();
    reef::follow::detach(&here, &mut out).await?;
    let printed = String::from_utf8(out)?;
    assert!(printed.contains("detached this session"), "{printed}");
    let ended = tokio::time::timeout(PATIENCE, follower).await???;
    assert_eq!(ended, reef::follow::Ended::Detached);
    assert!(!here.inbox.exists(), "the inbox is let go of");
    // every entry once, in the order written, the new session between the two files
    let transcript = sx.ax.service.session_transcript(&id, 0).await?;
    let mut expected: Vec<String> = (0..12).map(|k| format!("line {k}")).collect();
    expected.push("session s2 in plan".to_string());
    expected.extend((0..3).map(|k| format!("more {k}")));
    expected.push("note: the session was detached in its terminal".to_string());
    assert_eq!(lines_of(&transcript), expected);
    assert_eq!(transcript.session.state, SessionState::Detached);
    assert_eq!(transcript.session.claude.as_deref(), Some("s2"));
    Ok(())
}

/// A session whose Claude Code exits reads as detached within moments, its follower ending of
/// its own accord
#[tokio::test(flavor = "multi_thread")]
async fn an_attached_session_ends_with_its_claude() -> anyhow::Result<()> {
    let sx = Sx::new("exits", &["A"], "permission").await?;
    // Claude Code, played by a process that runs until it is killed
    let mut claude = std::process::Command::new("sleep").arg("600").spawn()?;
    let terminal = Terminal::new(&sx, "s1", claude.id())?;
    append(&terminal.transcript("s1"), &said("c0", "working"))?;
    let follow = sx.attach_terminal(&terminal).await?;
    let id = follow.id().to_string();
    let (_stop, follower) = sx.follower(follow);
    sx.transcript_when(&id, "the first line", |transcript| {
        lines_of(transcript).contains(&"working".to_string())
    })
    .await?;
    // killed and not yet reaped, which still reads as gone
    claude.kill()?;
    let ended = tokio::time::timeout(Duration::from_secs(10), follower).await???;
    claude.wait()?;
    assert_eq!(ended, reef::follow::Ended::Exited);
    let transcript = sx.ax.service.session_transcript(&id, 0).await?;
    assert_eq!(transcript.session.state, SessionState::Detached);
    assert_eq!(
        lines_of(&transcript).last().map(String::as_str),
        Some("note: the session's Claude Code exited")
    );
    Ok(())
}

/// A terminal's session takes no orders from the app: every act is refused with a 409 saying
/// it is answered in its terminal, and a question it asked shows in its transcript without
/// waiting on the app
#[tokio::test(flavor = "multi_thread")]
async fn a_terminals_session_is_answered_in_its_terminal() -> anyhow::Result<()> {
    let sx = Sx::new("answers", &["A"], "permission").await?;
    let terminal = Terminal::new(&sx, "s1", std::process::id())?;
    // Claude asks a question, which the terminal hasn't answered yet
    let question = json!({
        "type": "assistant",
        "uuid": "q1",
        "timestamp": "2026-11-25T10:00:00.000Z",
        "message": { "role": "assistant", "content": [{
            "type": "tool_use",
            "id": "toolu_q1",
            "name": "AskUserQuestion",
            "input": { "questions": [{
                "question": "Which greeting?",
                "header": "Greeting",
                "options": [{ "label": "Hello" }, { "label": "Hi" }],
                "multiSelect": false,
            }] },
        }] },
    });
    append(&terminal.transcript("s1"), &format!("{question}\n"))?;
    let follow = sx.attach_terminal(&terminal).await?;
    let id = follow.id().to_string();
    let (stop, follower) = sx.follower(follow);
    let transcript = sx
        .transcript_when(&id, "the question", |transcript| {
            transcript
                .entries
                .iter()
                .any(|entry| matches!(entry.kind, EntryKind::Asked { .. }))
        })
        .await?;
    // shown as asked, with nothing waiting on the app
    assert!(transcript.session.prompts.is_empty(), "{transcript:#?}");
    // and every act refused, over the API as the app's server sends it
    let acts = [
        SessionAct::Start,
        SessionAct::Pause,
        SessionAct::Stop,
        SessionAct::Answer {
            prompt: "toolu_q1".to_string(),
            answer: Answer::Allow,
        },
        SessionAct::Say {
            text: "go on".to_string(),
        },
    ];
    for act in acts {
        let path = reef_api::paths::session_orders(&id);
        let (status, acted): (_, SessionActed) = sx.ax.client.post(&path, &act).await?;
        assert_eq!(status.as_u16(), 409, "{act:?}: {acted:?}");
        assert!(
            matches!(acted, SessionActed::InTerminal { .. }),
            "{act:?}: {acted:?}"
        );
    }
    // no loop was started for its pane, and it still waits on nothing
    let loops = sx.client()?.loops().await?;
    let pane = terminal.here()?.pane;
    assert!(loops.iter().all(|feeding| feeding.pane != pane));
    let transcript = sx.ax.service.session_transcript(&id, 0).await?;
    assert!(transcript.session.prompts.is_empty());
    assert!(transcript.session.live());
    let _ = stop.send(());
    let ended = tokio::time::timeout(PATIENCE, follower).await???;
    assert_eq!(ended, reef::follow::Ended::Stopped);
    Ok(())
}

/// A pane shows one terminal session: attaching another on it detaches the one before, which
/// its follower's last report doesn't bring back, while a runner on its own pane is untouched;
/// and a terminal's session must name a safe id of its own
#[tokio::test(flavor = "multi_thread")]
async fn a_second_session_on_a_pane_detaches_the_first() -> anyhow::Result<()> {
    let sx = Sx::new("replaced", &["A"], "permission").await?;
    let service = &sx.ax.service;
    // what a terminal's session on a pane attaches with
    let attach = |id: Option<&str>, pane: reef_api::Pane| reef_api::Attach {
        id: id.map(str::to_string),
        pane,
        project: "runs".to_string(),
        plan: None,
        host: "box".to_string(),
        cwd: sx.repo.clone(),
        state: SessionState::Working,
        entries: Vec::new(),
        prompts: Vec::new(),
        claude: Some("s1".to_string()),
    };
    let pane: reef_api::Pane = "tmux:/tmp/tmux-1000/default:%3".parse()?;
    let runner: reef_api::Pane = "reef:k3x9-7".parse()?;
    service
        .attach_session(attach(None, runner.clone()), &reef_api::NOBODY)
        .await?;
    let first = service
        .attach_session(attach(Some("t1-1"), pane.clone()), &reef_api::NOBODY)
        .await?;
    assert_eq!(first.claude.as_deref(), Some("s1"));
    let second = service
        .attach_session(attach(Some("t2-2"), pane.clone()), &reef_api::NOBODY)
        .await?;
    // the first let go, the second shown, the runner as it was
    let sessions = service.project_sessions("runs").await?;
    let state = |id: &str| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .map(|session| session.state)
    };
    assert_eq!(state("t1-1"), Some(SessionState::Detached));
    assert_eq!(state(&second.id), Some(SessionState::Working));
    assert_eq!(state("k3x9-7"), Some(SessionState::Working));
    // a report from the first's follower keeps it let go, which it reads as replaced
    let report = reef_api::Report {
        entries: Vec::new(),
        state: SessionState::Working,
        claude: Some("s1".to_string()),
        prompts: Vec::new(),
    };
    let reported = service.report_session("t1-1", report).await?;
    assert_eq!(reported.state, SessionState::Detached);
    // an id that isn't safe in a path, or none, is refused on a terminal's pane
    for id in [Some("Bad/Id"), Some(""), None] {
        let refused = service
            .attach_session(attach(id, pane.clone()), &reef_api::NOBODY)
            .await
            .expect_err("an unsafe id is refused");
        assert!(
            refused
                .downcast_ref::<reef_service::store::Unkeepable>()
                .is_some(),
            "{refused:#}"
        );
    }
    Ok(())
}
