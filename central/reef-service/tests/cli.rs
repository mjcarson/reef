//! The CLI against a served service: each command run in-process, as the terminal runs it
//!
//! Each case serves the in-memory twin on a loopback port of its own, names that port in a
//! config of the case's own, and runs `reef`'s commands from the working directory it picks. The
//! JSON API's suite already runs every route against both backends, so these run on the twin.

// with `embedded`, the shared fixtures' pool open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef_model::spec::parse_node_file;
use reef_model::{Payload, PlanName};
use reef_render::ROOT;
use reef_render::book::{Plan, read_nodes};
use reef_render::paths;
use reef_service::MemoryBackend;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use common::git;
use common::http::{Ax, TESTER, fake_client, stamped};

/// A served twin, a config naming its address, and a credentials file of the case's own
struct Cx {
    /// The served service, and the case's own directory
    ax: Ax<MemoryBackend>,
    /// The config file naming the server's address
    conf: PathBuf,
    /// The credentials file every command is named, so no token of the developer's is sent
    credentials: PathBuf,
}

impl Cx {
    /// Serves a fresh twin and writes a config naming its address
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its directory
    async fn new(case: &str) -> anyhow::Result<Cx> {
        // a twin with `reef` registered and `reef/central` listed, served on a port of its own
        let ax = common::Fx::memory(&format!("cli-{case}"))
            .await?
            .api()
            .await?;
        Cx::over(ax)
    }

    /// Serves a fresh twin that asks every caller to sign in, and writes a config naming its
    /// address
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its directory
    async fn signing_in(case: &str) -> anyhow::Result<Cx> {
        // as `new`, with sign-in required
        let ax = common::Fx::memory(&format!("cli-{case}"))
            .await?
            .api_signing_in()
            .await?;
        Cx::over(ax)
    }

    /// Serves a fresh twin that asks every caller to sign in over TLS beside its loopback port,
    /// and writes a config naming its TLS root and the CA it is trusted by, as another host's
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its directory
    async fn over_tls(case: &str) -> anyhow::Result<Cx> {
        // as `signing_in`, over TLS too
        let ax = common::Fx::memory(&format!("cli-{case}"))
            .await?
            .api_over_tls()
            .await?;
        let tls = ax.tls.as_ref().expect("the fixture serves over TLS");
        // the root another host calls, by the name its certificate names, and the CA's root
        let conf = ax.dir.join("reef.toml");
        std::fs::write(
            &conf,
            format!(
                "[api]\nurl = \"https://localhost:{}/\"\nca = \"{}\"\n",
                tls.address.port(),
                tls.minted.ca.display()
            ),
        )?;
        let credentials = ax.dir.join("home/reef/credentials.toml");
        Ok(Cx {
            ax,
            conf,
            credentials,
        })
    }

    /// Serves a fresh twin handing its users a fake client, stamped, and writes a config naming
    /// its address
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its directory
    /// * `build` - The build the fake client says it is
    async fn serving(case: &str, build: &reef_api::Build) -> anyhow::Result<Cx> {
        // the client a release would hold, read as the service reads one
        let fx = common::Fx::memory(&format!("cli-{case}")).await?;
        let client = fake_client(&fx.dir.join("release/bin/reef"), build)?;
        let served = reef_service::Served::read(&client).await?;
        Cx::over(fx.api_serving(served).await?)
    }

    /// Writes a config naming a served twin's address, beside a credentials file of the case's
    /// own
    ///
    /// # Arguments
    ///
    /// * `ax` - The served twin
    fn over(ax: Ax<MemoryBackend>) -> anyhow::Result<Cx> {
        // the config the CLI reads its address and reef's own CA from
        let conf = ax.dir.join("reef.toml");
        std::fs::write(&conf, ax.cli_config())?;
        let credentials = ax.dir.join("home/reef/credentials.toml");
        Ok(Cx {
            ax,
            conf,
            credentials,
        })
    }

    /// A client of the case's service, as the CLI makes one from the case's config and
    /// credentials
    fn client(&self) -> anyhow::Result<reef::Client> {
        // the config the CLI reads, and the login the case's credentials file keeps
        let mut conf = reef::Conf::load(Some(&self.conf))?;
        conf.credentials = Some(self.credentials.clone());
        reef::Client::new(&conf)
    }

    /// Runs one `reef` command from a directory, returning what it printed
    ///
    /// # Arguments
    ///
    /// * `cwd` - The working directory to run it from
    /// * `args` - The command line after `reef --conf <config> --credentials <file>`
    async fn reef(&self, cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
        // the command line as typed, with the case's config and credentials named
        let conf = self.conf.to_str().expect("the config is a utf-8 path");
        let credentials = self
            .credentials
            .to_str()
            .expect("the credentials are a utf-8 path");
        let line = ["reef", "--conf", conf, "--credentials", credentials]
            .into_iter()
            .chain(args.iter().copied());
        let cli = reef::Cli::from_args(line)?;
        // run it, keeping what it printed
        let mut out = Vec::new();
        reef::run(cli, cwd, &mut out).await?;
        Ok(String::from_utf8(out)?)
    }

    /// Runs one `reef` command that is to fail, returning what it printed before it did and its
    /// error
    ///
    /// # Arguments
    ///
    /// * `cwd` - The working directory to run it from
    /// * `args` - The command line after `reef --conf <config> --credentials <file>`
    async fn failing(&self, cwd: &Path, args: &[&str]) -> anyhow::Result<(String, String)> {
        // the command line as typed, with the case's config and credentials named
        let conf = self.conf.to_str().expect("the config is a utf-8 path");
        let credentials = self
            .credentials
            .to_str()
            .expect("the credentials are a utf-8 path");
        let line = ["reef", "--conf", conf, "--credentials", credentials]
            .into_iter()
            .chain(args.iter().copied());
        let cli = reef::Cli::from_args(line)?;
        // run it, keeping what it printed and how it failed
        let mut out = Vec::new();
        let error = reef::run(cli, cwd, &mut out)
            .await
            .expect_err(&format!("`reef {}` succeeded", args.join(" ")));
        Ok((String::from_utf8(out)?, error.to_string()))
    }

    /// Makes a fresh repo in the case's directory
    ///
    /// # Arguments
    ///
    /// * `name` - The repo's directory name
    fn repo(&self, name: &str) -> anyhow::Result<PathBuf> {
        git::repo(&self.ax.dir.join(name))
    }

    /// Writes YAML to a file in the case's directory, returning its path for a command line
    ///
    /// # Arguments
    ///
    /// * `name` - The file's name
    /// * `text` - The YAML
    fn yaml(&self, name: &str, text: &str) -> anyhow::Result<String> {
        let path = self.ax.dir.join(name);
        std::fs::write(&path, text)?;
        Ok(path.to_str().expect("the case's dir is utf-8").to_string())
    }

    /// Puts YAML from a file, from the `reef` project's repo, returning what it printed
    ///
    /// # Arguments
    ///
    /// * `name` - The file to write the YAML to
    /// * `text` - The YAML
    async fn put(&self, name: &str, text: &str) -> anyhow::Result<String> {
        let file = self.yaml(name, text)?;
        self.reef(&self.ax.repo, &["node", "put", &file]).await
    }
}

/// A piece of work's YAML that says why and what, with the needs given
///
/// # Arguments
///
/// * `id` - The node's id
/// * `needs` - The ids it needs
fn work(id: &str, needs: &[&str]) -> String {
    format!(
        "id: {id}\ntitle: {id} Some work\nneeds: [{}]\ndescription:\n  why: Without {id}, \
         something is missing.\n  what: With {id}, it is there.\n",
        needs.join(", ")
    )
}

/// A group's YAML, with two members inline that say why and what
const GROUP: &str = "\
id: G
title: G A group
nodes:
  - id: m1
    title: m1 Some work
    description:
      why: Without m1, something is missing.
      what: With m1, it is there.
  - id: m2
    title: m2 Some work
    description:
      why: Without m2, something is missing.
      what: With m2, it is there.
";

/// The first word of each line a command printed: the names it listed
///
/// # Arguments
///
/// * `out` - What the command printed
fn names(out: &str) -> Vec<&str> {
    out.lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect()
}

/// `reef node log` prints a node's newest 200 edits and ends naming the cursor onto the rest,
/// which `--older` prints, and `--all` prints every edit
#[tokio::test]
async fn logs_page_back() -> anyhow::Result<()> {
    let cx = Cx::new("logs-page-back").await?;
    let repo = cx.ax.repo.clone();
    // C1 added, then its body edited 204 times, through the JSON API
    let nodes = reef_api::paths::nodes(&cx.ax.plan);
    let total = reef_api::HISTORY_MAX as u64 + 5;
    let mut c1 = common::work("C1", 0);
    for read in 0..total {
        c1.body = format!("# C1\n\nEdit {read}.\n");
        let read = (read > 0).then_some(read);
        let puts = reef_api::Puts::new(vec![reef_api::NodePut::new(c1.clone(), read)]);
        let (status, _): (_, reef_api::Checked) = cx.ax.client.post(&nodes, &puts).await?;
        anyhow::ensure!(status.is_success(), "put {read:?}: {status}");
    }
    // the newest 200, newest first, then the cursor onto the other five
    let log = cx.reef(&repo, &["node", "log", "C1"]).await?;
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 201, "{log}");
    assert!(lines[0].starts_with(&format!("v{total} ")), "{log}");
    assert!(lines[199].starts_with("v6 "), "{log}");
    let cursor = lines[200]
        .strip_prefix("older edits are kept: pass --older ")
        .expect("the page names its cursor");
    // `--older` prints those five, C1's addition last, and names no cursor
    let older = cx
        .reef(&repo, &["node", "log", "C1", "--older", cursor])
        .await?;
    let lines: Vec<&str> = older.lines().collect();
    assert_eq!(lines.len(), 5, "{older}");
    assert!(lines[0].starts_with("v5 "), "{older}");
    assert!(
        lines[4].starts_with("v1 ") && lines[4].ends_with(": added"),
        "{older}"
    );
    // `--all` prints every edit, newest first, and no cursor
    let all = cx.reef(&repo, &["node", "log", "C1", "--all"]).await?;
    let lines: Vec<&str> = all.lines().collect();
    assert_eq!(lines.len(), total as usize, "{all}");
    assert!(lines[0].starts_with(&format!("v{total} ")) && lines[204].starts_with("v1 "));
    assert!(!all.contains("--older"), "{all}");
    // and a cursor no page named is refused, saying where one comes from
    let (_, error) = cx
        .failing(&repo, &["node", "log", "C1", "--older", "nope"])
        .await?;
    assert!(error.contains("no history's cursor"), "{error}");
    Ok(())
}

/// `reef node log`, `page log` and `queue log` print who made each edit and what it changed, and
/// `reef comment ls` and `queue ls` name the user signed in
#[tokio::test]
async fn histories_name_their_authors() -> anyhow::Result<()> {
    let cx = Cx::signing_in("histories").await?;
    let repo = cx.ax.repo.clone();
    let password = cx.ax.dir.join("password");
    std::fs::write(&password, "correct horse battery\n")?;
    let password = password.to_str().expect("the case's dir is utf-8");
    // ada, the first user, logged in
    cx.reef(&repo, &["user", "add", "ada", "--password-file", password])
        .await?;
    cx.reef(
        &repo,
        &["login", "--user", "ada", "--password-file", password],
    )
    .await?;
    // a node never edited says so
    let log = cx.reef(&repo, &["node", "log", "C1"]).await?;
    assert_eq!(log, "no edit of `reef/central/C1` is kept\n");
    // ada puts C1 and sets it in progress
    cx.put("c1.yaml", &work("C1", &[])).await?;
    cx.reef(&repo, &["status", "C1", "in-progress"]).await?;
    let log = cx.reef(&repo, &["node", "log", "C1"]).await?;
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2, "{log}");
    assert!(lines[0].starts_with("v2 "), "{log}");
    assert!(
        lines[0].ends_with("  ada: status unset → in-progress"),
        "{log}"
    );
    assert!(
        lines[1].starts_with("v1 ") && lines[1].ends_with("  ada: added"),
        "{log}"
    );
    // a page, and the queue, keep theirs
    let page = cx.yaml("a.md", "# A\n")?;
    cx.reef(&repo, &["page", "put", "design/a.md", &page])
        .await?;
    let log = cx.reef(&repo, &["page", "log", "design/a.md"]).await?;
    assert!(log.trim_end().ends_with("  ada: added"), "{log}");
    cx.reef(&repo, &["queue", "add", "C1"]).await?;
    let log = cx.reef(&repo, &["queue", "log"]).await?;
    assert!(
        log.trim_end()
            .ends_with("  ada: queued reef/central/C1 at the bottom"),
        "{log}"
    );
    let listed = cx.reef(&repo, &["queue", "ls"]).await?;
    assert!(listed.contains("queued by ada"), "{listed}");
    // a thread ada starts, listed with her name, and her reply too
    let node = cx.ax.plan.node("C1")?;
    let ada = reef_api::Author {
        user: Some("ada".to_string()),
        since: None,
        by: reef_api::Writer::User,
    };
    let new = reef_api::NewThread {
        target: reef_api::Target::Node,
        body: "Why?".to_string(),
        by: reef_api::Writer::User,
        review: false,
    };
    let reef_api::Commented::Added { thread, .. } =
        cx.ax.service.start_thread(&node, new, &ada).await?
    else {
        anyhow::bail!("the thread wasn't kept");
    };
    let number = thread.number.to_string();
    cx.reef(&repo, &["comment", "reply", &number, "Because."])
        .await?;
    let listed = cx.reef(&repo, &["comment", "ls"]).await?;
    assert!(
        listed.contains(&format!("    #{number} by ada:")),
        "{listed}"
    );
    assert!(listed.contains("by ada:\n        Because."), "{listed}");
    // and a history with nothing kept says so
    let log = cx.reef(&repo, &["page", "log", "design/none.md"]).await?;
    assert_eq!(log, "no edit of page `design/none.md` is kept\n");
    Ok(())
}

/// A token from `reef login` lets the CLI write, until `reef logout` revokes it
#[tokio::test]
async fn login_lets_the_cli_write_until_logout() -> anyhow::Result<()> {
    let cx = Cx::signing_in("login").await?;
    let repo = cx.ax.repo.clone();
    let password = cx.ax.dir.join("password");
    std::fs::write(&password, "correct horse battery\n")?;
    let password = password.to_str().expect("the case's dir is utf-8");
    // the first user, from this host, is an admin
    let added = cx
        .reef(&repo, &["user", "add", "ada", "--password-file", password])
        .await?;
    assert!(added.contains("added `ada`, an admin"), "{added}");
    // nobody logged in can't write, and is told how to
    let node = cx.yaml("c1.yaml", &work("C1", &[]))?;
    let (_, refused) = cx.failing(&repo, &["node", "put", &node]).await?;
    assert!(refused.contains("asks you to sign in"), "{refused}");
    assert!(refused.contains(&cx.ax.address.to_string()), "{refused}");
    assert!(refused.contains("run `reef login`"), "{refused}");
    // a wrong password logs nobody in, and keeps nothing
    let wrong = cx.ax.dir.join("wrong");
    std::fs::write(&wrong, "not the password\n")?;
    let wrong = wrong.to_str().expect("the case's dir is utf-8");
    let (_, failed) = cx
        .failing(&repo, &["login", "--user", "ada", "--password-file", wrong])
        .await?;
    assert!(failed.contains("don't match"), "{failed}");
    assert!(!cx.credentials.exists(), "a failed login kept a file");
    // logged in, the token is kept privately and the password nowhere
    let login = cx
        .reef(
            &repo,
            &["login", "--user", "ada", "--password-file", password],
        )
        .await?;
    assert!(login.contains("logged in to"), "{login}");
    assert!(login.contains("as `ada`, an admin"), "{login}");
    let kept = std::fs::read_to_string(&cx.credentials)?;
    assert!(kept.contains("token = \"reef_"), "{kept}");
    assert!(!kept.contains("correct horse"), "{kept}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&cx.credentials)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    // so the CLI writes, and says whom it is taken for
    let put = cx.reef(&repo, &["node", "put", &node]).await?;
    assert!(put.contains("C1"), "{put}");
    let me = cx.reef(&repo, &["whoami"]).await?;
    assert!(me.starts_with("`ada`, an admin"), "{me}");
    // an admin adds another user, and lists both
    let bob = cx
        .reef(&repo, &["user", "add", "bob", "--password-file", password])
        .await?;
    assert!(bob.contains("added `bob` to"), "{bob}");
    let users = cx.reef(&repo, &["user", "ls"]).await?;
    assert_eq!(names(&users), ["ada", "bob"]);
    // logged out, the token is revoked and forgotten, so writing is refused again
    let out = cx.reef(&repo, &["logout"]).await?;
    assert!(out.contains("logged out of"), "{out}");
    assert!(!std::fs::read_to_string(&cx.credentials)?.contains("reef_"));
    let other = cx.yaml("c2.yaml", &work("C2", &[]))?;
    let (_, refused) = cx.failing(&repo, &["node", "put", &other]).await?;
    assert!(refused.contains("run `reef login`"), "{refused}");
    let again = cx.reef(&repo, &["logout"]).await?;
    assert!(again.contains("not logged in"), "{again}");
    Ok(())
}

/// The CLI changes a password, lists and revokes tokens, and removes a user, and says what it
/// refuses
#[tokio::test]
async fn the_cli_manages_passwords_users_and_tokens() -> anyhow::Result<()> {
    let cx = Cx::signing_in("manage").await?;
    let repo = cx.ax.repo.clone();
    // a password file for each password the case uses
    let file = |name: &str, password: &str| -> anyhow::Result<String> {
        let path = cx.ax.dir.join(name);
        std::fs::write(&path, format!("{password}\n"))?;
        Ok(path.to_str().expect("the case's dir is utf-8").to_string())
    };
    let old = file("old", "correct horse battery")?;
    let new = file("new", "a newer horse battery")?;
    let wrong = file("wrong", "not the password")?;
    // ada, the first admin, logged in, and bob and carol beside her
    cx.reef(&repo, &["user", "add", "ada", "--password-file", &old])
        .await?;
    cx.reef(
        &repo,
        &[
            "login",
            "--user",
            "ada",
            "--password-file",
            &old,
            "--label",
            "first",
        ],
    )
    .await?;
    for name in ["bob", "carol"] {
        cx.reef(&repo, &["user", "add", name, "--password-file", &old])
            .await?;
    }
    // her one token, marked as this reef's
    let listed = cx.reef(&repo, &["token", "ls"]).await?;
    assert_eq!(listed.lines().count(), 1, "{listed}");
    assert!(
        listed.contains("cli") && listed.contains("first  (this reef)"),
        "{listed}"
    );
    // her own password needs her current one, right, and keeps her token signing in
    let (_, refused) = cx
        .failing(
            &repo,
            &[
                "user",
                "passwd",
                "--current-file",
                &wrong,
                "--password-file",
                &new,
            ],
        )
        .await?;
    assert!(refused.contains("doesn't match"), "{refused}");
    let changed = cx
        .reef(
            &repo,
            &[
                "user",
                "passwd",
                "--current-file",
                &old,
                "--password-file",
                &new,
            ],
        )
        .await?;
    assert!(changed.contains("changed your password"), "{changed}");
    assert!(cx.reef(&repo, &["whoami"]).await?.starts_with("`ada`"));
    // a second login, by the new one, leaves two tokens, and `--others` revokes the first
    let (_, refused) = cx
        .failing(&repo, &["login", "--user", "ada", "--password-file", &old])
        .await?;
    assert!(refused.contains("don't match"), "{refused}");
    cx.reef(
        &repo,
        &[
            "login",
            "--user",
            "ada",
            "--password-file",
            &new,
            "--label",
            "second",
        ],
    )
    .await?;
    let listed = cx.reef(&repo, &["token", "ls"]).await?;
    assert_eq!(listed.lines().count(), 2, "{listed}");
    let first = names(&listed)[0].to_string();
    let revoked = cx.reef(&repo, &["token", "revoke", "--others"]).await?;
    assert_eq!(revoked.trim(), format!("revoked {first}"));
    let listed = cx.reef(&repo, &["token", "ls"]).await?;
    assert_eq!(listed.lines().count(), 1, "{listed}");
    assert!(listed.contains("second  (this reef)"), "{listed}");
    // a token of nobody's is said so
    let (_, refused) = cx
        .failing(&repo, &["token", "revoke", "0123456789abcdef"])
        .await?;
    assert!(refused.contains("no token of yours"), "{refused}");
    // bob's, set by an admin, revokes the token he had
    cx.reef(&repo, &["logout"]).await?;
    cx.reef(&repo, &["login", "--user", "bob", "--password-file", &old])
        .await?;
    cx.reef(&repo, &["logout"]).await?;
    cx.reef(
        &repo,
        &["login", "--user", "carol", "--password-file", &old],
    )
    .await?;
    // carol, no admin, sets nobody's password, lists nobody's tokens and removes nobody
    let (_, refused) = cx
        .failing(&repo, &["user", "passwd", "bob", "--password-file", &new])
        .await?;
    assert!(refused.contains("only an admin"), "{refused}");
    let (_, refused) = cx.failing(&repo, &["token", "ls", "--user", "bob"]).await?;
    assert!(refused.contains("only an admin"), "{refused}");
    let (_, refused) = cx.failing(&repo, &["user", "rm", "bob"]).await?;
    assert!(refused.contains("only an admin"), "{refused}");
    // back as ada, she sets bob's, which revokes his token, and lists his tokens
    cx.reef(&repo, &["login", "--user", "ada", "--password-file", &new])
        .await?;
    let bobs = cx.reef(&repo, &["token", "ls", "--user", "bob"]).await?;
    assert!(bobs.contains("no tokens sign bob in"), "{bobs}");
    let set = cx
        .reef(&repo, &["user", "passwd", "bob", "--password-file", &new])
        .await?;
    assert!(set.contains("set the password of `bob`"), "{set}");
    // she removes carol, revoking her token, and carol is gone
    let removed = cx.reef(&repo, &["user", "rm", "carol"]).await?;
    assert!(
        removed.contains("removed `carol`") && removed.contains("revoking 1 token"),
        "{removed}"
    );
    let users = cx.reef(&repo, &["user", "ls"]).await?;
    assert_eq!(names(&users), ["ada", "bob"]);
    let (_, refused) = cx.failing(&repo, &["user", "rm", "carol"]).await?;
    assert!(refused.contains("no user is named `carol`"), "{refused}");
    // and the last admin stays
    let (_, refused) = cx.failing(&repo, &["user", "rm", "ada"]).await?;
    assert!(refused.contains("last admin"), "{refused}");
    Ok(())
}

/// reef serves over TLS beside its loopback port, and the CLI given `[api] url` and `ca` signs in
/// and writes over it, refusing a certificate it has no root for
#[tokio::test]
async fn the_cli_signs_in_and_writes_over_tls() -> anyhow::Result<()> {
    let cx = Cx::over_tls("tls").await?;
    let repo = cx.ax.repo.clone();
    let port = cx.ax.tls.as_ref().expect("served over TLS").address.port();
    let root = format!("https://localhost:{port}");
    let password = cx.ax.dir.join("password");
    std::fs::write(&password, "correct horse battery\n")?;
    let password = password.to_str().expect("the case's dir is utf-8");
    // the first user, over TLS from this host, is an admin
    let added = cx
        .reef(&repo, &["user", "add", "ada", "--password-file", password])
        .await?;
    assert!(added.contains("added `ada`, an admin"), "{added}");
    // nobody logged in is told where, by the root the config names
    let node = cx.yaml("c1.yaml", &work("C1", &[]))?;
    let (_, refused) = cx.failing(&repo, &["node", "put", &node]).await?;
    assert!(refused.contains(&root), "{refused}");
    assert!(refused.contains("run `reef login`"), "{refused}");
    // a login keeps its token by that root, with no `/` at its end
    let login = cx
        .reef(
            &repo,
            &["login", "--user", "ada", "--password-file", password],
        )
        .await?;
    assert!(login.contains(&root), "{login}");
    let kept = std::fs::read_to_string(&cx.credentials)?;
    assert!(kept.contains(&format!("\"{root}\"")), "{kept}");
    // so the CLI finds the project, writes over TLS, and reads it back
    let put = cx.reef(&repo, &["node", "put", &node]).await?;
    assert!(put.contains("C1"), "{put}");
    let got = cx.reef(&repo, &["node", "get", "C1"]).await?;
    assert!(got.contains("C1 Some work"), "{got}");
    // and the loopback port still answers this host, over plain HTTP
    let me = cx
        .ax
        .client
        .get::<reef_api::Me>(reef_api::paths::ME)
        .await?;
    assert_eq!(me.0, reqwest::StatusCode::OK);
    // a CLI with no root for the certificate refuses it, saying so rather than that nothing runs
    let untrusting = cx.ax.dir.join("untrusting.toml");
    std::fs::write(&untrusting, format!("[api]\nurl = \"{root}\"\n"))?;
    let credentials = cx
        .credentials
        .to_str()
        .expect("the credentials are a utf-8 path");
    let untrusting = untrusting.to_str().expect("the case's dir is utf-8");
    let line = [
        "reef",
        "--conf",
        untrusting,
        "--credentials",
        credentials,
        "whoami",
    ];
    let mut out = Vec::new();
    let error = reef::run(reef::Cli::from_args(line)?, &repo, &mut out)
        .await
        .expect_err("a certificate no root signs was trusted");
    let message = format!("{error:#}");
    assert!(message.contains("certificate"), "{message}");
    assert!(!message.contains("isn't running"), "{message}");
    Ok(())
}

/// With no login kept, the CLI is refused on loopback too, told to run `reef login`, and says
/// the service asks every caller to sign in
#[tokio::test]
async fn the_cli_with_no_login_is_told_to_run_reef_login() -> anyhow::Result<()> {
    let cx = Cx::signing_in("no-login").await?;
    let repo = cx.ax.repo.clone();
    // a put with no token kept is refused, saying how to sign in
    let node = cx.yaml("c1.yaml", &work("C1", &[]))?;
    let (_, refused) = cx.failing(&repo, &["node", "put", &node]).await?;
    assert!(refused.contains("reef login"), "{refused}");
    // and the service says it asks every caller
    let me = cx.reef(&repo, &["whoami"]).await?;
    assert!(me.contains("asks every caller to sign in"), "{me}");
    Ok(())
}

/// Every write through the CLI names the user it signs in, on loopback too: a put and a status, a
/// start, a capture, a queued entry, and a reply to a thread and its resolve
#[tokio::test]
async fn every_write_through_the_cli_names_its_user() -> anyhow::Result<()> {
    let cx = Cx::new("every-write").await?;
    let repo = cli_project(&cx, &["A"]).await?;
    let node = PlanName::new("cli", "docs")?.node("A")?;
    let client = cx.client()?;
    // A put, then set in progress, each edit in its history under the user's name
    cx.reef(&repo, &["status", "A", "in-progress"]).await?;
    let log = cx.reef(&repo, &["node", "log", "A"]).await?;
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2, "{log}");
    assert!(
        lines[0].ends_with("  tester: status unset → in-progress"),
        "{log}"
    );
    assert!(lines[1].ends_with("  tester: added"), "{log}");
    // A started on a branch, its record naming who started it
    git::run(&repo, &["checkout", "-q", "-b", "work"])?;
    cx.reef(&repo, &["start", "A"]).await?;
    let records = client.work("cli").await?;
    let record = records
        .iter()
        .find(|record| record.node == node)
        .expect("A's start is recorded");
    assert_eq!(record.started_by.describe().as_deref(), Some(TESTER));
    // its work captured, the revision naming who sent it
    write_files_in(&repo, &[("a.txt", b"A's work\n")])?;
    commit_all(&repo, "A")?;
    cx.reef(&repo, &["node", "capture"]).await?;
    let revisions = client.revisions(&node).await?;
    assert_eq!(revisions.len(), 1, "{revisions:?}");
    assert_eq!(revisions[0].author.describe().as_deref(), Some(TESTER));
    // queued, naming who queued it
    cx.reef(&repo, &["queue", "add", "A"]).await?;
    let listed = cx.reef(&repo, &["queue", "ls"]).await?;
    assert!(listed.contains("queued by tester"), "{listed}");
    // and a thread on it, replied to and resolved from the CLI, each act naming the user
    let number = comment_on(&cx, "A", "Is it done?").await?;
    cx.reef(&repo, &["comment", "reply", &number.to_string(), "It is."])
        .await?;
    cx.reef(&repo, &["comment", "resolve", &number.to_string()])
        .await?;
    let thread = thread_of(&cx, number).await?;
    let authors: Vec<Option<String>> = thread
        .comments
        .iter()
        .map(|comment| comment.author.describe())
        .collect();
    assert_eq!(
        authors,
        vec![Some(TESTER.to_string()), Some(TESTER.to_string())]
    );
    let resolved = thread.resolved_by().expect("the resolve linked a revision");
    assert_eq!(resolved.author.describe().as_deref(), Some(TESTER));
    Ok(())
}

