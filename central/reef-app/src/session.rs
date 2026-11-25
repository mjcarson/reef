//! The sessions panel: the sessions attached to a project, a runner's or a terminal's, each
//! one's transcript as it comes, and the prompts a runner's waits on, answered here
//!
//! - **Beside the canvas, opened from the bar.** The panel is a column of the plan's page, so
//!   moving between a project's plans keeps it. The bar's button counts the live sessions and the
//!   prompts waiting on the app, so a question is seen with the panel shut.
//! - **Followed on the project's stream.** A runner's report, an act on it and a loop changed
//!   each bump the sessions' counter, which reads the sessions and the open transcript again.
//! - **A transcript is read after what it holds.** Each read asks for the entries after those
//!   held, and follows the end as they come.
//! - **Each prompt a card.** A tool's permission is allowed, or denied with what Claude is told.
//!   A question shows each of Claude's questions with its choices, one or several picked, or an
//!   answer in the user's own words, sent as Claude Code's own dialog sends them. A plan shows
//!   as the service rendered it, to approve, or to reject with feedback Claude plans again from.
//! - **A runner's loop is started, paused and stopped here,** as `reef loop start`, `pause` and
//!   `stop` do a terminal's.
//! - **A terminal's own session is a row to read.** `! reef loop run` typed in a Claude Code
//!   session attaches it, and its row shows its pane, its loop's node and its transcript live,
//!   with no buttons, cards or reply box, since its terminal answers it. A terminal's loop with
//!   no session attached is listed beside, to read.

use crate::calls::{self, Runners as Listing};
use crate::canvas::dom::{self, El};
use crate::live::use_session_changes;
use dioxus::prelude::*;
use reef_api::{
    Answer, Entry, EntryKind, Loop, LoopEdited, LoopState, Prompt, PromptKind, Question, Session,
    SessionAct, SessionActed, SessionState,
};
use std::collections::BTreeMap;

/// A project's runners as the page reads them, shared through context with the bar and the panel
#[derive(Clone, Copy)]
pub struct Runners {
    /// The sessions and loops as last read
    pub listing: Resource<Result<Listing, ServerFnError>>,
    /// How many changes to them have come, which an open transcript reads again after
    pub changes: ReadSignal<u64>,
    /// Whether the panel is open
    pub open: Signal<bool>,
    /// The session picked, else the first live one is shown
    pub selected: Signal<Option<String>>,
    /// Why the last act did nothing, or failed, until the next
    pub problem: Signal<Option<String>>,
}

impl Runners {
    /// How many sessions are live, and how many prompts wait on the app across them
    pub fn counts(&self) -> (usize, usize) {
        match &*self.listing.read() {
            Some(Ok(listing)) => {
                let live = listing.sessions.iter().filter(|session| session.live());
                let waiting = live.clone().map(|session| session.prompts.len()).sum();
                (live.count(), waiting)
            }
            _ => (0, 0),
        }
    }

    /// Asks something of a session, keeps why if it did nothing, and reads the sessions again
    ///
    /// # Arguments
    ///
    /// * `id` - The session's id
    /// * `act` - What is asked of it
    pub fn act(self, id: String, act: SessionAct) {
        let (mut problem, mut listing) = (self.problem, self.listing);
        spawn(async move {
            // what came of it, in words for one that did nothing
            let said = match calls::act_on_session(id, act).await {
                Ok(SessionActed::Ordered { .. }) => None,
                Ok(SessionActed::Looped { edited, .. }) => looped(&edited),
                Ok(SessionActed::NoPrompt { .. }) => {
                    Some("That prompt no longer waits for an answer.".to_string())
                }
                Ok(SessionActed::NotWaiting { .. }) => {
                    Some("Claude isn't waiting for a reply.".to_string())
                }
                Ok(SessionActed::Gone { .. }) => {
                    Some("The runner is gone: run `reef loop run` again to attach one.".to_string())
                }
                Ok(SessionActed::InTerminal { .. }) => Some(
                    "That session is a terminal's, so it is answered in its terminal.".to_string(),
                ),
                Err(error) => Some(format!("Asking the session failed: {error}")),
            };
            problem.set(said);
            // the change stream says so too, but the panel needn't wait for it
            listing.restart();
        });
    }
}

/// What a loop's change that changed nothing says, or `None` for one that changed it
///
/// # Arguments
///
/// * `edited` - What came of the loop's change
fn looped(edited: &LoopEdited) -> Option<String> {
    match edited {
        LoopEdited::Started { .. }
        | LoopEdited::Resumed { .. }
        | LoopEdited::Edited { .. }
        | LoopEdited::Stopped { .. } => None,
        LoopEdited::NoLoop { .. } => Some("The runner's loop isn't started.".to_string()),
        LoopEdited::Busy { .. } => {
            Some("The runner's loop feeds another project or plan.".to_string())
        }
        other => Some(format!("The loop wasn't changed: {other:?}")),
    }
}

