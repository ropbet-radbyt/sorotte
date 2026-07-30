use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TlsCertificateBundleFingerprint([u8; 32]);

pub(crate) struct TlsCertificateBundleSnapshot {
    private_key_pem: Vec<u8>,
    certificate_pem: Vec<u8>,
    chain_pem: Vec<u8>,
    fingerprint: TlsCertificateBundleFingerprint,
}

impl TlsCertificateBundleSnapshot {
    pub(crate) fn fingerprint(&self) -> TlsCertificateBundleFingerprint {
        self.fingerprint
    }
}

fn fingerprint_tls_certificate_bundle_members(
    members: [(&str, &[u8]); 3],
) -> TlsCertificateBundleFingerprint {
    let mut digest = Sha256::new();
    digest.update(b"sorotte-tls-certificate-bundle-v1\0");
    for (filename, contents) in members {
        digest.update(
            u64::try_from(filename.len())
                .expect("TLS bundle filename length must fit u64")
                .to_be_bytes(),
        );
        digest.update(filename.as_bytes());
        digest.update(
            u64::try_from(contents.len())
                .expect("TLS bundle member length must fit u64")
                .to_be_bytes(),
        );
        digest.update(contents);
    }
    TlsCertificateBundleFingerprint(digest.finalize().into())
}

pub(crate) fn read_tls_certificate_bundle_snapshot(
    path: &Path,
) -> io::Result<TlsCertificateBundleSnapshot> {
    let [private_key_filename, certificate_filename, chain_filename] = TLS_REQUIRED_CERT_FILENAMES;
    let private_key_pem = fs::read(path.join(private_key_filename))?;
    let certificate_pem = fs::read(path.join(certificate_filename))?;
    let chain_pem = fs::read(path.join(chain_filename))?;
    let fingerprint = fingerprint_tls_certificate_bundle_members([
        (private_key_filename, &private_key_pem),
        (certificate_filename, &certificate_pem),
        (chain_filename, &chain_pem),
    ]);
    Ok(TlsCertificateBundleSnapshot {
        private_key_pem,
        certificate_pem,
        chain_pem,
        fingerprint,
    })
}

#[cfg(test)]
pub(crate) fn tls_certificate_bundle_fingerprint(
    path: &Path,
) -> Option<TlsCertificateBundleFingerprint> {
    read_tls_certificate_bundle_snapshot(path)
        .ok()
        .map(|snapshot| snapshot.fingerprint())
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct TlsCertificateBundleMetadataClock {
    revision: Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(test)]
impl TlsCertificateBundleMetadataClock {
    pub(crate) fn new() -> Self {
        Self {
            revision: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    pub(crate) fn fingerprint(&self) -> TlsCertificateBundleFingerprint {
        let mut bytes = [0_u8; 32];
        bytes[24..].copy_from_slice(&self.revision.load(Ordering::SeqCst).to_be_bytes());
        TlsCertificateBundleFingerprint(bytes)
    }

    pub(crate) fn advance(&self) {
        self.revision
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |revision| {
                revision.checked_add(1)
            })
            .expect("TLS certificate bundle test revision should not overflow");
    }
}

fn tls_certificates_from_pem(pem: &[u8], path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
    let mut reader = io::BufReader::new(pem);
    let certificates = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
    if certificates.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("tls certificate file '{}' is empty", path.display()),
        ));
    }
    Ok(certificates)
}

fn tls_private_key_from_pem(pem: &[u8], path: &Path) -> io::Result<PrivateKeyDer<'static>> {
    let mut reader = io::BufReader::new(pem);
    rustls_pemfile::private_key(&mut reader)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("tls private key file '{}' has no key", path.display()),
        )
    })
}

pub(crate) fn load_tls_server_config(path: &Path) -> io::Result<Arc<ServerConfig>> {
    load_tls_server_config_from_snapshot(path, &read_tls_certificate_bundle_snapshot(path)?)
}

pub(crate) fn load_tls_server_config_from_snapshot(
    path: &Path,
    snapshot: &TlsCertificateBundleSnapshot,
) -> io::Result<Arc<ServerConfig>> {
    let mut certificate_chain =
        tls_certificates_from_pem(&snapshot.certificate_pem, &path.join("cert.pem"))?;
    certificate_chain.extend(tls_certificates_from_pem(
        &snapshot.chain_pem,
        &path.join("chain.pem"),
    )?);
    let private_key =
        tls_private_key_from_pem(&snapshot.private_key_pem, &path.join("privkey.pem"))?;
    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificate_chain, private_key)
        .map_err(|source| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("tls certificate bundle is invalid: {source}"),
            )
        })?;
    Ok(Arc::new(server_config))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle_fingerprint_for_members(members: [&[u8]; 3]) -> TlsCertificateBundleFingerprint {
        fingerprint_tls_certificate_bundle_members([
            ("privkey.pem", members[0]),
            ("cert.pem", members[1]),
            ("chain.pem", members[2]),
        ])
    }

    #[test]
    fn bundle_fingerprint_changes_for_every_required_member_and_equal_length_replacement() {
        let before = bundle_fingerprint_for_members([b"key-1", b"cert-1", b"chain-1"]);
        for after in [
            bundle_fingerprint_for_members([b"key-2", b"cert-1", b"chain-1"]),
            bundle_fingerprint_for_members([b"key-1", b"cert-2", b"chain-1"]),
            bundle_fingerprint_for_members([b"key-1", b"cert-1", b"chain-2"]),
        ] {
            assert_ne!(
                before, after,
                "changing any required TLS bundle member must change the rotation token"
            );
        }
    }
}