/// `reef user log` and `reef loop log` print who did what, and `user ls`, `loop ls`, `project
/// ls` and `plan ls` name who added, started, registered or listed each
#[tokio::test]
async fn acts_name_their_authors() -> anyhow::Result<()> {
    let cx = Cx::signing_in("acts").await?;
    let password = cx.ax.dir.join("password");
    std::fs::write(&password, "correct horse battery\n")?;
    let password = password.to_str().expect("the case's dir is utf-8");
    let repo = cx.repo("cli")?;
    // ada, the first user, added from this host by nobody signed in, then signed in
    cx.reef(&repo, &["user", "add", "ada", "--password-file", password])
        .await?;
    cx.reef(
        &repo,
        &["login", "--user", "ada", "--password-file", password],
    )
    .await?;
    // ada adds bob, registers a project and lists a plan in it
    cx.reef(&repo, &["user", "add", "bob", "--password-file", password])
        .await?;
    cx.reef(&repo, &["project", "add", "cli"]).await?;
    cx.reef(&repo, &["plan", "new", "docs"]).await?;
    // `user ls` names who added bob, and `user log` who did each act, newest first
    let users = cx.reef(&repo, &["user", "ls"]).await?;
    assert_eq!(users, "ada  admin\nbob  added by ada\n");
    let log = cx.reef(&repo, &["user", "log"]).await?;
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2, "{log}");
    assert!(lines[0].ends_with(" +0000  ada: added bob"), "{log}");
    assert!(lines[1].ends_with(" +0000: added ada, an admin"), "{log}");
    // `project ls` and `plan ls` name who registered and listed them, the fixture's nobody
    let projects = cx.reef(&repo, &["project", "ls"]).await?;
    let lines: Vec<&str> = projects.lines().collect();
    assert!(
        lines[0].starts_with("cli ") && lines[0].ends_with(", registered by ada"),
        "{projects}"
    );
    assert!(
        lines[1].starts_with("reef ") && !lines[1].contains("registered by"),
        "{projects}"
    );
    let plans = cx.reef(&repo, &["plan", "ls"]).await?;
    assert!(plans.ends_with(", listed by ada\n"), "{plans}");
    // ada starts a loop in a pane and pauses it; `loop ls` and `loop log` say who
    cx.reef(&repo, &["loop", "start", "--pane", "claude:4242"])
        .await?;
    cx.reef(&repo, &["loop", "pause", "--pane", "claude:4242"])
        .await?;
    let listed = cx.reef(&repo, &["loop", "ls"]).await?;
    assert!(listed.trim_end().ends_with(", started by ada"), "{listed}");
    let log = cx.reef(&repo, &["loop", "log"]).await?;
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2, "{log}");
    assert!(
        lines[0].ends_with("  ada: paused the loop in Claude Code process 4242"),
        "{log}"
    );
    assert!(
        lines[1].ends_with("  ada: started a loop in Claude Code process 4242, feeding every plan"),
        "{log}"
    );
    Ok(())
}

/// A project registered from its repo and a plan listed in it show in `reef plan ls`
#[tokio::test]
async fn project_add_and_plan_new_show_in_plan_ls() -> anyhow::Result<()> {
    let cx = Cx::new("plan-ls").await?;
    // a project registered from a directory deep in its repo is registered for the repo's root
    let repo = cx.repo("cli")?;
    let deep = repo.join("src");
    std::fs::create_dir_all(&deep)?;
    let added = cx.reef(&deep, &["project", "add", "cli"]).await?;
    assert_eq!(added, format!("registered `cli` for {}\n", repo.display()));
    // a plan listed in the working directory's project, once
    let listed = cx.reef(&deep, &["plan", "new", "docs"]).await?;
    assert_eq!(listed, "listed `cli/docs`\n");
    let again = cx.reef(&repo, &["plan", "new", "docs"]).await?;
    assert_eq!(again, "`cli/docs` is already listed\n");
    // shows in `plan ls`, with the revision readers load
    let plans = cx.reef(&repo, &["plan", "ls"]).await?;
    assert_eq!(names(&plans), ["cli/docs"]);
    assert!(plans.contains("revision "), "{plans}");
    // `--long` sums it up beneath: idle, holding nothing, its listing its last update
    let long = cx.reef(&repo, &["plan", "ls", "--long"]).await?;
    let lines: Vec<&str> = long.lines().collect();
    let [listed, summed] = lines[..] else {
        anyhow::bail!("`plan ls --long` printed {long:?}");
    };
    assert!(listed.starts_with("cli/docs  revision "), "{long}");
    let summed = summed.trim_start();
    assert!(
        summed
            .starts_with("idle, no sessions, 0 of 0 nodes done, 0 in the last 24 hours, updated "),
        "{long}"
    );
    assert!(summed.ends_with(" by tester"), "{long}");
    // and `project ls` lists it beside the project the fixture registered, by name
    let projects = cx.reef(&repo, &["project", "ls"]).await?;
    assert_eq!(names(&projects), ["cli", "reef"]);
    // `--project` reaches another project's plans from here
    let other = cx.reef(&repo, &["plan", "ls", "--project", "reef"]).await?;
    assert_eq!(names(&other), ["reef/central"]);
    Ok(())
}

/// A linked worktree picks its main repo's project
#[tokio::test]
async fn linked_worktree_picks_its_main_repos_project() -> anyhow::Result<()> {
    let cx = Cx::new("worktree").await?;
    // a project for a repo, and a worktree of it beside it
    let repo = cx.repo("cli")?;
    cx.reef(&repo, &["project", "add", "cli"]).await?;
    let linked = git::worktree(&repo, &cx.ax.dir.join("cli-linked"))?;
    // from the worktree, the main repo's project is the one shown
    let shown = cx.reef(&linked, &["project", "show"]).await?;
    assert_eq!(names(&shown), ["cli"]);
    assert!(shown.contains(&repo.display().to_string()), "{shown}");
    // and a plan listed from it lands in that project
    let listed = cx.reef(&linked, &["plan", "new", "docs"]).await?;
    assert_eq!(listed, "listed `cli/docs`\n");
    Ok(())
}

/// A directory no project holds, and a project that isn't registered, say how to fix it
#[tokio::test]
async fn unregistered_dir_says_how_to_register() -> anyhow::Result<()> {
    let cx = Cx::new("unregistered").await?;
    // a repo no project is registered for
    let lonely = cx.repo("lonely")?;
    let error = cx.reef(&lonely, &["plan", "ls"]).await.unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("no project is registered for"),
        "{message}"
    );
    assert!(message.contains("reef project add"), "{message}");
    assert!(message.contains("--project"), "{message}");
    // a project named that isn't registered
    let error = cx
        .reef(&lonely, &["project", "show", "--project", "nope"])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("`nope`"), "{error}");
    Ok(())
}

/// A taken name, and a repo another project holds, are refused, saying which
#[tokio::test]
async fn taken_name_and_repo_are_refused() -> anyhow::Result<()> {
    let cx = Cx::new("taken").await?;
    // `cli` registered for one repo
    let repo = cx.repo("cli")?;
    cx.reef(&repo, &["project", "add", "cli"]).await?;
    // the name again, for another repo
    let other = cx.repo("other")?;
    let error = cx
        .reef(&other, &["project", "add", "cli"])
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("a project named `cli` is already registered"),
        "{error}"
    );
    // the repo again, under another name, named by `--repo` from elsewhere
    let repo_arg = repo.to_str().expect("the repo is a utf-8 path");
    let error = cx
        .reef(&other, &["project", "add", "again", "--repo", repo_arg])
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("already registered as `cli`"),
        "{error}"
    );
    Ok(())
}

/// A put naming a need that isn't a node gets check's message, and writes nothing
#[tokio::test]
async fn put_naming_an_unknown_need_gets_checks_message() -> anyhow::Result<()> {
    let cx = Cx::new("unknown-need").await?;
    // a node that needs one the plan doesn't have
    let error = cx
        .put("c99.yml", &work("C99", &["nope"]))
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.starts_with("the put was refused"), "{message}");
    assert!(
        message.contains("reef/central/C99: needs `nope`, which is not a node"),
        "{message}"
    );
    // and nothing was written
    let error = cx
        .reef(&cx.ax.repo, &["node", "get", "C99"])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not in its plan"), "{error}");
    Ok(())
}

/// A node got, edited and put back is written, and a put over a version since moved on, or a
/// new node over one that exists, is stale
#[tokio::test]
async fn got_node_put_back_and_stale_put_refused() -> anyhow::Result<()> {
    let cx = Cx::new("round-trip").await?;
    let repo = cx.ax.repo.clone();
    // a new node, written by hand with no version line
    let new = work("C1", &[]);
    assert_eq!(cx.put("c1.yml", &new).await?, "put `C1` at version 1\n");
    // got after the version it was read at, and the YAML it was put as
    let got = cx.reef(&repo, &["node", "get", "C1"]).await?;
    assert!(got.starts_with("# version: 1\nid: C1\n"), "{got}");
    assert!(got.contains("title: C1 Some work\n"), "{got}");
    // edited and put back, it moves on a version
    let edited = got.replace("title: C1 Some work", "title: C1 Edited work");
    assert_eq!(cx.put("c1.yml", &edited).await?, "put `C1` at version 2\n");
    let again = cx.reef(&repo, &["node", "get", "C1"]).await?;
    assert!(again.starts_with("# version: 2\n"), "{again}");
    assert!(again.contains("title: C1 Edited work\n"), "{again}");
    // what was got before that is stale now, and says what reads it again
    let error = cx.put("c1.yml", &got).await.unwrap_err().to_string();
    assert!(
        error.contains("`C1`: it changed since it was read"),
        "{error}"
    );
    assert!(error.contains("`reef node get C1`"), "{error}");
    // and a new node over one that exists says to keep the version line
    let error = cx.put("c1.yml", &new).await.unwrap_err().to_string();
    assert!(error.contains("`C1`: it already exists"), "{error}");
    assert!(error.contains("`# version:`"), "{error}");
    // a group goes in with its members inline, each a node of its own in it
    let put = cx.put("g.yml", GROUP).await?;
    assert_eq!(
        put,
        "put `G` at version 1\nput `m1` at version 1\nput `m2` at version 1\n"
    );
    let member = cx.reef(&repo, &["node", "get", "m1"]).await?;
    assert!(member.contains("group: G\n"), "{member}");
    let group = cx.reef(&repo, &["node", "get", "G"]).await?;
    assert!(!group.contains("nodes:"), "{group}");
    Ok(())
}

/// A node's body put as a node file through the CLI is stored, drawn and exported, a put of YAML
/// alone keeps it, and one put at a stale version is refused, writing nothing
#[tokio::test]
async fn body_put_through_the_cli_is_stored_drawn_and_exported() -> anyhow::Result<()> {
    let cx = Cx::new("body-put").await?;
    let repo = cx.ax.repo.clone();
    // a group and its members, put as YAML, the group with no body yet
    cx.put("g.yml", GROUP).await?;
    let got = cx.reef(&repo, &["node", "get", "G", "--md"]).await?;
    assert!(got.starts_with("---\n# version: 1\nid: G\n"), "{got}");
    assert!(got.ends_with("\n---\n"), "{got}");
    // got as a node file, given a body with a fence of its own, and put back
    let body = "# G A group\n\nThe group's own words.\n\n```reef\nfocus = \"G\"\n```\n";
    let written = format!("{got}{body}");
    assert_eq!(cx.put("g.md", &written).await?, "put `G` at version 2\n");
    // stored as written, and printed back after its frontmatter
    let stored = cx
        .ax
        .service
        .get(&cx.ax.plan, "G")
        .await?
        .expect("G is stored");
    assert_eq!(stored.body, body);
    let again = cx.reef(&repo, &["node", "get", "G", "--md"]).await?;
    assert!(again.starts_with("---\n# version: 2\nid: G\n"), "{again}");
    assert!(again.ends_with(&format!("\n---\n{body}")), "{again}");
    // drawn in the sidebar, its first heading and its fence left out as a book's are
    let payload = cx.ax.service.payload(&cx.ax.plan).await?;
    let drawn = payload
        .nodes
        .iter()
        .find(|node| node.id == "G")
        .expect("G is drawn");
    assert!(
        drawn.html.contains("The group's own words."),
        "{}",
        drawn.html
    );
    assert!(!drawn.html.contains("focus"), "{}", drawn.html);
    // the version-1 file put again is stale, and leaves the body as it was
    let error = cx.put("g.md", &got).await.unwrap_err().to_string();
    assert!(
        error.contains("`G`: it changed since it was read"),
        "{error}"
    );
    let kept = cx
        .ax
        .service
        .get(&cx.ax.plan, "G")
        .await?
        .expect("G is stored");
    assert_eq!((kept.version, kept.body.as_str()), (2, body));
    // YAML alone, retitling it, keeps the body
    let yaml = cx.reef(&repo, &["node", "get", "G"]).await?;
    let retitled = yaml.replace("title: G A group", "title: G The group");
    assert_eq!(cx.put("g.yml", &retitled).await?, "put `G` at version 3\n");
    let kept = cx
        .ax
        .service
        .get(&cx.ax.plan, "G")
        .await?
        .expect("G is stored");
    assert_eq!(kept.body, body);
    // exported, the body is its file's, its members inline in its frontmatter
    cx.reef(&repo, &["export", "out"]).await?;
    let file = std::fs::read_to_string(repo.join("out/src/001-g.md"))?;
    let (spec, fm) = parse_node_file(&file)
        .map_err(|error| anyhow::anyhow!("001-g.md: {error:?}"))?
        .expect("the file is G's");
    assert_eq!(spec.id, "G");
    assert_eq!(spec.nodes.len(), 2);
    assert_eq!(fm.body, body);
    Ok(())
}

/// A node another needs can't be removed, nor a group without its members, and the nodes named
/// together go
#[tokio::test]
async fn rm_names_every_node_it_removes() -> anyhow::Result<()> {
    let cx = Cx::new("rm").await?;
    let repo = cx.ax.repo.clone();
    // a group of two, and work that needs one of them
    cx.put("g.yml", GROUP).await?;
    cx.put("w.yml", &work("W", &["m1"])).await?;
    // a node another needs is kept, with check's message
    let error = cx.reef(&repo, &["node", "rm", "m1"]).await.unwrap_err();
    let message = error.to_string();
    assert!(message.starts_with("the removal was refused"), "{message}");
    assert!(
        message.contains("reef/central/W: needs `m1`, which is not a node"),
        "{message}"
    );
    // a group alone is kept, since its members would name a group that isn't a node
    let error = cx.reef(&repo, &["node", "rm", "G"]).await.unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("reef/central/m1: group `G` is not a node"),
        "{message}"
    );
    // a node the plan doesn't have is named, and nothing is sent
    let error = cx
        .reef(&repo, &["node", "rm", "W", "nope"])
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "node `reef/central/nope` is not in its plan"
    );
    // what needs the member goes first, then the group with its members named
    assert_eq!(cx.reef(&repo, &["node", "rm", "W"]).await?, "removed `W`\n");
    let removed = cx.reef(&repo, &["node", "rm", "G", "m1", "m2"]).await?;
    assert_eq!(removed, "removed `G`\nremoved `m1`\nremoved `m2`\n");
    let error = cx.reef(&repo, &["node", "get", "G"]).await.unwrap_err();
    assert!(error.to_string().contains("not in its plan"), "{error}");
    Ok(())
}

/// A node's plan is the project's only plan, and with several, the one `--plan` names
#[tokio::test]
async fn node_commands_take_the_only_plan_or_the_one_named() -> anyhow::Result<()> {
    let cx = Cx::new("pick-plan").await?;
    let repo = cx.ax.repo.clone();
    // with one plan, it is the one put to
    cx.put("a.yml", &work("A", &[])).await?;
    // with two, a command says which there are and how to name one
    cx.reef(&repo, &["plan", "new", "other"]).await?;
    let error = cx
        .reef(&repo, &["node", "get", "A"])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("`central` (follows `"), "{error}");
    assert!(error.contains("`other` (follows `"), "{error}");
    assert!(error.contains("by its name alone"), "{error}");
    assert!(error.contains("`--plan`"), "{error}");
    // and each plan named is its own
    let got = cx
        .reef(&repo, &["node", "get", "A", "--plan", "central"])
        .await?;
    assert!(got.starts_with("# version: 1\nid: A\n"), "{got}");
    let error = cx
        .reef(&repo, &["node", "get", "A", "--plan", "other"])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not in its plan"), "{error}");
    Ok(())
}