/// Reads a project's runners, again after each change to them, and shares them with the page
///
/// # Arguments
///
/// * `project` - The project
pub fn use_runners(project: ReadSignal<String>) -> Runners {
    // the project's runners' changes, and the sessions and loops read again after each
    let changes = use_session_changes(project);
    let listing = use_resource(move || async move {
        let _ = changes();
        calls::runners(project()).await
    });
    let open = use_signal(|| false);
    let selected = use_signal(|| None::<String>);
    let problem = use_signal(|| None::<String>);
    use_context_provider(|| Runners {
        listing,
        changes,
        open,
        selected,
        problem,
    })
}

/// The bar's button that opens the panel, counting the live sessions and the prompts that wait
#[component]
pub fn SessionsButton() -> Element {
    let runners = use_context::<Runners>();
    let (live, waiting) = runners.counts();
    let mut open = runners.open;
    let class = match waiting {
        0 => "reef-button reef-app-sessions-toggle",
        _ => "reef-button reef-app-sessions-toggle some",
    };
    rsx! {
        button {
            class,
            "aria-pressed": "{open}",
            title: "The sessions attached to central reef, reef's runners' and terminals', and what the runners wait on",
            onclick: move |_| {
                let shown = *open.peek();
                open.set(!shown);
            },
            "Sessions"
            span { class: "reef-app-count", "{live}" }
            if waiting > 0 {
                span { class: "reef-app-waiting", "{waiting} waiting" }
            }
        }
    }
}

/// The sessions panel's rows, as a listing splits into them
#[derive(Debug, Clone, PartialEq)]
pub struct Rows {
    /// The live sessions, a runner's or a terminal's, each beside its pane's loop
    pub live: Vec<(Session, Option<Loop>)>,
    /// The sessions that let go or went silent, each beside its pane's loop
    pub ended: Vec<(Session, Option<Loop>)>,
    /// The loops in terminals that no live session is attached on
    pub terminals: Vec<Loop>,
}

/// Splits a project's sessions and loops into the panel's rows: each session beside its pane's
/// loop, live or ended, and each terminal's loop that no live session shows
///
/// # Arguments
///
/// * `listing` - The sessions and loops
pub fn rows_of(listing: &Listing) -> Rows {
    // each session beside the loop its pane keeps
    let rows: Vec<(Session, Option<Loop>)> = listing
        .sessions
        .iter()
        .map(|session| {
            let feeding = listing.loops.iter().find(|l| l.pane == session.pane);
            (session.clone(), feeding.cloned())
        })
        .collect();
    let (live, ended): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(session, _)| session.live());
    // a terminal's loop is shown by its live session's row, else on its own
    let terminals = listing
        .loops
        .iter()
        .filter(|feeding| !feeding.pane.runs())
        .filter(|feeding| !live.iter().any(|(session, _)| session.pane == feeding.pane))
        .cloned()
        .collect();
    Rows {
        live,
        ended,
        terminals,
    }
}

/// The sessions panel: each session attached, a terminal's loops with none, and the picked
/// session's transcript
#[component]
pub fn SessionPanel() -> Element {
    let runners = use_context::<Runners>();
    let mut open = runners.open;
    if !*open.read() {
        return rsx! {};
    }
    let problem = runners.problem.read().clone();
    let listing = runners.listing.read().clone();
    let body = match listing {
        None => rsx! {
            p { class: "reef-app-muted", "Loading…" }
        },
        Some(Err(error)) => rsx! {
            p { class: "reef-app-error", "Reading the sessions failed: {error}" }
        },
        Some(Ok(listing)) => {
            // the session picked, else the first live one
            let picked = runners
                .selected
                .read()
                .clone()
                .filter(|id| listing.sessions.iter().any(|session| session.id == *id))
                .or_else(|| {
                    listing
                        .sessions
                        .iter()
                        .find(|session| session.live())
                        .map(|session| session.id.clone())
                });
            // each session beside its loop, a runner's with its buttons and a terminal's to read
            let Rows {
                live,
                ended,
                terminals,
            } = rows_of(&listing);
            let row = |(session, feeding): (Session, Option<Loop>)| {
                let picked = picked.as_deref() == Some(session.id.as_str());
                match session.pane.runs() {
                    true => rsx! {
                        RunnerRow { key: "{session.id}", picked, session, feeding }
                    },
                    false => rsx! {
                        AttachedRow { key: "{session.id}", picked, session, feeding }
                    },
                }
            };
            rsx! {
                if live.is_empty() {
                    p { class: "reef-app-muted",
                        "Nothing is attached. Run "
                        code { "reef loop run" }
                        " in the project's repo to attach a runner, or type "
                        code { "! reef loop run" }
                        " in a Claude Code session to show that session here."
                    }
                }
                ul { class: "reef-app-runners", {live.into_iter().map(row)} }
                if !ended.is_empty() {
                    details { class: "reef-app-ended",
                        summary { "Ended ({ended.len()})" }
                        ul { class: "reef-app-runners", {ended.into_iter().map(row)} }
                    }
                }
                if !terminals.is_empty() {
                    h3 { class: "reef-app-sessions-sub", "In terminals" }
                    ul { class: "reef-app-runners",
                        for feeding in terminals {
                            TerminalRow { key: "{feeding.pane}", feeding }
                        }
                    }
                }
                if let Some(id) = picked {
                    SessionView { key: "{id}", id }
                }
            }
        }
    };
    rsx! {
        aside { class: "reef-app-sessions", "aria-label": "Sessions",
            div { class: "reef-app-sessions-head",
                h2 { "Sessions" }
                button {
                    class: "reef-app-queue-off",
                    title: "Close",
                    onclick: move |_| open.set(false),
                    "×"
                }
            }
            if let Some(problem) = problem {
                p { class: "reef-app-error", "{problem}" }
            }
            {body}
        }
    }
}

