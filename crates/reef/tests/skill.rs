//! The skill for central reef: where it is written, and that it names only what reef has
//!
//! The repo cases make scratch repos under cargo's target dir with real git, as the git suite
//! does. The skill's text is read as `reef skill` writes it, and each tool and command it names
//! is looked up in the MCP server's router and the CLI's own command tree.

use clap::CommandFactory;
use reef::Cli;
use reef::skill::{self, FILES, PLACE};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A scratch directory of the case's own under cargo's target dir, empty
///
/// # Arguments
///
/// * `name` - The case's name, which the directory is named after
fn scratch(name: &str) -> PathBuf {
    // a fresh directory each run, so an earlier run's files can't pass this one
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("skill-{name}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// Makes a git repo in a directory, with no one's config
///
/// # Arguments
///
/// * `dir` - The directory to make it in
fn git_init(dir: &Path) {
    // a repo of its own, so its root is nearer than the one cargo's target dir is in
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["init", "-q"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .status()
        .expect("git runs");
    assert!(status.success(), "git init in {}", dir.display());
}

/// The names of the files in a directory, sorted
///
/// # Arguments
///
/// * `dir` - The directory to list
fn listed(dir: &Path) -> Vec<String> {
    // each entry's name, in order so two listings compare
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Two runs from a subdirectory leave one skill at the repo's root, the same each time
#[test]
fn two_runs_leave_one_skill_at_the_repo_root() {
    // a repo with a subdirectory to run from
    let root = scratch("repo");
    git_init(&root);
    let deeper = root.join("crates/deeper");
    std::fs::create_dir_all(&deeper).unwrap();
    // the first run writes at the root and says where
    let mut said = Vec::new();
    let first = skill::write(&deeper, &mut said).unwrap();
    assert_eq!(first, root.join(PLACE));
    let said = String::from_utf8(said).unwrap();
    assert_eq!(
        said,
        format!("wrote the reef skill to {}\n", first.display())
    );
    // a second run writes the same place, and the same bytes
    let second = skill::write(&deeper, &mut Vec::new()).unwrap();
    assert_eq!(first, second);
    let mut names: Vec<String> = FILES.iter().map(|(name, _)| name.to_string()).collect();
    names.sort();
    assert_eq!(listed(&first), names);
    for (name, text) in FILES {
        assert_eq!(&std::fs::read_to_string(first.join(name)).unwrap(), text);
    }
    // and nothing was written where it ran
    assert_eq!(listed(&deeper), Vec::<String>::new());
    assert!(!root.join("crates/.claude").exists());
}

/// A skill written over another of its name, such as the mdbook skill, replaces its files
#[test]
fn a_run_writes_over_the_mdbook_skill() {
    // a repo holding a skill of the same name, its files the mdbook skill's names
    let root = scratch("over");
    git_init(&root);
    let place = root.join(PLACE);
    std::fs::create_dir_all(&place).unwrap();
    std::fs::write(place.join("SKILL.md"), "the static mode's\n").unwrap();
    std::fs::write(place.join("reference.md"), "its reference\n").unwrap();
    // a run puts this skill's text in their place
    skill::write(&root, &mut Vec::new()).unwrap();
    for (name, text) in FILES {
        assert_eq!(&std::fs::read_to_string(place.join(name)).unwrap(), text);
    }
}

/// A linked worktree's root is where its `.git` file is, and outside every repo it is the
/// directory itself
#[test]
fn the_skill_goes_at_the_nearest_root() {
    // a worktree, whose `.git` is a file naming its repo's git dir
    let worktree = scratch("worktree");
    std::fs::write(
        worktree.join(".git"),
        "gitdir: /elsewhere/.git/worktrees/w\n",
    )
    .unwrap();
    let inside = worktree.join("src/deep");
    assert_eq!(skill::place(&inside), worktree.join(PLACE));
    // a directory no repo holds keeps the skill itself
    let nowhere = Path::new("/reef-skill-test-nowhere/a/b");
    assert_eq!(skill::place(nowhere), nowhere.join(PLACE));
}

/// Every `mcp__reef__<tool>` the text names, a wildcard or a placeholder left out
///
/// # Arguments
///
/// * `text` - The skill's text
fn tools_named(text: &str) -> BTreeSet<String> {
    // the word after each prefix, which a `*` or a `<` leaves empty
    let prefix = "mcp__reef__";
    text.match_indices(prefix)
        .map(|(at, _)| {
            text[at + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<String>()
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// Every command line in a code span of the text that runs reef
///
/// # Arguments
///
/// * `text` - The skill's text
fn spans_run(text: &str) -> Vec<String> {
    // the text between each pair of backticks on a line, fences' own lines left out, and a
    // fence named in prose, such as a ```reef fence, read as no span
    let mut lines = Vec::new();
    for line in text
        .lines()
        .filter(|line| !line.trim_start().starts_with("```"))
    {
        let line = line.replace("```", "");
        let spans = line.split('`').skip(1).step_by(2);
        lines.extend(
            spans
                .filter(|span| span.starts_with("reef "))
                .map(str::to_string),
        );
    }
    lines
}

/// Every line of the text's ```sh fences that runs reef, its comment cut off
///
/// # Arguments
///
/// * `text` - The skill's text
fn fences_run(text: &str) -> Vec<String> {
    // the lines between a ```sh line and the fence that closes it
    let mut lines = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        match (inside, line.trim()) {
            (false, "```sh") => inside = true,
            (true, "```") => inside = false,
            (true, run) if run.starts_with("reef ") => {
                let run = run.split(" #").next().unwrap_or(run).trim_end();
                lines.push(run.to_string());
            }
            _ => {}
        }
    }
    lines
}

/// Why a command line names something reef's CLI doesn't have, if it does
///
/// It walks the subcommands it names, then looks up each `--flag` among the last one's
/// arguments and the global ones. A placeholder such as `<ID>` stands for a value.
///
/// # Arguments
///
/// * `run` - The command line, `reef` first
fn unknown_in(run: &str) -> Option<String> {
    // the command tree clap parses with, walked from the top
    let top = Cli::command();
    let mut command = &top;
    let mut path = vec!["reef"];
    let mut valued = false;
    for word in run.split_whitespace().skip(1) {
        if let Some(long) = word.strip_prefix("--") {
            // a flag of the command reached, or one every command takes
            let has = |command: &clap::Command| {
                command
                    .get_arguments()
                    .any(|arg| arg.get_long() == Some(long))
            };
            if !has(command) && !has(&top) {
                return Some(format!("`{}` has no `--{long}`", path.join(" ")));
            }
            valued = true;
            continue;
        }
        // a subcommand until the first value, which ends the path
        match (valued, command.find_subcommand(word)) {
            (false, Some(sub)) => {
                command = sub;
                path.push(word);
            }
            _ => valued = true,
        }
    }
    // a command with subcommands needs one named
    if command.has_subcommands() && !valued {
        return Some(format!("`{}` needs a subcommand", path.join(" ")));
    }
    if command.has_subcommands() {
        return Some(format!("`{}` has no subcommand in `{run}`", path.join(" ")));
    }
    None
}

/// The skill names every tool reef's MCP server has and no other, and only commands and flags
/// reef's CLI has
#[test]
fn the_skill_names_every_tool_and_only_reefs_commands() {
    // the tools named across the skill's files, against the router's
    let text: String = FILES.iter().map(|(_, text)| *text).collect();
    let named = tools_named(&text);
    let served: BTreeSet<String> = reef::mcp::tool_names().into_iter().collect();
    assert!(!served.is_empty());
    let missing: Vec<&String> = served.difference(&named).collect();
    assert!(missing.is_empty(), "the skill never names {missing:?}");
    let unknown: Vec<&String> = named.difference(&served).collect();
    assert!(
        unknown.is_empty(),
        "the skill names tools reef has not: {unknown:?}"
    );
    // every command a span names walks to a command reef has, its flags among its own
    let spans = spans_run(&text);
    assert!(spans.len() > 10, "found only {spans:?}");
    let wrong: Vec<String> = spans.iter().filter_map(|run| unknown_in(run)).collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
    // and every line of the reference's commands parses whole
    let fenced = fences_run(&text);
    assert!(fenced.len() > 10, "found only {fenced:?}");
    for run in fenced {
        let words: Vec<&str> = run.split_whitespace().collect();
        if let Err(error) = Cli::from_args(&words) {
            panic!("`{run}` doesn't parse: {error}");
        }
    }
}

/// The lookup itself finds a made-up command, a made-up flag and a command left short
#[test]
fn a_command_reef_lacks_is_found() {
    // a subcommand, a flag and a bare parent reef doesn't take
    assert!(unknown_in("reef node fetch C38").is_some());
    assert!(unknown_in("reef node get C38 --yaml").is_some());
    assert!(unknown_in("reef page").is_some());
    // while ones it takes pass, placeholders and global flags too
    assert_eq!(unknown_in("reef node get <ID> --md"), None);
    assert_eq!(unknown_in("reef status <ID> <STATUS> --conf x.toml"), None);
    assert_eq!(unknown_in("reef check"), None);
}
