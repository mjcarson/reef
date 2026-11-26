//! Times a plan's payload and its writes, with its docs in the repo and as the plan's own pages
//!
//! ```sh
//! cargo +nightly run --release -p reef-service --features embedded --example payload_bench
//! cargo +stable run --release -p reef-service --example payload_bench -- --rounds 20
//! ```
//!
//! The plan is the example book's 202 nodes. Its `src` is copied to a fresh directory outside
//! git, which is registered as the project's repo, and each node is flattened out of its group
//! as the store keeps it, its `docs` rebased from its file onto the repo's root, as an import
//! puts them. The plan is written unchecked, and the problems check finds are printed. It is
//! written twice: once with its docs in the repo, and once into a project whose repo is empty,
//! with the book's 100 other files, 2.4 MB, as the plan's own pages (C28).
//!
//! Each figure is timed over three warm-up rounds, then `--rounds` timed ones (10 by default):
//!
//! - `Renderer::prepare` alone: the plan's stored nodes built into a graph, then checked for doc
//!   links and briefed from their docs by a fresh renderer, as each build of a payload does;
//! - `Service::payload` called in process, and `GET …/payload` from a reqwest client on loopback,
//!   its body decoded into a `Payload`;
//! - a checked put of one node, at the version it was read at, which is one row's write;
//! - a checked batch of two nodes, which is a revision of its own;
//! - and, for the plan with pages, a checked edit of one page, which is one row's write.
//!
//! It runs over the in-memory twin, and with `embedded` over an in-process pool of two shards
//! too, its data under `central/target`.

// a handler's future over shoal's client nests past the compiler's default depth
#![recursion_limit = "256"]

use reef_api::{lift, paths};
use reef_model::spec::parse_node_file;
use reef_model::{Graph, Payload, PlanName, nodes_from_records};
use reef_render::paths::{Target, resolve};
use reef_render::{DocSource, Layered, MemSource, Renderer, RepoSource, Sections};
#[cfg(feature = "embedded")]
use reef_service::Conf;
use reef_service::{
    Asker, Backend, Checked, NodePut, NodeWrite, PageEdit, PageEdited, PageWrite, PlanWrite,
    Service, api,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How many untimed rounds each side runs first
const WARMUP: usize = 3;

/// Times the example book's payload over each backend this build has
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `--rounds N`, else ten
    let rounds = rounds(std::env::args().skip(1))?;
    // the example book, copied to a directory of its own outside git
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let docs = std::env::temp_dir().join(format!("reef-payload-bench-{}", std::process::id()));
    copy_dir(&root.join("example-book/src"), &docs)?;
    let (nodes, pages) = flatten(&docs)?;
    let bytes: usize = pages.iter().map(|page| page.text.len()).sum();
    println!(
        "{} nodes and {} pages ({:.1} MB) from the example book, docs at {}",
        nodes.len(),
        pages.len(),
        bytes as f64 / 1e6,
        docs.display()
    );
    // a repo with no docs in it, for the plan whose docs are its own pages
    let empty = docs.with_extension("empty");
    if empty.exists() {
        std::fs::remove_dir_all(&empty)?;
    }
    std::fs::create_dir_all(&empty)?;
    let book = Book {
        docs: &docs,
        empty: &empty,
        nodes: &nodes,
        pages: &pages,
    };
    // over the twin, always
    bench("twin", Service::memory(), &book, rounds).await?;
    // and over an in-process pool, with the engine
    #[cfg(feature = "embedded")]
    {
        let mut conf = Conf::default();
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/bench/payload");
        if data.exists() {
            std::fs::remove_dir_all(&data)?;
        }
        std::fs::create_dir_all(&data)?;
        conf.shoal.data = Some(data.canonicalize()?);
        conf.shoal.cores = 2;
        let service = Service::open(&conf).await?;
        bench("shoal", service, &book, rounds).await?;
    }
    std::fs::remove_dir_all(&docs)?;
    std::fs::remove_dir_all(&empty)?;
    Ok(())
}

/// The example book, as the plans the bench writes are made of
struct Book<'a> {
    /// The repo holding the book's docs
    docs: &'a Path,
    /// A repo holding none
    empty: &'a Path,
    /// The plan's nodes, each a node of its own
    nodes: &'a [NodeWrite],
    /// The book's other files, as the plan's own pages
    pages: &'a [PageWrite],
}