/// One runner: where it runs, where its session stands, its loop's node, and its loop's buttons
///
/// # Arguments
///
/// * `session` - The runner's session
/// * `feeding` - Its loop, if one is started
/// * `picked` - Whether its transcript is the one shown
#[component]
fn RunnerRow(session: Session, feeding: Option<Loop>, picked: bool) -> Element {
    let runners = use_context::<Runners>();
    let mut selected = runners.selected;
    let state = session.state;
    // its checkout's last part, and its loop's node and state
    let place = session
        .cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let node = feeding
        .as_ref()
        .and_then(|feeding| feeding.node.as_ref())
        .map(|node| node.id.clone());
    let paused = feeding
        .as_ref()
        .is_some_and(|feeding| feeding.state == LoopState::Paused);
    let live = session.live();
    let waiting = session.prompts.len();
    // the buttons its loop takes: a start, else a pause, and a stop while it has one
    let (start, pause, stop) = (
        live && (feeding.is_none() || paused),
        live && feeding.is_some() && !paused,
        feeding.is_some(),
    );
    let acting = session.id.clone();
    let act = move |act: SessionAct| {
        let id = acting.clone();
        move |event: MouseEvent| {
            event.stop_propagation();
            runners.act(id.clone(), act.clone());
        }
    };
    let id = session.id.clone();
    rsx! {
        li {
            class: if picked { "reef-app-runner picked" } else { "reef-app-runner" },
            "data-session": "{session.id}",
            onclick: move |_| selected.set(Some(id.clone())),
            div { class: "reef-app-runner-top",
                strong { "{session.host}" }
                span { class: "reef-app-runner-where", "{place}" }
                span { class: "reef-app-session-state s-{state_class(state)}", "{state.describe()}" }
                if waiting > 0 {
                    span { class: "reef-app-waiting", "{waiting} waiting" }
                }
            }
            div { class: "reef-app-runner-loop",
                match (&feeding, &node) {
                    (None, _) => rsx! { span { class: "reef-app-muted", "No loop started" } },
                    (Some(_), None) => rsx! { span { class: "reef-app-muted", if paused { "Paused, holding nothing" } else { "Running, holding nothing" } } },
                    (Some(_), Some(node)) => rsx! { span { if paused { "Paused on " } else { "On " } code { "{node}" } } },
                }
            }
            if start || pause || stop {
                div { class: "reef-app-actions",
                    if start {
                        button { class: "reef-button", "data-act": "start", onclick: act(SessionAct::Start),
                            if paused { "Resume" } else { "Start a loop" }
                        }
                    }
                    if pause {
                        button { class: "reef-button", "data-act": "pause", onclick: act(SessionAct::Pause), "Pause" }
                    }
                    if stop {
                        button { class: "reef-button", "data-act": "stop", onclick: act(SessionAct::Stop), "Stop" }
                    }
                }
            }
        }
    }
}

/// A terminal's own session, attached by `! reef loop run`: its pane, host, checkout, where it
/// stands and its loop's node, to read and pick, with no buttons, since its terminal answers it
///
/// # Arguments
///
/// * `session` - The session
/// * `feeding` - Its pane's loop, if one is started
/// * `picked` - Whether its transcript is the one shown
#[component]
pub fn AttachedRow(session: Session, feeding: Option<Loop>, picked: bool) -> Element {
    // picked through the panel's context, which a row drawn alone has none of
    let runners = try_use_context::<Runners>();
    let state = session.state;
    // its checkout's last part, and its loop's node
    let place = session
        .cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let node = feeding
        .as_ref()
        .and_then(|feeding| feeding.node.as_ref())
        .map(|node| node.id.clone());
    let paused = feeding
        .as_ref()
        .is_some_and(|feeding| feeding.state == LoopState::Paused);
    let id = session.id.clone();
    rsx! {
        li {
            class: if picked { "reef-app-runner attached picked" } else { "reef-app-runner attached" },
            "data-session": "{session.id}",
            onclick: move |_| {
                if let Some(runners) = runners {
                    let mut selected = runners.selected;
                    selected.set(Some(id.clone()));
                }
            },
            div { class: "reef-app-runner-top",
                strong { "{session.host}" }
                span { class: "reef-app-runner-where", "{place}" }
                span { class: "reef-app-session-state s-{state_class(state)}", "{state.describe()}" }
            }
            div { class: "reef-app-runner-where", "In {session.pane.describe()}" }
            div { class: "reef-app-runner-loop",
                match (&feeding, &node) {
                    (None, _) => rsx! { span { class: "reef-app-muted", "No loop started, answered in its terminal" } },
                    (Some(_), None) => rsx! { span { class: "reef-app-muted", if paused { "Paused, holding nothing" } else { "Running, holding nothing" } } },
                    (Some(_), Some(node)) => rsx! { span { if paused { "Paused on " } else { "On " } code { "{node}" } } },
                }
            }
        }
    }
}

