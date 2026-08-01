use super::*;
use serde::Deserialize;
use std::io::Read as _;

const TLS_BUNDLE_CURRENT_MANIFEST_FILENAME: &str = "current.json";
const TLS_BUNDLE_GENERATIONS_DIRECTORY: &str = "generations";
const TLS_BUNDLE_MANIFEST_SCHEMA: &str = "sorotte-tls-bundle-v1";
const TLS_BUNDLE_MANIFEST_MAX_BYTES: u64 = 16 * 1024;
pub(crate) const TLS_BUNDLE_MEMBER_MAX_BYTES: u64 = 4 * 1024 * 1024;
const TLS_BUNDLE_GENERATION_MAX_BYTES: usize = 128;
const TLS_BUNDLE_CAPTURE_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TlsCertificateBundleFingerprint([u8; 32]);

#[derive(Debug)]
pub(crate) struct TlsCertificateBundleSnapshot {
    member_root: PathBuf,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TlsBundleCurrentManifest {
    schema: String,
    generation: String,
    members: TlsBundleManifestMembers,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TlsBundleManifestMembers {
    #[serde(rename = "privkey.pem")]
    private_key: TlsBundleManifestMember,
    #[serde(rename = "cert.pem")]
    certificate: TlsBundleManifestMember,
    #[serde(rename = "chain.pem")]
    chain: TlsBundleManifestMember,
}

impl TlsBundleManifestMembers {
    fn ordered(&self) -> [(&'static str, &TlsBundleManifestMember); 3] {
        [
            (TLS_REQUIRED_CERT_FILENAMES[0], &self.private_key),
            (TLS_REQUIRED_CERT_FILENAMES[1], &self.certificate),
            (TLS_REQUIRED_CERT_FILENAMES[2], &self.chain),
        ]
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TlsBundleManifestMember {
    length: u64,
    sha256: String,
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

fn invalid_tls_bundle(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn unstable_tls_bundle(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, message.into())
}

fn metadata_is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn require_plain_directory(path: &Path, role: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata_is_link_or_reparse_point(&metadata) {
        return Err(invalid_tls_bundle(format!(
            "{role} '{}' must be a plain directory, not a link or reparse point",
            path.display()
        )));
    }
    Ok(())
}

fn read_plain_file_bounded(path: &Path, role: &str, max_bytes: u64) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata_is_link_or_reparse_point(&metadata) {
        return Err(invalid_tls_bundle(format!(
            "{role} '{}' must be a plain regular file, not a link or reparse point",
            path.display()
        )));
    }
    if metadata.len() > max_bytes {
        return Err(invalid_tls_bundle(format!(
            "{role} '{}' is {} bytes, exceeding the {max_bytes}-byte limit",
            path.display(),
            metadata.len()
        )));
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len()).expect("bounded TLS file length must fit usize"),
    );
    fs::File::open(path)?
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).expect("TLS file length must fit u64") > max_bytes {
        return Err(invalid_tls_bundle(format!(
            "{role} '{}' changed beyond the {max_bytes}-byte limit while being read",
            path.display()
        )));
    }
    Ok(bytes)
}

fn read_followed_file_bounded(path: &Path, role: &str, max_bytes: u64) -> io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(invalid_tls_bundle(format!(
            "{role} '{}' must resolve to a regular file",
            path.display()
        )));
    }
    if before.len() > max_bytes {
        return Err(invalid_tls_bundle(format!(
            "{role} '{}' is {} bytes, exceeding the {max_bytes}-byte limit",
            path.display(),
            before.len()
        )));
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(before.len()).expect("bounded TLS file length must fit usize"),
    );
    (&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).expect("TLS file length must fit u64") > max_bytes {
        return Err(invalid_tls_bundle(format!(
            "{role} '{}' changed beyond the {max_bytes}-byte limit while being read",
            path.display()
        )));
    }
    let after = file.metadata()?;
    if !after.is_file() || after.len() != before.len() {
        return Err(unstable_tls_bundle(format!(
            "{role} '{}' changed metadata while being read",
            path.display()
        )));
    }
    Ok(bytes)
}

fn read_current_manifest_bytes_if_present(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let manifest_path = path.join(TLS_BUNDLE_CURRENT_MANIFEST_FILENAME);
    match fs::symlink_metadata(&manifest_path) {
        Ok(_) => read_plain_file_bounded(
            &manifest_path,
            "TLS current-generation manifest",
            TLS_BUNDLE_MANIFEST_MAX_BYTES,
        )
        .map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn validate_manifest_generation(generation: &str) -> io::Result<()> {
    let bytes = generation.as_bytes();
    if bytes.is_empty()
        || bytes.len() > TLS_BUNDLE_GENERATION_MAX_BYTES
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes[bytes.len() - 1].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(invalid_tls_bundle(format!(
            "TLS bundle generation {generation:?} must be 1-{TLS_BUNDLE_GENERATION_MAX_BYTES} \
             ASCII letters, digits, '-' or '_', beginning and ending with a letter or digit"
        )));
    }
    Ok(())
}

fn validate_manifest_member(filename: &str, member: &TlsBundleManifestMember) -> io::Result<()> {
    if member.length > TLS_BUNDLE_MEMBER_MAX_BYTES {
        return Err(invalid_tls_bundle(format!(
            "TLS manifest member {filename} declares {} bytes, exceeding the \
             {TLS_BUNDLE_MEMBER_MAX_BYTES}-byte limit",
            member.length
        )));
    }
    if member.sha256.len() != 64
        || !member
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(invalid_tls_bundle(format!(
            "TLS manifest member {filename} has a non-canonical SHA-256 digest"
        )));
    }
    Ok(())
}

fn parse_current_manifest(bytes: &[u8]) -> io::Result<TlsBundleCurrentManifest> {
    let manifest = serde_json::from_slice::<TlsBundleCurrentManifest>(bytes).map_err(|error| {
        invalid_tls_bundle(format!(
            "TLS current-generation manifest is invalid JSON: {error}"
        ))
    })?;
    if manifest.schema != TLS_BUNDLE_MANIFEST_SCHEMA {
        return Err(invalid_tls_bundle(format!(
            "unsupported TLS bundle manifest schema {:?}; expected {TLS_BUNDLE_MANIFEST_SCHEMA:?}",
            manifest.schema
        )));
    }
    validate_manifest_generation(&manifest.generation)?;
    for (filename, member) in manifest.members.ordered() {
        validate_manifest_member(filename, member)?;
    }
    Ok(manifest)
}

fn snapshot_from_members(
    member_root: PathBuf,
    private_key_pem: Vec<u8>,
    certificate_pem: Vec<u8>,
    chain_pem: Vec<u8>,
) -> TlsCertificateBundleSnapshot {
    let fingerprint = fingerprint_tls_certificate_bundle_members([
        (TLS_REQUIRED_CERT_FILENAMES[0], &private_key_pem),
        (TLS_REQUIRED_CERT_FILENAMES[1], &certificate_pem),
        (TLS_REQUIRED_CERT_FILENAMES[2], &chain_pem),
    ]);
    TlsCertificateBundleSnapshot {
        member_root,
        private_key_pem,
        certificate_pem,
        chain_pem,
        fingerprint,
    }
}

fn read_loose_bundle_once_with(
    path: &Path,
    read_member: &mut impl FnMut(&Path) -> io::Result<Vec<u8>>,
) -> io::Result<TlsCertificateBundleSnapshot> {
    let [private_key_filename, certificate_filename, chain_filename] = TLS_REQUIRED_CERT_FILENAMES;
    let private_key_pem = read_member(&path.join(private_key_filename))?;
    let certificate_pem = read_member(&path.join(certificate_filename))?;
    let chain_pem = read_member(&path.join(chain_filename))?;
    Ok(snapshot_from_members(
        path.to_path_buf(),
        private_key_pem,
        certificate_pem,
        chain_pem,
    ))
}

fn read_stable_loose_bundle_snapshot_with(
    path: &Path,
    mut read_member: impl FnMut(&Path) -> io::Result<Vec<u8>>,
) -> io::Result<TlsCertificateBundleSnapshot> {
    for _ in 0..TLS_BUNDLE_CAPTURE_ATTEMPTS {
        let first = read_loose_bundle_once_with(path, &mut read_member)?;
        let second = read_loose_bundle_once_with(path, &mut read_member)?;
        if first.fingerprint() == second.fingerprint() {
            return Ok(second);
        }
    }
    Err(unstable_tls_bundle(format!(
        "loose TLS bundle '{}' did not produce two identical consecutive captures; \
         publish immutable generations through {TLS_BUNDLE_CURRENT_MANIFEST_FILENAME}",
        path.display()
    )))
}

fn read_manifest_generation_snapshot_with_hook(
    path: &Path,
    manifest: &TlsBundleCurrentManifest,
    after_member_read: &mut impl FnMut(usize, &Path),
) -> io::Result<TlsCertificateBundleSnapshot> {
    let generations_root = path.join(TLS_BUNDLE_GENERATIONS_DIRECTORY);
    require_plain_directory(&generations_root, "TLS generations root")?;
    let member_root = generations_root.join(&manifest.generation);
    require_plain_directory(&member_root, "TLS generation")?;
    let mut members = Vec::with_capacity(TLS_REQUIRED_CERT_FILENAMES.len());
    for (index, (filename, expected)) in manifest.members.ordered().into_iter().enumerate() {
        let member_path = member_root.join(filename);
        let contents =
            read_plain_file_bounded(&member_path, "TLS generation member", expected.length)?;
        after_member_read(index + 1, &member_path);
        let actual_length =
            u64::try_from(contents.len()).expect("bounded TLS member length must fit u64");
        if actual_length != expected.length {
            return Err(invalid_tls_bundle(format!(
                "TLS generation member '{}' length mismatch: manifest {}, captured {actual_length}",
                member_path.display(),
                expected.length
            )));
        }
        let actual_sha256 = lowercase_hex(Sha256::digest(&contents));
        if actual_sha256 != expected.sha256 {
            return Err(invalid_tls_bundle(format!(
                "TLS generation member '{}' SHA-256 mismatch: manifest {}, captured {actual_sha256}",
                member_path.display(),
                expected.sha256
            )));
        }
        members.push(contents);
    }
    let [private_key_pem, certificate_pem, chain_pem]: [Vec<u8>; 3] = members
        .try_into()
        .expect("TLS manifest requires exactly three ordered members");
    Ok(snapshot_from_members(
        member_root,
        private_key_pem,
        certificate_pem,
        chain_pem,
    ))
}

fn read_manifest_bundle_snapshot_with_hook(
    path: &Path,
    mut after_member_read: impl FnMut(usize, &Path),
) -> io::Result<TlsCertificateBundleSnapshot> {
    require_plain_directory(path, "TLS bundle root")?;
    for _ in 0..TLS_BUNDLE_CAPTURE_ATTEMPTS {
        let before = read_current_manifest_bytes_if_present(path)?.ok_or_else(|| {
            unstable_tls_bundle(format!(
                "TLS current-generation manifest '{}' disappeared during capture",
                path.join(TLS_BUNDLE_CURRENT_MANIFEST_FILENAME).display()
            ))
        })?;
        let manifest = parse_current_manifest(&before)?;
        let snapshot =
            read_manifest_generation_snapshot_with_hook(path, &manifest, &mut after_member_read);
        let after = read_current_manifest_bytes_if_present(path)?;
        if after.as_deref() != Some(before.as_slice()) {
            continue;
        }
        return snapshot;
    }
    Err(unstable_tls_bundle(format!(
        "TLS current-generation manifest '{}' changed during all {TLS_BUNDLE_CAPTURE_ATTEMPTS} \
         capture attempts",
        path.join(TLS_BUNDLE_CURRENT_MANIFEST_FILENAME).display()
    )))
}

pub(crate) fn read_tls_certificate_bundle_snapshot(
    path: &Path,
) -> io::Result<TlsCertificateBundleSnapshot> {
    if read_current_manifest_bytes_if_present(path)?.is_some() {
        return read_manifest_bundle_snapshot_with_hook(path, |_, _| {});
    }
    let snapshot = read_stable_loose_bundle_snapshot_with(path, |member_path| {
        read_followed_file_bounded(
            member_path,
            "loose TLS bundle member",
            TLS_BUNDLE_MEMBER_MAX_BYTES,
        )
    })?;
    if read_current_manifest_bytes_if_present(path)?.is_some() {
        return read_manifest_bundle_snapshot_with_hook(path, |_, _| {});
    }
    Ok(snapshot)
}

#[cfg(test)]
pub(crate) fn read_tls_certificate_bundle_snapshot_with_test_reader(
    path: &Path,
    read_member: impl FnMut(&Path) -> io::Result<Vec<u8>>,
) -> io::Result<TlsCertificateBundleSnapshot> {
    read_stable_loose_bundle_snapshot_with(path, read_member)
}

#[cfg(test)]
pub(crate) fn read_tls_certificate_bundle_snapshot_with_test_hook(
    path: &Path,
    after_member_read: impl FnMut(usize, &Path),
) -> io::Result<TlsCertificateBundleSnapshot> {
    read_manifest_bundle_snapshot_with_hook(path, after_member_read)
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
    _path: &Path,
    snapshot: &TlsCertificateBundleSnapshot,
) -> io::Result<Arc<ServerConfig>> {
    let mut certificate_chain = tls_certificates_from_pem(
        &snapshot.certificate_pem,
        &snapshot.member_root.join("cert.pem"),
    )?;
    certificate_chain.extend(tls_certificates_from_pem(
        &snapshot.chain_pem,
        &snapshot.member_root.join("chain.pem"),
    )?);
    let private_key = tls_private_key_from_pem(
        &snapshot.private_key_pem,
        &snapshot.member_root.join("privkey.pem"),
    )?;
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
