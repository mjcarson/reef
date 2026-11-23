//! The sessions panel: the runners attached to a project, each one's session's transcript as it
//! comes, and the prompts it waits on, answered here
//!
//! - **Beside the canvas, opened from the bar.** The panel is a column of the plan's page, so
//!   moving between a project's plans keeps it. The bar's button counts the live sessions and the
//!   prompts waiting on the app, so a question is seen with the panel shut.
//! - **Followed on the project's stream.** A runner's report, an act on it and a loop changed
//!   each bump the sessions' counter, which reads the sessions and the open transcript again.
//! - **A transcript is read after what it holds.** Each read asks for the entries after those
//!   held, and follows the end as they come.
//! - **Each prompt a card.** A tool's permission is allowed, or denied with what Claude is told.
//! - **A runner's loop is started, paused and stopped here,** as `reef loop start`, `pause` and
//!   `stop` do a terminal's. A terminal's loops are listed beside, to read.

use crate::calls::{self, Runners as Listing};
use crate::canvas::dom::{self, El};
use crate::live::use_session_changes;
use dioxus::prelude::*;
use reef_api::{
    Answer, Entry, EntryKind, Loop, LoopEdited, LoopState, Prompt, PromptKind, Session, SessionAct,
    SessionActed, SessionState,
};

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
            title: "The sessions reef's runners run, and what they wait on",
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

/// The sessions panel: each runner, a terminal's loops, and the picked session's transcript
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
            let terminals: Vec<Loop> = listing
                .loops
                .iter()
                .filter(|feeding| !feeding.pane.runs())
                .cloned()
                .collect();
            let rows: Vec<(Session, Option<Loop>)> = listing
                .sessions
                .iter()
                .map(|session| {
                    let feeding = listing.loops.iter().find(|l| l.pane == session.pane);
                    (session.clone(), feeding.cloned())
                })
                .collect();
            rsx! {
                if rows.is_empty() {
                    p { class: "reef-app-muted",
                        "No runner is attached. Run "
                        code { "reef loop run" }
                        " in the project's repo to attach one."
                    }
                }
                ul { class: "reef-app-runners",
                    for (session , feeding) in rows {
                        RunnerRow {
                            key: "{session.id}",
                            picked: picked.as_deref() == Some(session.id.as_str()),
                            session,
                            feeding,
                        }
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
                    "Session on "
                    strong { "{shown.host}" }
                    if let Some(claude) = &shown.claude {
                        ", Claude "
                        code { "{claude}" }
                    }
                    ": {shown.state.describe()}"
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
                if shown.live() {
                    for prompt in shown.prompts.iter().cloned() {
                        PromptCard { key: "{prompt.id}", session: id.clone(), prompt }
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
        EntryKind::Asked { prompt } => rsx! {
            "Asked: {prompt_line(prompt)}"
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

/// A prompt's card: what it asks, and the buttons that answer it
///
/// # Arguments
///
/// * `session` - The session's id
/// * `prompt` - The prompt
#[component]
fn PromptCard(session: String, prompt: Prompt) -> Element {
    rsx! {
        AllowCard { session, prompt }
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