/// A session's state as a class name
///
/// # Arguments
///
/// * `state` - The state
fn state_class(state: SessionState) -> &'static str {
    match state {
        SessionState::Idle => "idle",
        SessionState::Working => "working",
        SessionState::Waiting => "waiting",
        SessionState::Paused => "paused",
        SessionState::Detached => "detached",
        SessionState::Gone => "gone",
    }
}

/// One loop in a terminal, to read: its pane, its node and whether it runs
///
/// # Arguments
///
/// * `feeding` - The loop
#[component]
fn TerminalRow(feeding: Loop) -> Element {
    let state = feeding.state.as_str();
    rsx! {
        li { class: "reef-app-runner terminal",
            div { class: "reef-app-runner-top",
                span { class: "reef-app-runner-where", "{feeding.pane.describe()}" }
                span { class: "reef-app-session-state", "{state}" }
            }
            if let Some(node) = &feeding.node {
                div { class: "reef-app-runner-loop", "On " code { "{node.id}" } }
            }
        }
    }
}

/// One session's transcript, read after what it holds as each change comes, its prompts' cards
/// and, while Claude waits for one, a box for the user's reply
///
/// # Arguments
///
/// * `id` - The session's id
#[component]
pub fn SessionView(id: String) -> Element {
    let runners = use_context::<Runners>();
    let changes = runners.changes;
    // the entries held, the number of the first, the session as last read, and the list's element
    let mut entries = use_signal(Vec::<Entry>::new);
    let mut first = use_signal(|| 0u64);
    let mut session = use_signal(|| None::<Session>);
    let mut problem = use_signal(|| None::<String>);
    let mut list = use_signal(|| None::<El>);
    // the entries after those held, read again after each change
    let reading = id.clone();
    let _read = use_resource(move || {
        let id = reading.clone();
        let _ = changes();
        async move {
            let held = *first.peek() + entries.peek().len() as u64;
            match calls::session_transcript(id, held).await {
                Ok(transcript) => {
                    // the ones held were let go, so it starts again from the oldest kept
                    if transcript.from != held {
                        first.set(transcript.from);
                        entries.set(transcript.entries);
                    } else if !transcript.entries.is_empty() {
                        entries.write().extend(transcript.entries);
                    }
                    session.set(Some(transcript.session));
                    problem.set(None);
                }
                Err(error) => problem.set(Some(format!("Reading the transcript failed: {error}"))),
            }
        }
    });
    // the end followed as entries come
    use_effect(move || {
        let _ = entries.read().len();
        if let Some(el) = list.peek().as_ref() {
            dom::scroll_to_bottom(el);
        }
    });
    let shown = session.read().clone();
    let held = entries.read();
    rsx! {
        section { class: "reef-app-session", "data-session": "{id}",
            if let Some(shown) = &shown {
                p { class: "reef-app-session-sum",
                    if shown.pane.runs() { "Session on " } else { "Terminal session on " }
                    strong { "{shown.host}" }
                    if let Some(claude) = &shown.claude {
                        ", Claude "
                        code { "{claude}" }
                    }
                    ": {shown.state.describe()}"
                    if !shown.pane.runs() {
                        ", answered in its terminal"
                    }
                }
            }
            ol {
                class: "reef-app-transcript",
                onmounted: move |event| list.set(dom::mounted(&event.data())),
                for (k , entry) in held.iter().enumerate() {
                    li { key: "{k}", class: "reef-app-entry {entry_class(&entry.kind)}", {entry_body(&entry.kind)} }
                }
            }
            if let Some(shown) = &shown {
                if shown.live() && shown.pane.runs() {
                    for prompt in shown.prompts.iter().cloned() {
                        PromptCard {
                            key: "{prompt.id}",
                            session: id.clone(),
                            unchanged: unchanged(&held, &prompt),
                            prompt,
                        }
                    }
                    if shown.state == SessionState::Waiting {
                        ReplyBox { session: id.clone() }
                    }
                }
            }
            if let Some(problem) = problem.read().clone() {
                p { class: "reef-app-error", "{problem}" }
            }
        }
    }
}