/// Writes the plan into a service twice, its docs in the repo and as its own pages, serves it,
/// and times each
///
/// # Arguments
///
/// * `label` - The backend's name, for the output
/// * `service` - A fresh service
/// * `book` - The book the plans are made of
/// * `rounds` - How many timed rounds each figure takes
async fn bench<B: Backend + 'static>(
    label: &str,
    service: Service<B>,
    book: &Book<'_>,
    rounds: usize,
) -> anyhow::Result<()> {
    // the plan with its docs in the repo, written unchecked
    let plan = PlanName::new("example", "book")?;
    service
        .add_project("example", book.docs, Asker::default())
        .await?;
    service.add_plan(&plan, &reef_api::NOBODY).await?;
    service
        .store()
        .write_plan(&plan, book.nodes.to_vec().into())
        .await?;
    // and the same plan in a project whose repo is empty, its docs as its own pages
    let paged = PlanName::new("paged", "book")?;
    service
        .add_project("paged", book.empty, Asker::default())
        .await?;
    service.add_plan(&paged, &reef_api::NOBODY).await?;
    let write = PlanWrite {
        nodes: book.nodes.to_vec(),
        pages: Some(book.pages.to_vec()),
    };
    service.store().write_plan(&paged, write).await?;
    // served over plain HTTP on a loopback port of its own, as only a bench is, so the figures
    // are of the payload and its JSON, not of TLS
    let service = Arc::new(service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(api::serve(
        listener,
        service.clone(),
        std::future::pending(),
    ));
    // each timed the same way
    let repo = RepoSource::new(book.docs.to_path_buf());
    let pages: MemSource = book
        .pages
        .iter()
        .map(|page| (page.path.clone(), page.text.clone()))
        .collect();
    let in_pages = Layered::new(&pages, RepoSource::new(book.empty.to_path_buf()));
    time_plan(
        &format!("{label}, docs in the repo"),
        &service,
        &plan,
        &repo,
        address,
        rounds,
        false,
    )
    .await?;
    time_plan(
        &format!("{label}, docs as pages"),
        &service,
        &paged,
        &in_pages,
        address,
        rounds,
        true,
    )
    .await?;
    // stop serving, then close the service once nothing holds it
    server.abort();
    let _ = server.await;
    if let Ok(service) = Arc::try_unwrap(service) {
        close(service).await?;
    }
    Ok(())
}

/// Times one plan's build, payload and writes, and prints the figures
///
/// # Arguments
///
/// * `label` - What the figures are of, for the output
/// * `service` - The service the plan is in
/// * `plan` - The plan
/// * `source` - Where its docs are read from, for timing `prepare` alone
/// * `address` - Where the service is served
/// * `rounds` - How many timed rounds each figure takes
/// * `paged` - Whether the plan's docs are its own pages, so a page edit is timed too
async fn time_plan<B: Backend + 'static, S: DocSource>(
    label: &str,
    service: &Service<B>,
    plan: &PlanName,
    source: &S,
    address: std::net::SocketAddr,
    rounds: usize,
    paged: bool,
) -> anyhow::Result<()> {
    let problems = service.check(plan).await?;
    // the docs' part of a build alone: a fresh renderer checking links and briefing nodes
    let records = service.list(plan).await?.records(plan)?;
    let mut prepare = Vec::with_capacity(rounds);
    for round in 0..WARMUP + rounds {
        let (mut graph, _) = Graph::build(nodes_from_records(records.clone()));
        let renderer = Renderer::new(source, Sections::default());
        let start = Instant::now();
        let found = renderer.prepare(&mut graph);
        let took = start.elapsed();
        std::hint::black_box(found);
        if round >= WARMUP {
            prepare.push(took);
        }
    }
    // in process: the service's own build
    let mut local = Vec::with_capacity(rounds);
    for round in 0..WARMUP + rounds {
        let start = Instant::now();
        let payload = service.payload(plan).await?;
        let took = start.elapsed();
        std::hint::black_box(payload);
        if round >= WARMUP {
            local.push(took);
        }
    }
    // over HTTP: the same build, sent as JSON and decoded
    let url = format!("http://{address}{}", paths::payload(plan));
    let client = reqwest::Client::new();
    let mut remote = Vec::with_capacity(rounds);
    let mut bytes = 0;
    for round in 0..WARMUP + rounds {
        let start = Instant::now();
        let body = client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let payload: Payload = serde_json::from_slice(&body)?;
        let took = start.elapsed();
        bytes = body.len();
        std::hint::black_box(payload);
        if round >= WARMUP {
            remote.push(took);
        }
    }
    // a checked put of one node at the version read, and a batch of two, each a retitling
    let ids: Vec<String> = records
        .iter()
        .rev()
        .take(2)
        .map(|r| r.spec.id.clone())
        .collect();
    let mut put = Vec::with_capacity(rounds);
    let mut batch = Vec::with_capacity(rounds);
    for round in 0..WARMUP + rounds {
        let one = retitled(service, plan, &ids[..1], round).await?;
        let start = Instant::now();
        let checked = service.put(plan, one, &reef_api::NOBODY).await?;
        let took = start.elapsed();
        anyhow::ensure!(matches!(checked, Checked::Applied { .. }), "{checked:?}");
        if round >= WARMUP {
            put.push(took);
        }
        let two = retitled(service, plan, &ids, round).await?;
        let start = Instant::now();
        let checked = service.put(plan, two, &reef_api::NOBODY).await?;
        let took = start.elapsed();
        anyhow::ensure!(matches!(checked, Checked::Applied { .. }), "{checked:?}");
        if round >= WARMUP {
            batch.push(took);
        }
    }
    println!(
        "{label}: {} problems; payload {:.1} KiB\n  prepare:    {}\n  in process: {}\n  over HTTP:  {}\n  node put:   {}\n  node batch: {}",
        problems.len(),
        bytes as f64 / 1024.0,
        figures(&mut prepare),
        figures(&mut local),
        figures(&mut remote),
        figures(&mut put),
        figures(&mut batch)
    );
    // and a checked edit of one page, a line added and taken away again in turn
    if paged {
        let path = "features/scan-rows.md";
        let mut edits = Vec::with_capacity(rounds);
        for round in 0..WARMUP + rounds {
            let page = service
                .page(plan, path)
                .await?
                .ok_or_else(|| anyhow::anyhow!("no page `{path}`"))?;
            let text = match page.text.strip_suffix("\nEdited.\n") {
                Some(text) => text.to_string(),
                None => format!("{}\nEdited.\n", page.text),
            };
            let edit = PageEdit::Put {
                page: PageWrite::new(path, text),
                read: Some(page.version),
            };
            let start = Instant::now();
            let edited = service.edit_page(plan, edit, &reef_api::NOBODY).await?;
            let took = start.elapsed();
            anyhow::ensure!(matches!(edited, PageEdited::Applied { .. }), "{edited:?}");
            if round >= WARMUP {
                edits.push(took);
            }
        }
        println!("  page edit:  {}", figures(&mut edits));
    }
    Ok(())
}

