//! The docs each plan names: a folder or a summary read as its book, named, changed or cleared,
//! and checked as every write is (C122)
//!
//! - **A plan names a folder or a summary.** A path ending `.md` is a summary in mdbook's
//!   format, and any other a folder; a trailing `/` is dropped, and `.` or the empty path is the
//!   root. Either is read over the plan's pages and its repo's files at the branch it follows.
//! - **Docs that aren't there are refused.** The book is read with the docs the plan names now
//!   and with the new ones, under its write lock, over one cached read of its docs, and a change
//!   whose book adds a problem is [`DocsNamed::Refused`] with check's messages. Only the book's
//!   problems can differ between the two, so the plan itself isn't built twice.
//! - **Every other write checks the book too** ([`checked`](super::checked)), so a page edit that
//!   takes a chapter away is refused, a branch switch to docs without the chapters is refused,
//!   and `check` lists a chapter a fetch or the working tree took away.
//! - **A change is told to the change stream** as [`Change::PlanDocsNamed`], once it is kept in
//!   the plan's history.

use reef_api::{Change, DocsNamed};
use reef_model::{Diagnostic, PlanName};
use reef_render::chapters::{docs_path, read_book};
use reef_render::{Cached, DocSource, Layered};
use tracing::instrument;

use super::{Asker, Service, new_problems, page_source};
use crate::backend::Backend;
use crate::store::{DocsProposal, DocsSet, NotListed, Project};
use crate::tree::Docs;

impl<B: Backend> Service<B> {
    /// Names the docs a plan reads as its book, or clears them so its book is its root node,
    /// then its pages, unless the docs named aren't there or would add a problem
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `docs` - The folder or summary to name, from the root of its docs, or `None` for none
    /// * `asker` - Who changes it, from the caller's token
    #[instrument(name = "Service::set_plan_docs", skip_all, err(Debug))]
    pub async fn set_plan_docs(
        &self,
        plan: &PlanName,
        docs: Option<String>,
        asker: &Asker<'_>,
    ) -> anyhow::Result<DocsNamed> {
        // make sure the plan's project is registered and get it to read the plan's repo from
        let project = self.project_named(&plan.project).await?;
        // refuse a plan its project doesn't list
        let listed = self.store.plans(&plan.project).await?;
        if !listed.iter().any(|info| info.name == *plan) {
            return Err(NotListed(plan.clone()).into());
        }
        // normalize the path so `.` names the root and a trailing `/` is dropped
        let docs = docs.map(|docs| docs_path(&docs));
        // name the docs under the plan's write lock unless they add problems to its book
        let set = self
            .store
            .set_plan_docs(plan, docs.clone(), &asker.author(), move |proposal| {
                docs_added(project, proposal)
            })
            .await?;
        match set {
            // pass check's problems back since nothing was written
            DocsSet::Refused(problems) => Ok(DocsNamed::Refused(problems)),
            DocsSet::Applied { changed } => {
                // tell the change stream once the change is kept, if it changed what is named
                if changed {
                    self.publish(Change::PlanDocsNamed {
                        plan: plan.clone(),
                        docs: docs.clone(),
                    });
                }
                // read the plan's listing again so the answer names the docs it reads now
                let info = self
                    .store
                    .plans(&plan.project)
                    .await?
                    .into_iter()
                    .find(|info| info.name == *plan)
                    .ok_or_else(|| NotListed(plan.clone()))?;
                Ok(DocsNamed::Applied {
                    plan: info,
                    changed,
                })
            }
        }
    }
}

/// The problems a change to the docs a plan names would add to its book, its docs read at the
/// branch it follows as its write lock read it
///
/// Both books are read over one cached read of the plan's pages and its repo, so a doc both
/// name is read once.
///
/// # Arguments
///
/// * `project` - The plan's project, whose repo the docs are read from
/// * `proposal` - The plan's pages and branch, and the docs it names now and would name
#[instrument(name = "docs_added", skip_all, err(Debug))]
async fn docs_added(project: Project, proposal: DocsProposal) -> anyhow::Result<Vec<Diagnostic>> {
    // find where the plan's docs are read at the branch its write lock read
    let repo = Docs::of(&project, proposal.branch.as_deref())?;
    // read both books on a blocking thread since the docs may be files on disk or git objects
    let added = tokio::task::spawn_blocking(move || {
        // lay the plan's pages over its repo and cache each read so both books share them
        let pages = page_source(proposal.pages);
        let source = Cached::new(Layered::new(&pages, repo.source()?));
        // keep only the problems the new docs find that the docs named now don't
        let before = book_problems(&source, proposal.from.as_deref());
        let after = book_problems(&source, proposal.to.as_deref());
        anyhow::Ok(new_problems(&before, after))
    })
    .await??;
    Ok(added)
}

/// The problems a plan's book has when it is read from the docs it names, or none when it
/// names none, since its root node and pages always read
///
/// # Arguments
///
/// * `source` - The plan's pages over its repo
/// * `named` - The folder or summary the plan names, if any
fn book_problems<S: DocSource>(source: &S, named: Option<&str>) -> Vec<Diagnostic> {
    match named {
        // read the book from the docs it names and keep only its problems
        Some(named) => read_book(source, named).1,
        // add no problems for a plan that names no docs since its root node and pages are there
        None => Vec::new(),
    }
}
