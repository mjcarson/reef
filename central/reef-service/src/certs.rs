//! reef's own certificates: a CA of its own, and a certificate it signs for this host's names,
//! which every listener serves when no certificate is configured
//!
//! - **Made once, then kept.** The CA and the certificate are written to one directory, `[api]
//!   certs`, else `$XDG_DATA_HOME/reef/certs`, and read back on every start, so a browser or a
//!   client that trusts the CA once keeps trusting it.
//! - **The certificate is made again from the kept CA** when the names it must cover change, or
//!   it nears its end, so trust in `ca.pem` outlives each certificate.
//! - **For this host's names.** `localhost`, `127.0.0.1` and `::1`, the host's own name, and any
//!   others the caller adds, such as a wide listener's address or `[api] names`.
//! - **Keys kept to their owner.** Both private keys are written readable by the service's
//!   account alone; the CA's certificate and the served one are readable by anyone, so a client
//!   on the host can trust the CA.

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use time::OffsetDateTime;
use tracing::instrument;

/// The CA's certificate, which a client or a browser trusts
pub const CA_FILE: &str = "ca.pem";

/// The CA's private key, which signs each certificate made again
pub const CA_KEY_FILE: &str = "ca-key.pem";

/// What the CA was made as: its name and when it starts and ends
pub const CA_RECORD_FILE: &str = "ca.toml";

/// The certificate every listener serves
pub const CERT_FILE: &str = "cert.pem";

/// The served certificate's private key
pub const KEY_FILE: &str = "key.pem";

/// What the served certificate was made for: its names and when it ends
pub const LEAF_FILE: &str = "leaf.toml";

/// How long the CA is good for, in days
pub const CA_DAYS: i64 = 3650;

/// How long each certificate is good for, in days, inside the most any browser accepts
pub const LEAF_DAYS: i64 = 397;

/// How near its end a certificate is made again, in days
pub const RENEW_DAYS: i64 = 30;

/// reef's own certificate as it is kept, and the CA that signed it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Own {
    /// The certificate every listener serves, as PEM
    pub cert: PathBuf,
    /// Its private key, as PEM
    pub key: PathBuf,
    /// The CA's certificate, as PEM, which a client or a browser trusts
    pub ca: PathBuf,
    /// Whether this start made the CA, so it can be said where to find it
    pub made_ca: bool,
    /// Whether this start made the certificate
    pub made_cert: bool,
}

/// What the CA was made as, kept beside it, so it signs each certificate as the issuer it is
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CaRecord {
    /// Its common name
    name: String,
    /// When it starts, in seconds since the epoch
    from: i64,
    /// When it ends, in seconds since the epoch
    until: i64,
}

/// What the served certificate was made for, kept beside it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Leaf {
    /// The names it covers
    names: BTreeSet<String>,
    /// When it ends, in seconds since the epoch
    until: i64,
}

/// The names reef's own certificate covers: this host's loopback names and its own name, then
/// any the caller adds
///
/// # Arguments
///
/// * `extra` - Further names or addresses, such as a wide listener's address or `[api] names`
///
/// # Examples
///
/// ```
/// use reef_service::certs::names;
///
/// let names = names(&["reef.example.com".to_string()]);
/// assert!(names.contains("localhost") && names.contains("127.0.0.1") && names.contains("::1"));
/// assert!(names.contains("reef.example.com"));
/// ```
pub fn names(extra: &[String]) -> BTreeSet<String> {
    // this host however it is called from itself
    let mut names: BTreeSet<String> = ["localhost", "127.0.0.1", "::1"]
        .into_iter()
        .map(str::to_string)
        .collect();
    // its own name, as other hosts on its network may call it
    if let Some(host) = host_name() {
        names.insert(host);
    }
    // and whatever else the caller names, each trimmed, the empty left out
    names.extend(
        extra
            .iter()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()),
    );
    names
}

/// This host's own name, as the kernel holds it, if it is one a certificate can name
fn host_name() -> Option<String> {
    // Linux keeps it here, which is where reef runs
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname").ok()?;
    let name = name.trim().to_ascii_lowercase();
    // a DNS name of letters, digits, dashes and dots, and not one already named
    let fits = !name.is_empty()
        && name.len() <= 253
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    (fits && name != "localhost").then_some(name)
}

