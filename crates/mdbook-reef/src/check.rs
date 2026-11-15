//! `mdbook-reef check`: the preprocessor's checks without a build, for CI.

use crate::plan::{self, Config, Plan};
use anyhow::{Context, Result};
use mdbook_preprocessor::config::Config as BookConfig;
use std::path::Path;

/// Load a book's config and its `src` directory.
pub fn open_book(dir: &Path) -> Result<(BookConfig, std::path::PathBuf, Config)> {
    let book_toml = dir.join("book.toml");
    let cfg = BookConfig::from_disk(&book_toml).with_context(|| format!("reading {}", book_toml.display()))?;
    let src = dir.join(&cfg.book.src);
    let reef: Config = cfg.get("preprocessor.reef")?.unwrap_or_default();
    Ok((cfg, src, reef))
}

/// Check every node and every fence. Returns the number of problems found.
pub fn check(dir: &Path, quiet: bool) -> Result<usize> {
    let (_, src, reef) = open_book(dir)?;
    let plan = Plan::load(&src, &reef);
    let mut problems: Vec<String> = plan.diags.iter().map(|d| d.to_string()).collect();
    let mut fences = 0;
    for rel in plan::markdown_files(&src, &[]) {
        let Ok(text) = std::fs::read_to_string(src.join(&rel)) else { continue };
        let body_off = reef_model::spec::frontmatter_is_yaml(&text).map_or(0, |fm| fm.body_offset);
        let body_line = text[..body_off].matches('\n').count();
        let found = plan::find_fences(&text[body_off..]);
        for (line, e) in plan::check_fences_together(&found, &rel, |p| src.join(p).is_file()) {
            problems.push(format!("{rel}:{}: {e}", line + body_line));
        }
        for f in found {
            fences += 1;
            match &f.spec {
                Ok(spec) => {
                    for e in spec.check(&plan.graph) {
                        problems.push(format!("{rel}:{}: {e}", f.line + body_line));
                    }
                }
                Err(e) => problems.push(format!("{rel}:{}: {e}", f.line + body_line)),
            }
        }
    }
    for p in &problems {
        eprintln!("error: {p}");
    }
    if !quiet {
        let g = &plan.graph;
        let groups = (0..g.len()).filter(|&i| g.is_group(i)).count();
        let done = (0..g.len()).filter(|&i| !g.is_group(i) && g.status(i) == reef_model::Status::Done).count();
        let ready = (0..g.len()).filter(|&i| g.is_ready(i)).count();
        let edges: usize = (0..g.len()).map(|i| g.needs(i).len() + g.helps(i).len()).sum();
        eprintln!(
            "{} nodes ({} groups), {} arrows, {} fences; {} of {} work items done, {} ready to start; {} problem(s)",
            g.len(),
            groups,
            edges,
            fences,
            done,
            g.len() - groups,
            ready,
            problems.len()
        );
    }
    Ok(problems.len())
}
