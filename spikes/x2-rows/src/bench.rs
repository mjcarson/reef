//! X2's timings: the example book's 202-node plan loaded, rewritten, pruned and put through
//! shoal, beside the same plan loaded from its markdown
//!
//! Every figure is the median, lowest and highest of `--rounds` timed rounds, after warm-up rounds
//! that are thrown away. The data directory is wiped first.

use reef_model::PlanName;
use std::collections::BTreeMap;
use std::time::Instant;

use crate::book;
use crate::pool::data_dir;
use crate::store::{ImportOptions, Put, SpecFormat};
use crate::{Args, PLAN, PROJECT, Session, ms};

/// Rounds run before the timed ones and thrown away
const WARM_UP: usize = 3;

/// A metric's samples, in milliseconds
#[derive(Debug, Default)]
struct Samples(Vec<f64>);

impl Samples {
    /// The median, lowest and highest sample
    fn summary(&self) -> [f64; 3] {
        // sort a copy, since the samples are kept in the order they were taken
        let mut sorted = self.0.clone();
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len();
        // the middle one, or the mean of the middle two
        let median = if n % 2 == 1 {
            sorted[n / 2]
        } else {
            (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
        };
        [median, sorted[0], sorted[n - 1]]
    }
}

/// Runs every timing and prints them as a table and as JSON
///
/// # Arguments
///
/// * `args` - The command line
pub async fn run(args: &Args) -> anyhow::Result<()> {
    let rounds = args.rounds.max(1);
    let mut metrics: BTreeMap<&'static str, Samples> = BTreeMap::new();
    // a fresh data directory, so earlier runs' rows and tombstones cost nothing
    let dir = data_dir(args.data.clone(), "bench");
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    let src = book::example_book();
    // the book loaded from its markdown, the way mdbook-reef does before rendering
    for round in 0..WARM_UP + rounds {
        let started = Instant::now();
        let (graph, _) = book::graph(book::records(&src)?);
        let elapsed = ms(started.elapsed());
        anyhow::ensure!(graph.nodes.len() == 202, "the book has {} nodes", graph.nodes.len());
        if round >= WARM_UP {
            metrics.entry("book-load-ms").or_default().0.push(elapsed);
        }
    }
    // the flattened specs, and whether each format gives every spec back
    let nodes = book::flatten(book::records(&src)?);
    for format in [SpecFormat::Json, SpecFormat::Yaml] {
        for node in &nodes {
            let back = format.decode(&format.encode(&node.spec)?)?;
            anyhow::ensure!(back == node.spec, "{} does not give `{}` back", format.name(), node.spec.id);
        }
    }
    // what each format costs over all 202 specs
    for (format, encode_key, decode_key) in [
        (SpecFormat::Json, "spec-encode-json-ms", "spec-decode-json-ms"),
        (SpecFormat::Yaml, "spec-encode-yaml-ms", "spec-decode-yaml-ms"),
    ] {
        for round in 0..WARM_UP + rounds {
            // encode them all
            let started = Instant::now();
            let texts = nodes
                .iter()
                .map(|node| format.encode(&node.spec))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let encoded = ms(started.elapsed());
            // and decode them all
            let started = Instant::now();
            for text in &texts {
                format.decode(text)?;
            }
            let decoded = ms(started.elapsed());
            if round >= WARM_UP {
                metrics.entry(encode_key).or_default().0.push(encoded);
                metrics.entry(decode_key).or_default().0.push(decoded);
            }
        }
    }
    // how big a revision's rows are
    let mut sizes = BTreeMap::new();
    for format in [SpecFormat::Json, SpecFormat::Yaml] {
        let mut bytes = 0;
        for node in &nodes {
            bytes += format.encode(&node.spec)?.len() + node.body.len() + node.spec.id.len();
        }
        sizes.insert(format.name(), bytes);
    }
    // a store with the plan twice: its specs as JSON, and as YAML
    let session = Session::open(&dir, true, PLAN).await?;
    let yaml = PlanName::new(PROJECT, "scans-yaml")?;
    session.store.add_plan(&yaml).await?;
    // rewrites: the first has nothing to replace, so it is a warm-up like the next few
    for round in 0..WARM_UP + rounds {
        let imported = session
            .store
            .import(&session.plan, &nodes, SpecFormat::Json, ImportOptions::default())
            .await?;
        if round >= WARM_UP {
            metrics.entry("plan-rewrite-ms").or_default().0.push(ms(imported.rewrite));
            metrics.entry("plan-prune-ms").or_default().0.push(ms(imported.prune));
        }
    }
    session
        .store
        .import(&yaml, &nodes, SpecFormat::Yaml, ImportOptions::default())
        .await?;
    // warm loads, from the partition the rewrites left resident, in both formats
    for (plan, format, key) in [
        (&session.plan, SpecFormat::Json, "plan-load-ms"),
        (&yaml, SpecFormat::Yaml, "plan-load-yaml-ms"),
    ] {
        for round in 0..WARM_UP + rounds {
            let started = Instant::now();
            let loaded = session.store.load(plan, format).await?;
            let (graph, _) = loaded.graph(plan)?;
            let elapsed = ms(started.elapsed());
            anyhow::ensure!(graph.nodes.len() == 202, "the store has {} nodes", graph.nodes.len());
            if round >= WARM_UP {
                metrics.entry(key).or_default().0.push(elapsed);
            }
        }
    }
    // puts of one node, each naming the version the last one wrote
    let id = nodes
        .iter()
        .find(|node| node.body.is_empty())
        .map(|node| node.spec.id.clone())
        .expect("the book lists members inline");
    let mut node = session
        .store
        .get_node(&session.plan, &id, SpecFormat::Json)
        .await?
        .ok_or_else(|| anyhow::anyhow!("node `{id}` is missing"))?;
    for round in 0..WARM_UP + rounds {
        // a put that changes the node's title
        node.spec.title = Some(format!("put {round}"));
        let started = Instant::now();
        let put = session
            .store
            .put_node(&session.plan, &node, Some(node.version), SpecFormat::Json)
            .await?;
        let elapsed = ms(started.elapsed());
        // carry the version it wrote into the next put
        let Put::Applied { version } = put else {
            anyhow::bail!("put {round} was refused: {put:?}");
        };
        node.version = version;
        if round >= WARM_UP {
            metrics.entry("node-put-ms").or_default().0.push(elapsed);
        }
    }
    // stop the pool, so a fresh process can open its data directory
    session.close().await?;
    // cold loads: a fresh process each round, whose first load reads the partition off disk; the
    // first restart still finds rows resident as it compacts the intent log, so it is thrown away
    let exe = std::env::current_exe()?;
    for round in 0..1 + rounds {
        let output = std::process::Command::new(&exe)
            .args(["load", "--format", "json", "--data"])
            .arg(&dir)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "a cold load failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // its one line of JSON says what the load and the pool's start took
        let line: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let field = |name: &str| {
            line[name]
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("a cold load printed no `{name}`"))
        };
        if round == 0 {
            continue;
        }
        metrics.entry("plan-load-cold-ms").or_default().0.push(field("load_ms")?);
        metrics.entry("shoal-ready-ms").or_default().0.push(field("ready_ms")?);
    }
    // the table, then the same as JSON for the plan
    println!("{rounds} timed rounds after {WARM_UP} warm-up rounds; medians [lowest to highest], ms");
    let mut json = serde_json::Map::new();
    for (key, samples) in &metrics {
        let [median, lo, hi] = samples.summary();
        println!("  {key:<22} {median:>9.3} [{lo:.3} to {hi:.3}]");
        json.insert(key.to_string(), serde_json::json!([round3(median), round3(lo), round3(hi)]));
    }
    println!("a revision's rows: {} bytes as JSON, {} bytes as YAML", sizes["json"], sizes["yaml"]);
    println!("{}", serde_json::Value::Object(json));
    Ok(())
}

/// A figure to three decimal places, as the plan records it
///
/// # Arguments
///
/// * `value` - The figure
fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