/// Marking a group's last member done makes the group done, and what needs it ready
#[tokio::test]
async fn group_done_when_its_last_member_is() -> anyhow::Result<()> {
    let cx = Cx::new("group-done").await?;
    let repo = cx.ax.repo.clone();
    // a group of two members, and a node that needs the group
    cx.put("g.yml", GROUP).await?;
    cx.put("d.yml", &work("D", &["G"])).await?;
    // the members are ready, and the node waits on the group
    let next = cx.reef(&repo, &["next"]).await?;
    assert_eq!(
        next,
        "Ready to start (2), in plan order:\n  m1  m1 Some work\n  m2  m2 Some work\n\n\
         Waiting (1):\n  D   D Some work\n        needs G (todo)\n"
    );
    // one member done leaves the group in progress, and the node waiting on it
    let done = cx.reef(&repo, &["status", "m1", "done"]).await?;
    assert_eq!(done, "`m1` is done, at version 2\n");
    let next = cx.reef(&repo, &["next"]).await?;
    assert!(
        next.starts_with("Ready to start (1), in plan order:\n  m2"),
        "{next}"
    );
    assert!(next.contains("needs G (in-progress)"), "{next}");
    // the last one done makes the group done, and the node ready
    cx.reef(&repo, &["status", "m2", "done"]).await?;
    let next = cx.reef(&repo, &["next"]).await?;
    assert_eq!(
        next,
        "Ready to start (1), in plan order:\n  D  D Some work\n\nWaiting (0):\n  nothing\n"
    );
    Ok(())
}

/// A group's status is never set, a status already set writes nothing, and a node not in the
/// plan is named
#[tokio::test]
async fn status_is_set_on_work_alone() -> anyhow::Result<()> {
    let cx = Cx::new("status").await?;
    let repo = cx.ax.repo.clone();
    cx.put("g.yml", GROUP).await?;
    // a group is done when its members are, so its own is refused
    let error = cx
        .reef(&repo, &["status", "G", "done"])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("`G` is a group"), "{error}");
    // a member's is set once, and setting it again says so and writes nothing
    let started = cx.reef(&repo, &["status", "m1", "in-progress"]).await?;
    assert_eq!(started, "`m1` is in-progress, at version 2\n");
    let again = cx.reef(&repo, &["status", "m1", "wip"]).await?;
    assert_eq!(again, "`m1` is already in-progress\n");
    let got = cx.reef(&repo, &["node", "get", "m1"]).await?;
    assert!(got.starts_with("# version: 2\n"), "{got}");
    assert!(got.contains("status: in-progress\n"), "{got}");
    // back to to do, which the canonical YAML leaves out
    cx.reef(&repo, &["status", "m1", "todo"]).await?;
    let got = cx.reef(&repo, &["node", "get", "m1"]).await?;
    assert!(!got.contains("status:"), "{got}");
    // and a node the plan doesn't have is named in it
    let error = cx
        .reef(&repo, &["status", "nope", "done"])
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("`reef/central/nope` is not in its plan"),
        "{error}"
    );
    Ok(())
}

/// A node's criteria are listed by number, one is set met with its test, and one met by its
/// metric, or one it doesn't have, is refused
#[tokio::test]
async fn accept_lists_and_sets_criteria() -> anyhow::Result<()> {
    let cx = Cx::new("accept").await?;
    let repo = cx.ax.repo.clone();
    // a node with a unit criterion and a performance one met by its metric's figures
    let yaml = format!(
        "{}acceptance:\n  - unit: Rows round-trip\n  - kind: performance\n    text: Scans are \
         fast\n    metric: scan-ms\n    at-most: 10\nperformance:\n  metrics:\n    - name: \
         scan-ms\n      before: 20\n      after: 5\n",
        work("C1", &[])
    );
    cx.put("c1.yml", &yaml).await?;
    // listed by number, with whether each is met
    let listed = cx.reef(&repo, &["accept", "C1"]).await?;
    assert_eq!(
        listed,
        "1. todo  unit: Rows round-trip\n2. by its metric  perf: Scans are fast\n"
    );
    // the first set met, with the test that checks it
    let set = [
        "accept",
        "C1",
        "1",
        "met",
        "--test",
        "tests/rows.rs::round_trip",
    ];
    let met = cx.reef(&repo, &set).await?;
    assert_eq!(met, "criterion 1 of `C1` is met, at version 2\n");
    let got = cx.reef(&repo, &["node", "get", "C1"]).await?;
    assert!(
        got.contains("- unit: Rows round-trip\n  status: met\n  test: tests/rows.rs::round_trip\n"),
        "{got}"
    );
    let listed = cx.reef(&repo, &["accept", "C1"]).await?;
    assert!(
        listed.starts_with("1. met  unit: Rows round-trip\n   test: tests/rows.rs::round_trip\n"),
        "{listed}"
    );
    // the second is met or not by its metric, so it isn't set by hand
    let error = cx
        .reef(&repo, &["accept", "C1", "2", "met"])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("by its metric"), "{error}");
    // and a third isn't there
    let error = cx
        .reef(&repo, &["accept", "C1", "3", "met"])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("`C1` has 2 acceptance criteria"), "{error}");
    Ok(())
}

/// A plan with no problems says so, and one with a problem prints it and fails, and `next`
/// warns of it
#[tokio::test]
async fn check_fails_on_a_plan_with_a_problem() -> anyhow::Result<()> {
    let cx = Cx::new("check").await?;
    let repo = cx.ax.repo.clone();
    // a node whose doc is in the working tree, which leaves the plan whole
    std::fs::create_dir_all(repo.join("docs"))?;
    std::fs::write(repo.join("docs/a.md"), "# A\n\nThe design.\n")?;
    let linked = format!("{}docs: [docs/a.md]\n", work("C1", &[]));
    cx.put("c1.yml", &linked).await?;
    let checked = cx.reef(&repo, &["check"]).await?;
    assert_eq!(checked, "`reef/central` has no problems\n");
    // the doc's removal breaks it: check prints the problem, then fails naming the count
    std::fs::remove_file(repo.join("docs/a.md"))?;
    let (printed, error) = cx.failing(&repo, &["check"]).await?;
    assert_eq!(
        printed,
        "error: reef/central/C1: doc `docs/a.md` not found (looked for docs/a.md)\n"
    );
    assert_eq!(error, "`reef/central` has 1 problem");
    // and next still says what is ready, after a warning that the plan has a problem
    let next = cx.reef(&repo, &["next"]).await?;
    assert!(
        next.starts_with(
            "warning: `reef/central` has 1 problem; `reef check` lists them\nReady to start (1)"
        ),
        "{next}"
    );
    Ok(())
}

/// A queued entry that waits shows the needs it waits on, and next skips it for the queue's
/// ready entries until they are done
#[tokio::test]
async fn a_waiting_entry_shows_the_needs_it_waits_on() -> anyhow::Result<()> {
    let cx = Cx::new("queue-waits").await?;
    let repo = cx.ax.repo.clone();
    // A, B needing A, and C
    cx.put("a.yml", &work("A", &[])).await?;
    cx.put("b.yml", &work("B", &["A"])).await?;
    cx.put("c.yml", &work("C", &[])).await?;
    // B queued, then C at the top
    let added = cx.reef(&repo, &["queue", "add", "B"]).await?;
    assert_eq!(added, "`B` is queued 1 of 1\n");
    let added = cx.reef(&repo, &["queue", "add", "C", "--top"]).await?;
    assert_eq!(added, "`C` is queued 1 of 2\n");
    // the queue says C is ready, and B waits on A, each queued by the case's own user
    let ls = cx.reef(&repo, &["queue", "ls"]).await?;
    assert_eq!(
        ls,
        "1. C  ready    C Some work\n      queued by tester\n2. B  waiting  B Some work\n      \
         needs A (todo)\n      queued by tester\n"
    );
    // next takes C first, then A in plan order, and says it skipped B, and why
    let next = cx.reef(&repo, &["next"]).await?;
    assert_eq!(
        next,
        "Ready to start (2), queued first, then in plan order:\n  C  C Some work\n  A  A Some \
         work\n\nSkipped in the queue (1):\n  B  B Some work\n       needs A (todo)\n\n\
         Waiting (1):\n  B  B Some work\n       needs A (todo)\n"
    );
    // once A is done, B is ready, and next takes the queue in its order
    cx.reef(&repo, &["status", "A", "done"]).await?;
    let ls = cx.reef(&repo, &["queue", "ls"]).await?;
    assert_eq!(
        ls,
        "1. C  ready  C Some work\n      queued by tester\n2. B  ready  B Some work\n      queued \
         by tester\n"
    );
    let next = cx.reef(&repo, &["next"]).await?;
    assert_eq!(
        next,
        "Ready to start (2), queued first, then in plan order:\n  C  C Some work\n  B  B Some \
         work\n\nWaiting (0):\n  nothing\n"
    );
    // and an entry taken off is gone
    let off = cx.reef(&repo, &["queue", "rm", "C", "B"]).await?;
    assert_eq!(off, "took `C` off the queue\ntook `B` off the queue\n");
    let ls = cx.reef(&repo, &["queue", "ls"]).await?;
    assert_eq!(
        ls,
        "the queue of `reef` is empty: `reef queue add <ID>` queues a node\n"
    );
    Ok(())
}

/// next spans the project's plans, naming nodes in full, and `--plan` narrows it to one, leaving
/// the other plans' entries out, and each plan's queue is placed among its own entries alone
#[tokio::test]
async fn next_spans_the_projects_plans_and_plan_narrows_it() -> anyhow::Result<()> {
    let cx = Cx::new("queue-spans").await?;
    let repo = cx.ax.repo.clone();
    // a second plan, and a piece of work in each
    cx.reef(&repo, &["plan", "new", "other"]).await?;
    let a = cx.yaml("a.yml", &work("A", &[]))?;
    cx.reef(&repo, &["node", "put", &a, "--plan", "central"])
        .await?;
    let x = cx.yaml("x.yml", &work("X", &[]))?;
    cx.reef(&repo, &["node", "put", &x, "--plan", "other"])
        .await?;
    // X queued by its full name, which needs no plan picked
    let added = cx.reef(&repo, &["queue", "add", "reef/other/X"]).await?;
    assert_eq!(added, "`reef/other/X` is queued 1 of 1\n");
    // across both plans, X first, every node named in full
    let next = cx.reef(&repo, &["next"]).await?;
    assert_eq!(
        next,
        "Ready to start (2), queued first, then in plan order:\n  reef/other/X    X Some work\n  \
         reef/central/A  A Some work\n\nWaiting (0):\n  nothing\n"
    );
    let ls = cx.reef(&repo, &["queue", "ls"]).await?;
    assert_eq!(
        ls,
        "1. reef/other/X  ready  X Some work\n                 queued by tester\n"
    );
    // narrowed to central, as next of that plan alone was before the queue
    let next = cx.reef(&repo, &["next", "--plan", "central"]).await?;
    assert_eq!(
        next,
        "Ready to start (1), in plan order:\n  A  A Some work\n\nWaiting (0):\n  nothing\n"
    );
    // each plan's queue is its own: A goes at the top of `central`'s, which holds it alone
    let added = cx
        .reef(&repo, &["queue", "add", "A", "--plan", "central", "--top"])
        .await?;
    assert_eq!(added, "`A` is queued 1 of 1\n");
    let ls = cx
        .reef(&repo, &["queue", "ls", "--plan", "central"])
        .await?;
    assert_eq!(ls, "1. A  ready  A Some work\n      queued by tester\n");
    // and a place beside another plan's entry is refused, saying why
    let (_, error) = cx
        .failing(
            &repo,
            &[
                "queue",
                "add",
                "A",
                "--plan",
                "central",
                "--before",
                "reef/other/X",
            ],
        )
        .await?;
    assert!(error.contains("among its own entries"), "{error}");
    Ok(())
}

/// Each queue command says what it refuses and how to see what is queued
#[tokio::test]
async fn queue_commands_say_what_they_refuse() -> anyhow::Result<()> {
    let cx = Cx::new("queue-refuses").await?;
    let repo = cx.ax.repo.clone();
    cx.put("a.yml", &work("A", &[])).await?;
    // a move of a node not queued, or one beside a node not queued
    let (_, error) = cx.failing(&repo, &["queue", "move", "A", "--top"]).await?;
    assert_eq!(error, "`A` isn't queued: `reef queue ls` lists what is");
    let (_, error) = cx
        .failing(&repo, &["queue", "add", "A", "--before", "B"])
        .await?;
    assert_eq!(error, "`B` isn't queued: `reef queue ls` lists what is");
    // a move that says nowhere, and a removal of a node not queued
    let (_, error) = cx.failing(&repo, &["queue", "move", "A"]).await?;
    assert!(error.contains("needs where to"), "{error}");
    let (_, error) = cx.failing(&repo, &["queue", "rm", "A"]).await?;
    assert_eq!(error, "`A` isn't queued: `reef queue ls` lists what is");
    // a node not in its plan, and one of another project
    let (_, error) = cx.failing(&repo, &["queue", "add", "nope"]).await?;
    assert_eq!(error, "node `reef/central/nope` is not in its plan");
    let (_, error) = cx.failing(&repo, &["queue", "add", "app/main/A"]).await?;
    assert!(error.contains("not of project `reef`"), "{error}");
    // and nothing was queued
    let ls = cx.reef(&repo, &["queue", "ls"]).await?;
    assert!(ls.starts_with("the queue of `reef` is empty"), "{ls}");
    Ok(())
}

/// The keys of a node in the viewer's payload that say where it was written, which a book's
/// nodes and a stored plan's give differently
const PROVENANCE: [&str; 5] = ["source", "line", "name", "inline", "page"];

/// Copies a directory's files and directories into another, making it
///
/// # Arguments
///
/// * `from` - The directory to copy
/// * `to` - Where to copy it to
fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        // each directory copied in turn, each file as it is
        if path.is_dir() {
            copy_dir(&path, &to.join(entry.file_name()))?;
        } else {
            std::fs::copy(&path, to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Writes files under a directory, making the directories each is in
///
/// # Arguments
///
/// * `dir` - The directory to write them under
/// * `files` - Each file's path under it, and its text
fn write_files(dir: &Path, files: &[(&str, &str)]) -> anyhow::Result<()> {
    for (path, text) in files {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)?;
    }
    Ok(())
}

/// Where a book's links are served once it is imported
struct Served<'a> {
    /// Where the service serves its plan's docs
    docs: &'a str,
    /// The book's `src`, from the repo's root
    prefix: &'a str,
    /// The book's pages, by their paths from its `src`, which the plan keeps at the same paths
    pages: &'a HashSet<String>,
    /// The id of the node heading each of the book's node files, by the file's path from its
    /// `src`
    nodes: &'a HashMap<String, String>,
}

