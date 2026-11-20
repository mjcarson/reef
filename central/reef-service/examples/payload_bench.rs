//! Times a plan's payload: built in process, and fetched over the JSON API on loopback
//!
//! ```sh
//! cargo +nightly run --release -p reef-service --features embedded --example payload_bench
//! cargo +stable run --release -p reef-service --example payload_bench -- --rounds 20
//! ```
//!
//! The plan is the example book's 202 nodes. Its `src` is copied to a fresh directory outside
//! git, which is registered as the project's repo, and each node is flattened out of its group
//! as the store keeps it, its `docs` rebased from its file onto the repo's root, as an import
//! will put them. The plan is written unchecked, and the problems check finds are printed.
//!
//! Each side is timed over three warm-up rounds, then `--rounds` timed ones (10 by default):
//! `Service::payload` called in process, and `GET …/payload` from a reqwest client on loopback,
//! its body decoded into a `Payload`. It runs over the in-memory twin, and with `embedded` over
//! an in-process pool of two shards too, its data under `central/target`.

// a handler's future over shoal's client nests past the compiler's default depth
#![recursion_limit = "256"]

use reef_api::paths;
use reef_model::spec::parse_node_file;
use reef_model::{NodeSpec, Payload, PlanName};
use reef_render::paths::{Target, resolve};
use reef_service::{Backend, Conf, NodeWrite, Service, api};
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
    let nodes = flatten(&docs)?;
    println!(
        "{} nodes from the example book, docs at {}",
        nodes.len(),
        docs.display()
    );
    // over the twin, always
    bench("twin", Service::memory(), &docs, nodes.clone(), rounds).await?;
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
        bench("shoal", service, &docs, nodes, rounds).await?;
    }
    std::fs::remove_dir_all(&docs)?;
    Ok(())
}

/// Writes the plan into a service, serves it, and times its payload both ways
///
/// # Arguments
///
/// * `label` - The backend's name, for the output
/// * `service` - A fresh service
/// * `docs` - The project's repo, holding the plan's docs
/// * `nodes` - The plan's nodes, each a node of its own
/// * `rounds` - How many timed rounds each side runs
async fn bench<B: Backend + 'static>(
    label: &str,
    service: Service<B>,
    docs: &Path,
    nodes: Vec<NodeWrite>,
    rounds: usize,
) -> anyhow::Result<()> {
    // the plan, written unchecked, and what check finds in it
    let plan = PlanName::new("example", "book")?;
    service.add_project("example", docs).await?;
    service.add_plan(&plan).await?;
    service.store().write_plan(&plan, nodes).await?;
    let problems = service.check(&plan).await?;
    // served on a loopback port of its own
    let service = Arc::new(service);
    let mut conf = Conf::default();
    conf.api.address = "127.0.0.1:0".parse()?;
    let listener = api::bind(&conf).await?;
    let url = format!("http://{}{}", listener.local_addr()?, paths::payload(&plan));
    let server = tokio::spawn(api::serve(
        listener,
        service.clone(),
        std::future::pending(),
    ));
    // in process: the service's own build
    let mut local = Vec::with_capacity(rounds);
    for round in 0..WARMUP + rounds {
        let start = Instant::now();
        let payload = service.payload(&plan).await?;
        let took = start.elapsed();
        std::hint::black_box(payload);
        if round >= WARMUP {
            local.push(took);
        }
    }
    // over HTTP: the same build, sent as JSON and decoded
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
    // the figures, as [median, lo, hi] in ms
    println!(
        "{label}: {} problems; payload {:.1} KiB\n  in process: {}\n  over HTTP:  {}",
        problems.len(),
        bytes as f64 / 1024.0,
        figures(&mut local),
        figures(&mut remote)
    );
    // stop serving, then close the service once nothing holds it
    server.abort();
    let _ = server.await;
    if let Ok(service) = Arc::try_unwrap(service) {
        close(service).await?;
    }
    Ok(())
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

/// Reads a book's nodes, each lifted out of its group as the store keeps it, depth first
///
/// # Arguments
///
/// * `src` - The book's `src`, which is also the project's repo
fn flatten(src: &Path) -> anyhow::Result<Vec<NodeWrite>> {
    let mut out = Vec::new();
    for rel in markdown_files(src)? {
        // a page with no frontmatter is not a node
        let text = std::fs::read_to_string(src.join(&rel))?;
        let Some((spec, fm)) =
            parse_node_file(&text).map_err(|error| anyhow::anyhow!("{rel}: {}", error.message))?
        else {
            continue;
        };
        lift(spec, fm.body.to_string(), &rel, &mut out);
    }
    Ok(out)
}

/// Pushes a spec, then each of its members after it, its links rebased onto the root
///
/// # Arguments
///
/// * `spec` - The spec to push, members and all
/// * `body` - Its body: a file node's page, or empty for a member
/// * `file` - The file it was written in, relative to `src`
/// * `out` - The flattened nodes so far
fn lift(mut spec: NodeSpec, body: String, file: &str, out: &mut Vec<NodeWrite>) {
    // take its members out of it, so the store holds it alone
    let members = std::mem::take(&mut spec.nodes);
    let id = spec.id.clone();
    // its docs as a stored node's: from the repo's root, not from its file
    spec.docs = spec.docs.iter().map(|link| rebase(file, link)).collect();
    let order = out.len() as u64;
    out.push(NodeWrite { spec, body, order });
    for mut member in members {
        // a member belongs to the node that lists it unless it names a group
        if member.group.is_none() {
            member.group = Some(id.clone());
        }
        // its body stays in its spec, as it was written
        lift(member, String::new(), file, out);
    }
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
