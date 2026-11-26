//! The Decisions tab: each plan the node's sessions asked approval for and its verdict, and each
//! question they asked and the answer picked, as reef's hooks kept them
//!
//! - **Read with the node's work.** A `decisions-recorded` change bumps the plan's work counter,
//!   as a capture does, so the tab reads them again without reading the plan.
//! - **In the order they were kept,** each saying where and when it was answered.
//! - **A plan as the service rendered it,** folded but for the last one approved, its verdict and
//!   any feedback beside it.
//! - **A question shows every choice it offered,** the ones picked marked, and an answer in the
//!   user's own words as it was written.

use crate::calls;
use crate::live::use_work_changes;
use crate::side::when;
use dioxus::prelude::*;
use reef_api::{Decision, DecisionKind, Outcome, Question};

/// The node's decisions, read again after each change to its plan's work
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
#[component]
pub fn Decisions(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    id: ReadSignal<String>,
) -> Element {
    // read in the client alone, as the plan's other work is, after each change to it
    let work = use_work_changes();
    let decisions = use_resource(move || async move {
        let _ = work();
        calls::node_decisions(project(), plan(), id()).await
    });
    let read = decisions.read();
    let kept = match &*read {
        None => {
            return rsx! {
                p { class: "reef-app-muted", "Loading…" }
            };
        }
        Some(Err(error)) => {
            return rsx! {
                p { class: "reef-app-error", "Reading the decisions failed: {error}" }
            };
        }
        Some(Ok(kept)) => kept,
    };
    if kept.is_empty() {
        return rsx! {
            p { class: "reef-app-muted reef-app-decisions-none",
                "No plan or question is kept for this node yet. reef's hooks keep each one a session asks, in a terminal or through "
                code { "reef loop run" }
                "."
            }
        };
    }
    // the last plan approved stays open
    let open = kept.iter().rposition(|decision| {
        matches!(decision.decided.kind, DecisionKind::Plan { .. })
            && decision.decided.outcome == Some(Outcome::Approved)
    });
    rsx! {
        div { class: "reef-app-decisions",
            for (k , decision) in kept.iter().enumerate() {
                {decision_card(decision, open == Some(k))}
            }
        }
    }
}

/// What came of a decision, in a word and as a class name
///
/// # Arguments
///
/// * `outcome` - What came of it, if anything yet
fn verdict(outcome: Option<&Outcome>) -> (&'static str, &'static str) {
    match outcome {
        None => ("Waiting", "waiting"),
        Some(Outcome::Approved) => ("Approved", "approved"),
        Some(Outcome::Rejected { .. }) => ("Rejected", "rejected"),
        Some(Outcome::Answered { .. }) => ("Answered", "answered"),
        Some(Outcome::Declined { .. }) => ("Not answered", "declined"),
    }
}

/// The labels picked for a question, and any answer in the user's own words, from the answer
/// kept for it
///
/// Several picks come joined by `, `, as Claude Code's own dialog joins them, so an answer whose
/// parts are all the question's labels is those picks, and any other is the user's own words.
///
/// # Arguments
///
/// * `question` - The question
/// * `answer` - The answer kept for it, if one was
///
/// # Examples
///
/// ```
/// use reef_api::{Choice, Question};
/// use reef_app::side::picked;
///
/// let choice = |label: &str| Choice { label: label.to_string(), description: String::new() };
/// let question = Question {
///     question: "Which?".to_string(),
///     header: String::new(),
///     options: vec![choice("Red"), choice("Blue")],
///     multi_select: true,
/// };
/// assert_eq!(picked(&question, Some("Red, Blue")), (vec!["Red", "Blue"], None));
/// assert_eq!(picked(&question, Some("Green")), (vec![], Some("Green")));
/// ```
pub fn picked<'a>(question: &Question, answer: Option<&'a str>) -> (Vec<&'a str>, Option<&'a str>) {
    let Some(answer) = answer else {
        return (Vec::new(), None);
    };
    // the answer whole, or its parts, if each is one of the question's labels
    let labelled = |part: &str| question.options.iter().any(|choice| choice.label == part);
    if labelled(answer) {
        return (vec![answer], None);
    }
    let parts: Vec<&str> = answer.split(", ").collect();
    match parts.iter().all(|part| labelled(part)) {
        true => (parts, None),
        false => (Vec::new(), Some(answer)),
    }
}

