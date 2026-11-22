//! The matrix: every rule tried on every case, as a markdown table and the notes behind it

use crate::rule::Rule;
use crate::scenario::{self, CASES, Outcome};

/// Runs every case, or only those named
///
/// # Arguments
///
/// * `only` - The cases to run, or all of them when empty
pub fn run(only: &[String]) -> anyhow::Result<Vec<Outcome>> {
    CASES
        .iter()
        .filter(|case| only.is_empty() || only.iter().any(|name| name == case.name))
        .map(scenario::run)
        .collect()
}

/// Writes the matrix as a markdown table, one row a case and one column a rule
///
/// # Arguments
///
/// * `outcomes` - The cases' outcomes
pub fn table(outcomes: &[Outcome]) -> String {
    // the header names each rule, then whether gix agreed
    let mut out = String::from("| Case |");
    for rule in Rule::ALL {
        out.push_str(&format!(" {} |", rule.name()));
    }
    out.push_str(" hybrid = CLI |\n|---|");
    for _ in Rule::ALL {
        out.push_str(":---:|");
    }
    out.push_str(":---:|\n");
    // a row a case, a tick where the rule got every check exactly right
    for outcome in outcomes {
        out.push_str(&format!("| {} |", outcome.case.title));
        for rule in Rule::ALL {
            out.push_str(if outcome.passed(rule) {
                " ✓ |"
            } else {
                " ✗ |"
            });
        }
        out.push_str(if outcome.hybrid_agrees() {
            " ✓ |\n"
        } else {
            " ✗ |\n"
        });
    }
    out
}

/// Writes what each rule did at each check, which says why a cell is a cross
///
/// # Arguments
///
/// * `outcomes` - The cases' outcomes
pub fn notes(outcomes: &[Outcome]) -> String {
    let mut out = String::new();
    for outcome in outcomes {
        out.push_str(&format!("{} ({})\n", outcome.case.name, outcome.case.title));
        for check in &outcome.checks {
            for (rule, verdict) in &check.verdicts {
                let mark = if verdict.pass { "ok  " } else { "FAIL" };
                out.push_str(&format!(
                    "  {} {mark} {:<10} {}\n",
                    check.node,
                    rule.name(),
                    verdict.note
                ));
            }
        }
    }
    out
}
