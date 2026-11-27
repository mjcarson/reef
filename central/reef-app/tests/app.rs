//! The app's server over shoal in-process: its pages, its server functions and reef's JSON API on
//! one port, with plans put through the CLI as a user would
//!
//! One server serves every case, since the service is the process's own. Each case works in a
//! project of its own.

// a handler's future over shoal's client nests past the compiler's default depth
#![recursion_limit = "256"]

use reef_service::{Conf, Service};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

/// The one server every case talks to, and what it needs to talk to it
struct Fx {
    /// The runtime the server, the service and every case run on
    runtime: Runtime,
    /// Where the app listens
    address: SocketAddr,
    /// The directory every case's repos and files go in
    dir: PathBuf,
    /// A config naming the app's address, for the CLI
    conf: PathBuf,
    /// A plain HTTP client
    http: reqwest::Client,
}

/// The server, started the first time a case asks
fn fx() -> &'static Fx {
    static FX: OnceLock<Fx> = OnceLock::new();
    FX.get_or_init(|| Fx::start().expect("the app's server starts"))
}

impl Fx {
    /// Opens the service over a fresh in-process pool and serves the app's router on a port of
    /// its own
    fn start() -> anyhow::Result<Fx> {
        // a fresh directory under cargo's target dir, rather than a tmpfs `/tmp`
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reef-app");
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        // Dioxus serves the client's files from `public` beside the binary, where dx bundles
        // them, and the suite's binary has none: an empty one serves the pages alone
        let exe = std::env::current_exe()?;
        let public = exe
            .parent()
            .ok_or_else(|| anyhow::anyhow!("{} has no directory", exe.display()))?
            .join("public");
        std::fs::create_dir_all(public)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        // the service over a pool of its own, and the app's router on any free local port
        let address = runtime.block_on(async {
            let mut conf = Conf::default();
            conf.shoal.data = Some(dir.join("shoal"));
            conf.shoal.cores = 1;
            let service = Service::shared(&conf).await?;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            let router = reef_app::server::router(service);
            tokio::spawn(async move { axum::serve(listener, router).await });
            anyhow::Ok(address)
        })?;
        // the CLI finds the app where the config says the JSON API is, served plain here, as
        // only a proxy on this host is, since the suite is of the app and not its listeners
        let conf = dir.join("reef.toml");
        std::fs::write(&conf, format!("[api]\nurl = \"http://{address}\"\n"))?;
        let http = reqwest::Client::builder().no_proxy().build()?;
        Ok(Fx {
            runtime,
            address,
            dir,
            conf,
            http,
        })
    }

    /// Runs one `reef` command from a directory, returning what it printed
    ///
    /// # Arguments
    ///
    /// * `cwd` - The working directory to run it from
    /// * `args` - The command line after `reef --conf <config>`
    async fn reef(&self, cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
        // the command line as typed, with the app's config named
        let conf = self.conf.to_str().expect("the config is a utf-8 path");
        let line = ["reef", "--conf", conf]
            .into_iter()
            .chain(args.iter().copied());
        let cli = reef::Cli::from_args(line)?;
        // run it, keeping what it printed
        let mut out = Vec::new();
        reef::run(cli, cwd, &mut out).await?;
        Ok(String::from_utf8(out)?)
    }

    /// Registers a fresh repo as a project, with its plans listed and one node put in each,
    /// all through the CLI, returning the repo
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name, which names its repo's directory too
    /// * `plans` - The plans to list in it
    async fn project(&self, project: &str, plans: &[&str]) -> anyhow::Result<PathBuf> {
        // a repo of its own, registered under the project's name
        let repo = repo(&self.dir.join(project))?;
        let path = repo.to_str().expect("the repo is a utf-8 path");
        self.reef(&repo, &["project", "add", project, "--repo", path])
            .await?;
        for plan in plans {
            // each plan listed, then given a node, which says why and what
            self.reef(&repo, &["plan", "new", plan]).await?;
            let file = self.dir.join(format!("{project}-{plan}.yaml"));
            std::fs::write(
                &file,
                format!(
                    "id: A1\ntitle: \"A1 The first of {plan}\"\ndescription:\n  why: \"{plan} has nothing in it.\"\n  what: \"{plan} has a node.\"\n"
                ),
            )?;
            let file = file.to_str().expect("the file is a utf-8 path");
            self.reef(&repo, &["node", "put", file, "--plan", plan])
                .await?;
        }
        Ok(repo)
    }