/// An entry's kind as a class name
///
/// # Arguments
///
/// * `kind` - The entry
fn entry_class(kind: &EntryKind) -> &'static str {
    match kind {
        EntryKind::User { .. } => "user",
        EntryKind::Text { .. } => "text",
        EntryKind::Tool { .. } => "tool",
        EntryKind::Output { error: true, .. } => "output error",
        EntryKind::Output { .. } => "output",
        EntryKind::Asked { .. } => "asked",
        EntryKind::Answered { .. } => "answered",
        EntryKind::Session { .. } => "session",
        EntryKind::Turn { .. } => "turn",
        EntryKind::Note { .. } => "note",
    }
}

/// What a prompt asks, in a line
///
/// # Arguments
///
/// * `prompt` - The prompt
pub fn prompt_line(prompt: &Prompt) -> String {
    match &prompt.kind {
        PromptKind::Permission { tool, .. } => format!("May Claude run {tool}?"),
        PromptKind::Question { questions } => questions
            .iter()
            .map(|question| question.question.clone())
            .collect::<Vec<_>>()
            .join(" "),
        PromptKind::Plan { .. } => "Claude's plan, for approval".to_string(),
    }
}

/// One entry of a transcript, drawn as a plain function's rows, since a transcript holds many
///
/// # Arguments
///
/// * `kind` - The entry
fn entry_body(kind: &EntryKind) -> Element {
    match kind {
        EntryKind::User { text } => rsx! { "▶ {text}" },
        EntryKind::Text { text } => rsx! { "{text}" },
        EntryKind::Tool { name, input } => rsx! {
            code { "{name}" }
            " {input}"
        },
        EntryKind::Output { text, error } => rsx! {
            details { open: *error,
                summary { if *error { "Error" } else { "Output" } }
                pre { "{text}" }
            }
        },
        EntryKind::Asked { prompt } => match &prompt.kind {
            // a question shows its choices beside it
            PromptKind::Question { questions } => rsx! {
                "Asked:"
                for question in questions.iter() {
                    div { class: "reef-app-entry-question",
                        "{question.question} "
                        span { class: "reef-app-muted", "({choices_line(question)})" }
                    }
                }
            },
            // a plan kept as it was asked, each version of it in turn
            PromptKind::Plan { html, .. } => rsx! {
                details {
                    summary { "Asked: Claude's plan, for approval" }
                    div { class: "reef-app-plan-body reef-app-comment-body", dangerous_inner_html: "{html}" }
                }
            },
            _ => rsx! { "Asked: {prompt_line(prompt)}" },
        },
        EntryKind::Answered { answer, .. } => rsx! { "Answered: {answer.describe()}" },
        EntryKind::Session { id, mode } => rsx! {
            "Session "
            code { "{id}" }
            ", {mode} mode"
        },
        EntryKind::Turn { turns, cost, ms } => {
            let secs = *ms as f64 / 1000.0;
            rsx! { "Turn ended: {turns} turns, {secs:.1} s, ${cost:.2} so far" }
        }
        EntryKind::Note { text } => rsx! { "{text}" },
    }
}

/// A question's choices in a line, their labels between dots
///
/// # Arguments
///
/// * `question` - The question
fn choices_line(question: &Question) -> String {
    // each label in the order offered
    let labels: Vec<&str> = question
        .options
        .iter()
        .map(|choice| choice.label.as_str())
        .collect();
    labels.join(" · ")
}

/// Whether a plan prompt asks for the same plan as the one asked before it, as when Claude asks
/// again after a rejection without changing a word
///
/// # Arguments
///
/// * `entries` - The transcript held
/// * `prompt` - The prompt
pub fn unchanged(entries: &[Entry], prompt: &Prompt) -> bool {
    let PromptKind::Plan { plan, .. } = &prompt.kind else {
        return false;
    };
    // the last plan asked before this one
    let before = entries.iter().rev().find_map(|entry| match &entry.kind {
        EntryKind::Asked { prompt: asked } if asked.id != prompt.id => match &asked.kind {
            PromptKind::Plan { plan, .. } => Some(plan),
            _ => None,
        },
        _ => None,
    });
    before.is_some_and(|before| before.trim() == plan.trim())
}

/// A prompt's card: what it asks, and the buttons that answer it
///
/// # Arguments
///
/// * `session` - The session's id
/// * `prompt` - The prompt
/// * `unchanged` - Whether it is a plan asked again word for word
#[component]
fn PromptCard(session: String, prompt: Prompt, unchanged: bool) -> Element {
    match prompt.kind.clone() {
        // a question, answered by its choices or the user's own words
        PromptKind::Question { questions } => rsx! {
            QuestionCard { session, prompt, questions }
        },
        // a plan, approved or rejected with feedback
        PromptKind::Plan { html, .. } => rsx! {
            PlanCard { session, prompt, html, unchanged }
        },
        // anything else, allowed or denied
        _ => rsx! {
            AllowCard { session, prompt }
        },
    }
}