/// Text with each link a book's payload writes from its root made to lead where the service's
/// payload links lead: under its plan's docs, to the plan's page if the link names one of the
/// book's pages, to its node's box if it is a markdown link into a node's file, and from the
/// repo's root otherwise
///
/// A link is a field that starts with the root, or an `href` or `src` that does. The root named
/// anywhere else is prose, such as a node's text quoting it, which the service swaps for its
/// docs' path as it is. A doc link is a field of its own, and leads to the file it reads.
///
/// # Arguments
///
/// * `text` - The text the links are in
/// * `book` - Where the book's links are served
fn served(text: &str, book: &Served) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(ROOT) {
        // the text before the link, then the link's path from where the service serves it
        out.push_str(&rest[..at]);
        let link = out.is_empty() || out.ends_with("href=\"") || out.ends_with("src=\"");
        let after = &rest[at + ROOT.len()..];
        let end = after
            .find(['"', '\'', '#', '<', '>', ' ', '\n', ')'])
            .unwrap_or(after.len());
        let path = &after[..end];
        // a markdown link into a node's file is one to its node's box, at no heading
        let node = paths::md_of(path)
            .iter()
            .find_map(|md| book.nodes.get(md.as_str()));
        if let Some(id) = node.filter(|_| out.ends_with("href=\"")) {
            out.push_str(&format!("javascript:void 0\" data-goto=\"{id}"));
            let rest_of = &after[end..];
            rest = rest_of.find('"').map_or(rest_of, |at| &rest_of[at..]);
            continue;
        }
        out.push_str(book.docs);
        // a page at the same path, else the repo's file
        let page = paths::md_of(path).iter().any(|md| book.pages.contains(md));
        match link && !page {
            true => out.push_str(&paths::join(book.prefix, path)),
            false => out.push_str(path),
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// A payload as JSON, without where each node was written, and with a book's links made to
/// lead where the service's do
///
/// # Arguments
///
/// * `payload` - The viewer's payload
/// * `book` - Where a book's links are served, for a book's payload
fn comparable(payload: Payload, book: Option<&Served>) -> anyhow::Result<Value> {
    /// Each string in a value with its book links served, as `served` makes them
    ///
    /// # Arguments
    ///
    /// * `value` - The value to change
    /// * `book` - Where the book's links are served
    fn serve_all(value: &mut Value, book: &Served) {
        match value {
            Value::String(text) => *text = served(text, book),
            Value::Array(items) => items.iter_mut().for_each(|item| serve_all(item, book)),
            Value::Object(map) => map.values_mut().for_each(|item| serve_all(item, book)),
            _ => {}
        }
    }
    let mut json = serde_json::to_value(payload)?;
    // where each node was written is left out
    for node in json["nodes"].as_array_mut().into_iter().flatten() {
        let node = node.as_object_mut().expect("a node is an object");
        PROVENANCE.iter().for_each(|key| {
            node.remove(*key);
        });
    }
    // and a book's links are served as the service's are
    if let Some(book) = book {
        serve_all(&mut json, book);
    }
    Ok(json)
}

/// Checks that the plan a book was imported into draws as mdbook-reef draws the book, apart
/// from where each node was written, naming the first node that doesn't
///
/// # Arguments
///
/// * `cx` - The case, whose plan the book was imported into
/// * `book` - The book's directory
/// * `prefix` - The book's `src`, from the repo's root
async fn draws_as_the_book(cx: &Cx, book: &Path, prefix: &str) -> anyhow::Result<()> {
    // the book's payload, as mdbook-reef builds it from its files
    let opened = reef::import::open_book(book)?;
    let plan = Plan::load(&opened.src, &opened.config);
    anyhow::ensure!(
        plan.diags.is_empty(),
        "the book has problems:\n{}",
        plan.errors()
    );
    let docs = reef_api::paths::docs(&cx.ax.plan);
    let (files, _) = read_nodes(&opened.src, &opened.config.roots);
    let pages: HashSet<String> = reef::import::pages_of(&opened.src, &files)?
        .into_iter()
        .collect();
    let nodes: HashMap<String, String> = files
        .iter()
        .map(|file| (file.rel.clone(), file.spec.id.clone()))
        .collect();
    let book = Served {
        docs: &docs,
        prefix,
        pages: &pages,
        nodes: &nodes,
    };
    let from_book = comparable(plan.payload(&HashSet::new()), Some(&book))?;
    // and the plan's, as the service serves it
    let stored = comparable(cx.ax.service.payload(&cx.ax.plan).await?, None)?;
    // the same nodes in the same order, each the same, or the first that differs named
    let ids = |json: &Value| -> Vec<String> {
        json["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|node| node["id"].as_str().unwrap_or_default().to_string())
            .collect()
    };
    assert_eq!(ids(&from_book), ids(&stored));
    for (book_node, stored_node) in from_book["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .zip(stored["nodes"].as_array().into_iter().flatten())
    {
        assert_eq!(book_node, stored_node, "node `{}` differs", book_node["id"]);
    }
    Ok(())
}

/// The example book imported into a plan draws as mdbook-reef draws the book, apart from where
/// each node was written
#[tokio::test]
async fn imported_book_draws_as_mdbook_does() -> anyhow::Result<()> {
    let cx = Cx::new("import-example").await?;
    // the example book, copied into the `reef` project's repo
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../example-book");
    let book = cx.ax.repo.join("example-book");
    copy_dir(&example.join("src"), &book.join("src"))?;
    std::fs::copy(example.join("book.toml"), book.join("book.toml"))?;
    // imported into the project's only plan, from the repo's root
    let out = cx.reef(&cx.ax.repo, &["import", "example-book"]).await?;
    assert!(
        out.starts_with(
            "imported 202 nodes and 100 pages from example-book into `reef/central` at revision "
        ),
        "{out}"
    );
    // its nodes check as the book's do, and draw as the book's do
    let checked = cx.reef(&cx.ax.repo, &["check"]).await?;
    assert_eq!(checked, "`reef/central` has no problems\n");
    draws_as_the_book(&cx, &book, "example-book/src").await
}

/// A synthetic book whose nodes link every way a node can, from its body, its description, its
/// design choices, its measurements' note and its criteria, inline, by reference, to a heading
/// of its own file and out of the book into the repo, and whose page links to a page, to a
/// node's file, out of the book, to a heading of its own, and names a page in a fence
const LINKED_BOOK: [(&str, &str); 6] = [
    ("README.md", "# The repo\n\n## Setup\n\nSet it up.\n"),
    (
        "book/book.toml",
        "[book]\nsrc = \"src\"\n\n[preprocessor.reef]\n",
    ),
    (
        "book/src/design.md",
        "# Design\n\n## Rows\n\nEach node a row: [notes](plan/notes.md), [g](plan/g.md#x), \
         [setup](../../README.md#setup), [rows](#rows).\n\n```reef\npage = \"plan/notes.md\"\n```\n",
    ),
    (
        "book/src/plan/a.md",
        "---\nid: a\ntitle: a Some work\ngroup: g\ndocs: [../design.md]\n\
         description: { why: Without a., what: With a. }\n---\nSee [g](g.md) and [rows](../design.md#rows).\n",
    ),
    (
        "book/src/plan/g.md",
        "---\nid: g\ntitle: g The group\ndocs: [../design.md#rows]\nnodes:\n  - id: m\n    \
         body: \"# M heading\\n\\nSee [the repo](../../../README.md#setup) and [rows][r].\\n\\n[r]: ../design.md#rows\\n\"\n    \
         description:\n      why: \"Without [rows](../design.md#rows), slow.\"\n      what: \"Fast, see [x](#x).\"\n      \
         details: \"![a picture](../picture.png)\"\n    decisions: [\"[the design](../design.md)\"]\n    \
         performance:\n      note: \"Measured as [rows](../design.md#rows) says.\"\n      metrics: [{ name: n, before: 2, after: 1 }]\n    \
         acceptance:\n      - unit: \"Works as [the design](../design.md) says\"\n---\n\
         # G\n\n## X\n\nThe group: [rows](../design.md#rows), [its own](#x), [the repo](../../../README.md), <a@b.org>.\n",
    ),
    ("book/src/plan/notes.md", "# Notes\n\nA doc, not a node.\n"),
];

/// Every link a book's node writes leads where it did once the book is imported, and a member's
/// body stays in its spec
#[tokio::test]
async fn import_rebases_every_link() -> anyhow::Result<()> {
    let cx = Cx::new("import-links").await?;
    write_files(&cx.ax.repo, &LINKED_BOOK)?;
    // imported from the book's own directory, which picks its project
    let out = cx.reef(&cx.ax.repo.join("book"), &["import"]).await?;
    assert!(
        out.starts_with("imported 3 nodes and 2 pages from . into `reef/central`"),
        "{out}"
    );
    draws_as_the_book(&cx, &cx.ax.repo.join("book"), "book/src").await?;
    // the links lead from the repo's root, out of the book where it was left, a README to
    // the index mdbook writes for it
    let payload = cx.ax.service.payload(&cx.ax.plan).await?;
    let docs = reef_api::paths::docs(&cx.ax.plan);
    let g = payload
        .nodes
        .iter()
        .find(|node| node.id == "g")
        .expect("g is drawn");
    assert!(
        g.html.contains(&format!("href=\"{docs}index.html\"")),
        "{}",
        g.html
    );
    assert!(
        g.html
            .contains(r#"<a href="javascript:void 0" data-goto="g">its own</a>"#),
        "{}",
        g.html
    );
    assert!(g.html.contains("href=\"mailto:a@b.org\""), "{}", g.html);
    // a link into a node's file names the node, the heading it named kept for an export
    let a = cx
        .ax
        .service
        .get(&cx.ax.plan, "a")
        .await?
        .expect("a is stored");
    assert!(a.body.starts_with("See [g](node:g) and "), "{}", a.body);
    let g_body = cx
        .ax
        .service
        .get(&cx.ax.plan, "g")
        .await?
        .expect("g is stored");
    assert!(
        g_body.body.contains("[its own](node:g#x)"),
        "{}",
        g_body.body
    );
    // the member keeps its body in its spec, its row's is empty, and it is titled by its id
    let m = cx
        .ax
        .service
        .get(&cx.ax.plan, "m")
        .await?
        .expect("m is stored");
    assert!(
        m.spec
            .body
            .as_deref()
            .is_some_and(|body| body.starts_with("# M heading")),
        "{m:?}"
    );
    assert_eq!(m.body, "");
    let drawn = payload
        .nodes
        .iter()
        .find(|node| node.id == "m")
        .expect("m is drawn");
    assert_eq!(drawn.title, "m");
    // its stored links are written from the repo's root, over which the book's pages lie
    assert_eq!(
        m.spec.description.as_ref().and_then(|d| d.why.as_deref()),
        Some("Without [rows](design.md#rows), slow.")
    );
    assert_eq!(m.spec.docs, Vec::<String>::new());
    let g_stored = cx
        .ax
        .service
        .get(&cx.ax.plan, "g")
        .await?
        .expect("g is stored");
    assert_eq!(g_stored.spec.docs, ["design.md#rows"]);
    // the book's pages are the plan's, at their paths from its `src`
    let pages = cx.ax.service.pages(&cx.ax.plan).await?.pages;
    let paths: Vec<&str> = pages.iter().map(|page| page.path.as_str()).collect();
    assert_eq!(paths, ["design.md", "plan/notes.md"]);
    // a page's links to pages and its own headings are as written, its fence too, and its
    // others lead to the repo's files from where it is
    assert_eq!(
        pages[0].text,
        "# Design\n\n## Rows\n\nEach node a row: [notes](plan/notes.md), [g](node:g#x), \
         [setup](README.md#setup), [rows](#rows).\n\n```reef\npage = \"plan/notes.md\"\n```\n"
    );
    // and served, they lead where they did: the page, the node's box, and the repo's README
    let page = cx
        .ax
        .service
        .doc_page(&cx.ax.plan, "design.html")
        .await?
        .expect("the page is served");
    for href in [
        format!("href=\"{docs}plan/notes.html\""),
        r#"href="javascript:void 0" data-goto="g""#.to_string(),
        format!("href=\"{docs}index.html#setup\""),
        format!("href=\"{docs}design.html#rows\""),
    ] {
        assert!(page.contains(&href), "{href} isn't in {page}");
    }
    Ok(())
}

/// An import refuses a plan that has nodes unless told to replace them, a book that names its
/// own sections, a book with a problem and an id a plan can't name, writing nothing
#[tokio::test]
async fn import_refuses() -> anyhow::Result<()> {
    let cx = Cx::new("import-refuses").await?;
    let repo = cx.ax.repo.clone();
    let node = |id: &str, extra: &str| {
        format!(
            "---\nid: \"{id}\"\ntitle: {id} Some work\n{extra}description: {{ why: W, what: X }}\n---\n"
        )
    };
    write_files(
        &repo,
        &[
            ("book/book.toml", "[preprocessor.reef]\n"),
            ("book/src/plan/a.md", &node("a", "")),
        ],
    )?;
    // a first import into the empty plan, then a second only with `--replace`
    cx.reef(&repo, &["import", "book"]).await?;
    let (_, error) = cx.failing(&repo, &["import", "book"]).await?;
    assert_eq!(
        error,
        "`reef/central` has 1 node, which an import replaces: pass `--replace` to replace them"
    );
    let out = cx.reef(&repo, &["import", "book", "--replace"]).await?;
    assert!(out.starts_with("imported 1 node from book"), "{out}");
    let revision = cx.ax.service.list(&cx.ax.plan).await?.revision;
    // a book briefed from sections of its own
    write_files(
        &repo,
        &[
            (
                "own/book.toml",
                "[preprocessor.reef]\nwhy-sections = [\"Why\"]\n",
            ),
            ("own/src/a.md", &node("a", "")),
        ],
    )?;
    let (_, error) = cx.failing(&repo, &["import", "own", "--replace"]).await?;
    assert!(error.contains("names its own doc sections"), "{error}");
    // a book with a problem says it at its file and line
    write_files(
        &repo,
        &[
            ("broken/book.toml", "[preprocessor.reef]\n"),
            ("broken/src/a.md", &node("a", "docs: [missing.md]\n")),
        ],
    )?;
    let (_, error) = cx
        .failing(&repo, &["import", "broken", "--replace"])
        .await?;
    assert!(
        error.starts_with("broken has 1 problem, so nothing was imported:"),
        "{error}"
    );
    assert!(
        error.contains("a.md:") && error.contains("missing.md"),
        "{error}"
    );
    // an id a plan can't name
    write_files(
        &repo,
        &[
            ("slash/book.toml", "[preprocessor.reef]\n"),
            ("slash/src/a.md", &node("a/b", "")),
        ],
    )?;
    let (_, error) = cx.failing(&repo, &["import", "slash", "--replace"]).await?;
    assert!(
        error.starts_with("a.md: `a/b` can't be a node of `reef/central`"),
        "{error}"
    );
    // a link to a repo file a page of the book would hide
    write_files(
        &repo,
        &[
            ("README.md", "# The repo\n"),
            ("shadow/book.toml", "[preprocessor.reef]\n"),
            ("shadow/src/a.md", &node("a", "docs: [README.md]\n")),
            ("shadow/src/README.md", "# The book's own\n"),
            (
                "shadow/src/b.md",
                &format!("{}[the repo](../../README.md)\n", node("b", "")),
            ),
        ],
    )?;
    let (_, error) = cx
        .failing(&repo, &["import", "shadow", "--replace"])
        .await?;
    assert_eq!(
        error,
        "b.md: the link `../../README.md` leads to the repo's `README.md`, which the book's page \
         at that path would hide once imported"
    );
    // a node file outside the book's roots, which would be kept as a page a book reads as a node
    write_files(
        &repo,
        &[
            (
                "rooted/book.toml",
                "[preprocessor.reef]\nroots = [\"plan\"]\n",
            ),
            ("rooted/src/plan/a.md", &node("a", "")),
            ("rooted/src/notes.md", &node("n", "")),
        ],
    )?;
    let (_, error) = cx
        .failing(&repo, &["import", "rooted", "--replace"])
        .await?;
    assert!(
        error.starts_with("notes.md has a node's frontmatter, an `id`, but is outside"),
        "{error}"
    );
    // and none of them wrote anything
    assert_eq!(cx.ax.service.list(&cx.ax.plan).await?.revision, revision);
    // a plan with pages and no nodes is replaced only when asked too
    cx.reef(&repo, &["plan", "new", "paged"]).await?;
    write_files(&repo, &[("plan.md", "# Plan\n")])?;
    cx.reef(
        &repo,
        &["page", "put", "plan.md", "plan.md", "--plan", "paged"],
    )
    .await?;
    let (_, error) = cx
        .failing(&repo, &["import", "book", "--plan", "paged"])
        .await?;
    assert_eq!(
        error,
        "`reef/paged` has 0 nodes and 1 page, which an import replaces: pass `--replace` to \
         replace them"
    );
    Ok(())
}

/// Copies the example book's `book.toml` and `src` into a directory
///
/// # Arguments
///
/// * `to` - The directory to copy it to
fn example_book(to: &Path) -> anyhow::Result<()> {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../example-book");
    copy_dir(&example.join("src"), &to.join("src"))?;
    std::fs::copy(example.join("book.toml"), to.join("book.toml"))?;
    Ok(())
}

/// Every file under a directory, by its path from it, with its text
///
/// # Arguments
///
/// * `dir` - The directory to read
fn read_tree(dir: &Path) -> anyhow::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(dir)?.to_string_lossy().to_string();
                out.push((rel, std::fs::read_to_string(&path)?));
            }
        }
    }
    // in an order that doesn't hang on the order they were read in
    out.sort();
    Ok(out)
}

/// Checks that two plans hold the same nodes, each with its spec and body, and in the same
/// places if asked, and the same pages
///
/// # Arguments
///
/// * `cx` - The case, whose service holds both plans
/// * `a` - One plan
/// * `b` - The other
/// * `placed` - Whether each node must have the same place in the written order too
async fn same_plans(cx: &Cx, a: &PlanName, b: &PlanName, placed: bool) -> anyhow::Result<()> {
    let mut first = cx.ax.service.list(a).await?.nodes;
    let mut second = cx.ax.service.list(b).await?.nodes;
    // by id, if their places may differ
    if !placed {
        first.sort_by(|x, y| x.spec.id.cmp(&y.spec.id));
        second.sort_by(|x, y| x.spec.id.cmp(&y.spec.id));
    }
    // each node its spec, its body and its place, if asked
    assert_eq!(first.len(), second.len());
    for (x, y) in first.iter().zip(&second) {
        assert_eq!(
            (&x.spec, &x.body),
            (&y.spec, &y.body),
            "`{}` differs",
            x.spec.id
        );
        if placed {
            assert_eq!(x.order, y.order, "`{}` moved", x.spec.id);
        }
    }
    // and each page
    let pages = |pages: Vec<reef_api::StoredPage>| -> Vec<(String, String)> {
        pages
            .into_iter()
            .map(|page| (page.path, page.text))
            .collect()
    };
    let first = cx.ax.service.pages(a).await?.pages;
    let second = cx.ax.service.pages(b).await?.pages;
    assert_eq!(pages(first), pages(second));
    Ok(())
}

/// A plan exported to a book and imported again gives the same specs, bodies and order, and
/// the book exports again to the same files
#[tokio::test]
async fn export_then_import_gives_equal_specs() -> anyhow::Result<()> {
    let cx = Cx::new("export-round-trip").await?;
    let repo = cx.ax.repo.clone();
    // the example book, imported
    example_book(&repo.join("example-book"))?;
    cx.reef(&repo, &["import", "example-book"]).await?;
    // exported to a new book in the repo, a file for each of the book's node files
    let out = cx
        .reef(&repo, &["export", "out", "--plan", "central"])
        .await?;
    assert!(
        out.starts_with("exported 202 nodes and 100 pages of `reef/central` to out in 112 files:"),
        "{out}"
    );
    // which mdbook-reef finds no problems in
    let opened = reef::import::open_book(&repo.join("out"))?;
    let plan = Plan::load(&opened.src, &opened.config);
    assert!(plan.diags.is_empty(), "{}", plan.errors());
    // imported again, into a plan of its own, each node and page as the first import wrote it
    cx.reef(&repo, &["import", "out", "--plan", "again"])
        .await?;
    let again = PlanName::new("reef", "again")?;
    same_plans(&cx, &cx.ax.plan, &again, true).await?;
    assert_eq!(cx.ax.service.pages(&again).await?.pages.len(), 100);
    // the summary lists each page after the node files
    let summary = std::fs::read_to_string(repo.join("out/src/SUMMARY.md"))?;
    assert!(
        summary.contains("\n# Pages\n\n- [") && summary.contains("](plan.md)\n"),
        "{summary}"
    );
    // and exported again, it writes the same files, titled by its own plan
    cx.reef(&repo, &["export", "out2", "--plan", "again"])
        .await?;
    let retitled: Vec<(String, String)> = read_tree(&repo.join("out"))?
        .into_iter()
        .map(|(path, text)| (path, text.replace("reef/central", "reef/again")))
        .collect();
    assert_eq!(retitled, read_tree(&repo.join("out2"))?);
    Ok(())
}

/// An export refuses a directory that holds anything, and one outside the project's repo,
/// leaving no directory it made behind
#[tokio::test]
async fn export_refuses() -> anyhow::Result<()> {
    let cx = Cx::new("export-refuses").await?;
    let repo = cx.ax.repo.clone();
    cx.put("a.yml", &work("A", &[])).await?;
    // a directory with anything in it
    write_files(&repo, &[("full/notes.md", "# Notes\n")])?;
    let (_, error) = cx.failing(&repo, &["export", "full"]).await?;
    assert_eq!(
        error,
        "full isn't empty: export writes a new book, or over a reef book with `--replace`"
    );
    // a directory outside the project's repo, which the export takes away again
    let (_, error) = cx
        .failing(&repo, &["export", "../outside", "--project", "reef"])
        .await?;
    assert!(error.contains("is not in project `reef`'s repo"), "{error}");
    assert!(!cx.ax.dir.join("outside").exists());
    // a page at the path a node's file is written to
    write_files(&repo, &[("clash.md", "# Clash\n")])?;
    cx.reef(&repo, &["page", "put", "001-a.md", "clash.md"])
        .await?;
    let (_, error) = cx.failing(&repo, &["export", "clash"]).await?;
    assert_eq!(
        error,
        "page `001-a.md` has the name the file of node `A` is written to: move the page"
    );
    assert!(!repo.join("clash").exists());
    cx.reef(&repo, &["page", "rm", "001-a.md"]).await?;
    // and the plan exports where it may
    let out = cx.reef(&repo, &["export", "book"]).await?;
    assert!(
        out.starts_with("exported 1 node of `reef/central` to book in 1 file:"),
        "{out}"
    );
    Ok(())
}

/// A plan's pages are listed, got, put and removed by version, each edit checked against the
/// plan, and a page put over a file of the repo says so
#[tokio::test]
async fn page_commands_edit_by_version() -> anyhow::Result<()> {
    let cx = Cx::new("page-commands").await?;
    let repo = cx.ax.repo.clone();
    // a plan keeps no pages at first
    let ls = cx.reef(&repo, &["page", "ls"]).await?;
    assert_eq!(ls, "`reef/central` keeps no pages\n");
    // a new page is put from a file, and put again as new is refused as taken
    write_files(
        &repo,
        &[(
            "scratch/design.md",
            "# Design\n\n## Rows\n\nEach node a row.\n",
        )],
    )?;
    let put = cx
        .reef(&repo, &["page", "put", "design.md", "scratch/design.md"])
        .await?;
    assert_eq!(put, "put page `design.md` at version 1\n");
    let (_, error) = cx
        .failing(&repo, &["page", "put", "design.md", "scratch/design.md"])
        .await?;
    assert_eq!(
        error,
        "the put was refused, since page `design.md` of `reef/central` is not as it was read: it \
         already exists: to change it, put what `reef page get design.md` prints, keeping its \
         `<!-- version: … -->` line"
    );
    // what get prints is put back as a change to the version it read
    let got = cx.reef(&repo, &["page", "get", "design.md"]).await?;
    assert_eq!(
        got,
        "<!-- version: 1 -->\n# Design\n\n## Rows\n\nEach node a row.\n"
    );
    write_files(&repo, &[("scratch/v1.md", &format!("{got}\nMore.\n"))])?;
    let put = cx
        .reef(&repo, &["page", "put", "design.md", "scratch/v1.md"])
        .await?;
    assert_eq!(put, "put page `design.md` at version 2\n");
    // and put at that version again it is stale
    let (_, error) = cx
        .failing(&repo, &["page", "put", "design.md", "scratch/v1.md"])
        .await?;
    assert!(error.contains("it changed since it was read"), "{error}");
    // a node links its heading, so it can't be taken out, nor lose that heading
    let linked =
        "id: A\ntitle: A Some work\ndocs: [design.md#rows]\ndescription: { why: W, what: X }\n";
    cx.put("a.yml", linked).await?;
    let (_, error) = cx.failing(&repo, &["page", "rm", "design.md"]).await?;
    assert!(
        error.starts_with(
            "the removal was refused, since it would add these problems to `reef/central`:"
        ),
        "{error}"
    );
    assert!(
        error.contains("doc `design.md#rows` not found (looked for design.md)"),
        "{error}"
    );
    write_files(
        &repo,
        &[(
            "scratch/v2.md",
            "<!-- version: 2 -->\n# Design\n\nNo rows.\n",
        )],
    )?;
    let (_, error) = cx
        .failing(&repo, &["page", "put", "design.md", "scratch/v2.md"])
        .await?;
    assert!(
        error.contains("design.md has no heading with the id `rows`"),
        "{error}"
    );
    // a page over a file of the repo says it hides it
    write_files(&repo, &[("notes.md", "# The repo's notes\n")])?;
    let put = cx
        .reef(&repo, &["page", "put", "notes.md", "scratch/design.md"])
        .await?;
    assert_eq!(
        put,
        "put page `notes.md` at version 1\nnote: the page hides the file `notes.md` in `reef`'s \
         repo, which its links led to\n"
    );
    let ls = cx.reef(&repo, &["page", "ls"]).await?;
    assert_eq!(ls, "design.md (version 2)\nnotes.md (version 1)\n");
    // once nothing links it, it comes out, and a path no page can have is refused
    cx.reef(&repo, &["node", "rm", "A"]).await?;
    let rm = cx.reef(&repo, &["page", "rm", "design.md"]).await?;
    assert_eq!(rm, "removed page `design.md`\n");
    let (_, error) = cx
        .failing(&repo, &["page", "put", "../out.md", "scratch/design.md"])
        .await?;
    assert_eq!(
        error,
        "`../out.md` has a part starting with `.`, which a book leaves out"
    );
    let (_, error) = cx.failing(&repo, &["page", "get", "design.md"]).await?;
    assert!(
        error.contains("no page `design.md` in `reef/central`"),
        "{error}"
    );
    Ok(())
}

/// A book whose page links into a group's file inline and by reference, at a heading of it, and
/// into another node's file, and whose node links a heading of the group's file
const NODE_LINKED_BOOK: [(&str, &str); 4] = [
    (
        "book/book.toml",
        "[book]\nsrc = \"src\"\n\n[preprocessor.reef]\n",
    ),
    (
        "book/src/design.md",
        "# Design\n\n## Rows\n\nSee [M](plan/m.md), [W](plan/w.md) and [rows][r].\n\n[r]: plan/m.md#rows\n",
    ),
    (
        "book/src/plan/m.md",
        "---\nid: M\ntitle: M The milestone\nnodes:\n  - id: C\n    title: C Some work\n    \
         description: { why: Without C., what: With C. }\n---\n# M\n\n## Rows\n\nRows.\n",
    ),
    (
        "book/src/plan/w.md",
        "---\nid: W\ntitle: W Other work\ndocs: [../design.md#rows]\n\
         description: { why: Without W., what: \"With W, as [M's rows](m.md#rows) say.\" }\n---\n# W\n",
    ),
];

/// A link into a node's file names the node once imported: the sidebar is given it as a link
/// to the node's box, a page or node naming a node the plan lacks is a problem, so the node
/// can't be removed while a page links it, and an export writes it back to the node's file
#[tokio::test]
async fn links_into_node_files_name_their_nodes() -> anyhow::Result<()> {
    let cx = Cx::new("node-links").await?;
    let repo = cx.ax.repo.clone();
    write_files(&repo, &NODE_LINKED_BOOK)?;
    let book = repo.join("book");
    let before = read_tree(&book)?;
    // imported, every link into a node's file names its node, at the heading it named
    let out = cx.reef(&repo, &["import", "book"]).await?;
    assert!(
        out.starts_with("imported 3 nodes and 1 page from book"),
        "{out}"
    );
    assert_eq!(
        cx.reef(&repo, &["check"]).await?,
        "`reef/central` has no problems\n"
    );
    let pages = cx.ax.service.pages(&cx.ax.plan).await?.pages;
    assert_eq!(
        pages[0].text,
        "# Design\n\n## Rows\n\nSee [M](node:M), [W](node:W) and [rows][r].\n\n[r]: node:M#rows\n"
    );
    let w = cx
        .ax
        .service
        .get(&cx.ax.plan, "W")
        .await?
        .expect("W is stored");
    assert_eq!(
        w.spec.description.as_ref().and_then(|d| d.what.as_deref()),
        Some("With W, as [M's rows](node:M#rows) say.")
    );
    // its doc link reads the page, as written
    assert_eq!(w.spec.docs, ["design.md#rows"]);
    // the sidebar is given each as a link to its node's box, in the served page and the payload
    let page = cx
        .ax
        .service
        .doc_page(&cx.ax.plan, "design.html")
        .await?
        .expect("the page is served");
    for anchor in [
        r#"<a href="javascript:void 0" data-goto="M">M</a>"#,
        r#"<a href="javascript:void 0" data-goto="W">W</a>"#,
        r#"<a href="javascript:void 0" data-goto="M">rows</a>"#,
    ] {
        assert!(page.contains(anchor), "{anchor} isn't in {page}");
    }
    let payload = cx.ax.service.payload(&cx.ax.plan).await?;
    let drawn = payload
        .nodes
        .iter()
        .find(|node| node.id == "W")
        .expect("W is drawn");
    assert!(
        drawn
            .what
            .contains(r#"<a href="javascript:void 0" data-goto="M">M's rows</a>"#),
        "{}",
        drawn.what
    );
    // so W can't be removed while the page links it, and the refusal names the page
    let (_, error) = cx.failing(&repo, &["node", "rm", "W"]).await?;
    assert!(
        error.contains("page `design.md`: the link `node:W` names `W`, which is not a node"),
        "{error}"
    );
    assert!(cx.ax.service.get(&cx.ax.plan, "W").await?.is_some());
    // and a page put naming a node the plan lacks is refused, writing nothing
    let got = cx.reef(&repo, &["page", "get", "design.md"]).await?;
    let gone = cx.yaml("design.md", &format!("{got}\nAnd [gone](node:C99).\n"))?;
    let (_, error) = cx
        .failing(&repo, &["page", "put", "design.md", &gone])
        .await?;
    assert!(
        error.contains("page `design.md`: the link `node:C99` names `C99`, which is not a node"),
        "{error}"
    );
    // a node's markdown naming one is refused the same, at the node
    let w_yaml = cx.reef(&repo, &["node", "get", "W"]).await?;
    let gone = w_yaml.replace("as [M's rows](node:M#rows) say", "as [gone](node:C99) says");
    let error = cx.put("w.yml", &gone).await.unwrap_err().to_string();
    assert!(
        error.contains("reef/central/W: the link `node:C99` names `C99`, which is not a node"),
        "{error}"
    );
    // exported to a new book, each leads to the file its node is written in
    cx.reef(&repo, &["export", "out"]).await?;
    let design = std::fs::read_to_string(repo.join("out/src/design.md"))?;
    assert_eq!(
        design,
        "# Design\n\n## Rows\n\nSee [M](001-m.md), [W](002-w.md) and [rows][r].\n\n[r]: 001-m.md#rows\n"
    );
    let w_file = std::fs::read_to_string(repo.join("out/src/002-w.md"))?;
    assert!(
        w_file.contains("as [M's rows](001-m.md#rows) say"),
        "{w_file}"
    );
    // and written back over its own book, each is as it was written
    let out = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(
        out.ends_with(", 0 removed\n  added src/SUMMARY.md\n"),
        "{out}"
    );
    let after: Vec<(String, String)> = read_tree(&book)?
        .into_iter()
        .filter(|(path, _)| path != "src/SUMMARY.md")
        .collect();
    kept_as_written(&before, &after)?;
    Ok(())
}

/// This plan's own book, imported, checks clean, draws as the book does and serves every
/// design page once the book is taken away, and its export has the page its overview's fence
/// names
#[tokio::test]
async fn imported_pages_outlive_the_book() -> anyhow::Result<()> {
    let cx = Cx::new("plan-outlives").await?;
    let repo = cx.ax.repo.clone();
    // this repo's plan, copied into the project's repo
    let book = this_plan(&repo)?;
    let pages = reef::import::pages_of(&book.join("src"), &read_nodes(&book.join("src"), &[]).0)?;
    // imported, every design page with it
    let out = cx.reef(&repo, &["import", "plan"]).await?;
    assert!(
        out.contains(&format!(
            " and {} pages from plan into `reef/central`",
            pages.len()
        )),
        "{out}"
    );
    draws_as_the_book(&cx, &book, "plan/src").await?;
    // the book taken away, the plan still checks clean
    std::fs::remove_dir_all(&book)?;
    let checked = cx.reef(&repo, &["check"]).await?;
    assert_eq!(checked, "`reef/central` has no problems\n");
    // and serves each of its pages
    for page in &pages {
        let html = paths::html_of(page);
        let served = cx.ax.service.doc_page(&cx.ax.plan, &html).await?;
        assert!(
            served.is_some_and(|html| html.contains("<main>")),
            "{page} isn't served"
        );
    }
    // exported, the overview's fence names a page the export has
    cx.reef(&repo, &["export", "out"]).await?;
    let overview = read_tree(&repo.join("out/src"))?
        .into_iter()
        .find(|(_, text)| text.starts_with("---\nid: central\n"))
        .expect("the overview is exported");
    assert!(overview.1.contains("page = \"plan.md\""), "{}", overview.0);
    assert!(repo.join("out/src/plan.md").is_file());
    Ok(())
}

/// Copies this repo's plan, its `book.toml` and its `src`, into a repo, returning where it is
///
/// # Arguments
///
/// * `repo` - The repo to copy it into
fn this_plan(repo: &Path) -> anyhow::Result<PathBuf> {
    let plan_book = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan");
    let book = repo.join("plan");
    copy_dir(&plan_book.join("src"), &book.join("src"))?;
    std::fs::copy(plan_book.join("book.toml"), book.join("book.toml"))?;
    Ok(book)
}

/// Checks that a book written over holds what it did: the same files, each node file's nodes
/// the same and its body byte for byte, and every other file byte for byte
///
/// # Arguments
///
/// * `before` - The book's files before, as `read_tree` reads them
/// * `after` - The book's files after
fn kept_as_written(before: &[(String, String)], after: &[(String, String)]) -> anyhow::Result<()> {
    // the same files
    let paths = |tree: &[(String, String)]| -> Vec<String> {
        tree.iter().map(|(path, _)| path.clone()).collect()
    };
    assert_eq!(paths(before), paths(after));
    for ((path, was), (_, now)) in before.iter().zip(after) {
        // each read as a node file, if it is one
        let was_node =
            parse_node_file(was).map_err(|error| anyhow::anyhow!("{path}: {error:?}"))?;
        let now_node =
            parse_node_file(now).map_err(|error| anyhow::anyhow!("{path}: {error:?}"))?;
        match (was_node, now_node) {
            // a node file's nodes the same, however its frontmatter is styled, and its body
            (Some((was_spec, was_fm)), Some((now_spec, now_fm))) => {
                assert_eq!(
                    reef_api::lift(was_spec),
                    reef_api::lift(now_spec),
                    "{path}'s nodes differ"
                );
                assert_eq!(was_fm.body, now_fm.body, "{path}'s body differs");
            }
            // and any other file as it was
            _ => assert_eq!(was, now, "{path} differs"),
        }
    }
    Ok(())
}

/// Checks that a book imported, then exported over itself, holds what it did, draws as it
/// did, is written as it is by a second export, and imports again as the same plan, returning
/// what the first export printed
///
/// # Arguments
///
/// * `cx` - The case, whose project's repo holds the book
/// * `book` - The book's directory
/// * `name` - The book's directory, as the command line names it from the repo's root
async fn written_back(cx: &Cx, book: &Path, name: &str) -> anyhow::Result<String> {
    let repo = &cx.ax.repo;
    let before = read_tree(book)?;
    // imported, then exported over the book it came from
    cx.reef(repo, &["import", name]).await?;
    let out = cx.reef(repo, &["export", name, "--replace"]).await?;
    let after = read_tree(book)?;
    kept_as_written(&before, &after)?;
    draws_as_the_book(cx, book, &format!("{name}/src")).await?;
    // a second export finds every file as it would write it
    let again = cx.reef(repo, &["export", name, "--replace"]).await?;
    assert!(again.contains(": 0 files written, "), "{again}");
    assert_eq!(read_tree(book)?, after);
    // and imported again, it is the same plan, each node in its place
    cx.reef(repo, &["import", name, "--plan", "again"]).await?;
    same_plans(cx, &cx.ax.plan, &PlanName::new("reef", "again")?, true).await?;
    Ok(out)
}

/// This plan, imported and exported over the book it came from, keeps every file at its path,
/// each node file's body, its pages, its summary, its `book.toml` and its other files as they
/// were, and a second export writes nothing
#[tokio::test]
async fn export_replace_writes_this_plan_back_as_it_was() -> anyhow::Result<()> {
    let cx = Cx::new("replace-this-plan").await?;
    let book = this_plan(&cx.ax.repo)?;
    // the viewer and a picture, which no export writes
    write_files(
        &book,
        &[
            ("reef.css", "/* the viewer */\n"),
            ("src/x.svg", "<svg/>\n"),
        ],
    )?;
    let out = written_back(&cx, &book, "plan").await?;
    assert!(
        out.contains(" pages of `reef/central` over the book in plan: ")
            && out.ends_with(" unchanged, 0 removed\n"),
        "{out}"
    );
    Ok(())
}

/// The example book, its node files deep under its `src` and unlisted in its summary, is
/// written back over itself as it was
#[tokio::test]
async fn export_replace_keeps_the_example_books_layout() -> anyhow::Result<()> {
    let cx = Cx::new("replace-example").await?;
    let book = cx.ax.repo.join("example-book");
    example_book(&book)?;
    let out = written_back(&cx, &book, "example-book").await?;
    assert!(
        out.starts_with(
            "exported 202 nodes and 100 pages of `reef/central` over the book in example-book: "
        ),
        "{out}"
    );
    Ok(())
}

/// Every link a node or a page writes leads to the same file and heading once the plan is
/// written back over its book: a heading of its own file, another node's file, a page, a
/// reference, a picture and the repo outside the book, and a book with no summary gets one
#[tokio::test]
async fn export_replace_rebases_links_from_each_file() -> anyhow::Result<()> {
    let cx = Cx::new("replace-links").await?;
    let repo = cx.ax.repo.clone();
    write_files(&repo, &LINKED_BOOK)?;
    let book = repo.join("book");
    let before = read_tree(&book)?;
    cx.reef(&repo, &["import", "book"]).await?;
    let out = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(
        out.ends_with(", 0 removed\n  added src/SUMMARY.md\n"),
        "{out}"
    );
    // a summary written as a new book's is, listing the node files, then the pages
    let mut after = read_tree(&book)?;
    let at = after
        .iter()
        .position(|(path, _)| path == "src/SUMMARY.md")
        .expect("a summary is written");
    let (_, summary) = after.remove(at);
    assert_eq!(
        summary,
        "# Summary\n\n- [a Some work](plan/a.md)\n- [g The group](plan/g.md)\n\n# Pages\n\n\
         - [Design](design.md)\n- [Notes](plan/notes.md)\n"
    );
    // and every other file as it was, each link as written
    kept_as_written(&before, &after)?;
    draws_as_the_book(&cx, &book, "book/src").await?;
    let again = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(
        again.contains(": 0 files written, 5 unchanged, 0 removed\n"),
        "{again}"
    );
    Ok(())
}

/// A book a plan is written back over: a root, two groups in files of their own, each with a
/// member, a page, a page no node links, a picture, and a summary in parts
const KEPT_BOOK: [(&str, &str); 8] = [
    (
        "book/book.toml",
        "[book]\nsrc = \"src\"\n\n[preprocessor.reef]\n",
    ),
    (
        "book/src/SUMMARY.md",
        "# Summary\n\n[Root](root.md)\n\n# Groups\n\n- [G The group](groups/g.md)\n\
         - [H Another](groups/h.md)\n\n# Design\n\n- [Design](design.md)\n",
    ),
    (
        "book/src/root.md",
        "---\nid: root\ntitle: Root\ndescription: { why: W, what: X }\n---\n# Root\n",
    ),
    (
        "book/src/groups/g.md",
        "---\nid: G\ntitle: G The group\ngroup: root\nnodes:\n  - id: g1\n    title: g1 First\n    \
         description: { why: W, what: X }\n---\n# G\n\nSee [the design](../design.md#rows).\n",
    ),
    (
        "book/src/groups/h.md",
        "---\nid: H\ntitle: H Another\ngroup: root\nnodes:\n  - id: h1\n    title: h1 Only\n    \
         description: { why: W, what: X }\n---\n# H\n",
    ),
    (
        "book/src/design.md",
        "# Design\n\n## Rows\n\nRows, as [G](groups/g.md) says.\n",
    ),
    ("book/src/old.md", "# Old notes\n"),
    ("book/src/picture.svg", "<svg/>\n"),
];

/// Nodes and pages put and removed through the terminal land in the book written back over:
/// a member in its group's file, a top-level node in a file of its own, a group's file and a
/// page removed, a new page listed after the last, and a group retitled in the summary. A file
/// moved by hand stays where it was put, and the book imports as the plan, each node in the
/// book's order
#[tokio::test]
async fn export_replace_places_new_and_removed_nodes() -> anyhow::Result<()> {
    let cx = Cx::new("replace-places").await?;
    let repo = cx.ax.repo.clone();
    write_files(&repo, &KEPT_BOOK)?;
    let book = repo.join("book");
    cx.reef(&repo, &["import", "book"]).await?;
    // a member of G and a node of its own put, H and its member removed, a page swapped
    cx.put(
        "g2.yml",
        "id: g2\ntitle: g2 Second\ngroup: G\ndocs: [design.md#rows]\ndescription: { why: W, what: X }\n",
    )
    .await?;
    cx.put(
        "t.yml",
        "id: T\ntitle: T Top\ndocs: [design.md]\ndescription: { why: W, what: X }\n",
    )
    .await?;
    cx.reef(&repo, &["node", "rm", "H", "h1"]).await?;
    cx.reef(&repo, &["page", "rm", "old.md"]).await?;
    write_files(&repo, &[("scratch/new.md", "# New notes\n")])?;
    cx.reef(&repo, &["page", "put", "notes/new.md", "scratch/new.md"])
        .await?;
    // and G retitled
    let got = cx.reef(&repo, &["node", "get", "G"]).await?;
    cx.put(
        "g.yml",
        &got.replace("title: G The group", "title: G Renamed"),
    )
    .await?;
    let before = read_tree(&book)?;
    // written back over the book, each change where it belongs
    let out = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(
        out.starts_with("exported 5 nodes and 2 pages of `reef/central` over the book in book: ")
            && out.ends_with(
                ", 2 removed\n  added src/notes/new.md\n  added src/t.md\n  removed src/groups/h.md\n  \
                 removed src/old.md\n"
            ),
        "{out}"
    );
    let summary = std::fs::read_to_string(book.join("src/SUMMARY.md"))?;
    assert_eq!(
        summary,
        "# Summary\n\n[Root](root.md)\n\n# Groups\n\n- [G Renamed](groups/g.md)\n- [T Top](t.md)\n\n\
         # Design\n\n- [Design](design.md)\n- [New notes](notes/new.md)\n"
    );
    // the new member listed in its group's file, after the one before it, its link from there
    let g = std::fs::read_to_string(book.join("src/groups/g.md"))?;
    let (spec, fm) = parse_node_file(&g)
        .map_err(|error| anyhow::anyhow!("{error:?}"))?
        .expect("G's file holds G");
    assert_eq!(spec.title.as_deref(), Some("G Renamed"));
    let members: Vec<&str> = spec.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(members, ["g1", "g2"]);
    assert_eq!(spec.nodes[1].docs, ["../design.md#rows"]);
    assert_eq!(fm.body, "# G\n\nSee [the design](../design.md#rows).\n");
    // the files gone, and the book's others left alone
    assert!(!book.join("src/groups/h.md").exists() && !book.join("src/old.md").exists());
    for (path, text) in &before {
        if ["book.toml", "src/picture.svg", "src/design.md"].contains(&path.as_str()) {
            assert_eq!(
                &std::fs::read_to_string(book.join(path))?,
                text,
                "{path} changed"
            );
        }
    }
    // a file moved by hand stays where it was put, its links leading from there
    std::fs::create_dir_all(book.join("src/groups"))?;
    std::fs::rename(book.join("src/t.md"), book.join("src/groups/t.md"))?;
    std::fs::write(
        book.join("src/SUMMARY.md"),
        summary.replace("(t.md)", "(groups/t.md)"),
    )?;
    let out = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(
        out.contains(": 1 file written, 5 unchanged, 0 removed\n"),
        "{out}"
    );
    let t = std::fs::read_to_string(book.join("src/groups/t.md"))?;
    assert!(t.contains("docs:\n- ../design.md\n"), "{t}");
    let again = cx.reef(&repo, &["export", "book", "--replace"]).await?;
    assert!(again.contains(": 0 files written, "), "{again}");
    // imported again, the plan comes back, each node in the order the book's files sort in
    cx.reef(&repo, &["import", "book", "--plan", "again"])
        .await?;
    let again = PlanName::new("reef", "again")?;
    same_plans(&cx, &cx.ax.plan, &again, false).await?;
    let ids: Vec<String> = cx
        .ax
        .service
        .list(&again)
        .await?
        .nodes
        .into_iter()
        .map(|node| node.spec.id)
        .collect();
    assert_eq!(ids, ["G", "g1", "g2", "T", "root"]);
    // which the plan takes on when the book is imported over it
    cx.reef(&repo, &["import", "book", "--plan", "central", "--replace"])
        .await?;
    draws_as_the_book(&cx, &book, "book/src").await
}

/// Runs an export over a directory that is to be refused, checking that it leaves the
/// directory as it was, returning the error
///
/// # Arguments
///
/// * `cx` - The case, whose project's repo the directory is named from
/// * `dir` - The directory, as the command line names it from the repo's root
async fn refused_over(cx: &Cx, dir: &str) -> anyhow::Result<String> {
    let path = cx.ax.repo.join(dir);
    let before = read_tree(&path)?;
    let (_, error) = cx
        .failing(
            &cx.ax.repo,
            &["export", dir, "--replace", "--project", "reef"],
        )
        .await?;
    assert_eq!(read_tree(&path)?, before, "{dir} changed");
    Ok(error)
}

/// An export over a directory refuses one that holds no reef book, one briefed from sections
/// of its own, one outside the project's repo, one whose files aren't all nodes or pages, a
/// page at a node file's path, and a file or directory in the way, leaving each as it was
#[tokio::test]
async fn export_replace_refuses() -> anyhow::Result<()> {
    let cx = Cx::new("replace-refuses").await?;
    let repo = cx.ax.repo.clone();
    cx.put("a.yml", &work("A", &[])).await?;
    write_files(
        &repo,
        &[
            // no book, and a book with no reef plan
            ("notes/a.md", "# A\n"),
            ("plain/book.toml", "[book]\ntitle = \"x\"\n"),
            ("plain/src/a.md", "# A\n"),
            // a book briefed from sections of its own
            (
                "own/book.toml",
                "[preprocessor.reef]\nwhy-sections = [\"Why\"]\n",
            ),
            ("own/src/a.md", "# A\n"),
            // a file that isn't a node, and a node heading two files
            ("bad/book.toml", "[preprocessor.reef]\n"),
            ("bad/src/a.md", "---\nid: a\nstatus: sideways\n---\n"),
            ("twice/book.toml", "[preprocessor.reef]\n"),
            ("twice/src/a.md", "---\nid: A\n---\n"),
            ("twice/src/b.md", "---\nid: A\n---\n"),
        ],
    )?;
    write_files(
        &cx.ax.dir,
        &[
            ("outside/book.toml", "[preprocessor.reef]\n"),
            ("outside/src/a.md", "# A\n"),
        ],
    )?;
    assert_eq!(
        refused_over(&cx, "notes").await?,
        "notes holds no `book.toml`: `--replace` writes over a reef book, and an export without \
         it writes a new one"
    );
    let error = refused_over(&cx, "plain").await?;
    assert!(error.contains("has no `[preprocessor.reef]`"), "{error}");
    let error = refused_over(&cx, "own").await?;
    assert!(error.contains("names its own doc sections"), "{error}");
    let error = refused_over(&cx, "../outside").await?;
    assert!(error.contains("is not in project `reef`'s repo"), "{error}");
    let error = refused_over(&cx, "bad").await?;
    assert!(
        error.starts_with("the book's files can't all be read as nodes"),
        "{error}"
    );
    assert_eq!(
        refused_over(&cx, "twice").await?,
        "`A` heads both a.md and b.md in the book, so which file is its isn't known"
    );
    // a page at the path of a node's file in the book
    cx.reef(&repo, &["export", "kept"]).await?;
    write_files(&repo, &[("scratch/page.md", "# A page\n")])?;
    cx.reef(&repo, &["page", "put", "001-a.md", "scratch/page.md"])
        .await?;
    assert_eq!(
        refused_over(&cx, "kept").await?,
        "page `001-a.md` has the path of node `A`'s file in the book: move the page, or the \
         node's file"
    );
    cx.reef(&repo, &["page", "rm", "001-a.md"]).await?;
    // a directory where a page goes, and a file where its directory goes
    cx.reef(&repo, &["page", "put", "notes/new.md", "scratch/page.md"])
        .await?;
    write_files(&repo, &[("kept/src/notes/new.md/held.txt", "held\n")])?;
    assert_eq!(
        refused_over(&cx, "kept").await?,
        "notes/new.md is a directory in the book's src, where the export would write a file"
    );
    std::fs::remove_dir_all(repo.join("kept/src/notes"))?;
    write_files(&repo, &[("kept/src/notes", "a file\n")])?;
    assert_eq!(
        refused_over(&cx, "kept").await?,
        "notes is a file in the book's src, where the export would make a directory for \
         notes/new.md"
    );
    // and once out of the way, the plan is written over the book
    std::fs::remove_file(repo.join("kept/src/notes"))?;
    let out = cx.reef(&repo, &["export", "kept", "--replace"]).await?;
    assert!(
        out.starts_with("exported 1 node and 1 page of `reef/central` over the book in kept: ")
            && out.ends_with("  added src/notes/new.md\n"),
        "{out}"
    );
    Ok(())
}

/// A field of a work record as `reef start` and `reef work` print it
///
/// # Arguments
///
/// * `out` - What the command printed
/// * `key` - The field's name, such as `tree`
fn field<'a>(out: &'a str, key: &str) -> &'a str {
    // the first line whose first word is the key, its value after the padding
    out.lines()
        .find_map(|line| {
            line.strip_prefix(key)
                .filter(|rest| rest.starts_with(' '))
                .map(str::trim)
        })
        .unwrap_or_else(|| panic!("no `{key}` in:\n{out}"))
}