    /// Gets a path from the app, returning its status and its body
    ///
    /// # Arguments
    ///
    /// * `path` - The path, with its query
    async fn get(&self, path: &str) -> anyhow::Result<(u16, String)> {
        let response = self
            .http
            .get(format!("http://{}{path}", self.address))
            .send()
            .await?;
        let status = response.status().as_u16();
        Ok((status, response.text().await?))
    }
}

/// Makes a fresh git repo with one commit
///
/// # Arguments
///
/// * `dir` - Where to make it, wiped first
fn repo(dir: &Path) -> anyhow::Result<PathBuf> {
    // wipe what an earlier run left, and start an empty repo there
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    for args in [
        &["init", "-q"][..],
        &["commit", "-q", "--allow-empty", "-m", "start"][..],
    ] {
        // from the directory alone, as the suite's author, with no one's config
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=reef",
                "-c",
                "user.email=reef@example.invalid",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(dir.canonicalize()?)
}

/// The JSON API answers on the app's own port: the projects, a plan and its payload
#[test]
fn the_json_api_answers_on_the_apps_port() -> anyhow::Result<()> {
    let fx = fx();
    fx.runtime.block_on(async {
        // a project and a plan put through the CLI, which found the app as the JSON API
        fx.project("api", &["central"]).await?;
        // the projects, as the JSON API lists them
        let (status, body) = fx.get(reef_api::paths::PROJECTS).await?;
        assert_eq!(status, 200, "{body}");
        let projects: Vec<reef_api::Project> = serde_json::from_str(&body)?;
        assert!(projects.iter().any(|project| project.name == "api"));
        // the plan's payload holds the node the CLI put
        let plan = reef_model::PlanName::new("api", "central")?;
        let (status, body) = fx.get(&reef_api::paths::payload(&plan)).await?;
        assert_eq!(status, 200, "{body}");
        let payload: reef_model::Payload = serde_json::from_str(&body)?;
        assert_eq!(
            payload
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["A1"]
        );
        anyhow::Ok(())
    })
}

/// The picker is rendered on the server with a link to every plan of every project
#[test]
fn the_picker_links_every_plan() -> anyhow::Result<()> {
    let fx = fx();
    fx.runtime.block_on(async {
        // two plans in one project
        fx.project("picker", &["first", "second"]).await?;
        // the front page, as the server renders it
        let (status, body) = fx.get("/").await?;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("picker"), "{body}");
        for plan in ["first", "second"] {
            let href = format!("href=\"/plans/picker/{plan}\"");
            assert!(body.contains(&href), "no {href} in {body}");
        }
        anyhow::Ok(())
    })
}

/// A plan's page is rendered on the server naming its plan, and a plan that isn't there says so
#[test]
fn a_plans_page_names_its_plan() -> anyhow::Result<()> {
    let fx = fx();
    fx.runtime.block_on(async {
        fx.project("page", &["central"]).await?;
        // the plan's own page
        let (status, body) = fx.get("/plans/page/central").await?;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("page/central"), "{body}");
        assert!(!body.contains("class=\"reef-app-error\""), "{body}");
        // a plan its project doesn't have
        let (_, body) = fx.get("/plans/page/missing").await?;
        assert!(body.contains("is not a plan of its project"), "{body}");
        // the reef.css the pages link
        let (status, body) = fx.get("/reef.css").await?;
        assert_eq!(status, 200);
        assert!(body.contains(".reef"), "reef.css is empty");
        anyhow::Ok(())
    })
}