/// The answers a question card sends, each question's own words, else its labels picked in the
/// order offered and joined by `, ` as Claude Code's own dialog joins them, or `None` while any
/// question has neither
///
/// # Arguments
///
/// * `questions` - The questions
/// * `picks` - Each question's labels picked, by its place
/// * `own` - Each question's answer in the user's own words, by its place
///
/// # Examples
///
/// ```
/// use reef_api::{Choice, Question};
/// use reef_app::session::answers_of;
///
/// let choice = |label: &str| Choice { label: label.to_string(), description: String::new() };
/// let question = Question {
///     question: "Which?".to_string(),
///     header: String::new(),
///     options: vec![choice("A"), choice("B")],
///     multi_select: true,
/// };
/// let picked = vec![vec!["B".to_string(), "A".to_string()]];
/// let answers = answers_of(&[question.clone()], &picked, &[String::new()]).unwrap();
/// assert_eq!(answers["Which?"], "A, B");
/// assert!(answers_of(&[question], &[Vec::new()], &[" ".to_string()]).is_none());
/// ```
pub fn answers_of(
    questions: &[Question],
    picks: &[Vec<String>],
    own: &[String],
) -> Option<BTreeMap<String, String>> {
    // each question answered by the user's own words, else by what was picked
    questions
        .iter()
        .enumerate()
        .map(|(k, question)| {
            let words = own.get(k).map(|words| words.trim()).unwrap_or_default();
            let picked: Vec<&str> = question
                .options
                .iter()
                .map(|choice| choice.label.as_str())
                .filter(|label| {
                    picks
                        .get(k)
                        .is_some_and(|picked| picked.iter().any(|p| p == label))
                })
                .collect();
            let answer = match (words.is_empty(), picked.is_empty()) {
                (false, _) => words.to_string(),
                (true, false) => picked.join(", "),
                (true, true) => return None,
            };
            Some((question.question.clone(), answer))
        })
        .collect()
}

/// A question's card: each of Claude's questions with its header and choices, one picked or
/// several, and a box for an answer in the user's own words, sent once each has one
///
/// # Arguments
///
/// * `session` - The session's id
/// * `prompt` - The prompt
/// * `questions` - Its questions
#[component]
fn QuestionCard(session: String, prompt: Prompt, questions: Vec<Question>) -> Element {
    let runners = use_context::<Runners>();
    // each question's labels picked and own words, by its place, and whether it was sent
    let count = questions.len();
    let mut picks = use_signal(|| vec![Vec::<String>::new(); count]);
    let mut own = use_signal(|| vec![String::new(); count]);
    let mut sent = use_signal(|| false);
    let answers = answers_of(&questions, &picks.read(), &own.read());
    // answered once, each question by its text
    let send = {
        let (session, id) = (session.clone(), prompt.id.clone());
        let answers = answers.clone();
        move |_| {
            let Some(answers) = answers.clone() else {
                return;
            };
            sent.set(true);
            let answer = Answer::Answers { answers };
            runners.act(
                session.clone(),
                SessionAct::Answer {
                    prompt: id.clone(),
                    answer,
                },
            );
        }
    };
    // or none given, which Claude is told
    let skip = {
        let (session, id) = (session.clone(), prompt.id.clone());
        move |_| {
            sent.set(true);
            let answer = Answer::Deny {
                message: "The user chose not to answer in reef's app.".to_string(),
            };
            runners.act(
                session.clone(),
                SessionAct::Answer {
                    prompt: id.clone(),
                    answer,
                },
            );
        }
    };
    rsx! {
        div { class: "reef-app-prompt reef-app-question", "data-prompt": "{prompt.id}",
            p { class: "reef-app-prompt-head", "Claude asks" }
            for (k , question) in questions.iter().cloned().enumerate() {
                fieldset { class: "reef-app-ask", "data-question": "{k}",
                    legend {
                        if !question.header.is_empty() {
                            span { class: "reef-app-tag", "{question.header}" }
                            " "
                        }
                        "{question.question}"
                    }
                    for choice in question.options.iter().cloned() {
                        label { class: "reef-app-choice", "data-choice": "{choice.label}",
                            input {
                                r#type: if question.multi_select { "checkbox" } else { "radio" },
                                name: "{prompt.id}-{k}",
                                checked: picks.read().get(k).is_some_and(|picked| picked.contains(&choice.label)),
                                onchange: {
                                    let (label, multi) = (choice.label.clone(), question.multi_select);
                                    move |_| {
                                        let mut picks = picks.write();
                                        let picked = &mut picks[k];
                                        // one of a radio's, or a checkbox's toggled
                                        match (multi, picked.contains(&label)) {
                                            (false, _) => *picked = vec![label.clone()],
                                            (true, true) => picked.retain(|p| *p != label),
                                            (true, false) => picked.push(label.clone()),
                                        }
                                    }
                                },
                            }
                            span { class: "reef-app-choice-text",
                                strong { "{choice.label}" }
                                if !choice.description.is_empty() {
                                    span { class: "reef-app-muted", " {choice.description}" }
                                }
                            }
                        }
                    }
                    input {
                        class: "reef-app-own",
                        r#type: "text",
                        placeholder: "Or answer in your own words",
                        value: "{own.read()[k]}",
                        oninput: move |event| own.write()[k] = event.value(),
                    }
                }
            }
            div { class: "reef-app-actions",
                button {
                    class: "reef-button",
                    "data-act": "answer",
                    disabled: answers.is_none() || *sent.read(),
                    onclick: send,
                    "Answer"
                }
                button {
                    class: "reef-button",
                    "data-act": "skip",
                    disabled: *sent.read(),
                    onclick: skip,
                    "Don't answer"
                }
            }
        }
    }
}