/// Reads reef's own certificate from its directory, making the CA if there is none and the
/// certificate again if it doesn't cover the names given or nears its end
///
/// # Arguments
///
/// * `dir` - The directory they are kept in, made if it isn't there
/// * `names` - The names the certificate must cover
/// * `now` - The time, which a certificate's end is measured from
#[instrument(name = "certs::ensure", skip_all, err(Debug))]
pub fn ensure(dir: &Path, names: &BTreeSet<String>, now: SystemTime) -> anyhow::Result<Own> {
    // the directory, readable by anyone so a client on this host finds the CA
    std::fs::create_dir_all(dir)
        .map_err(|error| anyhow::anyhow!("making {}: {error}", dir.display()))?;
    let now = OffsetDateTime::from(now);
    let own = Own {
        cert: dir.join(CERT_FILE),
        key: dir.join(KEY_FILE),
        ca: dir.join(CA_FILE),
        made_ca: false,
        made_cert: false,
    };
    // the CA kept, else a new one, which every certificate made before it no longer chains to
    let (ca_key, ca_params, made_ca) = match kept_ca(dir)? {
        Some((key, params)) => (key, params, false),
        None => {
            let (key, params) = make_ca(dir, now)?;
            (key, params, true)
        }
    };
    // the certificate kept, if the CA is the one that signed it and it still serves
    let fresh = !made_ca && serves(dir, names, now)?;
    if fresh {
        return Ok(Own { made_ca, ..own });
    }
    make_leaf(dir, names, now, &ca_key, ca_params)?;
    Ok(Own {
        made_ca,
        made_cert: true,
        ..own
    })
}

/// The CA kept in a directory, as its key and the parameters it signs with, or `None` if any of
/// its files isn't there or can't be read, so a new one is made
///
/// # Arguments
///
/// * `dir` - The directory it is kept in
fn kept_ca(dir: &Path) -> anyhow::Result<Option<(KeyPair, CertificateParams)>> {
    // its certificate, its key and its record, or none worth reading
    let (cert, key, record) = (
        dir.join(CA_FILE),
        dir.join(CA_KEY_FILE),
        dir.join(CA_RECORD_FILE),
    );
    if !cert.is_file() || !key.is_file() || !record.is_file() {
        return Ok(None);
    }
    let pem = std::fs::read_to_string(&key)
        .map_err(|error| anyhow::anyhow!("reading {}: {error}", key.display()))?;
    let Ok(key) = KeyPair::from_pem(&pem) else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&record)
        .map_err(|error| anyhow::anyhow!("reading {}: {error}", record.display()))?;
    let Ok(record) = toml::from_str::<CaRecord>(&text) else {
        return Ok(None);
    };
    // its name, key and dates as made, so what it signs chains to the certificate kept
    let (Ok(from), Ok(until)) = (
        OffsetDateTime::from_unix_timestamp(record.from),
        OffsetDateTime::from_unix_timestamp(record.until),
    ) else {
        return Ok(None);
    };
    Ok(Some((key, ca_params(&record.name, from, until))))
}

/// The name a new CA takes: reef's, this host's, and the day it was made, so two hosts' CAs
/// are told apart wherever both are trusted
///
/// # Arguments
///
/// * `now` - When it is made
fn ca_name(now: OffsetDateTime) -> String {
    // the host's name, else plain reef
    let host = host_name().unwrap_or_else(|| "this host".to_string());
    format!("reef's own CA on {host}, {}", now.date())
}

/// The parameters a CA of a name is signed with
///
/// # Arguments
///
/// * `name` - Its common name
/// * `from` - When it starts
/// * `until` - When it ends
fn ca_params(name: &str, from: OffsetDateTime, until: OffsetDateTime) -> CertificateParams {
    // a CA of its own that signs certificates for servers alone, with no CA below it
    let mut params = CertificateParams::default();
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, name);
    params
        .distinguished_name
        .push(DnType::OrganizationName, "reef");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params.not_before = from;
    params.not_after = until;
    params
}