/// The editor's save writes a node over the version it read, and writes nothing for a stale
/// save, one that would add a problem, or YAML that isn't the node's
#[test]
fn the_editor_saves_over_the_version_it_read() -> anyhow::Result<()> {
    use reef_app::calls::{Saved, node_text, save_node};
    let fx = fx();
    fx.runtime.block_on(async {
        let repo = fx.project("editor", &["central"]).await?;
        // the server functions as the panel calls them, on the server's side
        let read = || node_text("editor".into(), "central".into(), "A1".into());
        let save = |yaml: String, version: u64| {
            save_node(
                "editor".into(),
                "central".into(),
                "A1".into(),
                yaml,
                version,
            )
        };
        let failed = |error: dioxus::prelude::ServerFnError| anyhow::anyhow!("{error}");
        // a new title, over the version read, is written
        let text = read().await.map_err(failed)?;
        let renamed = text
            .yaml
            .replace("The first of central", "The first, renamed");
        let saved = save(renamed, text.version).await.map_err(failed)?;
        assert!(
            matches!(saved, Saved::Applied { version } if version > text.version),
            "{saved:?}"
        );
        let text = read().await.map_err(failed)?;
        assert!(text.yaml.contains("The first, renamed"), "{}", text.yaml);
        // the CLI moves it on, so a save from the older reading is stale and shows the newer
        fx.reef(&repo, &["status", "A1", "done"]).await?;
        let saved = save(text.yaml.clone(), text.version)
            .await
            .map_err(failed)?;
        match saved {
            Saved::Stale { now: Some(now) } => {
                assert!(now.version > text.version);
                assert!(now.yaml.contains("status: done"), "{}", now.yaml);
            }
            other => panic!("a save from an older reading was {other:?}"),
        }
        // a need the plan lacks would add a problem, so it is refused with check's message
        let text = read().await.map_err(failed)?;
        let broken = format!("{}needs: [NOPE]\n", text.yaml);
        match save(broken, text.version).await.map_err(failed)? {
            Saved::Refused { problems } => {
                assert!(problems.iter().any(|p| p.contains("NOPE")), "{problems:?}")
            }
            other => panic!("a need the plan lacks was {other:?}"),
        }
        // YAML that isn't a spec, and another node's, write nothing
        for yaml in ["id: [".to_string(), text.yaml.replace("id: A1", "id: A2")] {
            let saved = save(yaml.clone(), text.version).await.map_err(failed)?;
            assert!(matches!(saved, Saved::Invalid { .. }), "{yaml}: {saved:?}");
        }
        assert_eq!(read().await.map_err(failed)?, text);
        anyhow::Ok(())
    })
}

/// The queue's server functions move an entry by writing its one row, take one off, and the CLI
/// lists the queue as they left it
#[test]
fn the_queue_moves_an_entry_with_one_row() -> anyhow::Result<()> {
    use reef_api::{Place, QueueEdit, Queued};
    use reef_app::calls::{edit_queue, queue};
    let fx = fx();
    fx.runtime.block_on(async {
        // two plans of A1 each, and B1 in the first, queued after them through the CLI
        let repo = fx.project("queued", &["first", "second"]).await?;
        let file = fx.dir.join("queued-b1.yaml");
        std::fs::write(
            &file,
            "id: B1\ntitle: \"B1 The second of first\"\ndescription:\n  why: \"first has one node.\"\n  what: \"first has two.\"\n",
        )?;
        let file = file.to_str().expect("the file is a utf-8 path");
        fx.reef(&repo, &["node", "put", file, "--plan", "first"])
            .await?;
        for node in ["queued/first/A1", "queued/second/A1", "queued/first/B1"] {
            fx.reef(&repo, &["queue", "add", node]).await?;
        }
        let failed = |error: dioxus::prelude::ServerFnError| anyhow::anyhow!("{error}");
        // the queue as the panel reads it
        let ranks = |listing: reef_api::QueueListing| -> Vec<(String, String)> {
            listing
                .entries
                .into_iter()
                .map(|listed| (listed.entry.node.to_string(), listed.rank))
                .collect()
        };
        let before = ranks(queue("queued".into(), "first".into()).await.map_err(failed)?);
        // `first`'s queue alone, A1 then B1
        assert_eq!(before.len(), 2, "{before:?}");
        // B1 to the top, as a drag does it
        let edit = QueueEdit::Move {
            node: "queued/first/B1".parse()?,
            at: Place::Top,
        };
        let moved = edit_queue("queued".into(), edit).await.map_err(failed)?;
        assert!(matches!(moved, Queued::Applied { .. }), "{moved:?}");
        let after = ranks(queue("queued".into(), "first".into()).await.map_err(failed)?);
        // B1 is first, under a rank of its own, and A1's row is as it was
        assert_eq!(after[0].0, "queued/first/B1");
        assert_ne!(after[0].1, before[1].1);
        assert_eq!(&after[1..], &before[..1]);
        // and the CLI lists it first
        let ls = fx.reef(&repo, &["queue", "ls"]).await?;
        assert!(ls.starts_with("1. queued/first/B1 "), "{ls}");
        // taken off through the server function, it is gone from the CLI's listing too
        let edit = QueueEdit::Remove {
            node: "queued/first/B1".parse()?,
        };
        edit_queue("queued".into(), edit).await.map_err(failed)?;
        let ls = fx.reef(&repo, &["queue", "ls"]).await?;
        assert!(!ls.contains("B1"), "{ls}");
        anyhow::Ok(())
    })
}

