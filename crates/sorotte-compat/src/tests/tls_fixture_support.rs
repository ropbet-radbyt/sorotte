use super::*;

pub(super) const TEST_TLS_CERT_PEM: &str = include_str!("../../../../fixtures/tls/test_cert.pem");
pub(super) const TEST_TLS_CHAIN_PEM: &str = include_str!("../../../../fixtures/tls/test_chain.pem");
pub(super) const TEST_TLS_PRIVATE_KEY_PEM: &str =
    include_str!("../../../../fixtures/tls/test_privkey.pem");

pub(super) fn temporary_tls_directory_path(label: &str) -> PathBuf {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sorotte-compat-{label}-{}-{unique_suffix}",
        process::id()
    ))
}

pub(super) fn write_valid_tls_bundle(path: &Path) {
    fs::write(path.join("privkey.pem"), TEST_TLS_PRIVATE_KEY_PEM)
        .expect("valid private key fixture should write");
    fs::write(path.join("cert.pem"), TEST_TLS_CERT_PEM)
        .expect("valid certificate fixture should write");
    fs::write(path.join("chain.pem"), TEST_TLS_CHAIN_PEM)
        .expect("valid chain fixture should write");
}
