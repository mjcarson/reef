//! Waiting for a started server to answer, on loopback and on its TLS port
//!
//! The server writes its CA before it binds a port, and binds its ports before shoal is up, so
//! reefadm waits for the CA, copies it where every account can read it (a team server's
//! `/etc/reef/reef-ca.pem`, again whenever it differs), then asks `GET /api/v1/me` over HTTPS
//! with short timeouts until it answers. The TLS port is asked by the name its certificate
//! covers, resolved to this host. If the server never answers, the unit's journal says why.

use crate::exec::{Ask, Cmd, Me, Runner};
use crate::layout::{Layout, Mode};
use crate::served::Served;
use crate::systemd::Systemd;
use anyhow::{Result, bail};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// How long between looks
const BETWEEN: Duration = Duration::from_millis(500);

/// How many looks for the CA: half a minute
const CA_TRIES: u32 = 60;

/// How many asks of a port: a minute and a half, since a first start makes a CA and opens shoal
const ASK_TRIES: u32 = 180;

/// How many of the journal's lines to show when the server doesn't come up
const JOURNAL_LINES: u32 = 40;

/// Where a team server is asked from other hosts: the name its certificate covers, and the
/// address on this host it is reached at
///
/// # Arguments
///
/// * `served` - reef's config
/// * `hostname` - This host's name, when the config names none
pub fn tls_ask(served: &Served, hostname: &str) -> Option<(String, SocketAddr)> {
    let address = served.tls_address()?;
    let port = served.tls_port()?;
    // the first name the config gives, else the host's own, both of which reef's certificate covers
    let name = served
        .api
        .names
        .first()
        .cloned()
        .unwrap_or_else(|| hostname.to_string());
    // a wildcard listens on loopback too; a specific address only there
    let host = address
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or_default();
    let ip = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .ok()
        .filter(|ip| !ip.is_unspecified())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    Some((name, SocketAddr::new(ip, port)))
}

/// Waits until a started server answers, copying its CA out on the way, and returns what it
/// answered on loopback
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `served` - reef's config
/// * `hostname` - This host's name
pub fn wait(
    runner: &mut dyn Runner,
    layout: &Layout,
    served: &Served,
    hostname: &str,
) -> Result<Me> {
    // reef's own CA, which the server makes before it listens
    let certs = served.certs(layout);
    let ca = certs.join("ca.pem");
    let owner = layout.owner();
    let mut made = runner.dry();
    for _ in 0..CA_TRIES {
        if made {
            break;
        }
        made = crate::exec::exists(runner, &ca, owner)?;
        if !made {
            runner.sleep(BETWEEN);
        }
    }
    if !made {
        return failed(
            runner,
            layout,
            &format!("the server made no CA in {}", certs.display()),
        );
    }
    // a copy every account on a team server can read, made again whenever it differs
    let trusted = match (&layout.ca_copy, layout.mode) {
        (Some(copy), Mode::System) => {
            let same = runner
                .probe(&Cmd::new("cmp").arg("-s").arg(&ca).arg(copy).root())?
                .ok();
            if !same {
                runner.run(
                    &Cmd::new("install")
                        .args(["-m", "0644"])
                        .arg(&ca)
                        .arg(copy)
                        .root(),
                )?;
            }
            copy.clone()
        }
        _ => ca,
    };
    let pem = crate::exec::read(runner, &trusted, crate::exec::As::Me)?.map(String::into_bytes);
    // loopback, as this host's CLI reaches it
    let loopback = Ask {
        url: format!("https://{}/api/v1/me", served.loopback()),
        ca: pem.clone(),
        resolve: None,
    };
    let me = match ask(runner, &loopback)? {
        Ok(me) => me,
        Err(error) => return failed(runner, layout, &error),
    };
    // and the port other hosts reach, by the name they call it
    if let Some((name, address)) = tls_ask(served, hostname) {
        let outside = Ask {
            url: format!("https://{name}:{}/api/v1/me", address.port()),
            ca: pem,
            resolve: Some((name, address)),
        };
        if let Err(error) = ask(runner, &outside)? {
            return failed(runner, layout, &error);
        }
    }
    Ok(me)
}