/// Puts naming nodes as they stand, each retitled for a round, at the version read
///
/// # Arguments
///
/// * `service` - The service the plan is in
/// * `plan` - The plan
/// * `ids` - The nodes to retitle
/// * `round` - The round, which the titles name
async fn retitled<B: Backend + 'static>(
    service: &Service<B>,
    plan: &PlanName,
    ids: &[String],
    round: usize,
) -> anyhow::Result<Vec<NodePut>> {
    let mut puts = Vec::with_capacity(ids.len());
    for id in ids {
        // each node as it stands, a title of the round's
        let node = service
            .get(plan, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no node `{id}`"))?;
        let version = node.version;
        let mut write = NodeWrite::from(node);
        write.spec.title = Some(format!("{id} round {round}"));
        puts.push(NodePut::new(write, Some(version)));
    }
    Ok(puts)
}

/// Closes a service, stopping an in-process pool if it is one
///
/// # Arguments
///
/// * `service` - The service to close
async fn close<B: Backend + 'static>(service: Service<B>) -> anyhow::Result<()> {
    // only shoal's service has a pool to stop; the twin just drops
    #[cfg(feature = "embedded")]
    {
        let any: Box<dyn std::any::Any> = Box::new(service);
        if let Ok(service) = any.downcast::<Service>() {
            service.close().await?;
        }
    }
    #[cfg(not(feature = "embedded"))]
    drop(service);
    Ok(())
}