/// A project `cli` registered for a fresh repo, its plan `docs` holding the nodes given
///
/// # Arguments
///
/// * `cx` - The served twin
/// * `ids` - The ids of the nodes to put, each a piece of work
async fn cli_project(cx: &Cx, ids: &[&str]) -> anyhow::Result<PathBuf> {
    // the repo, its project and its plan
    let repo = cx.repo("cli")?;
    cx.reef(&repo, &["project", "add", "cli"]).await?;
    cx.reef(&repo, &["plan", "new", "docs"]).await?;
    // each node put from a file of its own
    for id in ids {
        let file = cx.yaml(&format!("{id}.yml"), &work(id, &[]))?;
        cx.reef(&repo, &["node", "put", &file]).await?;
    }
    Ok(repo)
}

/// A user's clone of a remote project's repo finds the project by its remote, apart from the
/// service's own clone: it puts, starts and captures the project's nodes there, and a doc
/// section it pushes can be linked once the project is fetched
#[tokio::test]
async fn a_clone_of_a_remote_project_starts_and_captures_its_nodes() -> anyhow::Result<()> {
    let cx = Cx::new("remote-clone").await?;
    // a repo published to a bare remote, registered by its URL from outside any checkout of it
    let design = "# Design\n\n## Old\n\nThe old words.\n";
    let (bare, _) = git::published(&cx.ax.dir.join("up"), &[("docs/design.md", design)])?;
    let url = format!("file://{}", bare.display());
    let out = cx
        .reef(&cx.ax.dir, &["project", "add", "up", "--url", &url])
        .await?;
    assert!(
        out.starts_with(&format!("registered `up` for {url} (`main` at ")),
        "{out}"
    );
    cx.reef(&cx.ax.dir, &["plan", "new", "docs", "--project", "up"])
        .await?;
    // a user's own clone of it, which the service never reads
    let bare_path = bare.to_str().expect("the remote is a utf-8 path");
    let mine = cx.ax.dir.join("mine");
    let mine_path = mine.to_str().expect("the case's dir is utf-8");
    git::run(&cx.ax.dir, &["clone", "-q", bare_path, mine_path])?;
    let mine = mine.canonicalize()?;
    // finds the project by its remote
    let shown = cx.reef(&mine, &["project", "show"]).await?;
    assert!(shown.starts_with("up ") && shown.contains(&url), "{shown}");
    // a node put from it, linking a section of the doc the service fetched
    let a = cx.yaml(
        "A.yml",
        &format!("{}docs: [docs/design.md#old]\n", work("A", &[])),
    )?;
    cx.reef(&mine, &["node", "put", &a]).await?;
    // started on a branch there, and its committed work captured from there
    git::run(&mine, &["checkout", "-q", "-b", "work"])?;
    let started = cx.reef(&mine, &["start", "A"]).await?;
    assert!(
        started.starts_with(&format!("started `A` in {}\n", mine.display())),
        "{started}"
    );
    git::write(&mine, "src/lib.rs", "fn a() {}\n")?;
    commit_all(&mine, "A")?;
    let captured = cx.reef(&mine, &["node", "capture"]).await?;
    assert!(
        captured.starts_with(&format!(
            "captured `A` as revision 1 in {}\n",
            mine.display()
        )),
        "{captured}"
    );
    // kept as the node's revision, its file the one the user's clone changed
    let client = cx.client()?;
    let node = PlanName::new("up", "docs")?.node("A")?;
    let revisions = client.revisions(&node).await?;
    let paths: Vec<&str> = revisions[0].files.iter().map(|file| file.path()).collect();
    assert_eq!((revisions.len(), paths), (1, vec!["src/lib.rs"]));
    // a section pushed from the clone can't be linked until the project is fetched
    let revised = format!("{design}\n## New\n\nA section of its own.\n");
    git::write(&mine, "docs/design.md", &revised)?;
    commit_all(&mine, "a new section")?;
    git::push(&mine)?;
    let b = cx.yaml(
        "B.yml",
        &format!("{}docs: [docs/design.md#new]\n", work("B", &[])),
    )?;
    let (_, refused) = cx.failing(&mine, &["node", "put", &b]).await?;
    assert!(refused.contains("design.md"), "{refused}");
    let fetched = cx.reef(&mine, &["project", "fetch"]).await?;
    assert!(
        fetched.starts_with("`up`'s `main` moved from "),
        "{fetched}"
    );
    cx.reef(&mine, &["node", "put", &b]).await?;
    // fetched again, it is where it was, and `ls` shows the remote at its tip
    let again = cx.reef(&mine, &["project", "fetch"]).await?;
    assert!(again.starts_with("`up`'s `main` is still at "), "{again}");
    let tip = git::run(&mine, &["rev-parse", "--short=7", "HEAD"])?;
    let listed = cx.reef(&mine, &["project", "ls"]).await?;
    assert!(
        listed.contains(&format!("{url} (`main` at {tip})")),
        "{listed}"
    );
    // a local project has nothing to fetch
    let (_, local) = cx.failing(&cx.ax.repo, &["project", "fetch"]).await?;
    assert!(local.contains("nothing to fetch"), "{local}");
    // and a checkout of another repo can't start the remote project's nodes by naming it
    let (_, elsewhere) = cx
        .failing(&cx.ax.repo, &["start", "B", "--project", "up"])
        .await?;
    assert!(
        elsewhere.contains("is not in project `up`'s repo"),
        "{elsewhere}"
    );
    Ok(())
}