/// The node menu's server function sets a node's status over the version it read, writes
/// nothing for a status the node has already, and refuses a group, whose status is its members'
#[test]
fn a_status_is_set_over_the_version_read() -> anyhow::Result<()> {
    use reef_app::calls::{StatusSet, node_text, set_status};
    use reef_model::Status;
    let fx = fx();
    fx.runtime.block_on(async {
        // a plan of A1, and a group G1 of one member put beside it through the CLI
        let repo = fx.project("statused", &["only"]).await?;
        let file = fx.dir.join("statused-g1.yaml");
        std::fs::write(
            &file,
            "id: G1\ntitle: \"G1 A group\"\ndescription:\n  why: \"only has no group.\"\n  what: \"only has one.\"\nnodes:\n- id: M1\n  title: \"M1 Its member\"\n  description:\n    why: \"G1 is empty.\"\n    what: \"G1 has a member.\"\n",
        )?;
        let file = file.to_str().expect("the file is a utf-8 path");
        fx.reef(&repo, &["node", "put", file, "--plan", "only"])
            .await?;
        let failed = |error: dioxus::prelude::ServerFnError| anyhow::anyhow!("{error}");
        // the status a node is at, as the editor reads it
        let read = || async {
            node_text("statused".into(), "only".into(), "A1".into())
                .await
                .map_err(failed)
        };
        let set = |id: &str, status: Status| {
            set_status("statused".into(), "only".into(), id.to_string(), status)
        };
        // A1 blocked is written over the version read, a version later
        let before = read().await?;
        match set("A1", Status::Blocked).await.map_err(failed)? {
            StatusSet::Applied { version } => assert!(version > before.version, "{version}"),
            other => panic!("blocking A1 was {other:?}"),
        }
        let blocked = read().await?;
        assert!(blocked.yaml.contains("status: blocked"), "{}", blocked.yaml);
        // blocked again writes nothing
        let again = set("A1", Status::Blocked).await.map_err(failed)?;
        assert_eq!(again, StatusSet::Unchanged);
        assert_eq!(read().await?.version, blocked.version);
        // back to do, the default, its status is left out of its YAML
        let undone = set("A1", Status::Todo).await.map_err(failed)?;
        assert!(matches!(undone, StatusSet::Applied { .. }), "{undone:?}");
        let todo = read().await?;
        assert!(!todo.yaml.contains("status:"), "{}", todo.yaml);
        // a group's status is its members', and a node the plan lacks has none
        assert_eq!(
            set("G1", Status::Done).await.map_err(failed)?,
            StatusSet::Group
        );
        assert_eq!(
            set("NOPE", Status::Done).await.map_err(failed)?,
            StatusSet::NotInPlan
        );
        // the member is a node like any other
        let member = set("M1", Status::InProgress).await.map_err(failed)?;
        assert!(matches!(member, StatusSet::Applied { .. }), "{member:?}");
        anyhow::Ok(())
    })
}

