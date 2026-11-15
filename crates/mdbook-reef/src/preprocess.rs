//! The mdbook preprocessor.

use crate::paths;
use crate::plan::{self, Config, Plan};
use anyhow::{Context, Result, bail};
use mdbook_preprocessor::book::{Book, BookItem};
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use reef_model::spec::frontmatter_is_yaml;
use std::collections::HashSet;

pub struct Reef;

impl Preprocessor for Reef {
    fn name(&self) -> &str {
        "reef"
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> mdbook_preprocessor::errors::Result<Book> {
        let config: Config = ctx.config.get("preprocessor.reef").context("reading [preprocessor.reef]")?.unwrap_or_default();
        let src = ctx.root.join(&ctx.config.book.src);
        let plan = Plan::load(&src, &config);
        if !plan.diags.is_empty() {
            bail!("reef found {} problem(s) in the plan:\n{}", plan.diags.len(), plan.errors());
        }
        let chapters: HashSet<String> = book
            .iter()
            .filter_map(|item| match item {
                BookItem::Chapter(ch) => ch.source_path.as_ref().map(|p| p.to_string_lossy().replace('\\', "/")),
                _ => None,
            })
            .collect();
        let mut payload: Option<String> = None;
        let mut errors: Vec<String> = Vec::new();
        book.for_each_chapter_mut(|ch| {
            if let Some(fm) = frontmatter_is_yaml(&ch.content) {
                ch.content = fm.body.to_string();
            }
            let fences = plan::find_fences(&ch.content);
            if fences.is_empty() {
                return;
            }
            let page = ch.path.as_ref().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let where_ = ch.source_path.as_ref().map_or(page.clone(), |p| p.to_string_lossy().to_string());
            let from = ch.source_path.as_ref().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            for (line, e) in plan::check_fences_together(&fences, &from, |p| chapters.contains(p)) {
                errors.push(format!("  {where_}: reef fence at line {line}: {e}"));
            }
            let mut content = ch.content.clone();
            for f in fences.iter().rev() {
                let spec = match &f.spec {
                    Ok(s) => s,
                    Err(e) => {
                        errors.push(format!("  {where_}: reef fence at line {}: {e}", f.line));
                        continue;
                    }
                };
                for e in spec.check(&plan.graph) {
                    errors.push(format!("  {where_}: reef fence at line {}: {e}", f.line));
                }
                content.replace_range(f.range.clone(), &plan::fence_html(spec));
            }
            let json = payload.get_or_insert_with(|| plan.payload_json(&chapters));
            content.push_str("\n\n");
            content.push_str(&plan::data_script(json, &paths::prefix_for(&page)));
            content.push('\n');
            ch.content = content;
        });
        if !errors.is_empty() {
            bail!("reef found {} problem(s) in its fences:\n{}", errors.len(), errors.join("\n"));
        }
        Ok(book)
    }

    fn supports_renderer(&self, renderer: &str) -> mdbook_preprocessor::errors::Result<bool> {
        Ok(renderer == "html")
    }
}

/// Run as mdbook calls a preprocessor: the book on stdin, the book on stdout.
pub fn run_stdio() -> Result<()> {
    let (ctx, book) = mdbook_preprocessor::parse_input(std::io::stdin())?;
    if ctx.mdbook_version != mdbook_preprocessor::MDBOOK_VERSION {
        eprintln!(
            "warning: mdbook-reef was built against mdbook {}, called from {}",
            mdbook_preprocessor::MDBOOK_VERSION,
            ctx.mdbook_version
        );
    }
    let book = Reef.run(&ctx, book)?;
    serde_json::to_writer(std::io::stdout(), &book)?;
    Ok(())
}