/// Two nodes started at once in two worktrees are recorded apart: each its own worktree,
/// branch and start, its own uncommitted work in it, and each worktree on its own node
#[tokio::test]
async fn two_worktrees_at_once_are_recorded_apart() -> anyhow::Result<()> {
    let cx = Cx::new("start-apart").await?;
    let repo = cli_project(&cx, &["A", "B"]).await?;
    // a file on main, then a worktree for each node, each with work left uncommitted
    let main = git::commit(&repo, "base.txt", "base\n", "base")?;
    let a = git::worktree(&repo, &cx.ax.dir.join("cli-a"))?;
    let b = git::worktree(&repo, &cx.ax.dir.join("cli-b"))?;
    std::fs::write(a.join("a.txt"), "A's, uncommitted\n")?;
    std::fs::write(b.join("base.txt"), "base, as B left it\n")?;
    // both started at once, each from its own worktree
    let (started_a, started_b) =
        tokio::join!(cx.reef(&a, &["start", "A"]), cx.reef(&b, &["start", "B"]));
    let (started_a, started_b) = (started_a?, started_b?);
    assert!(
        started_a.starts_with(&format!("started `A` in {}\n", a.display())),
        "{started_a}"
    );
    assert!(started_a.contains("`A` is in-progress"), "{started_a}");
    assert!(started_b.contains("`B` is in-progress"), "{started_b}");
    // each worktree is on its own node, on its own branch, both forked from main
    let on_a = cx.reef(&a, &["work"]).await?;
    let on_b = cx.reef(&b, &["work"]).await?;
    assert_eq!(field(&on_a, "node"), "cli/docs/A");
    assert_eq!(field(&on_b, "node"), "cli/docs/B");
    assert_eq!(field(&on_a, "worktree"), a.display().to_string());
    assert_eq!(field(&on_b, "worktree"), b.display().to_string());
    assert_eq!(field(&on_a, "branch"), "cli-a");
    assert_eq!(field(&on_b, "branch"), "cli-b");
    for on in [&on_a, &on_b] {
        assert_eq!(field(on, "base"), "main");
        assert_eq!(field(on, "start"), main);
        assert_eq!(field(on, "fork"), main);
    }
    // a record read by name, from any worktree, is the same one
    assert_eq!(cx.reef(&b, &["work", "A"]).await?, on_a);
    assert_eq!(cx.reef(&repo, &["work", "B"]).await?, on_b);
    // each start's tree holds its own worktree's uncommitted work, and none of the other's
    let (tree_a, tree_b) = (field(&on_a, "tree"), field(&on_b, "tree"));
    let files = |tree: &str| git::run(&repo, &["ls-tree", "-r", "--name-only", tree]);
    let base = |tree: &str| git::run(&repo, &["cat-file", "blob", &format!("{tree}:base.txt")]);
    assert_eq!(files(tree_a)?, "a.txt\nbase.txt");
    assert_eq!(files(tree_b)?, "base.txt");
    assert_eq!(base(tree_a)?, "base");
    assert_eq!(base(tree_b)?, "base, as B left it");
    // each pinned under its own ref, which every worktree of the repo shares
    for (id, tree) in [("A", tree_a), ("B", tree_b)] {
        let pin = format!("refs/reef/start/docs/{id}");
        assert_eq!(
            git::run(&repo, &["rev-parse", &format!("{pin}^{{tree}}")])?,
            tree
        );
        assert_eq!(git::run(&repo, &["rev-parse", &format!("{pin}^")])?, main);
    }
    // neither worktree's index was touched
    assert_eq!(git::run(&a, &["status", "--porcelain"])?, "?? a.txt");
    assert_eq!(git::run(&b, &["status", "--porcelain"])?, "M base.txt");
    // and both are in progress
    for id in ["A", "B"] {
        let got = cx.reef(&repo, &["node", "get", id]).await?;
        assert!(got.contains("status: in-progress"), "{got}");
    }
    Ok(())
}

/// A node's start still reads after its branch is rebased and git is pruned: its pin keeps the
/// start's commit and tree, which nothing else does
#[tokio::test]
async fn start_reads_after_a_rebase_and_a_prune() -> anyhow::Result<()> {
    let cx = Cx::new("start-pruned").await?;
    let repo = cli_project(&cx, &["A"]).await?;
    // an earlier node's commit on a branch off main, and work it left uncommitted
    git::run(&repo, &["checkout", "-q", "-b", "nodes"])?;
    let earlier = git::commit(&repo, "earlier.txt", "an earlier node\n", "earlier")?;
    std::fs::write(repo.join("left.txt"), "left uncommitted\n")?;
    // A started there, its start the earlier node's commit and what it left
    cx.reef(&repo, &["start", "A"]).await?;
    let record = cx.reef(&repo, &["work", "A"]).await?;
    assert_eq!(field(&record, "start"), earlier);
    let tree = field(&record, "tree").to_string();
    // then A's work committed with what was left, and main moving on
    std::fs::write(repo.join("a.txt"), "A's work\n")?;
    git::run(&repo, &["add", "-A"])?;
    git::run(&repo, &["commit", "-q", "-m", "A"])?;
    git::run(&repo, &["checkout", "-q", "main"])?;
    git::commit(&repo, "main.txt", "main moved on\n", "main")?;
    // the branch rebased onto main, which rewrites the earlier node's commit
    git::run(&repo, &["checkout", "-q", "nodes"])?;
    git::run(&repo, &["rebase", "-q", "main"])?;
    let rewritten = !git::succeeds(&repo, &["merge-base", "--is-ancestor", &earlier, "nodes"])?;
    assert!(rewritten, "the rebase left {earlier} on the branch");
    // nothing but the pin left naming the start: every reflog expired, and git pruned
    let prune = || -> anyhow::Result<()> {
        git::run(&repo, &["update-ref", "-d", "ORIG_HEAD"])?;
        git::run(&repo, &["reflog", "expire", "--expire=now", "--all"])?;
        git::run(&repo, &["gc", "-q", "--prune=now"])?;
        Ok(())
    };
    prune()?;
    // the start's commit and its tree are still there, and the record reads as it did
    assert!(git::succeeds(&repo, &["cat-file", "-e", &earlier])?);
    assert!(git::succeeds(&repo, &["cat-file", "-e", &tree])?);
    let files = git::run(&repo, &["ls-tree", "-r", "--name-only", &tree])?;
    assert_eq!(files, "earlier.txt\nleft.txt");
    assert_eq!(cx.reef(&repo, &["work", "A"]).await?, record);
    // without the pin, the same prune takes both
    git::run(&repo, &["update-ref", "-d", "refs/reef/start/docs/A"])?;
    git::run(&repo, &["checkout", "-q", "nodes"])?;
    prune().or_else(|_| {
        // ORIG_HEAD is already gone the second time
        git::run(&repo, &["reflog", "expire", "--expire=now", "--all"])?;
        git::run(&repo, &["gc", "-q", "--prune=now"]).map(|_| ())
    })?;
    assert!(!git::succeeds(&repo, &["cat-file", "-e", &tree])?);
    assert!(!git::succeeds(&repo, &["cat-file", "-e", &earlier])?);
    Ok(())
}

/// A start's base is `--base`, the repo's default, or the upstream of the base branch it is on,
/// and a start it can't record is refused, leaving the node's pin and record as they were
#[tokio::test]
async fn start_takes_its_base_and_refuses_what_it_cant_record() -> anyhow::Result<()> {
    let cx = Cx::new("start-base").await?;
    let repo = cli_project(&cx, &["A", "B", "C"]).await?;
    let group = cx.yaml("G.yml", GROUP)?;
    cx.reef(&repo, &["node", "put", &group]).await?;
    let main = git::run(&repo, &["rev-parse", "HEAD"])?;
    // on main with no upstream, nothing brings other work in
    let on_main = cx.reef(&repo, &["start", "A"]).await?;
    assert_eq!(field(&on_main, "branch"), "main");
    assert_eq!(field(&on_main, "base"), "none");
    assert_eq!(field(&on_main, "fork"), "none");
    let record = cx.reef(&repo, &["work", "A"]).await?;
    // on main tracking an upstream no fetch made, the upstream is the base
    git::run(&repo, &["remote", "add", "origin", "/nowhere/at/all"])?;
    git::run(&repo, &["update-ref", "refs/remotes/origin/main", &main])?;
    git::run(&repo, &["branch", "-q", "--set-upstream-to=origin/main"])?;
    let tracking = cx.reef(&repo, &["start", "B"]).await?;
    assert_eq!(field(&tracking, "base"), "origin/main");
    assert_eq!(field(&tracking, "fork"), main);
    // a base named is the base, on any branch
    git::run(&repo, &["checkout", "-q", "-b", "side"])?;
    let side = git::commit(&repo, "s.txt", "side\n", "side")?;
    git::run(&repo, &["branch", "-q", "other", &main])?;
    let named = cx.reef(&repo, &["start", "C", "--base", "other"]).await?;
    assert_eq!(field(&named, "base"), "other");
    assert_eq!(field(&named, "start"), side);
    assert_eq!(field(&named, "fork"), main);
    // refused, each saying why: started already, a base that names nothing, a group, a node
    // not in the plan, and a checkout of another project's repo
    let pin = git::run(&repo, &["rev-parse", "refs/reef/start/docs/A"])?;
    let (_, again) = cx.failing(&repo, &["start", "A"]).await?;
    assert!(again.contains("was started already"), "{again}");
    assert!(again.contains("--restart"), "{again}");
    let (_, base) = cx
        .failing(&repo, &["start", "A", "--restart", "--base", "nope"])
        .await?;
    assert!(base.contains("`--base nope`"), "{base}");
    let (_, grouped) = cx.failing(&repo, &["start", "G"]).await?;
    assert!(grouped.contains("is a group"), "{grouped}");
    let (_, missing) = cx.failing(&repo, &["start", "Z"]).await?;
    assert!(
        missing.contains("`Z` is not a node of `cli/docs`"),
        "{missing}"
    );
    let elsewhere = ["start", "A", "--restart", "--project", "cli"];
    let (_, other) = cx.failing(&cx.ax.repo, &elsewhere).await?;
    assert!(other.contains("is not in project `cli`'s repo"), "{other}");
    // none of which moved A's pin or its record
    assert_eq!(
        git::run(&repo, &["rev-parse", "refs/reef/start/docs/A"])?,
        pin
    );
    assert_eq!(cx.reef(&repo, &["work", "A"]).await?, record);
    // A started again from here replaces its record and its pin, and this worktree is on it
    let restarted = cx.reef(&repo, &["start", "A", "--restart"]).await?;
    assert_eq!(field(&restarted, "branch"), "side");
    assert_ne!(
        git::run(&repo, &["rev-parse", "refs/reef/start/docs/A"])?,
        pin
    );
    assert_eq!(
        field(&cx.reef(&repo, &["work"]).await?, "node"),
        "cli/docs/A"
    );
    // and a worktree where nothing started says so
    let fresh = git::worktree(&repo, &cx.ax.dir.join("cli-fresh"))?;
    let (_, none) = cx.failing(&fresh, &["work"]).await?;
    assert!(
        none.contains("no node of `cli` has been started in"),
        "{none}"
    );
    Ok(())
}

#[path = "../../../crates/reef/tests/support/oracle.rs"]
mod oracle;

/// A node's changes as the service keeps them: each revision, and each of its files fetched
///
/// # Arguments
///
/// * `cx` - The served twin
/// * `id` - The node's id in `cli/docs`
async fn changes_of(
    cx: &Cx,
    id: &str,
) -> anyhow::Result<Vec<(reef_api::Revision, Vec<reef_api::FileDiff>)>> {
    // a client of the case's service, as the CLI makes one
    let client = cx.client()?;
    let node = PlanName::new("cli", "docs")?.node(id)?;
    // every revision, then every file each lists
    let mut changes = Vec::new();
    for revision in client.revisions(&node).await? {
        let mut files = Vec::with_capacity(revision.files.len());
        for file in &revision.files {
            let read = client
                .revision_file(&node, revision.revision, file.path())
                .await?;
            files.push(read.expect("a file its revision lists reads"));
        }
        changes.push((revision, files));
    }
    Ok(changes)
}

/// Writes files into a worktree, making their directories
///
/// # Arguments
///
/// * `dir` - The worktree
/// * `files` - Each file's path and what it holds
fn write_files_in(dir: &Path, files: &[(&str, &[u8])]) -> anyhow::Result<()> {
    for (path, bytes) in files {
        // its directory first, as git would check it out
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().expect("a file is in a directory"))?;
        std::fs::write(path, bytes)?;
    }
    Ok(())
}

/// Commits everything in a worktree, returning the commit
///
/// # Arguments
///
/// * `dir` - The worktree
/// * `message` - The commit's message
fn commit_all(dir: &Path, message: &str) -> anyhow::Result<String> {
    // stage it all, then commit it
    git::run(dir, &["add", "-A"])?;
    git::run(dir, &["commit", "-q", "-m", message])?;
    git::run(dir, &["rev-parse", "HEAD"])
}

/// A file of numbered lines, each `line N`, for edits that leave most of it alone
///
/// # Arguments
///
/// * `count` - How many lines
fn numbered(count: usize) -> String {
    (1..=count).map(|n| format!("line {n}\n")).collect()
}

/// A captured diff matches `git diff` of its base and head when it is taken: committed and
/// uncommitted changes of every kind, each file as the service serves it, and a capture taken
/// again of the same work is unchanged
#[tokio::test]
async fn a_capture_matches_git_diff_when_taken() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let cx = Cx::new("capture-diff").await?;
    let repo = cli_project(&cx, &["A"]).await?;
    // a base on main holding every kind of file a change can touch
    write_files_in(
        &repo,
        &[
            ("edited.txt", numbered(40).as_bytes()),
            ("gone.txt", b"to be deleted\n"),
            ("renamed.txt", numbered(20).as_bytes()),
            ("run.sh", b"#!/bin/sh\necho hi\n"),
            ("blob.bin", b"\x00\x01 binary\n"),
            ("crlf.txt", b"one\r\ntwo\r\n"),
            ("no newline.txt", b"first\nlast"),
        ],
    )?;
    commit_all(&repo, "base")?;
    // A started on a branch off it
    git::run(&repo, &["checkout", "-q", "-b", "work"])?;
    cx.reef(&repo, &["start", "A"]).await?;
    // some of A's work committed: an edit, a delete, a rename and a mode change
    let edited = numbered(40).replace("line 3\n", "line three\n");
    write_files_in(&repo, &[("edited.txt", edited.as_bytes())])?;
    std::fs::remove_file(repo.join("gone.txt"))?;
    std::fs::rename(repo.join("renamed.txt"), repo.join("now named.txt"))?;
    let mut mode = std::fs::metadata(repo.join("run.sh"))?.permissions();
    mode.set_mode(0o755);
    std::fs::set_permissions(repo.join("run.sh"), mode)?;
    commit_all(&repo, "A, part one")?;
    // and some left uncommitted: binary, CRLF and no-final-newline edits, and a staged new file
    write_files_in(
        &repo,
        &[
            ("blob.bin", b"\x00\x02 binary, changed\n"),
            ("crlf.txt", b"one\r\n2\r\n"),
            ("no newline.txt", b"first\nlast, edited"),
            ("src/new.rs", b"fn main() {}\n"),
        ],
    )?;
    git::run(&repo, &["add", "src/new.rs"])?;
    // captured from the worktree, as revision 1
    let out = cx.reef(&repo, &["node", "capture"]).await?;
    assert!(
        out.starts_with(&format!(
            "captured `A` as revision 1 in {}\n",
            repo.display()
        )),
        "{out}"
    );
    assert!(out.contains(", the start\n"), "{out}");
    // what the service keeps is git's diff of the capture's base and tree, file for file
    let changes = changes_of(&cx, "A").await?;
    let [(revision, files)] = &changes[..] else {
        panic!("one capture kept {} revisions", changes.len());
    };
    assert_eq!(field(&out, "tree"), revision.shas.tree);
    assert_eq!(
        field(&out, "head"),
        git::run(&repo, &["rev-parse", "HEAD"])?
    );
    // the outline is reef's, not git's: only the Rust file has one, and the rest is git's
    let outlined: Vec<&str> = files
        .iter()
        .filter(|file| file.outline.is_some())
        .map(|file| file.path())
        .collect();
    assert_eq!(outlined, ["src/new.rs"]);
    let new_rs = files.iter().find(|file| file.path() == "src/new.rs");
    let items = &new_rs.and_then(|file| file.outline.as_ref()).unwrap().items;
    assert_eq!(items.len(), 1);
    assert_eq!(
        (items[0].label(), items[0].change),
        ("fn main".to_string(), reef_api::ItemChange::Added)
    );
    let unoutlined: Vec<reef_api::FileDiff> = files
        .iter()
        .map(|file| reef_api::FileDiff {
            outline: None,
            ..file.clone()
        })
        .collect();
    let expected = oracle::git_diff(&repo, &revision.shas.base, &revision.shas.tree);
    assert_eq!(unoutlined, expected);
    let paths: Vec<&str> = files.iter().map(|file| file.path()).collect();
    assert_eq!(
        paths,
        [
            "blob.bin",
            "crlf.txt",
            "edited.txt",
            "gone.txt",
            "no newline.txt",
            "now named.txt",
            "run.sh",
            "src/new.rs"
        ]
    );
    // and the summary it lists is each file's own
    let summaries: Vec<reef_api::FileSummary> = files.iter().map(|file| file.summary()).collect();
    assert_eq!(revision.files, summaries);
    // the same work captured again, by id too, is unchanged and adds no revision
    let again = cx.reef(&repo, &["node", "capture", "A"]).await?;
    assert!(
        again.starts_with("`A` is unchanged since revision 1\n"),
        "{again}"
    );
    assert_eq!(changes_of(&cx, "A").await?.len(), 1);
    let record = cx.reef(&repo, &["work", "A"]).await?;
    assert_eq!(field(&record, "captures"), "2");
    // and capturing left the user's own staging as it was
    let status = git::run(&repo, &["status", "--porcelain"])?;
    let lines: Vec<&str> = status.lines().map(str::trim).collect();
    assert!(lines.contains(&"A  src/new.rs"), "{status}");
    assert!(lines.contains(&"M blob.bin"), "{status}");
    Ok(())
}

