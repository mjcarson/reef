//! X2: reef's projects, plans and nodes as rows in an in-process shoal pool, priced
//!
//! ```sh
//! cargo +nightly run --release -- import  [--data DIR] [--format json|yaml] [--chunk N] [--die-after N]
//! cargo +nightly run --release -- verify  [--data DIR] [--format json|yaml]
//! cargo +nightly run --release -- stale   [--data DIR]
//! cargo +nightly run --release -- race    [--data DIR] [--no-lock]
//! cargo +nightly run --release -- load    [--data DIR] [--format json|yaml]
//! cargo +nightly run --release -- bench   [--data DIR] [--rounds N]
//! ```
//!
//! Each run starts its own pool over its data directory, so a run killed midway is followed by
//! one that finds what it left on disk.

mod bench;
mod book;
mod pool;
mod store;
mod tables;

use reef_model::{Graph, Node, PlanName};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use pool::Pool;
use store::{ImportOptions, Pause, Put, SpecFormat, Store, StoredNode};

/// The project the example book is imported into
pub const PROJECT: &str = "example";

/// The plan the example book is imported as
pub const PLAN: &str = "scans";

/// What a run was asked to do
#[derive(Debug, Clone)]
pub struct Args {
    /// The subcommand
    pub command: String,
    /// The data directory, if one was named
    pub data: Option<PathBuf>,
    /// How specs are serialized
    pub format: SpecFormat,
    /// How many rows an import sends in one bundle
    pub chunk: usize,
    /// Abort an import once this many rows are written
    pub die_after: Option<usize>,
    /// Leave writes to a plan unserialized, to show the race the lock closes
    pub no_lock: bool,
    /// How many timed rounds a bench runs
    pub rounds: usize,
}

impl Args {
    /// Reads the command line
    pub fn parse() -> anyhow::Result<Args> {
        let mut words = std::env::args().skip(1);
        let command = words
            .next()
            .ok_or_else(|| anyhow::anyhow!("name a command: import, verify, stale, race, load or bench"))?;
        let mut args = Args {
            command,
            data: None,
            format: SpecFormat::Json,
            chunk: usize::MAX,
            die_after: None,
            no_lock: false,
            rounds: 10,
        };
        // each flag takes the word after it, but `--no-lock`
        while let Some(flag) = words.next() {
            let mut value = || {
                words
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("`{flag}` needs a value"))
            };
            match flag.as_str() {
                "--data" => args.data = Some(PathBuf::from(value()?)),
                "--format" => args.format = SpecFormat::parse(&value()?)?,
                "--chunk" => args.chunk = value()?.parse()?,
                "--die-after" => args.die_after = Some(value()?.parse()?),
                "--rounds" => args.rounds = value()?.parse()?,
                "--no-lock" => args.no_lock = true,
                other => anyhow::bail!("unknown flag `{other}`"),
            }
        }
        Ok(args)
    }
}

/// A pool, a store over it, and the example plan listed in it
pub struct Session {
    /// The in-process pool
    pub pool: Pool,
    /// The store every command goes through
    pub store: Store,
    /// The example plan's full name
    pub plan: PlanName,
}

impl Session {
    /// Starts a pool over a data directory and lists the example project and plan in it
    ///
    /// # Arguments
    ///
    /// * `dir` - The data directory
    /// * `locking` - Whether to serialize writes to each plan
    /// * `plan` - The plan's name within the example project
    pub async fn open(dir: &Path, locking: bool, plan: &str) -> anyhow::Result<Session> {
        // start the pool and connect to it
        let pool = Pool::start(dir).await?;
        let client = pool::connect(pool.addr).await?;
        let store = Store::new(client, locking);
        // list the project and its plan, which a run before this one may already have done
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize()?;
        store.add_project(PROJECT, &repo.to_string_lossy()).await?;
        let plan = PlanName::new(PROJECT, plan)?;
        store.add_plan(&plan).await?;
        Ok(Session { pool, store, plan })
    }