/// A set of rounds as `[median, lo, hi]` in milliseconds
///
/// # Arguments
///
/// * `rounds` - Each round's time, sorted in place
fn figures(rounds: &mut [Duration]) -> String {
    rounds.sort();
    let ms = |took: Duration| took.as_secs_f64() * 1000.0;
    let median = ms(rounds[rounds.len() / 2]);
    let lo = ms(rounds[0]);
    let hi = ms(rounds[rounds.len() - 1]);
    format!(
        "[{median:.3}, {lo:.3}, {hi:.3}] ms over {} rounds",
        rounds.len()
    )
}

/// The number of timed rounds a command line asks for
///
/// # Arguments
///
/// * `args` - The arguments after the program's name
fn rounds(mut args: impl Iterator<Item = String>) -> anyhow::Result<usize> {
    let mut rounds = 10;
    while let Some(arg) = args.next() {
        // `--rounds N`, at least one
        anyhow::ensure!(
            arg == "--rounds",
            "unknown argument `{arg}`: usage: payload_bench [--rounds N]"
        );
        let count = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("`--rounds` needs a count"))?;
        rounds = count.parse()?;
        anyhow::ensure!(rounds > 0, "`--rounds` must be at least 1");
    }
    Ok(rounds)
}

/// Copies a directory's files and directories into a fresh one
///
/// # Arguments
///
/// * `from` - The directory to copy
/// * `to` - Where to copy it, wiped first
fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    // wipe what an earlier run left
    if to.exists() {
        std::fs::remove_dir_all(to)?;
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        // directories by recursion, files by copy
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Every markdown file under a book's `src` but its summary, relative to it and sorted
///
/// # Arguments
///
/// * `src` - The book's `src`
fn markdown_files(src: &Path) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = vec![src.to_path_buf()];
    // walk the whole of `src`, since the example book names no roots
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            // dotfiles are never nodes, and the summary is mdbook's
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if name.ends_with(".md") && name != "SUMMARY.md" {
                let rel = path.strip_prefix(src)?.to_string_lossy().replace('\\', "/");
                out.push(rel);
            }
        }
    }
    // the order files sort in is the order the plan was written in
    out.sort();
    Ok(out)
}

/// Reads a book's nodes, each lifted out of its group as the store keeps it, depth first, and
/// its other files as pages
///
/// # Arguments
///
/// * `src` - The book's `src`, which is also the project's repo
fn flatten(src: &Path) -> anyhow::Result<(Vec<NodeWrite>, Vec<PageWrite>)> {
    let mut out = Vec::new();
    let mut pages = Vec::new();
    for rel in markdown_files(src)? {
        // a file with no node's frontmatter is a page
        let text = std::fs::read_to_string(src.join(&rel))?;
        let Some((spec, fm)) =
            parse_node_file(&text).map_err(|error| anyhow::anyhow!("{rel}: {}", error.message))?
        else {
            pages.push(PageWrite::new(rel, text));
            continue;
        };
        // the node and each of its members, its own body on the node alone
        let mut body = fm.body.to_string();
        for mut spec in lift(spec) {
            // its docs as a stored node's: from the repo's root, not from its file
            spec.docs = spec.docs.iter().map(|link| rebase(&rel, link)).collect();
            let order = out.len() as u64;
            out.push(NodeWrite {
                spec,
                body: std::mem::take(&mut body),
                order,
            });
        }
    }
    Ok((out, pages))
}

/// A link written in a file, as a link from the repo's root
///
/// # Arguments
///
/// * `file` - The file the link was written in, relative to the root
/// * `link` - The link as written
fn rebase(file: &str, link: &str) -> String {
    match resolve(file, link) {
        // a URL or an absolute path is left as written
        Target::External(link) => link,
        Target::Internal { path, anchor } => match anchor {
            Some(anchor) => format!("{path}#{anchor}"),
            None => path,
        },
    }
}