/// A capture outlines each Rust file from both of its sides, and the service keeps the outline
/// with the file and in its revision's listing: a struct's field and its impl's methods under
/// the struct, a fn removed and a struct added alone, and a file with no grammar has none
#[tokio::test]
async fn a_capture_outlines_its_rust_files() -> anyhow::Result<()> {
    use reef_api::ItemChange::{Added, Changed, Removed};
    let cx = Cx::new("capture-outline").await?;
    let repo = cli_project(&cx, &["A"]).await?;
    // a base with a struct, its impl and a loose fn, and a file no grammar reads
    let base = "\
pub struct Scaler {
    count: u32,
}

impl Scaler {
    pub fn run(&self) -> u32 {
        self.count
    }
}

fn gone() {}
";
    write_files_in(
        &repo,
        &[
            ("src/scaler.rs", base.as_bytes()),
            ("notes.md", b"# Notes\n"),
        ],
    )?;
    commit_all(&repo, "base")?;
    git::run(&repo, &["checkout", "-q", "-b", "work"])?;
    cx.reef(&repo, &["start", "A"]).await?;
    // a field added, a method changed and one added, the fn removed and a struct added
    let edited = "\
pub struct Scaler {
    count: u32,
    rate: u32,
}

impl Scaler {
    pub fn run(&self) -> u32 {
        self.count * self.rate
    }

    pub fn stop(&self) {}
}

struct Fresh;
";
    write_files_in(
        &repo,
        &[
            ("src/scaler.rs", edited.as_bytes()),
            ("notes.md", b"# Notes\n\nMore.\n"),
        ],
    )?;
    cx.reef(&repo, &["node", "capture"]).await?;
    // the file as served holds its outline, and its revision's listing the same
    let changes = changes_of(&cx, "A").await?;
    let (revision, files) = &changes[0];
    let scaler = files
        .iter()
        .find(|file| file.path() == "src/scaler.rs")
        .unwrap();
    let outline = scaler.outline.as_ref().expect("the Rust file is outlined");
    assert_eq!(outline.language, "rust");
    let items: Vec<(String, Option<&str>, reef_api::ItemChange, u32, u32)> = outline
        .items
        .iter()
        .map(|item| {
            (
                item.label(),
                item.owner.as_ref().map(|owner| owner.name.as_str()),
                item.change,
                item.added,
                item.removed,
            )
        })
        .collect();
    assert_eq!(
        items,
        [
            ("struct Scaler".to_string(), None, Changed, 1, 0),
            ("fn run".to_string(), Some("Scaler"), Changed, 1, 1),
            ("fn stop".to_string(), Some("Scaler"), Added, 2, 0),
            ("fn gone".to_string(), None, Removed, 0, 1),
            ("struct Fresh".to_string(), None, Added, 1, 0),
        ]
    );
    let listed = revision
        .files
        .iter()
        .find(|file| file.path() == "src/scaler.rs")
        .unwrap();
    assert_eq!(listed.outline.as_ref(), Some(outline));
    // and the markdown has none
    let notes = files.iter().find(|file| file.path() == "notes.md").unwrap();
    assert!(notes.outline.is_none());
    Ok(())
}

/// A seal leaves out untracked files no commit or add took, and keeps those its base has: a
/// `__pycache__` the node's run left stays out, a staged file goes in, and what the start left
/// untracked is the start's, not shown as deleted
#[tokio::test]
async fn a_seal_leaves_out_untracked_files() -> anyhow::Result<()> {
    let cx = Cx::new("capture-untracked").await?;
    let repo = cli_project(&cx, &["A"]).await?;
    git::commit(&repo, "a.py", "print(1)\n", "base")?;
    // an earlier node's work left untracked, which A's start takes along
    git::run(&repo, &["checkout", "-q", "-b", "work"])?;
    write_files_in(&repo, &[("left.txt", b"left by an earlier node\n")])?;
    cx.reef(&repo, &["start", "A"]).await?;
    // A edits what is tracked and what was left, stages one new file and runs its code
    write_files_in(
        &repo,
        &[
            ("a.py", b"print(2)\n"),
            ("left.txt", b"left, then edited\n"),
            ("b.py", b"print(3)\n"),
            ("__pycache__/a.cpython-312.pyc", b"\x00bytecode"),
            ("notes.txt", b"never added\n"),
        ],
    )?;
    git::run(&repo, &["add", "b.py"])?;
    cx.reef(&repo, &["node", "capture"]).await?;
    // the revision holds the tracked, the staged and the start's own, and no other
    let changes = changes_of(&cx, "A").await?;
    let (revision, files) = &changes[0];
    let paths: Vec<(&str, reef_api::FileStatus)> = files
        .iter()
        .map(|file| (file.path(), file.status))
        .collect();
    use reef_api::FileStatus::{Added, Modified};
    assert_eq!(
        paths,
        [("a.py", Modified), ("b.py", Added), ("left.txt", Modified)]
    );
    // and it is still git's diff of its base and tree
    let expected = oracle::git_diff(&repo, &revision.shas.base, &revision.shas.tree);
    assert_eq!(files, &expected);
    Ok(())
}

/// A node's changes still read after its commits are squashed and its branch deleted: its
/// worktree, branch and pin gone, every reflog expired and git pruned, the heads it read no
/// longer resolve, and every revision and file reads back as it was
#[tokio::test]
async fn changes_read_after_a_squash_and_a_deleted_branch() -> anyhow::Result<()> {
    let cx = Cx::new("capture-squash").await?;
    let repo = cli_project(&cx, &["A"]).await?;
    git::commit(&repo, "base.txt", "base\n", "base")?;
    // A worked in a worktree of its own, two commits of it, then captured
    let a = git::worktree(&repo, &cx.ax.dir.join("cli-a"))?;
    cx.reef(&a, &["start", "A"]).await?;
    let first = git::commit(&a, "a.txt", "A's first\n", "A one")?;
    let second = git::commit(&a, "a.txt", "A's first\nA's second\n", "A two")?;
    cx.reef(&a, &["node", "capture"]).await?;
    let before = changes_of(&cx, "A").await?;
    assert_eq!(before[0].0.shas.head, second);
    // A squash-merged into main, then its worktree, branch and pin deleted
    git::run(&repo, &["merge", "--squash", "-q", "cli-a"])?;
    git::run(&repo, &["commit", "-q", "-m", "A, squashed"])?;
    git::run(
        &repo,
        &["worktree", "remove", "--force", a.to_str().expect("utf-8")],
    )?;
    git::run(&repo, &["branch", "-q", "-D", "cli-a"])?;
    git::run(&repo, &["update-ref", "-d", "refs/reef/start/docs/A"])?;
    // every reflog expired and git pruned, so the commits A's capture read are gone, and only
    // its tree is left, as the squash's own
    git::run(&repo, &["reflog", "expire", "--expire=now", "--all"])?;
    git::run(&repo, &["gc", "-q", "--prune=now"])?;
    for gone in [&first, &second] {
        assert!(
            !git::succeeds(&repo, &["cat-file", "-e", gone])?,
            "{gone} survived the prune"
        );
    }
    let squashed = git::run(&repo, &["rev-parse", "main^{tree}"])?;
    assert_eq!(before[0].0.shas.tree, squashed);
    // and every revision and file reads back byte for byte
    let after = changes_of(&cx, "A").await?;
    assert_eq!(
        serde_json::to_string(&after)?,
        serde_json::to_string(&before)?
    );
    Ok(())
}

/// A node's diff holds only its own changes after its branch is rebased onto a newer base: in a
/// row on one branch, its start is carried over what main brought in, and a rebase conflict in
/// an earlier node's file is named on the revision
#[tokio::test]
async fn a_rebased_node_holds_only_its_own_changes() -> anyhow::Result<()> {
    let cx = Cx::new("capture-rebase").await?;
    let repo = cli_project(&cx, &["A", "B"]).await?;
    // rebases make commits, so the repo names who makes them
    git::run(&repo, &["config", "user.name", "reef"])?;
    git::run(&repo, &["config", "user.email", "reef@example.invalid"])?;
    git::commit(&repo, "shared.txt", "one\ntwo\nthree\n", "base")?;
    // an earlier node's commit on a branch of nodes in a row, then A's start and work
    git::run(&repo, &["checkout", "-q", "-b", "nodes"])?;
    git::commit(&repo, "earlier.txt", "an earlier node\n", "earlier")?;
    cx.reef(&repo, &["start", "A"]).await?;
    git::commit(&repo, "a.txt", "A's work\n", "A")?;
    // main moves on with another node's change, and the branch is rebased onto it
    git::run(&repo, &["checkout", "-q", "main"])?;
    git::commit(&repo, "main.txt", "main moved on\n", "main")?;
    git::run(&repo, &["checkout", "-q", "nodes"])?;
    git::run(&repo, &["rebase", "-q", "main"])?;
    // A's capture holds A's file alone, its start carried over what main brought in
    let out = cx.reef(&repo, &["node", "capture", "A"]).await?;
    assert!(out.contains(", the start carried forward\n"), "{out}");
    assert!(!out.contains("conflicts"), "{out}");
    let changes = changes_of(&cx, "A").await?;
    let (revision, files) = &changes[0];
    let paths: Vec<&str> = files.iter().map(|file| file.path()).collect();
    assert_eq!(paths, ["a.txt"]);
    assert_eq!(
        files,
        &oracle::git_diff(&repo, &revision.shas.base, &revision.shas.tree)
    );
    assert_eq!(
        revision.shas.merge_base.as_deref(),
        Some(git::run(&repo, &["rev-parse", "main"])?.as_str())
    );
    // B starts where A left off, after an earlier node's edit of the shared file
    git::commit(
        &repo,
        "shared.txt",
        "one\nours\nthree\n",
        "A's last, in the shared file",
    )?;
    cx.reef(&repo, &["start", "B"]).await?;
    git::commit(&repo, "b.txt", "B's work\n", "B")?;
    // main changes the same line, and the rebase's conflict is resolved by hand
    git::run(&repo, &["checkout", "-q", "main"])?;
    git::commit(&repo, "shared.txt", "one\ntheirs\nthree\n", "main's edit")?;
    git::run(&repo, &["checkout", "-q", "nodes"])?;
    assert!(!git::succeeds(&repo, &["rebase", "-q", "main"])?);
    std::fs::write(repo.join("shared.txt"), "one\nresolved\nthree\n")?;
    git::run(&repo, &["add", "shared.txt"])?;
    git::run(&repo, &["-c", "core.editor=true", "rebase", "--continue"])?;
    // B's revision names the conflicted file, whose resolution its diff holds beside B's file
    let out = cx.reef(&repo, &["node", "capture", "B"]).await?;
    assert_eq!(field(&out, "conflicts"), "shared.txt");
    let changes = changes_of(&cx, "B").await?;
    let (revision, files) = &changes[0];
    assert_eq!(revision.conflicts, ["shared.txt"]);
    let paths: Vec<&str> = files.iter().map(|file| file.path()).collect();
    assert_eq!(paths, ["b.txt", "shared.txt"]);
    Ok(())
}

/// A capture is refused, saying why, where nothing was started, for a node worked elsewhere or
/// never started, and once its start's tree is pruned
#[tokio::test]
async fn capture_refuses_what_it_cant_take() -> anyhow::Result<()> {
    let cx = Cx::new("capture-refused").await?;
    let repo = cli_project(&cx, &["A", "B"]).await?;
    // nothing started in the repo yet
    let (_, none) = cx.failing(&repo, &["node", "capture"]).await?;
    assert!(
        none.contains("no node of `cli` has been started in"),
        "{none}"
    );
    // A started in a worktree of its own is captured there, not here
    let a = git::worktree(&repo, &cx.ax.dir.join("cli-a"))?;
    std::fs::write(a.join("wip.txt"), "A's, uncommitted\n")?;
    cx.reef(&a, &["start", "A"]).await?;
    let (_, elsewhere) = cx.failing(&repo, &["node", "capture", "A"]).await?;
    assert!(
        elsewhere.contains(&format!("`A` is worked in {}, not here", a.display())),
        "{elsewhere}"
    );
    // B was never started
    let (_, never) = cx.failing(&a, &["node", "capture", "B"]).await?;
    assert!(never.contains("`B` has not been started"), "{never}");
    // A's pin deleted and git pruned takes its start's tree, which is said
    git::run(&repo, &["update-ref", "-d", "refs/reef/start/docs/A"])?;
    git::run(&repo, &["reflog", "expire", "--expire=now", "--all"])?;
    git::run(&repo, &["gc", "-q", "--prune=now"])?;
    let (_, pruned) = cx.failing(&a, &["node", "capture"]).await?;
    assert!(pruned.contains("is gone from"), "{pruned}");
    assert!(pruned.contains("--restart"), "{pruned}");
    assert!(changes_of(&cx, "A").await?.is_empty());
    Ok(())
}

/// Starts a general thread on a node of `cli/docs` through the JSON API, as the app does, and
/// hands back its number
///
/// # Arguments
///
/// * `cx` - The served twin
/// * `id` - The node's id
/// * `body` - The thread's first comment
async fn comment_on(cx: &Cx, id: &str, body: &str) -> anyhow::Result<u32> {
    // a thread on the node as a whole
    let node = PlanName::new("cli", "docs")?.node(id)?;
    let new = reef_api::NewThread {
        target: reef_api::Target::Node,
        body: body.to_string(),
        by: reef_api::Writer::User,
        review: false,
    };
    let path = reef_api::paths::node_comments(&node);
    let (_, commented): (_, reef_api::Commented) = cx.ax.client.post(&path, &new).await?;
    match commented {
        reef_api::Commented::Added { thread, .. } => Ok(thread.number),
        other => panic!("a thread on `{node}` wasn't kept: {other:?}"),
    }
}

/// Reads a thread of `cli/docs` as the CLI's client does, which must be there
///
/// # Arguments
///
/// * `cx` - The served twin
/// * `number` - The thread's number
async fn thread_of(cx: &Cx, number: u32) -> anyhow::Result<reef_api::Thread> {
    // a client of the case's service, as the CLI makes one
    let client = cx.client()?;
    let plan = PlanName::new("cli", "docs")?;
    Ok(client
        .thread(&plan, number)
        .await?
        .expect("the thread is there"))
}

/// Commits what is staged and a file, with a message as written, returning the commit
///
/// # Arguments
///
/// * `dir` - The worktree
/// * `path` - The file's path in it
/// * `text` - What it holds
/// * `message` - The whole message, trailers and all
fn commit_with(dir: &Path, path: &str, text: &str, message: &str) -> anyhow::Result<String> {
    // write it, stage it, commit it with the message as it is
    std::fs::write(dir.join(path), text)?;
    git::run(dir, &["add", path])?;
    git::run(dir, &["commit", "-q", "-m", message])?;
    git::run(dir, &["rev-parse", "HEAD"])
}

/// A commit's `Reef-Resolves` trailer resolves its comment once the commit is captured, the
/// revision it was captured in linked to it, and `reef comment resolve` links one by hand; both
/// links read the same once the node's commits are squashed, its branch deleted and git pruned
#[tokio::test]
async fn a_trailer_resolves_its_comment_and_the_link_survives_a_squash() -> anyhow::Result<()> {
    let cx = Cx::new("resolve-squash").await?;
    let repo = cli_project(&cx, &["A"]).await?;
    git::commit(&repo, "base.txt", "base\n", "base")?;
    // A worked in a worktree of its own, captured once, then commented on twice
    let a = git::worktree(&repo, &cx.ax.dir.join("cli-a"))?;
    cx.reef(&a, &["start", "A"]).await?;
    git::commit(&a, "a.txt", "A's first\n", "A one")?;
    cx.reef(&a, &["node", "capture"]).await?;
    let lock = comment_on(&cx, "A", "Take the lock first").await?;
    let docs = comment_on(&cx, "A", "And the docs?").await?;
    assert_eq!((lock, docs), (1, 2));
    // the fix committed naming the first thread, and captured as revision 2
    let fix = commit_with(
        &a,
        "a.txt",
        "A's first, locked\n",
        "Take the lock\n\nIt raced the reader.\n\nReef-Resolves: 1",
    )?;
    let out = cx.reef(&a, &["node", "capture"]).await?;
    assert!(out.starts_with("captured `A` as revision 2 in "), "{out}");
    assert!(
        out.contains(&format!(
            "resolved #1, answered by {} Take the lock\n",
            &fix[..12]
        )),
        "{out}"
    );
    // the thread is resolved, revision 2 linked with the commit that answered it
    let resolved = thread_of(&cx, lock).await?;
    assert_eq!(resolved.state, reef_api::ThreadState::Resolved);
    let linked = resolved.resolved_by().expect("a revision was linked");
    assert_eq!(
        (linked.node.to_string(), linked.revision),
        ("cli/docs/A".to_string(), 2)
    );
    let commit = linked
        .commit
        .as_ref()
        .expect("the trailer's commit was kept");
    assert_eq!(
        (commit.commit.as_str(), commit.summary.as_str()),
        (fix.as_str(), "Take the lock")
    );
    // a capture with nothing new reads the trailer again, and says and writes nothing of it
    let again = cx.reef(&a, &["node", "capture"]).await?;
    assert!(
        again.starts_with("`A` is unchanged since revision 2\n"),
        "{again}"
    );
    assert!(!again.contains("#1"), "{again}");
    assert_eq!(thread_of(&cx, lock).await?, resolved);
    // the second resolved by hand from the worktree, linking A's latest revision
    let out = cx.reef(&a, &["comment", "resolve", "2"]).await?;
    assert_eq!(
        out,
        "resolved #2 of `cli/docs`, answered by revision 2 of `cli/docs/A`\n"
    );
    let by_hand = thread_of(&cx, docs).await?;
    assert_eq!(by_hand.state, reef_api::ThreadState::Resolved);
    assert_eq!(by_hand.resolved_by().map(|linked| linked.revision), Some(2));
    assert!(
        by_hand
            .resolved_by()
            .is_some_and(|linked| linked.commit.is_none())
    );
    let before = changes_of(&cx, "A").await?;
    // A squash-merged into main, then its worktree, branch and pin deleted
    git::run(&repo, &["merge", "--squash", "-q", "cli-a"])?;
    git::run(&repo, &["commit", "-q", "-m", "A, squashed"])?;
    git::run(
        &repo,
        &["worktree", "remove", "--force", a.to_str().expect("utf-8")],
    )?;
    git::run(&repo, &["branch", "-q", "-D", "cli-a"])?;
    git::run(&repo, &["update-ref", "-d", "refs/reef/start/docs/A"])?;
    git::run(&repo, &["reflog", "expire", "--expire=now", "--all"])?;
    git::run(&repo, &["gc", "-q", "--prune=now"])?;
    // the commit that answered the thread is gone from git
    assert!(
        !git::succeeds(&repo, &["cat-file", "-e", &fix])?,
        "{fix} survived the prune"
    );
    // and both threads still name the revision that answered them, which reads as it did
    assert_eq!(thread_of(&cx, lock).await?, resolved);
    assert_eq!(thread_of(&cx, docs).await?, by_hand);
    let after = changes_of(&cx, "A").await?;
    assert_eq!(
        serde_json::to_string(&after)?,
        serde_json::to_string(&before)?
    );
    assert_eq!(after[1].0.revision, 2);
    Ok(())
}

/// A trailer that names no thread, or no number, is said and resolves nothing; one whose change
/// resolved its thread before it was reopened leaves it open; and `reef comment resolve` refuses
/// a thread, node or revision that isn't there, saying what to do
#[tokio::test]
async fn trailers_and_resolve_say_what_they_cant_resolve() -> anyhow::Result<()> {
    let cx = Cx::new("resolve-refused").await?;
    let repo = cli_project(&cx, &["A"]).await?;
    git::commit(&repo, "base.txt", "base\n", "base")?;
    // A worked on a branch of the main worktree, commented on before anything was captured
    git::run(&repo, &["checkout", "-q", "-b", "nodes"])?;
    cx.reef(&repo, &["start", "A"]).await?;
    let lock = comment_on(&cx, "A", "Take the lock first").await?;
    // a commit naming a thread that isn't there, and no number at all
    let stray = commit_with(
        &repo,
        "a.txt",
        "A\n",
        "Stray\n\nReef-Resolves: 9\nReef-Resolves: the lock",
    )?;
    let out = cx.reef(&repo, &["node", "capture"]).await?;
    let short = &stray[..12];
    assert!(
        out.contains(&format!(
            "Reef-Resolves: 9 in {short} names no thread of `cli/docs`\n"
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "Reef-Resolves: the lock in {short} names no thread: give it"
        )),
        "{out}"
    );
    assert!(thread_of(&cx, lock).await?.is_open());
    // the thread resolved by a commit, then reopened, as the user does in the app
    let fix = commit_with(&repo, "a.txt", "A, locked\n", "Lock\n\nReef-Resolves: #1")?;
    let out = cx.reef(&repo, &["node", "capture"]).await?;
    assert!(out.contains("resolved #1, answered by"), "{out}");
    let resolved = thread_of(&cx, lock).await?;
    let mark = reef_api::MarkThread {
        state: reef_api::ThreadState::Open,
        read: resolved.version,
    };
    let path = reef_api::paths::thread_state(&PlanName::new("cli", "docs")?, lock);
    let (_, _): (_, reef_api::Marked) = cx.ax.client.put(&path, &mark).await?;
    // the commit amended, its message kept, is the same change, and the thread stays open
    std::fs::write(repo.join("a.txt"), "A, locked, amended\n")?;
    git::run(&repo, &["commit", "-q", "-a", "--amend", "--no-edit"])?;
    let amended = git::run(&repo, &["rev-parse", "HEAD"])?;
    assert_ne!(amended, fix);
    let out = cx.reef(&repo, &["node", "capture"]).await?;
    assert!(
        out.contains(&format!(
            "left #1 open: {} resolved it before it was reopened",
            &amended[..12]
        )),
        "{out}"
    );
    assert!(thread_of(&cx, lock).await?.is_open());
    // a new commit naming it resolves it again
    commit_with(
        &repo,
        "a.txt",
        "A, locked twice\n",
        "Lock again\n\nReef-Resolves: 1",
    )?;
    let out = cx.reef(&repo, &["node", "capture"]).await?;
    assert!(out.contains("resolved #1, answered by"), "{out}");
    assert_eq!(thread_of(&cx, lock).await?.resolutions.len(), 2);
    // a thread that isn't there, a revision the node lacks, a node the plan lacks
    let (_, none) = cx.failing(&repo, &["comment", "resolve", "9"]).await?;
    assert!(
        none.contains("`cli/docs` has no comment thread 9"),
        "{none}"
    );
    let (_, revision) = cx
        .failing(&repo, &["comment", "resolve", "1", "--revision", "9"])
        .await?;
    assert!(
        revision.contains("`cli/docs/A` has no revision 9"),
        "{revision}"
    );
    let (_, uncaptured) = cx
        .failing(&repo, &["comment", "resolve", "1", "--node", "B"])
        .await?;
    assert!(
        uncaptured.contains("`cli/docs/B` has no captured revision to link"),
        "{uncaptured}"
    );
    // and a worktree no node was started in names none
    let other = git::worktree(&repo, &cx.ax.dir.join("cli-other"))?;
    let (_, unstarted) = cx.failing(&other, &["comment", "resolve", "1"]).await?;
    assert!(
        unstarted.contains("no node of `cli` has been started in")
            && unstarted.contains("with `--node`"),
        "{unstarted}"
    );
    Ok(())
}