/// A node's captured revisions are served by the app's server functions, each listing its files,
/// and each file's hunks fetched alone, as the Changes tab opens it
#[test]
fn revisions_are_served_a_file_at_a_time() -> anyhow::Result<()> {
    use reef_app::calls::{revision_file, revisions};
    let fx = fx();
    fx.runtime.block_on(async {
        // A1 started in its project's repo, a file written there and staged, and captured
        let repo = fx.project("captured", &["central"]).await?;
        fx.reef(&repo, &["start", "A1"]).await?;
        std::fs::write(repo.join("a.txt"), "one\ntwo\n")?;
        let staged = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "a.txt"])
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .status()?;
        anyhow::ensure!(staged.success(), "git add failed");
        let out = fx.reef(&repo, &["node", "capture"]).await?;
        assert!(out.starts_with("captured `A1` as revision 1"), "{out}");
        let failed = |error: dioxus::prelude::ServerFnError| anyhow::anyhow!("{error}");
        // the revision lists its file without its hunks
        let listed = revisions("captured".into(), "central".into(), "A1".into())
            .await
            .map_err(failed)?;
        let [revision] = &listed[..] else {
            anyhow::bail!("one capture listed {} revisions", listed.len());
        };
        assert_eq!(revision.revision, 1);
        assert_eq!(revision.files[0].path(), "a.txt");
        assert_eq!((revision.files[0].added, revision.files[0].removed), (2, 0));
        // and the file is fetched alone, its lines numbered, or none for a path not listed
        let file = |path: &str| {
            revision_file(
                "captured".into(),
                "central".into(),
                "A1".into(),
                1,
                path.into(),
            )
        };
        let read = file("a.txt").await.map_err(failed)?;
        let read = read.ok_or_else(|| anyhow::anyhow!("the listed file wasn't served"))?;
        let lines: Vec<(Option<u32>, &str)> = read.hunks[0]
            .lines
            .iter()
            .map(|line| (line.new, line.text.as_str()))
            .collect();
        assert_eq!(lines, [(Some(1), "one"), (Some(2), "two")]);
        assert!(file("b.txt").await.map_err(failed)?.is_none());
        anyhow::Ok(())
    })
}

/// A node's timeline is served whole: its edits, its start, its capture and its comment, newest
/// first
#[test]
fn a_nodes_timeline_is_served() -> anyhow::Result<()> {
    use reef_api::Target;
    use reef_app::calls::{node_timeline, start_thread};
    let fx = fx();
    fx.runtime.block_on(async {
        // A1 put, started, a file staged and captured, then a thread started on it
        let repo = fx.project("timeline", &["central"]).await?;
        fx.reef(&repo, &["start", "A1"]).await?;
        std::fs::write(repo.join("a.txt"), "one\ntwo\n")?;
        let staged = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "a.txt"])
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .status()?;
        anyhow::ensure!(staged.success(), "git add failed");
        fx.reef(&repo, &["node", "capture"]).await?;
        let failed = |error: dioxus::prelude::ServerFnError| anyhow::anyhow!("{error}");
        start_thread(
            "timeline".into(),
            "central".into(),
            "A1".into(),
            Target::Node,
            "Why *this*?\n\nMore below.".into(),
            false,
        )
        .await
        .map_err(failed)?;
        // newest first: the comment, the capture, the status `reef start` set once it recorded
        // the start, the start, and A1's addition
        let timeline = node_timeline("timeline".into(), "central".into(), "A1".into(), 0)
            .await
            .map_err(failed)?;
        let told: Vec<(&str, String)> = timeline
            .moments
            .iter()
            .map(|moment| (moment.kind(), moment.describe()))
            .collect();
        let kinds: Vec<&str> = told.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(
            kinds,
            ["comment", "capture", "edit", "start", "edit"],
            "{told:?}"
        );
        assert_eq!(told[0].1, "commented in thread #1: Why *this*?");
        assert_eq!(told[1].1, "captured revision 1: 1 file +2 −0");
        assert_eq!(told[2].1, "status unset → in-progress");
        assert!(told[3].1.starts_with("started on main at "), "{told:?}");
        assert_eq!(told[4].1, "added");
        assert!(!timeline.older);
        assert!(
            timeline
                .moments
                .windows(2)
                .all(|pair| pair[0].at >= pair[1].at)
        );
        anyhow::Ok(())
    })
}