/// Makes a new CA in a directory, writing its certificate and its key
///
/// # Arguments
///
/// * `dir` - The directory it is kept in
/// * `now` - When it starts
fn make_ca(dir: &Path, now: OffsetDateTime) -> anyhow::Result<(KeyPair, CertificateParams)> {
    // a key of its own, and a certificate of its name signed by it, from a moment ago
    let key = KeyPair::generate()?;
    let name = ca_name(now);
    let from = now - time::Duration::minutes(5);
    let params = ca_params(&name, from, now + time::Duration::days(CA_DAYS));
    let cert = params.clone().self_signed(&key)?;
    // the certificate for anyone to trust, the key for its owner alone, and what it was made as
    write(&dir.join(CA_FILE), &cert.pem(), 0o644)?;
    write(&dir.join(CA_KEY_FILE), &key.serialize_pem(), 0o600)?;
    let record = CaRecord {
        name,
        from: params.not_before.unix_timestamp(),
        until: params.not_after.unix_timestamp(),
    };
    write(&dir.join(CA_RECORD_FILE), &toml::to_string(&record)?, 0o644)?;
    tracing::info!(dir = %dir.display(), "made reef's own CA");
    Ok((key, params))
}

/// Whether the certificate kept in a directory covers the names given and is far from its end
///
/// # Arguments
///
/// * `dir` - The directory it is kept in
/// * `names` - The names it must cover
/// * `now` - The time
fn serves(dir: &Path, names: &BTreeSet<String>, now: OffsetDateTime) -> anyhow::Result<bool> {
    // the certificate, its key and what it was made for, or nothing to serve
    let leaf = dir.join(LEAF_FILE);
    if !dir.join(CERT_FILE).is_file() || !dir.join(KEY_FILE).is_file() || !leaf.is_file() {
        return Ok(false);
    }
    // one whose record can't be read is made again rather than trusted
    let text = std::fs::read_to_string(&leaf)
        .map_err(|error| anyhow::anyhow!("reading {}: {error}", leaf.display()))?;
    let Ok(leaf) = toml::from_str::<Leaf>(&text) else {
        return Ok(false);
    };
    // every name it must cover, and a month or more left
    let left = leaf.until - now.unix_timestamp();
    Ok(names.is_subset(&leaf.names) && left > RENEW_DAYS * 86_400)
}

/// Makes the certificate every listener serves, signed by the CA, for the names given
///
/// # Arguments
///
/// * `dir` - The directory it is kept in
/// * `names` - The names it covers
/// * `now` - When it starts
/// * `ca_key` - The CA's key, which signs it
/// * `ca_params` - The CA's parameters, which name it as the issuer
fn make_leaf(
    dir: &Path,
    names: &BTreeSet<String>,
    now: OffsetDateTime,
    ca_key: &KeyPair,
    ca_params: CertificateParams,
) -> anyhow::Result<()> {
    // the CA as an issuer: the same name and key as the one kept, so it chains to that one
    let issuer = ca_params.self_signed(ca_key)?;
    // a key of its own, for a server, under each name
    let key = KeyPair::generate()?;
    let mut params = CertificateParams::new(names.iter().cloned().collect::<Vec<_>>())?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, "reef");
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    params.not_before = now - time::Duration::minutes(5);
    params.not_after = now + time::Duration::days(LEAF_DAYS);
    let until = params.not_after.unix_timestamp();
    let cert = params.signed_by(&key, &issuer, ca_key)?;
    // the certificate alone, since a client holds the CA as its root, its key, and what it
    // was made for
    write(&dir.join(CERT_FILE), &cert.pem(), 0o644)?;
    write(&dir.join(KEY_FILE), &key.serialize_pem(), 0o600)?;
    let leaf = Leaf {
        names: names.clone(),
        until,
    };
    write(&dir.join(LEAF_FILE), &toml::to_string(&leaf)?, 0o644)?;
    tracing::info!(dir = %dir.display(), ?names, "made reef's own certificate");
    Ok(())
}

