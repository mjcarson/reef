//! One headless Claude Code session: spawned over stream-json in the runner's checkout, written
//! to, read from, and ended
//!
//! - **Every prompt comes to the runner.** The session runs with `--permission-prompt-tool
//!   stdio`, so each one it would show a user is written to its stdout, and waits for an answer
//!   on its stdin.
//! - **Its MCP server and hooks find the runner's loop.** `REEF_PANE` names the runner's pane,
//!   and every `ZELLIJ_*`, `TMUX*` and `CLAUDE*` variable of the runner's own is left out, so a
//!   runner started inside a multiplexer, or by a Claude session, hands none of that on.
//! - **Ended by closing its stdin.** A session with nothing left to read ends, its SessionEnd hook
//!   run; one that doesn't is killed.

use reef_api::Pane;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};

/// How a runner starts each session
#[derive(Debug, Clone)]
pub struct Spawn {
    /// The Claude Code to run
    pub claude: String,
    /// More arguments for it, such as `--model`
    pub args: Vec<String>,
    /// The permission mode it starts in, if not Claude Code's default
    pub mode: Option<String>,
    /// The checkout it runs in
    pub cwd: PathBuf,
    /// The runner's pane, which its MCP server and hooks find the loop by
    pub pane: Pane,
    /// The config the runner was named, which its MCP server and hooks read too
    pub conf: Option<PathBuf>,
    /// The credentials file the runner was named, which its MCP server and hooks sign in with
    pub credentials: Option<PathBuf>,
}

impl Spawn {
    /// The arguments the session runs with: the Agent SDK's own, then the runner's
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::run::drive::Spawn;
    ///
    /// let spawn = Spawn {
    ///     claude: "claude".to_string(),
    ///     args: vec!["--model".to_string(), "sonnet".to_string()],
    ///     mode: Some("auto".to_string()),
    ///     cwd: "/src/reef".into(),
    ///     pane: "reef:k3x9-4242".parse().unwrap(),
    ///     conf: None,
    ///     credentials: None,
    /// };
    /// assert_eq!(
    ///     spawn.args().join(" "),
    ///     "-p --input-format stream-json --output-format stream-json --verbose \
    ///      --permission-prompt-tool stdio --permission-mode auto --model sonnet"
    /// );
    /// ```
    pub fn args(&self) -> Vec<String> {
        // stream-json both ways, and every prompt sent to the runner
        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-prompt-tool",
            "stdio",
        ]
        .map(str::to_string)
        .to_vec();
        if let Some(mode) = &self.mode {
            args.extend(["--permission-mode".to_string(), mode.clone()]);
        }
        args.extend(self.args.iter().cloned());
        args
    }
}

/// Whether a variable of the runner's own is left out of what a session is handed
///
/// # Arguments
///
/// * `name` - The variable's name
///
/// # Examples
///
/// ```
/// use reef::run::drive::scrubbed;
///
/// assert!(scrubbed("CLAUDE_PID") && scrubbed("TMUX_PANE") && scrubbed("ZELLIJ_PANE_ID"));
/// assert!(scrubbed("REEF_PANE") && !scrubbed("PATH") && !scrubbed("REEF_CONFIG"));
/// ```
pub fn scrubbed(name: &str) -> bool {
    // a pane's, or a Claude session's, which would point the session at the wrong loop
    ["CLAUDE", "ZELLIJ", "TMUX"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || name == "REEF_PANE"
}

/// One line from the session, or its end
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// A line it wrote to its stdout
    Out(String),
    /// A line it wrote to its stderr
    Err(String),
    /// It exited, with its code if it had one
    Exited(Option<i32>),
}

/// A session running
pub struct Claude {
    /// Its stdin, gone once closed
    stdin: Option<ChildStdin>,
    /// What it writes, and its end
    lines: mpsc::UnboundedReceiver<Line>,
    /// Kills it, once
    kill: Option<oneshot::Sender<()>>,
}

impl Claude {
    /// Starts a session in the runner's checkout
    ///
    /// # Arguments
    ///
    /// * `spawn` - How the runner starts its sessions
    pub fn spawn(spawn: &Spawn) -> anyhow::Result<Claude> {
        let mut cmd = Command::new(&spawn.claude);
        cmd.args(spawn.args()).current_dir(&spawn.cwd);
        // nothing of the runner's own pane or session, and the runner's pane named instead
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(scrubbed) {
                cmd.env_remove(name);
            }
        }
        cmd.env("REEF_PANE", spawn.pane.to_string());
        if let Some(conf) = &spawn.conf {
            cmd.env("REEF_CONFIG", conf);
        }
        // and the credentials file it was named, which every `reef` the session runs signs in with
        if let Some(credentials) = &spawn.credentials {
            cmd.env(crate::credentials::CREDENTIALS_VAR, credentials);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|error| {
            anyhow::anyhow!(
                "starting `{}` in {}: {error}",
                spawn.claude,
                spawn.cwd.display()
            )
        })?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (sender, lines) = mpsc::unbounded_channel();
        // stderr's lines as they come
        if let Some(stderr) = stderr {
            let sender = sender.clone();
            tokio::spawn(async move {
                let mut read = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = read.next_line().await {
                    let _ = sender.send(Line::Err(line));
                }
            });
        }
        // stdout's lines until it ends or is killed, then its exit
        let (kill, killed) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let mut killed = killed;
            if let Some(stdout) = stdout {
                let mut read = BufReader::new(stdout).lines();
                loop {
                    tokio::select! {
                        line = read.next_line() => match line {
                            Ok(Some(line)) => {
                                let _ = sender.send(Line::Out(line));
                            }
                            _ => break,
                        },
                        _ = &mut killed => {
                            let _ = child.start_kill();
                            break;
                        }
                    }
                }
            }
            let code = child.wait().await.ok().and_then(|status| status.code());
            let _ = sender.send(Line::Exited(code));
        });
        Ok(Claude {
            stdin,
            lines,
            kill: Some(kill),
        })
    }

    /// Writes one JSON line to the session
    ///
    /// # Arguments
    ///
    /// * `value` - The line
    pub async fn send(&mut self, value: &Value) -> anyhow::Result<()> {
        // a closed session takes nothing more
        let Some(stdin) = self.stdin.as_mut() else {
            anyhow::bail!("the session's stdin is closed");
        };
        stdin.write_all(format!("{value}\n").as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Closes the session's stdin, which ends a session with nothing left to do
    pub fn close(&mut self) {
        // dropping the pipe closes it
        self.stdin.take();
    }

    /// Kills the session
    pub fn kill(&mut self) {
        // the reader kills it, and says when it exited
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
    }

    /// The next line from the session, or its end, or `None` once it has ended
    pub async fn next(&mut self) -> Option<Line> {
        self.lines.recv().await
    }
}

/// The checkout a directory is in, which each session runs at the root of
///
/// # Arguments
///
/// * `dir` - The directory
pub fn checkout_root(dir: &Path) -> anyhow::Result<PathBuf> {
    // gix's view, dropped at once, since its handles aren't kept across awaits
    Ok(crate::git::Checkout::open(dir)?.root().to_path_buf())
}