/// Comments are kept and read through the app's server functions: a thread on the node and one
/// on a line of its revision, a reply, and a mark over the version read, a stale one refused;
/// then a review started, its comments read back pending and unmarkable, and submitted at once
#[test]
fn comments_are_kept_through_server_functions() -> anyhow::Result<()> {
    use reef_api::{Anchor, Commented, Marked, ReviewState, Reviewed, Side, Target, ThreadState};
    use reef_app::calls::{
        mark_review, mark_thread, plan_reviews, plan_threads, reply_to, start_thread,
    };
    let fx = fx();
    fx.runtime.block_on(async {
        // A1 started and captured with one new file
        let repo = fx.project("commented", &["central"]).await?;
        fx.reef(&repo, &["start", "A1"]).await?;
        std::fs::write(repo.join("a.txt"), "one\ntwo\n")?;
        let staged = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "a.txt"])
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .status()?;
        anyhow::ensure!(staged.success(), "git add failed");
        fx.reef(&repo, &["node", "capture"]).await?;
        let failed = |error: dioxus::prelude::ServerFnError| anyhow::anyhow!("{error}");
        let (project, plan) = (|| "commented".to_string(), || "central".to_string());
        // a general thread, and one on the file's second line, each kept
        let general = start_thread(
            project(),
            plan(),
            "A1".into(),
            Target::Node,
            "Looks *good*".into(),
            false,
        )
        .await
        .map_err(failed)?;
        assert!(
            matches!(general, Commented::Added { comment: 1, .. }),
            "{general:?}"
        );
        let line = Target::Line(Anchor {
            revision: 1,
            path: "a.txt".to_string(),
            side: Side::New,
            line: 2,
        });
        let on_line = start_thread(
            project(),
            plan(),
            "A1".into(),
            line.clone(),
            "Why two?".into(),
            false,
        )
        .await
        .map_err(failed)?;
        assert!(
            matches!(on_line, Commented::Added { comment: 2, .. }),
            "{on_line:?}"
        );
        // a reply in the line's thread, then the plan's threads read back rendered
        let replied = reply_to(project(), plan(), 2, "Because.".into(), false)
            .await
            .map_err(failed)?;
        assert!(
            matches!(replied, Commented::Added { comment: 3, .. }),
            "{replied:?}"
        );
        let threads = plan_threads(project(), plan()).await.map_err(failed)?;
        let read: Vec<(u32, Target, usize)> = threads
            .iter()
            .map(|thread| (thread.number, thread.target.clone(), thread.comments.len()))
            .collect();
        assert_eq!(read, [(1, Target::Node, 1), (2, line, 2)]);
        assert_eq!(threads[0].comments[0].html, "<p>Looks <em>good</em></p>\n");
        // resolved over the version read, then stale over the same version
        let resolved = mark_thread(project(), plan(), 2, ThreadState::Resolved, 1)
            .await
            .map_err(failed)?;
        let Marked::Marked { thread } = resolved else {
            anyhow::bail!("the mark was refused: {resolved:?}");
        };
        assert_eq!((thread.state, thread.version), (ThreadState::Resolved, 2));
        let stale = mark_thread(project(), plan(), 2, ThreadState::Open, 1)
            .await
            .map_err(failed)?;
        assert!(matches!(stale, Marked::Stale { .. }), "{stale:?}");
        // a thread started in a review, which starts one, and a reply added to it
        let pending = start_thread(
            project(),
            plan(),
            "A1".into(),
            Target::Node,
            "Later, maybe.".into(),
            true,
        )
        .await
        .map_err(failed)?;
        let Commented::Added { thread, comment } = pending else {
            anyhow::bail!("the review's thread wasn't kept: {pending:?}");
        };
        assert!(thread.pending && thread.comments[0].review.is_some());
        assert_eq!(comment, 5);
        let added = reply_to(project(), plan(), 5, "And this.".into(), true)
            .await
            .map_err(failed)?;
        assert!(
            matches!(added, Commented::Added { comment: 6, .. }),
            "{added:?}"
        );
        let reviews = plan_reviews(project(), plan()).await.map_err(failed)?;
        assert_eq!(reviews.len(), 1);
        assert_eq!((reviews[0].number, reviews[0].comments), (4, 2));
        // the app reads it pending, and it can't be marked until the review is submitted
        let threads = plan_threads(project(), plan()).await.map_err(failed)?;
        assert!(
            threads
                .iter()
                .any(|thread| thread.number == 5 && thread.pending)
        );
        let early = mark_thread(project(), plan(), 5, ThreadState::Resolved, 1)
            .await
            .map_err(failed)?;
        assert!(matches!(early, Marked::Pending { thread: 5 }), "{early:?}");
        // submitted over the version read, it is published, and no review is open
        let submitted = mark_review(
            project(),
            plan(),
            "A1".into(),
            ReviewState::Submitted,
            reviews[0].version,
        )
        .await
        .map_err(failed)?;
        assert!(
            matches!(submitted, Reviewed::Marked { .. }),
            "{submitted:?}"
        );
        let threads = plan_threads(project(), plan()).await.map_err(failed)?;
        let published = threads.iter().find(|thread| thread.number == 5).unwrap();
        assert!(!published.pending && published.comments.iter().all(|c| c.review.is_none()));
        assert!(
            plan_reviews(project(), plan())
                .await
                .map_err(failed)?
                .is_empty()
        );
        anyhow::Ok(())
    })
}
