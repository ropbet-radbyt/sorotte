use super::*;

pub(crate) fn tls_certificate_bundle_is_available(path: &Path) -> bool {
    TLS_REQUIRED_CERT_FILENAMES
        .iter()
        .all(|filename| path.join(filename).is_file())
}

pub(crate) fn tls_certificate_file_modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path.join(TLS_CERT_FILENAME))
        .ok()
        .and_then(|metadata| metadata.modified().ok())
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