/// A plan's card: the plan as the service rendered it, to approve, or to reject with feedback
/// that Claude plans again from
///
/// # Arguments
///
/// * `session` - The session's id
/// * `prompt` - The prompt
/// * `html` - The plan as HTML, raw HTML in it escaped
/// * `unchanged` - Whether it is the plan asked before, word for word
#[component]
fn PlanCard(session: String, prompt: Prompt, html: String, unchanged: bool) -> Element {
    let runners = use_context::<Runners>();
    let mut feedback = use_signal(String::new);
    let mut sent = use_signal(|| false);
    let unsaid = feedback.read().trim().is_empty();
    // answered once: approved as it is, or rejected with what to change
    let answer = {
        let (session, id) = (session.clone(), prompt.id.clone());
        move |approve: bool| {
            let (session, id) = (session.clone(), id.clone());
            move |_| {
                sent.set(true);
                let answer = match approve {
                    true => Answer::Approve,
                    false => Answer::Reject {
                        feedback: feedback.peek().trim().to_string(),
                    },
                };
                runners.act(
                    session.clone(),
                    SessionAct::Answer {
                        prompt: id.clone(),
                        answer,
                    },
                );
            }
        }
    };
    rsx! {
        div { class: "reef-app-prompt reef-app-plan-card", "data-prompt": "{prompt.id}",
            p { class: "reef-app-prompt-head", "Claude's plan, for approval" }
            if unchanged {
                p { class: "reef-app-unchanged", "Unchanged from the plan asked before it." }
            }
            div { class: "reef-app-plan-body reef-app-comment-body", dangerous_inner_html: "{html}" }
            textarea {
                class: "reef-app-compose",
                placeholder: "What to change, to reject it with: Claude plans again from it",
                rows: "3",
                value: "{feedback}",
                oninput: move |event| feedback.set(event.value()),
            }
            div { class: "reef-app-actions",
                button {
                    class: "reef-button",
                    "data-act": "approve",
                    disabled: *sent.read(),
                    onclick: answer(true),
                    "Approve"
                }
                button {
                    class: "reef-button",
                    "data-act": "reject",
                    disabled: unsaid || *sent.read(),
                    title: "Claude plans again from what you write",
                    onclick: answer(false),
                    "Reject with feedback"
                }
            }
        }
    }
}

/// A card that allows what a prompt asks, or denies it with what Claude is told
///
/// # Arguments
///
/// * `session` - The session's id
/// * `prompt` - The prompt
#[component]
fn AllowCard(session: String, prompt: Prompt) -> Element {
    let runners = use_context::<Runners>();
    let mut message = use_signal(String::new);
    let mut sent = use_signal(|| false);
    // answered once, as it stands
    let prompt_id = prompt.id.clone();
    let answer = move |answer: Answer| {
        let (session, id) = (session.clone(), prompt_id.clone());
        move |_| {
            sent.set(true);
            runners.act(
                session.clone(),
                SessionAct::Answer {
                    prompt: id.clone(),
                    answer: answer.clone(),
                },
            );
        }
    };
    let deny = Answer::Deny {
        message: message.read().clone(),
    };
    rsx! {
        div { class: "reef-app-prompt", "data-prompt": "{prompt.id}",
            p { class: "reef-app-prompt-head", "{prompt_line(&prompt)}" }
            match &prompt.kind {
                PromptKind::Permission { input, .. } => rsx! { pre { class: "reef-app-prompt-input", "{input}" } },
                PromptKind::Plan { html, .. } => rsx! { div { class: "reef-app-plan-body reef-app-comment-body", dangerous_inner_html: "{html}" } },
                PromptKind::Question { .. } => rsx! {},
            }
            textarea {
                class: "reef-app-compose",
                placeholder: "What Claude is told if you deny it (optional)",
                rows: "2",
                value: "{message}",
                oninput: move |event| message.set(event.value()),
            }
            div { class: "reef-app-actions",
                button { class: "reef-button", "data-act": "allow", disabled: *sent.read(), onclick: answer(Answer::Allow), "Allow" }
                button { class: "reef-button", "data-act": "deny", disabled: *sent.read(), onclick: answer(deny), "Deny" }
            }
        }
    }
}

