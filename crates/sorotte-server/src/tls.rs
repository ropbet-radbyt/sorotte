use super::*;

pub(crate) fn tls_certificate_bundle_is_available(path: &Path) -> bool {
    TLS_REQUIRED_CERT_FILENAMES
        .iter()
        .all(|filename| path.join(filename).is_file())
}

pub(crate) fn tls_certificate_bundle_modified_time(path: &Path) -> Option<SystemTime> {
    tls_certificate_bundle_modified_time_with(path, |certificate_path| {
        fs::metadata(certificate_path)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
    })
}

fn tls_certificate_bundle_modified_time_with(
    path: &Path,
    mut modified_time: impl FnMut(&Path) -> Option<SystemTime>,
) -> Option<SystemTime> {
    TLS_REQUIRED_CERT_FILENAMES
        .iter()
        .filter_map(|filename| modified_time(&path.join(filename)))
        .max()
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

    pub(crate) fn modified_time(&self) -> SystemTime {
        UNIX_EPOCH + std::time::Duration::from_secs(self.revision.load(Ordering::SeqCst))
    }

    pub(crate) fn advance(&self) {
        self.revision
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |revision| {
                revision.checked_add(1)
            })
            .expect("TLS certificate bundle test revision should not overflow");
    }
}

fn tls_certificates_from_pem(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
    let file = fs::File::open(path)?;
    let mut reader = io::BufReader::new(file);
    let certificates = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
    if certificates.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("tls certificate file '{}' is empty", path.display()),
        ));
    }
    Ok(certificates)
}

fn tls_private_key_from_pem(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
    let file = fs::File::open(path)?;
    let mut reader = io::BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("tls private key file '{}' has no key", path.display()),
        )
    })
}

pub(crate) fn load_tls_server_config(path: &Path) -> io::Result<Arc<ServerConfig>> {
    let mut certificate_chain = tls_certificates_from_pem(&path.join("cert.pem"))?;
    certificate_chain.extend(tls_certificates_from_pem(&path.join("chain.pem"))?);
    let private_key = tls_private_key_from_pem(&path.join("privkey.pem"))?;
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

    fn bundle_modified_time_for_member_seconds(seconds: [u64; 3]) -> Option<SystemTime> {
        tls_certificate_bundle_modified_time_with(Path::new("test-bundle"), |path| {
            let filename = path
                .file_name()
                .and_then(|filename| filename.to_str())
                .expect("test bundle member should have a UTF-8 filename");
            let index = TLS_REQUIRED_CERT_FILENAMES
                .iter()
                .position(|candidate| *candidate == filename)
                .expect("only required TLS bundle members should be inspected");
            Some(UNIX_EPOCH + std::time::Duration::from_secs(seconds[index]))
        })
    }

    #[test]
    fn bundle_modified_time_uses_the_latest_required_member() {
        assert_eq!(
            bundle_modified_time_for_member_seconds([11, 37, 23]),
            Some(UNIX_EPOCH + std::time::Duration::from_secs(37))
        );
        assert_eq!(
            bundle_modified_time_for_member_seconds([41, 17, 29]),
            Some(UNIX_EPOCH + std::time::Duration::from_secs(41))
        );
        assert_eq!(
            bundle_modified_time_for_member_seconds([13, 19, 43]),
            Some(UNIX_EPOCH + std::time::Duration::from_secs(43))
        );
    }

    #[test]
    #[should_panic(
        expected = "changing any required TLS bundle member must change the rotation token"
    )]
    fn known_defect_tls_bundle_member_change_below_latest_mtime_is_not_detected() {
        let before = bundle_modified_time_for_member_seconds([10, 30, 20]);
        let after_privkey_edit = bundle_modified_time_for_member_seconds([11, 30, 20]);

        assert_ne!(
            before, after_privkey_edit,
            "changing any required TLS bundle member must change the rotation token"
        );
    }
}