/// Has a pane's loop take a node through the JSON API, as next_task will, and hands back what
/// came of it
///
/// # Arguments
///
/// * `cx` - The served twin
/// * `pane` - The pane's key
/// * `id` - The node's id in `cli/docs`
async fn hold_in(cx: &Cx, pane: &str, id: &str) -> anyhow::Result<reef_api::LoopEdited> {
    // a hold of the node, sent as the CLI's client sends one
    let edit = reef_api::EditLoop {
        pane: pane.parse()?,
        edit: reef_api::LoopEdit::Hold {
            node: Some(PlanName::new("cli", "docs")?.node(id)?),
            members: Vec::new(),
        },
    };
    let (_, edited): (_, reef_api::LoopEdited) =
        cx.ax.client.post(reef_api::paths::LOOPS, &edit).await?;
    Ok(edited)
}

/// A pane's loop is started, resumed, listed, handed to a clear's new session by the
/// SessionStart hook with its node kept, paused and stopped; a second pane can't take its node,
/// and every command says what it can't do
#[tokio::test]
async fn a_loop_follows_its_pane_through_a_clear() -> anyhow::Result<()> {
    let cx = Cx::new("loop-pane").await?;
    let repo = cli_project(&cx, &["A", "B"]).await?;
    // no loop yet
    let none = cx.reef(&repo, &["loop", "ls"]).await?;
    assert_eq!(
        none,
        "no loop feeds `cli`: `reef loop start` starts one in this pane\n"
    );
    // started in a pane, fed by the session that ran it, then started again, running already
    let pane = "claude:4242";
    let start = ["loop", "start", "--pane", pane, "--session", "s1"];
    let out = cx.reef(&repo, &start).await?;
    assert_eq!(
        out,
        "started a loop in Claude Code process 4242, feeding it every plan of `cli`\n"
    );
    let again = cx.reef(&repo, &["loop", "start", "--pane", pane]).await?;
    assert_eq!(
        again,
        "the loop in Claude Code process 4242 is running, feeding it every plan of `cli`\n"
    );
    // a start of one plan where it feeds every plan is busy
    let (_, busy) = cx
        .failing(&repo, &["loop", "start", "--pane", pane, "--plan", "docs"])
        .await?;
    assert!(
        busy.contains("has a loop already, feeding it every plan of `cli`"),
        "{busy}"
    );
    // it takes A, as next_task will, and lists it
    let held = hold_in(&cx, pane, "A").await?;
    assert!(
        matches!(held, reef_api::LoopEdited::Edited { .. }),
        "{held:?}"
    );
    let listed = cx.reef(&repo, &["loop", "ls"]).await?;
    assert_eq!(
        listed,
        "claude:4242  running, feeding every plan of `cli`, on `cli/docs/A`, session s1, started \
         by tester\n"
    );
    // a clear starts a new session in the pane, and the hook hands it the loop, printing nothing
    let client = cx.client()?;
    let cleared = r#"{"session_id":"s2","source":"clear","hook_event_name":"SessionStart"}"#;
    let mut printed = Vec::new();
    reef::loops::session_start(
        &client,
        cleared,
        &repo,
        &pane.parse()?,
        None,
        None,
        &mut printed,
    )
    .await?;
    assert!(printed.is_empty());
    let listed = cx.reef(&repo, &["loop", "ls"]).await?;
    assert_eq!(
        listed,
        "claude:4242  running, feeding every plan of `cli`, on `cli/docs/A`, session s2, started \
         by tester\n"
    );
    // a second pane's loop can't take A, and takes B
    let other = "claude:5151";
    cx.reef(&repo, &["loop", "start", "--pane", other, "--plan", "docs"])
        .await?;
    let refused = hold_in(&cx, other, "A").await?;
    assert!(
        matches!(&refused, reef_api::LoopEdited::Held { by, .. } if by.pane.to_string() == pane),
        "{refused:?}"
    );
    assert!(matches!(
        hold_in(&cx, other, "B").await?,
        reef_api::LoopEdited::Edited { .. }
    ));
    // paused, the first stops once A is done, and starting it again runs it
    let paused = cx.reef(&repo, &["loop", "pause", "--pane", pane]).await?;
    assert_eq!(
        paused,
        "paused the loop in Claude Code process 4242: it stops once `cli/docs/A` is done\n"
    );
    let listed = cx.reef(&repo, &["loop", "ls"]).await?;
    assert!(
        listed.contains("claude:4242  paused, feeding every plan of `cli`, on `cli/docs/A`"),
        "{listed}"
    );
    assert!(
        listed.contains("claude:5151  running, feeding `cli/docs`, on `cli/docs/B`"),
        "{listed}"
    );
    // stopped, it lets go of A, which the second can then take
    let stopped = cx.reef(&repo, &["loop", "stop", "--pane", pane]).await?;
    assert_eq!(
        stopped,
        "stopped the loop in Claude Code process 4242, which fed it every plan of `cli`, \
         letting go of `cli/docs/A`\n"
    );
    assert!(matches!(
        hold_in(&cx, other, "A").await?,
        reef_api::LoopEdited::Edited { .. }
    ));
    // a pane with no loop says so, and the hook in one leaves it be
    let (_, gone) = cx
        .failing(&repo, &["loop", "pause", "--pane", pane])
        .await?;
    assert!(
        gone.contains("no loop runs in Claude Code process 4242: `reef loop start` starts one"),
        "{gone}"
    );
    let mut printed = Vec::new();
    reef::loops::session_start(
        &client,
        cleared,
        &repo,
        &pane.parse()?,
        None,
        None,
        &mut printed,
    )
    .await?;
    assert!(printed.is_empty());
    // and a pane's key that names none says what one is
    let (_, bad) = cx
        .failing(&repo, &["loop", "stop", "--pane", "screen:1"])
        .await?;
    assert!(bad.contains("`screen:1` is no pane"), "{bad}");
    Ok(())
}

/// A checkout on each branch of a project with two plans works in the plan that follows its
/// branch, with no `--plan`: its node commands, its check and its next
#[tokio::test]
async fn a_checkout_on_each_branch_works_in_its_plan() -> anyhow::Result<()> {
    let cx = Cx::new("checkout-plans").await?;
    let repo = cx.ax.repo.clone();
    // `central`, which reads the working tree, holds A
    cx.put("a.yml", &work("A", &[])).await?;
    // `side` follows `b2`, which a linked worktree has checked out, and holds B
    git::run(&repo, &["branch", "b2"])?;
    let linked = cx.ax.dir.join("b2-checkout");
    let linked_path = linked.to_str().expect("a utf-8 path");
    git::run(&repo, &["worktree", "add", "-q", linked_path, "b2"])?;
    let said = cx
        .reef(&repo, &["plan", "new", "side", "--branch", "b2"])
        .await?;
    assert!(said.contains("`reef/side` follows `b2`"), "{said}");
    let b = cx.yaml("b.yml", &work("B", &[]))?;
    cx.reef(&linked, &["node", "put", &b]).await?;
    // each checkout gets its own plan's node, and not the other's
    let got = cx.reef(&repo, &["node", "get", "A"]).await?;
    assert!(got.contains("id: A"), "{got}");
    let got = cx.reef(&linked, &["node", "get", "B"]).await?;
    assert!(got.contains("id: B"), "{got}");
    let (_, error) = cx.failing(&linked, &["node", "get", "A"]).await?;
    assert!(error.contains("reef/side/A"), "{error}");
    // each is checked alone, and next looks in its own plan
    let checked = cx.reef(&linked, &["check"]).await?;
    assert!(checked.contains("`reef/side` has no problems"), "{checked}");
    let next = cx.reef(&repo, &["next"]).await?;
    assert!(next.contains("A ") && !next.contains("B "), "{next}");
    let next = cx.reef(&linked, &["next"]).await?;
    assert!(next.contains("B ") && !next.contains("A "), "{next}");
    // and `--all` looks across both
    let next = cx.reef(&linked, &["next", "--all"]).await?;
    assert!(next.contains("A ") && next.contains("B "), "{next}");
    // the plans list what each follows
    let listed = cx.reef(&repo, &["plan", "ls"]).await?;
    assert!(
        listed.contains("reads its project's working tree"),
        "{listed}"
    );
    assert!(listed.contains("follows `b2`"), "{listed}");
    Ok(())
}

/// A worktree on a branch no plan follows, such as a group member's on `reef/<id>`, works in
/// the plan of the node started there, and a checkout that picks nothing names each plan's
/// branch
#[tokio::test]
async fn a_members_worktree_works_in_its_nodes_plan() -> anyhow::Result<()> {
    let cx = Cx::new("member-plan").await?;
    let repo = cx.ax.repo.clone();
    // two plans: `central`, reading the working tree, and `side` following `b2`
    cx.put("a.yml", &work("A", &[])).await?;
    git::run(&repo, &["branch", "b2"])?;
    cx.reef(&repo, &["plan", "new", "side", "--branch", "b2"])
        .await?;
    let b = cx.yaml("b.yml", &work("B", &[]))?;
    cx.reef(&repo, &["node", "put", &b, "--plan", "side"])
        .await?;
    // a worktree on a branch of its own, which no plan follows, picks none yet
    let member = git::worktree(&repo, &cx.ax.dir.join("reef-b"))?;
    let (_, error) = cx.failing(&member, &["node", "get", "B"]).await?;
    assert!(error.contains("`central` (follows `"), "{error}");
    assert!(error.contains("`side` (follows `b2`)"), "{error}");
    assert!(
        error.contains("this checkout's `reef-b` picks none of them"),
        "{error}"
    );
    // once B is started there, the worktree works in B's plan
    cx.reef(&member, &["start", "B", "--plan", "side"]).await?;
    let got = cx.reef(&member, &["node", "get", "B"]).await?;
    assert!(got.contains("id: B"), "{got}");
    // and next looks in that plan alone, where B is in progress and `central`'s A isn't listed
    let next = cx.reef(&member, &["next"]).await?;
    assert!(!next.contains("A "), "{next}");
    Ok(())
}

/// A loop started with no `--plan` in a checkout of one plan's branch is held to that plan, and
/// takes only that plan's work, while one started in the main checkout is held to its own
#[tokio::test]
async fn a_loop_started_on_a_branch_takes_only_that_branchs_plan() -> anyhow::Result<()> {
    let cx = Cx::new("loop-branch").await?;
    let repo = cx.ax.repo.clone();
    // `central`, reading the working tree, holds A, and `side`, following `b2`, holds S
    cx.put("a.yml", &work("A", &[])).await?;
    git::run(&repo, &["branch", "b2"])?;
    let linked = cx.ax.dir.join("b2-loop");
    let linked_path = linked.to_str().expect("a utf-8 path");
    git::run(&repo, &["worktree", "add", "-q", linked_path, "b2"])?;
    cx.reef(&repo, &["plan", "new", "side", "--branch", "b2"])
        .await?;
    let s = cx.yaml("s.yml", &work("S", &[]))?;
    cx.reef(&linked, &["node", "put", &s]).await?;
    // started in `b2`'s checkout, the loop feeds `side` alone
    let pane = "claude:6001";
    let out = cx.reef(&linked, &["loop", "start", "--pane", pane]).await?;
    assert_eq!(
        out,
        "started a loop in Claude Code process 6001, feeding it `reef/side`\n"
    );
    // and takes S, passing over `central`'s A, which it doesn't feed
    let took = cx.reef(&linked, &["loop", "take", "--pane", pane]).await?;
    assert!(took.contains("S Some work"), "{took}");
    assert!(!took.contains("A Some work"), "{took}");
    // one started in the main checkout feeds `central`
    let out = cx
        .reef(&repo, &["loop", "start", "--pane", "claude:6002"])
        .await?;
    assert_eq!(
        out,
        "started a loop in Claude Code process 6002, feeding it `reef/central`\n"
    );
    Ok(())
}

/// A paused loop kept over every plan is resumed as it was kept, though the checkout it is
/// started again in now picks one plan of several
#[tokio::test]
async fn a_paused_loop_is_resumed_as_it_was_kept() -> anyhow::Result<()> {
    let cx = Cx::new("loop-kept").await?;
    let repo = cx.ax.repo.clone();
    // a loop over a project of one plan, which feeds every plan, paused
    cx.put("a.yml", &work("A", &[])).await?;
    let pane = "claude:7001";
    let out = cx.reef(&repo, &["loop", "start", "--pane", pane]).await?;
    assert_eq!(
        out,
        "started a loop in Claude Code process 7001, feeding it every plan of `reef`\n"
    );
    cx.reef(&repo, &["loop", "pause", "--pane", pane]).await?;
    // a second plan, following `b2`, so the main checkout now picks `central`
    git::run(&repo, &["branch", "b2"])?;
    cx.reef(&repo, &["plan", "new", "side", "--branch", "b2"])
        .await?;
    // started again there, it resumes over every plan, rather than being busy with one
    let out = cx.reef(&repo, &["loop", "start", "--pane", pane]).await?;
    assert_eq!(
        out,
        "the loop in Claude Code process 7001 is running, feeding it every plan of `reef`\n"
    );
    Ok(())
}

/// A plan whose branch was merged and deleted follows the branch that holds it with `--merged`,
/// the old branch's last commit the one the service last fetched of it
#[tokio::test]
async fn a_merged_branch_is_followed_with_merged() -> anyhow::Result<()> {
    let cx = Cx::new("merged-branch").await?;
    // a repo published to a bare remote, its `b2` a branch with a doc of its own
    let (bare, work_dir) = git::published(&cx.ax.dir.join("up"), &[("docs/main.md", "# Main\n")])?;
    git::run(&work_dir, &["checkout", "-q", "-b", "b2"])?;
    git::write(&work_dir, "docs/b2.md", "# B2\n\n## Only b2\n\nWords.\n")?;
    commit_all(&work_dir, "b2's doc")?;
    git::run(&work_dir, &["push", "-q", "origin", "b2"])?;
    // registered by its URL, with a plan following `b2` and a node linking its doc
    let url = format!("file://{}", bare.display());
    cx.reef(&cx.ax.dir, &["project", "add", "up", "--url", &url])
        .await?;
    cx.reef(
        &cx.ax.dir,
        &["plan", "new", "p2", "--branch", "b2", "--project", "up"],
    )
    .await?;
    let a = cx.yaml(
        "A.yml",
        &format!("{}docs: [docs/b2.md#only-b2]\n", work("A", &[])),
    )?;
    cx.reef(
        &cx.ax.dir,
        &["node", "put", &a, "--project", "up", "--plan", "p2"],
    )
    .await?;
    // `b2` merged into `main`, pushed, and deleted, on the remote and here
    git::run(&work_dir, &["checkout", "-q", "main"])?;
    git::run(
        &work_dir,
        &["merge", "-q", "--no-ff", "-m", "merge b2", "b2"],
    )?;
    git::run(&work_dir, &["push", "-q", "origin", "main"])?;
    git::run(&work_dir, &["push", "-q", "origin", "--delete", "b2"])?;
    git::run(&work_dir, &["branch", "-q", "-D", "b2"])?;
    cx.reef(&cx.ax.dir, &["project", "fetch", "--project", "up"])
        .await?;
    // `--merged` from the clone follows `main`, which holds `b2`'s last commit and its doc
    let out = cx
        .reef(
            &work_dir,
            &["plan", "branch", "p2", "--merged", "--project", "up"],
        )
        .await?;
    assert!(
        out.starts_with("`up/p2`'s branch was merged into `main`\n"),
        "{out}"
    );
    assert!(out.contains("`up/p2` follows `main` at "), "{out}");
    let listed = cx
        .reef(&work_dir, &["plan", "ls", "--project", "up"])
        .await?;
    assert!(listed.contains("follows `main`"), "{listed}");
    Ok(())
}

/// A checkout still on a branch its plan moved from keeps working in the plan, and is told
/// where it moved
#[tokio::test]
async fn a_checkout_on_a_plans_old_branch_is_told_where_it_moved() -> anyhow::Result<()> {
    let cx = Cx::new("old-branch").await?;
    let repo = cx.ax.repo.clone();
    // `central` beside `side`, which follows `b2`, checked out in a linked worktree
    cx.put("a.yml", &work("A", &[])).await?;
    git::run(&repo, &["branch", "b2"])?;
    git::run(&repo, &["branch", "b3"])?;
    let linked = cx.ax.dir.join("b2-old");
    let linked_path = linked.to_str().expect("a utf-8 path");
    git::run(&repo, &["worktree", "add", "-q", linked_path, "b2"])?;
    cx.reef(&repo, &["plan", "new", "side", "--branch", "b2"])
        .await?;
    let s = cx.yaml("s.yml", &work("S", &[]))?;
    cx.reef(&linked, &["node", "put", &s]).await?;
    // `side` moves to `b3`, and remembers it followed `b2`
    let out = cx.reef(&repo, &["plan", "branch", "side", "b3"]).await?;
    assert!(out.contains("`reef/side` follows `b3`"), "{out}");
    // the worktree still on `b2` works in `side`, and next says where it went
    let got = cx.reef(&linked, &["node", "get", "S"]).await?;
    assert!(got.contains("id: S"), "{got}");
    let next = cx.reef(&linked, &["next"]).await?;
    assert!(
        next.starts_with(
            "`reef/side` follows `b3` now, not `b2`, which this checkout is on: check out its \
             branch to work where it is\n"
        ),
        "{next}"
    );
    assert!(next.contains("S ") && !next.contains("A "), "{next}");
    Ok(())
}

/// `reef node move` moves a subtree of needs to another plan, says what moved, and refuses a
/// node others left behind need, saying how to move it whole
#[tokio::test]
async fn node_move_moves_a_subtree_and_refuses_a_need_left_behind() -> anyhow::Result<()> {
    let cx = Cx::new("node-move").await?;
    let repo = cx.ax.repo.clone();
    // `central`'s A, and B needing it, beside an empty `side`
    cx.put("a.yml", &work("A", &[])).await?;
    cx.put("b.yml", &work("B", &["A"])).await?;
    cx.reef(&repo, &["plan", "new", "side"]).await?;
    // A alone is refused, B left behind needing it
    let (_, error) = cx
        .failing(
            &repo,
            &["node", "move", "A", "--to", "side", "--plan", "central"],
        )
        .await?;
    assert!(
        error.starts_with("nothing moved, since the move would add these problems"),
        "{error}"
    );
    assert!(error.contains("reef/central/B"), "{error}");
    // both at once move, and `side` holds them
    let out = cx
        .reef(
            &repo,
            &[
                "node", "move", "A", "B", "--to", "side", "--plan", "central",
            ],
        )
        .await?;
    assert_eq!(out, "moved `A`, `B` from `reef/central` to `reef/side`\n");
    let got = cx
        .reef(&repo, &["node", "get", "B", "--plan", "side"])
        .await?;
    assert!(got.contains("needs:\n- A\n"), "{got}");
    Ok(())
}

/// A binary of a case's own to update: a script that says it is an old `reef`
///
/// # Arguments
///
/// * `path` - Where to write it
fn old_reef(path: &Path) -> anyhow::Result<String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().expect("a binary is in a directory"))?;
    std::fs::write(path, "#!/bin/sh\necho old\n")?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path.to_str().expect("the case's dir is utf-8").to_string())
}

/// `reef update` says what it would install when only asked, swaps a binary for the client the
/// service hands out, and leaves a reefadm release's own `reef` to reefadm
#[tokio::test]
async fn update_swaps_a_binary_for_the_served_client() -> anyhow::Result<()> {
    let build = stamped("dc5e68cd25c95db18f885357f505e97af24175f9", 1_795_800_000);
    let cx = Cx::serving("update", &build).await?;
    let repo = cx.ax.repo.clone();
    let target = cx.ax.dir.join("home/ada/.local/bin/reef");
    let into = old_reef(&target)?;
    // only asked, it says what it would install, and installs nothing
    let said = cx
        .reef(&repo, &["update", "--check", "--into", &into])
        .await?;
    assert!(
        said.contains("reef 0.1.0 dc5e68cd25c9 (2026-11-27)"),
        "{said}"
    );
    assert!(said.contains("`reef update` installs it"), "{said}");
    assert_eq!(std::fs::read_to_string(&target)?, "#!/bin/sh\necho old\n");
    // forced, it swaps the binary for the client
    let said = cx
        .reef(&repo, &["update", "--force", "--into", &into])
        .await?;
    assert!(
        said.starts_with(&format!(
            "updated {} to reef 0.1.0 dc5e68cd25c9 (2026-11-27)",
            target.display()
        )),
        "{said}"
    );
    let client = cx.ax.dir.join("release/bin/reef");
    assert_eq!(std::fs::read(&target)?, std::fs::read(&client)?);
    // which says the build the service named
    let output = std::process::Command::new(&target)
        .args(["version", "--json"])
        .output()?;
    let said: reef_api::Build = serde_json::from_slice(&output.stdout)?;
    assert_eq!(said, build);
    // a reefadm release's own `reef`, or the host's link into it, is reefadm's to replace
    let release = cx
        .ax
        .dir
        .join("opt/reef/releases/dc5e68cd25c9-20261127000000");
    let own = old_reef(&release.join("bin/reef"))?;
    std::fs::write(release.join("release.toml"), "")?;
    let (_, error) = cx.failing(&repo, &["update", "--into", &own]).await?;
    assert!(error.contains("`reefadm upgrade`"), "{error}");
    assert_eq!(std::fs::read_to_string(&own)?, "#!/bin/sh\necho old\n");
    Ok(())
}

/// `reef update` against a service that hands out no client says so, and changes nothing
#[tokio::test]
async fn update_says_when_the_service_hands_out_no_client() -> anyhow::Result<()> {
    let cx = Cx::new("update-none").await?;
    let target = cx.ax.dir.join("home/ada/.local/bin/reef");
    let into = old_reef(&target)?;
    let (_, error) = cx
        .failing(&cx.ax.repo, &["update", "--into", &into])
        .await?;
    assert!(error.contains("hands its users no `reef`"), "{error}");
    assert_eq!(std::fs::read_to_string(&target)?, "#!/bin/sh\necho old\n");
    Ok(())
}