/// Writes a file whole with a mode, through a file beside it renamed over it, so a reader never
/// sees half of one
///
/// # Arguments
///
/// * `path` - The file
/// * `text` - What it holds
/// * `mode` - Its permissions
fn write(path: &Path, text: &str, mode: u32) -> anyhow::Result<()> {
    // written beside it with its mode from the start, then put in its place
    let partial = path.with_extension("partial");
    let _ = std::fs::remove_file(&partial);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&partial)
        .map_err(|error| anyhow::anyhow!("writing {}: {error}", partial.display()))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&partial, path)
        .map_err(|error| anyhow::anyhow!("writing {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CA_FILE, CERT_FILE, KEY_FILE, LEAF_DAYS, RENEW_DAYS, ensure, names};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    /// A fresh directory of a test's own, under central's target dir
    ///
    /// # Arguments
    ///
    /// * `name` - The test's name
    fn fresh(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/tmp/certs")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A missing certificate is made once, a CA and a certificate for this host's names, kept
    /// across starts, and made again from the same CA when its names change or it nears its end
    #[test]
    fn a_missing_certificate_is_made_once_and_made_again_from_its_ca() {
        let dir = fresh("made-once");
        let now = SystemTime::now();
        let wanted = names(&[]);
        // the first start makes both, the keys readable by their owner alone
        let first = ensure(&dir, &wanted, now).unwrap();
        assert!(first.made_ca && first.made_cert);
        let mode = |file: &str| {
            std::fs::metadata(dir.join(file))
                .unwrap()
                .permissions()
                .mode()
        };
        assert_eq!(mode(KEY_FILE) & 0o777, 0o600);
        assert_eq!(mode("ca-key.pem") & 0o777, 0o600);
        assert_eq!(mode(CA_FILE) & 0o777, 0o644);
        let ca = std::fs::read_to_string(dir.join(CA_FILE)).unwrap();
        let cert = std::fs::read_to_string(dir.join(CERT_FILE)).unwrap();
        // the next start reads them back as they were
        let again = ensure(&dir, &wanted, now).unwrap();
        assert!(!again.made_ca && !again.made_cert);
        assert_eq!(std::fs::read_to_string(dir.join(CERT_FILE)).unwrap(), cert);
        // a name it doesn't cover makes it again, from the same CA
        let wider = names(&["reef.example.com".to_string()]);
        let renamed = ensure(&dir, &wider, now).unwrap();
        assert!(!renamed.made_ca && renamed.made_cert);
        assert_eq!(std::fs::read_to_string(dir.join(CA_FILE)).unwrap(), ca);
        let named = std::fs::read_to_string(dir.join(CERT_FILE)).unwrap();
        assert_ne!(named, cert);
        // and fewer names than it covers changes nothing
        assert!(!ensure(&dir, &wanted, now).unwrap().made_cert);
        // a month from its end it is made again, still from the same CA
        let late = now + Duration::from_secs(((LEAF_DAYS - RENEW_DAYS + 1) * 86_400) as u64);
        let renewed = ensure(&dir, &wider, late).unwrap();
        assert!(!renewed.made_ca && renewed.made_cert);
        assert_eq!(std::fs::read_to_string(dir.join(CA_FILE)).unwrap(), ca);
        // a CA whose key is gone is made again, and the certificate with it
        std::fs::remove_file(dir.join("ca-key.pem")).unwrap();
        let lost = ensure(&dir, &wider, late).unwrap();
        assert!(lost.made_ca && lost.made_cert);
        assert_ne!(std::fs::read_to_string(dir.join(CA_FILE)).unwrap(), ca);
    }

    /// The certificate made chains to the CA kept, for every name it covers, as a client that
    /// trusts `ca.pem` checks it, even once it was made again from that CA
    #[test]
    fn the_certificate_chains_to_the_kept_ca() {
        use tokio_rustls::rustls::RootCertStore;
        use tokio_rustls::rustls::client::WebPkiServerVerifier;
        use tokio_rustls::rustls::client::danger::ServerCertVerifier;
        use tokio_rustls::rustls::pki_types::pem::PemObject;
        use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
        let dir = fresh("chains");
        let now = SystemTime::now();
        ensure(&dir, &names(&[]), now).unwrap();
        // made again, so the CA that signs it is the kept one signed anew
        let own = ensure(&dir, &names(&["reef.example.com".to_string()]), now).unwrap();
        // the kept CA alone as the root
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from_pem_file(&own.ca).unwrap())
            .unwrap();
        let provider = std::sync::Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
        let verifier = WebPkiServerVerifier::builder_with_provider(roots.into(), provider)
            .build()
            .unwrap();
        // the certificate alone is served
        let chain: Vec<CertificateDer> = CertificateDer::pem_file_iter(&own.cert)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(chain.len(), 1);
        // every name it covers, the loopback addresses and a DNS name alike
        for name in ["localhost", "127.0.0.1", "::1", "reef.example.com"] {
            let server = ServerName::try_from(name).unwrap();
            verifier
                .verify_server_cert(&chain[0], &[], &server, &[], UnixTime::now())
                .unwrap_or_else(|error| panic!("{name}: {error}"));
        }
        // and not one it doesn't
        let other = ServerName::try_from("elsewhere.example.com").unwrap();
        assert!(
            verifier
                .verify_server_cert(&chain[0], &[], &other, &[], UnixTime::now())
                .is_err()
        );
    }
}
