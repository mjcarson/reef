//! Certificates for the cases that serve over TLS: a CA of the case's own, and a certificate it
//! signs for this host, written as the PEM files reef's config names

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use std::path::{Path, PathBuf};

/// The PEM files a case serves over TLS with, and the root a client trusts them by
#[derive(Debug, Clone)]
pub struct Minted {
    /// The server's certificate, signed by the CA, for `[api.tls] cert`
    pub cert: PathBuf,
    /// Its private key, for `[api.tls] key`
    pub key: PathBuf,
    /// The CA's certificate, for a client's `[api] ca`
    pub ca: PathBuf,
}

/// Mints a CA, and a certificate it signs for `localhost` and `127.0.0.1`, into a directory
///
/// # Arguments
///
/// * `dir` - The directory the three PEM files are written in
pub fn minted(dir: &Path) -> anyhow::Result<Minted> {
    // a CA of the case's own
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "reef's test CA");
    let ca_key = KeyPair::generate()?;
    let ca = ca_params.self_signed(&ca_key)?;
    // the server's certificate, for the names a case calls it by
    let params = CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])?;
    let key = KeyPair::generate()?;
    let cert = params.signed_by(&key, &ca, &ca_key)?;
    // each as a PEM file of its own
    std::fs::create_dir_all(dir)?;
    let minted = Minted {
        cert: dir.join("cert.pem"),
        key: dir.join("key.pem"),
        ca: dir.join("ca.pem"),
    };
    std::fs::write(&minted.cert, cert.pem())?;
    std::fs::write(&minted.key, key.serialize_pem())?;
    std::fs::write(&minted.ca, ca.pem())?;
    Ok(minted)
}