/// A box for the user's reply to a Claude whose turn ended mid-node
///
/// # Arguments
///
/// * `session` - The session's id
#[component]
fn ReplyBox(session: String) -> Element {
    let runners = use_context::<Runners>();
    let mut text = use_signal(String::new);
    let empty = text.read().trim().is_empty();
    let send = move |_| {
        let said = text.peek().clone();
        text.set(String::new());
        runners.act(session.clone(), SessionAct::Say { text: said });
    };
    rsx! {
        div { class: "reef-app-composer reef-app-reply",
            textarea {
                class: "reef-app-compose",
                placeholder: "Claude waits for your reply",
                rows: "3",
                value: "{text}",
                oninput: move |event| text.set(event.value()),
            }
            div { class: "reef-app-actions",
                button { class: "reef-button", "data-act": "reply", disabled: empty, onclick: send, "Reply" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Rows, rows_of, unchanged};
    use crate::calls::Runners as Listing;
    use reef_api::{
        Answer, Entry, EntryKind, Loop, LoopState, Pane, Prompt, PromptKind, Session, SessionState,
    };

    /// A session on a pane, standing as said
    ///
    /// # Arguments
    ///
    /// * `id` - Its id
    /// * `pane` - Its pane's key
    /// * `state` - Where it stands
    fn session(id: &str, pane: &str, state: SessionState) -> Session {
        Session {
            id: id.to_string(),
            pane: pane.parse().unwrap(),
            project: "reef".to_string(),
            plan: None,
            host: "box".to_string(),
            cwd: "/src/reef".into(),
            attached_at: 0,
            seen_at: u64::MAX,
            state,
            claude: None,
            entries: 0,
            prompts: Vec::new(),
        }
    }

    /// A running loop on a pane, holding a node of `reef/central`
    ///
    /// # Arguments
    ///
    /// * `pane` - Its pane's key
    /// * `node` - The node it holds
    fn looped(pane: &str, node: &str) -> Loop {
        Loop {
            pane: pane.parse::<Pane>().unwrap(),
            project: "reef".to_string(),
            plan: None,
            session: None,
            acted: None,
            node: Some(format!("reef/central/{node}").parse().unwrap()),
            state: LoopState::Running,
            version: 1,
            started_at: 0,
            width: 1,
            opens: false,
            pid: None,
            members: Vec::new(),
        }
    }

    /// A runner and a terminal's session are live rows beside their loops, a session replaced on
    /// its pane is ended, and only a terminal's loop no live session shows is listed on its own
    #[test]
    fn a_listing_splits_into_the_panels_rows() {
        let runner = session("r1", "reef:r1", SessionState::Idle);
        let attached = session("t2", "claude:7", SessionState::Working);
        let replaced = session("t1", "claude:7", SessionState::Detached);
        let (on_runner, on_terminal, alone) = (
            looped("reef:r1", "C45"),
            looped("claude:7", "C84"),
            looped("tmux:/tmp/t:%3", "C85"),
        );
        let listing = Listing {
            sessions: vec![runner.clone(), replaced.clone(), attached.clone()],
            loops: vec![on_runner.clone(), on_terminal.clone(), alone.clone()],
        };
        assert_eq!(
            rows_of(&listing),
            Rows {
                live: vec![
                    (runner, Some(on_runner)),
                    (attached, Some(on_terminal.clone()))
                ],
                ended: vec![(replaced, Some(on_terminal))],
                terminals: vec![alone],
            }
        );
    }

    /// A plan prompt, by its id and text
    ///
    /// # Arguments
    ///
    /// * `id` - The prompt's id
    /// * `plan` - The plan's markdown
    fn plan(id: &str, plan: &str) -> Prompt {
        Prompt {
            id: id.to_string(),
            kind: PromptKind::Plan {
                plan: plan.to_string(),
                html: String::new(),
            },
        }
    }

    /// An entry raising a prompt
    ///
    /// # Arguments
    ///
    /// * `prompt` - The prompt
    fn asked(prompt: Prompt) -> Entry {
        Entry {
            at: 0,
            kind: EntryKind::Asked { prompt },
        }
    }

    /// A plan asked again word for word after a rejection is marked unchanged, and a revised one,
    /// or the first, isn't
    #[test]
    fn a_plan_asked_again_unchanged_is_marked() {
        let first = plan("p1", "# Plan\n\nWrite it.");
        let rejected = Entry {
            at: 1,
            kind: EntryKind::Answered {
                prompt: "p1".to_string(),
                answer: Answer::Reject {
                    feedback: "In capitals.".to_string(),
                },
            },
        };
        // the first plan has nothing before it
        let entries = vec![asked(first.clone())];
        assert!(!unchanged(&entries, &first));
        // the same text asked again is unchanged, whatever its trailing space
        let again = plan("p2", "# Plan\n\nWrite it.\n");
        let entries = vec![asked(first.clone()), rejected.clone(), asked(again.clone())];
        assert!(unchanged(&entries, &again));
        // a revised one isn't
        let revised = plan("p3", "# Plan\n\nWrite it in capitals.");
        let entries = vec![asked(first), rejected, asked(revised.clone())];
        assert!(!unchanged(&entries, &revised));
    }
}