/// One decision as a card: what it asked, what came of it, where and when it was answered, and
/// whose session asked it
///
/// # Arguments
///
/// * `decision` - The decision
/// * `open` - Whether its plan is shown unfolded
fn decision_card(decision: &Decision, open: bool) -> Element {
    let decided = &decision.decided;
    let (state, class) = verdict(decided.outcome.as_ref());
    let at = when(decided.answered_at.unwrap_or(decided.asked_at));
    // whose session asked it, if that is known
    let by = decided.author.describe();
    let (what, kind) = match &decided.kind {
        DecisionKind::Plan { .. } => ("Plan", "plan"),
        DecisionKind::Question { .. } => ("Question", "question"),
    };
    // a plan's feedback, or a question's words when it was left unanswered
    let said = match &decided.outcome {
        Some(Outcome::Rejected { feedback }) if !feedback.is_empty() => Some(feedback.clone()),
        Some(Outcome::Declined { message }) if !message.is_empty() => Some(message.clone()),
        _ => None,
    };
    let answers = match &decided.outcome {
        Some(Outcome::Answered { answers }) => Some(answers),
        _ => None,
    };
    rsx! {
        article {
            key: "{decision.number}",
            class: "reef-app-decision {kind} {class}",
            "data-decision": "{decision.number}",
            header { class: "reef-app-decision-head",
                span { class: "reef-app-decision-number", "#{decision.number}" }
                strong { "{what}" }
                span { class: "reef-app-tag reef-app-verdict {class}", "{state}" }
                span { class: "reef-app-muted reef-app-decision-when",
                    "{decided.via.describe()}, {at}"
                    if let Some(by) = by {
                        ", by "
                        span { class: "reef-app-author", "{by}" }
                    }
                }
            }
            if let Some(said) = said {
                blockquote { class: "reef-app-feedback", "{said}" }
            }
            match &decided.kind {
                DecisionKind::Plan { html, .. } => rsx! {
                    details { class: "reef-app-decision-plan", open,
                        summary { "The plan" }
                        div { class: "reef-app-plan-body reef-app-comment-body", dangerous_inner_html: "{html}" }
                    }
                },
                DecisionKind::Question { questions } => rsx! {
                    for question in questions.iter() {
                        {question_body(question, answers.and_then(|answers| answers.get(&question.question)).map(String::as_str))}
                    }
                },
            }
        }
    }
}

/// One question of a decision: its text, every choice it offered with the ones picked marked,
/// and an answer in the user's own words
///
/// # Arguments
///
/// * `question` - The question
/// * `answer` - The answer kept for it, if one was
fn question_body(question: &Question, answer: Option<&str>) -> Element {
    let (labels, words) = picked(question, answer);
    rsx! {
        div { class: "reef-app-decision-question",
            p { class: "reef-app-decision-asked",
                if !question.header.is_empty() {
                    span { class: "reef-app-tag", "{question.header}" }
                    " "
                }
                "{question.question}"
            }
            ul { class: "reef-app-decision-choices",
                for choice in question.options.iter() {
                    li {
                        class: if labels.contains(&choice.label.as_str()) { "picked" } else { "" },
                        "data-choice": "{choice.label}",
                        if labels.contains(&choice.label.as_str()) {
                            "✓ "
                        }
                        strong { "{choice.label}" }
                        if !choice.description.is_empty() {
                            span { class: "reef-app-muted", " {choice.description}" }
                        }
                    }
                }
            }
            if let Some(words) = words {
                p { class: "reef-app-own-answer", "Answered in their own words: {words}" }
            }
        }
    }
}