/// Asks a port until it answers, or says why it never did
///
/// # Arguments
///
/// * `runner` - The runner
/// * `ask` - Where to ask
fn ask(runner: &mut dyn Runner, ask: &Ask) -> Result<std::result::Result<Me, String>> {
    let mut last = String::new();
    for _ in 0..ASK_TRIES {
        match runner.me(ask) {
            Ok(me) => return Ok(Ok(me)),
            Err(error) => last = format!("{error:#}"),
        }
        runner.sleep(BETWEEN);
    }
    Ok(Err(format!("{} never answered: {last}", ask.url)))
}

/// Shows the unit's journal, then fails saying why
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `why` - What didn't happen
fn failed<T>(runner: &mut dyn Runner, layout: &Layout, why: &str) -> Result<T> {
    let journal = runner.probe(&Systemd::of(layout).journal(JOURNAL_LINES))?;
    runner.say(&format!("reef's journal, its last {JOURNAL_LINES} lines:"));
    for line in journal.stdout.lines() {
        runner.say(&format!("    {line}"));
    }
    bail!("reef didn't come up: {why}")
}

#[cfg(test)]
mod tests {
    use super::{tls_ask, wait};
    use crate::Env;
    use crate::exec::{Me, Output, Script};
    use crate::layout::{Layout, Mode};
    use crate::served::{Served, TeamFlags};
    use std::net::SocketAddr;

    /// A team server's layout and config
    fn team() -> (Layout, Served) {
        let layout = Layout::new(Mode::System, &Env::from_pairs([("HOME", "/home/ada")])).unwrap();
        let flags = TeamFlags {
            names: vec!["reef.example.com".into()],
            port: 12443,
            own_cert: false,
            cores: 2,
            memory: "256MiB".into(),
        };
        let served = Served::team(&layout, &flags);
        (layout, served)
    }

    /// The TLS port is asked by its first name, at loopback for a wildcard address
    #[test]
    fn the_tls_port_is_asked_by_name() {
        let (_, served) = team();
        let (name, address) = tls_ask(&served, "host").unwrap();
        assert_eq!(name, "reef.example.com");
        assert_eq!(address, "127.0.0.1:12443".parse::<SocketAddr>().unwrap());
        // with no name, the host's; on a specific address, there
        let mut served = served;
        served.api.names.clear();
        served.api.tls.as_mut().unwrap().address = "10.0.0.5:443".into();
        let (name, address) = tls_ask(&served, "host").unwrap();
        assert_eq!(name, "host");
        assert_eq!(address, "10.0.0.5:443".parse::<SocketAddr>().unwrap());
    }

    /// A team server's CA is copied out once it exists, then both ports are asked until they answer
    #[test]
    fn a_team_server_is_waited_for_on_both_ports() {
        let (layout, served) = team();
        let mut script = Script::default();
        script.answer(
            "sudo test -e /var/lib/reef/certs/ca.pem",
            Output::printed(""),
        );
        script.prints("cat /etc/reef/reef-ca.pem", "PEM");
        // loopback refuses once before it answers
        script.mes.push_back(Err("refused".into()));
        script.mes.push_back(Ok(Me {
            required: true,
            user: None,
        }));
        let me = wait(&mut script, &layout, &served, "host").unwrap();
        assert!(me.required);
        assert_eq!(
            script.log,
            [
                "run: sudo install -m 0644 /var/lib/reef/certs/ca.pem /etc/reef/reef-ca.pem",
                "me: https://127.0.0.1:12180/api/v1/me",
                "me: https://127.0.0.1:12180/api/v1/me",
                "me: https://reef.example.com:12443/api/v1/me",
            ]
        );
    }

    /// A server that never makes its CA fails, showing the journal
    #[test]
    fn a_server_that_never_comes_up_shows_its_journal() {
        let (layout, served) = team();
        let mut script = Script::default();
        script.prints(
            "sudo journalctl -u reef.service -b --no-pager -n 40",
            "reef-app: config refused\n",
        );
        let error = wait(&mut script, &layout, &served, "host").unwrap_err();
        assert!(error.to_string().contains("made no CA"), "{error}");
        assert!(
            script
                .said
                .iter()
                .any(|line| line.contains("config refused"))
        );
    }
}