    /// Stops the pool
    pub async fn close(self) -> anyhow::Result<()> {
        // the client goes first, then every shard is stopped
        drop(self.store);
        self.pool.close().await
    }
}

/// The comparable parts of a node: everything but where it was written and whether it was inline
#[derive(Debug, PartialEq)]
struct Shape<'a> {
    /// The id
    id: &'a str,
    /// The title
    title: &'a str,
    /// The written status
    status: Option<reef_model::Status>,
    /// Whether it is optional
    optional: bool,
    /// Its group
    parent: Option<&'a str>,
    /// What it needs
    needs: &'a [String],
    /// What it helps
    helps: &'a [String],
    /// Whether it starts collapsed
    collapsed: Option<bool>,
    /// Its doc links as written
    docs: &'a [String],
    /// Its body
    body: &'a str,
    /// Its description as written
    description: Option<&'a reef_model::Description>,
    /// Its design choices as written
    decisions: &'a [String],
    /// Its measurements
    performance: Option<&'a reef_model::Performance>,
    /// Its acceptance criteria
    acceptance: &'a [reef_model::Criterion],
}

impl<'a> Shape<'a> {
    /// Takes the comparable parts of a node
    ///
    /// # Arguments
    ///
    /// * `node` - The node to compare
    fn of(node: &'a Node) -> Shape<'a> {
        Shape {
            id: &node.id,
            title: &node.title,
            status: node.status,
            optional: node.optional,
            parent: node.parent.as_deref(),
            needs: &node.needs,
            helps: &node.helps,
            collapsed: node.collapsed,
            docs: &node.docs_raw,
            body: &node.body,
            description: node.description_raw.as_ref(),
            decisions: &node.decisions_raw,
            performance: node.performance.as_ref(),
            acceptance: &node.acceptance,
        }
    }
}

/// Every way two graphs' nodes differ, in written order
///
/// # Arguments
///
/// * `book` - The graph the book builds
/// * `stored` - The graph the store's rows build
fn differences(book: &Graph, stored: &Graph) -> Vec<String> {
    let mut out = Vec::new();
    // the same nodes, in the same order
    let ids = |graph: &Graph| graph.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
    if ids(book) != ids(stored) {
        out.push(format!(
            "the nodes differ: the book has {} and the store {}",
            book.nodes.len(),
            stored.nodes.len()
        ));
        return out;
    }
    // and each one the same
    for (left, right) in book.nodes.iter().zip(&stored.nodes) {
        if Shape::of(left) != Shape::of(right) {
            out.push(format!("node `{}` differs:\n  book:  {:?}\n  store: {:?}", left.id, Shape::of(left), Shape::of(right)));
        }
    }
    out
}

/// Imports the example book as a new revision
///
/// # Arguments
///
/// * `args` - The command line
async fn import(args: &Args) -> anyhow::Result<()> {
    let session = Session::open(&pool::data_dir(args.data.clone(), "run"), true, PLAN).await?;
    // read and flatten the book
    let nodes = book::flatten(book::records(&book::example_book())?);
    let options = ImportOptions {
        chunk: args.chunk,
        die_after: args.die_after,
        pause: None,
    };
    let imported = session.store.import(&session.plan, &nodes, args.format, options).await?;
    println!(
        "imported {} nodes as revision {:016x} in {:.3} ms, pruned the last in {:.3} ms, recovered {} rows",
        nodes.len(),
        imported.revision,
        ms(imported.rewrite),
        ms(imported.prune),
        imported.recovered
    );
    session.close().await
}

/// Loads the current revision and checks it builds the book's graph
///
/// # Arguments
///
/// * `args` - The command line
async fn verify(args: &Args) -> anyhow::Result<bool> {
    let session = Session::open(&pool::data_dir(args.data.clone(), "run"), true, PLAN).await?;
    // the graph the book builds, and the one the store's rows build
    let (book, book_diags) = book::graph(book::records(&book::example_book())?);
    let loaded = session.store.load(&session.plan, args.format).await?;
    let revision = loaded.revision;
    let (stored, stored_diags) = loaded.graph(&session.plan)?;
    // the listings name the project and its plan
    let projects = session.store.list_projects().await?;
    let plans = session.store.list_plans(PROJECT).await?;
    let listed = projects.iter().any(|p| p.name == PROJECT) && plans.iter().any(|p| p.name == PLAN);
    let mut problems = differences(&book, &stored);
    if !listed {
        problems.push(format!("the listings miss `{PROJECT}/{PLAN}`"));
    }
    if stored_diags.len() != book_diags.len() {
        problems.push(format!(
            "the store's graph has {} problems and the book's {}: {:?}",
            stored_diags.len(),
            book_diags.len(),
            stored_diags.iter().map(ToString::to_string).collect::<Vec<_>>()
        ));
    }
    // say what was found
    for problem in &problems {
        println!("{problem}");
    }
    if problems.is_empty() {
        println!(
            "verified: {} nodes at revision {revision:016x} build the book's graph, with {} problems; {} projects and {} plans listed",
            stored.nodes.len(),
            stored_diags.len(),
            projects.len(),
            plans.len()
        );
    }
    session.close().await?;
    Ok(problems.is_empty())
}

/// Imports the book if the plan has no revision yet, and returns the first node's id
///
/// # Arguments
///
/// * `session` - The session to import into
/// * `format` - How to serialize the specs
async fn ensure_imported(session: &Session, format: SpecFormat) -> anyhow::Result<String> {
    let nodes = book::flatten(book::records(&book::example_book())?);
    // a plan with no revision gets the book
    let row = session.store.plan_row(&session.plan).await?;
    if row.is_none_or(|row| row.current == 0) {
        session
            .store
            .import(&session.plan, &nodes, format, ImportOptions::default())
            .await?;
    }
    // a member listed inline, which is what most nodes are
    Ok(nodes
        .iter()
        .find(|node| node.body.is_empty())
        .map(|node| node.spec.id.clone())
        .expect("the book lists members inline"))
}

/// Two readers edit one node from the same version: the first put is written, the second refused
///
/// # Arguments
///
/// * `args` - The command line
async fn stale(args: &Args) -> anyhow::Result<bool> {
    let session = Session::open(&pool::data_dir(args.data.clone(), "run"), true, PLAN).await?;
    let id = ensure_imported(&session, args.format).await?;
    // two readers read the node at the same version
    let read = session
        .store
        .get_node(&session.plan, &id, args.format)
        .await?
        .ok_or_else(|| anyhow::anyhow!("node `{id}` is missing"))?;
    let edit = |title: &str| StoredNode {
        spec: reef_model::NodeSpec {
            title: Some(title.to_string()),
            ..read.spec.clone()
        },
        ..read.clone()
    };
    // the first writes its edit, and the second, still at the old version, is refused
    let first = session
        .store
        .put_node(&session.plan, &edit("first writer"), Some(read.version), args.format)
        .await?;
    let second = session
        .store
        .put_node(&session.plan, &edit("second writer"), Some(read.version), args.format)
        .await?;
    // the node holds the first writer's edit
    let now = session.store.get_node(&session.plan, &id, args.format).await?;
    let title = now.as_ref().and_then(|node| node.spec.title.clone());
    println!("node `{id}` read at version {}: first put {first:?}, second put {second:?}, title now {title:?}", read.version);
    let held = matches!(first, Put::Applied { .. })
        && matches!(second, Put::Stale(_))
        && title.as_deref() == Some("first writer");
    session.close().await?;
    Ok(held)
}

/// A put made between an import's rows and its flip, from a version read before the import
///
/// With the plan's lock the put waits for the flip and is refused as stale. Without it, the put
/// is written into the revision the import retires, and lost.
///
/// # Arguments
///
/// * `args` - The command line
async fn race(args: &Args) -> anyhow::Result<()> {
    let session = Session::open(&pool::data_dir(args.data.clone(), "run"), !args.no_lock, PLAN).await?;
    let id = ensure_imported(&session, args.format).await?;
    let nodes = book::flatten(book::records(&book::example_book())?);
    // a reader reads the node before the import starts
    let read = session
        .store
        .get_node(&session.plan, &id, args.format)
        .await?
        .ok_or_else(|| anyhow::anyhow!("node `{id}` is missing"))?;
    let edit = StoredNode {
        spec: reef_model::NodeSpec {
            title: Some("edited during an import".to_string()),
            ..read.spec.clone()
        },
        ..read.clone()
    };
    // the import pauses between its rows and its flip
    let pause = Pause::default();
    let options = ImportOptions {
        pause: Some(&pause),
        ..ImportOptions::default()
    };
    let import = session.store.import(&session.plan, &nodes, args.format, options);
    let racer = async {
        // once the import has written its rows, put the edit
        pause.reached.notified().await;
        let put = session.store.put_node(&session.plan, &edit, Some(read.version), args.format);
        tokio::pin!(put);
        // a put the lock holds back can't finish until the import flips, so give it a while
        let early = tokio::select! {
            put = &mut put => Some(put),
            _ = tokio::time::sleep(Duration::from_millis(200)) => None,
        };
        // then let the import flip, and wait for a put still held back
        pause.resume.notify_one();
        match early {
            Some(put) => (put, true),
            None => (put.await, false),
        }
    };
    let (imported, (put, before_flip)) = tokio::join!(import, racer);
    imported?;
    let put = put?;
    // what the plan holds once the import is current
    let now = session.store.get_node(&session.plan, &id, args.format).await?;
    let kept = now.as_ref().and_then(|node| node.spec.title.as_deref()) == Some("edited during an import");
    let outcome = match (put, kept) {
        (Put::Applied { .. }, true) => "kept",
        (Put::Applied { .. }, false) => "lost",
        (Put::Stale(_), _) => "refused",
    };
    println!(
        "lock {}: put {put:?} {} the flip; outcome: {outcome}",
        if args.no_lock { "off" } else { "on" },
        if before_flip { "before" } else { "after" }
    );
    session.close().await
}

/// Loads the plan once and prints what that took, for a bench's cold rounds
///
/// # Arguments
///
/// * `args` - The command line
async fn load(args: &Args) -> anyhow::Result<()> {
    let session = Session::open(&pool::data_dir(args.data.clone(), "run"), true, PLAN).await?;
    // the first load after the pool starts reads the partition off disk
    let started = Instant::now();
    let loaded = session.store.load(&session.plan, args.format).await?;
    let (graph, _) = loaded.graph(&session.plan)?;
    let load_ms = ms(started.elapsed());
    println!(
        "{{\"load_ms\":{load_ms:.4},\"ready_ms\":{:.4},\"nodes\":{}}}",
        ms(session.pool.ready_in),
        graph.nodes.len()
    );
    session.close().await
}

/// A duration in milliseconds
///
/// # Arguments
///
/// * `elapsed` - The duration
pub fn ms(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

/// Runs the command asked for
#[tokio::main]
async fn main() -> ExitCode {
    // shoal's spans and warnings, filtered by RUST_LOG
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    // run the command, and fail with its causes
    let result = async {
        let args = Args::parse()?;
        match args.command.as_str() {
            "import" => import(&args).await.map(|()| true),
            "verify" => verify(&args).await,
            "stale" => stale(&args).await,
            "race" => race(&args).await.map(|()| true),
            "load" => load(&args).await.map(|()| true),
            "bench" => bench::run(&args).await.map(|()| true),
            other => anyhow::bail!("unknown command `{other}`"),
        }
    }
    .await;
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("x2: {error:#}");
            ExitCode::FAILURE
        }
    }
}
