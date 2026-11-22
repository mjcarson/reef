//! Every rule on every case, pinned: the chosen rule's passes and its known limits, and where
//! each other rule fails

use std::sync::OnceLock;

use x3_capture::matrix;
use x3_capture::rule::Rule;
use x3_capture::scenario::Outcome;

/// Each case and whether each rule passes it, in `Rule::ALL`'s order:
/// merge-base, start, captured, trailers, patch-id, carried
const EXPECTED: &[(&str, [bool; 6])] = &[
    ("parallel", [true, true, true, true, true, true]),
    ("amend", [true, true, true, true, true, true]),
    ("rebase", [true, false, true, true, true, true]),
    ("need-unmerged", [false, true, true, true, true, true]),
    ("need-merged", [true, false, true, true, true, true]),
    ("in-a-row", [false, true, true, true, true, true]),
    ("in-a-row-rebased", [false, false, false, true, true, true]),
    ("main-merged", [true, false, true, false, true, true]),
    ("amend-into-last", [false, true, false, false, false, true]),
    ("left-uncommitted", [false, true, false, false, false, true]),
    ("on-main", [false, true, true, true, true, true]),
    ("two-at-once", [false, false, false, false, false, false]),
    ("rebase-conflict", [false, false, false, true, false, false]),
];

/// The cases the chosen rule can't get right, and why that is allowed
const LIMITS: &[(&str, &str)] = &[
    (
        "two-at-once",
        "two nodes' edits in one worktree can't be told apart, so nodes at once get worktrees",
    ),
    (
        "rebase-conflict",
        "a conflict carrying the start forward is flagged on its files",
    ),
];

/// Runs every case once for every test here
fn outcomes() -> &'static Vec<Outcome> {
    static OUTCOMES: OnceLock<Vec<Outcome>> = OnceLock::new();
    OUTCOMES.get_or_init(|| matrix::run(&[]).expect("every case runs"))
}

/// The matrix is exactly as pinned, case by case and rule by rule
#[test]
fn the_matrix_holds() {
    let outcomes = outcomes();
    // the same cases, in the same order
    let names: Vec<&str> = outcomes.iter().map(|outcome| outcome.case.name).collect();
    let expected: Vec<&str> = EXPECTED.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, expected);
    // each cell, with the notes printed when one differs
    for (outcome, (name, passes)) in outcomes.iter().zip(EXPECTED) {
        for (rule, pass) in Rule::ALL.iter().zip(passes) {
            assert_eq!(
                outcome.passed(*rule),
                *pass,
                "{name} under {}:\n{}",
                rule.name(),
                matrix::notes(std::slice::from_ref(outcome))
            );
        }
    }
}

/// The chosen rule passes every case through amend and rebase, apart from its known limits
#[test]
fn the_chosen_rule_passes_all_but_its_limits() {
    for outcome in outcomes() {
        let limit = LIMITS.iter().find(|(name, _)| *name == outcome.case.name);
        match limit {
            // a limit must still fail, or it isn't one any more
            Some((name, why)) => assert!(!outcome.passed(Rule::CHOSEN), "{name} passes now: {why}"),
            None => assert!(
                outcome.passed(Rule::CHOSEN),
                "{} fails:\n{}",
                outcome.case.name,
                matrix::notes(std::slice::from_ref(outcome))
            ),
        }
    }
}

/// A conflict carrying the start forward names the file it happened in
#[test]
fn a_conflicted_carry_names_its_files() {
    let outcome = outcomes()
        .iter()
        .find(|outcome| outcome.case.name == "rebase-conflict")
        .expect("the conflict case ran");
    // D's check, the one after the rebase
    let check = outcome
        .checks
        .iter()
        .find(|check| check.node == "D")
        .expect("D was checked");
    let verdict = &check.verdicts[&Rule::Carried];
    assert!(
        verdict.note.contains("shared.txt conflicted"),
        "{}",
        verdict.note
    );
}

/// gix reads every case's diff exactly as git's CLI does
#[test]
fn gix_agrees_on_every_case() {
    for outcome in outcomes() {
        assert!(outcome.gix_agrees(), "{} disagrees", outcome.case.name);
    }
}
