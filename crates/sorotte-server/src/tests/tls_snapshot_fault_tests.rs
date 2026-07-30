use super::*;
use sha2::{Digest as _, Sha256};

const TLS_FAULT_HISTORY_LENGTH: usize = 3;
const TLS_FAULT_STEP_COUNT: usize = 6;

#[derive(Debug, Clone, Copy)]
enum TlsBundleFaultStep {
    Stable,
    MissingPrivateKey,
    MissingCertificate,
    MissingChain,
    InvalidRevision,
    ValidRevision,
}

impl TlsBundleFaultStep {
    fn from_digit(digit: usize) -> Self {
        match digit {
            0 => Self::Stable,
            1 => Self::MissingPrivateKey,
            2 => Self::MissingCertificate,
            3 => Self::MissingChain,
            4 => Self::InvalidRevision,
            5 => Self::ValidRevision,
            _ => unreachable!("TLS bundle fault digit must be in range"),
        }
    }

    fn missing_filename(self) -> Option<&'static str> {
        match self {
            Self::MissingPrivateKey => Some("privkey.pem"),
            Self::MissingCertificate => Some("cert.pem"),
            Self::MissingChain => Some("chain.pem"),
            Self::Stable | Self::InvalidRevision | Self::ValidRevision => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TlsBundleObservation {
    Unavailable,
    Unchanged,
    Changed { valid: bool },
}

#[derive(Debug)]
struct TlsRotationModel {
    context_available: bool,
    accepts_tls: bool,
    attempts: u32,
}

impl TlsRotationModel {
    fn apply(&mut self, observation: TlsBundleObservation) -> bool {
        let accepted_before_observation = self.accepts_tls;
        if accepted_before_observation {
            match observation {
                TlsBundleObservation::Unavailable | TlsBundleObservation::Unchanged => {}
                TlsBundleObservation::Changed { valid } => {
                    self.context_available = valid;
                    self.attempts = self.attempts.saturating_add(1);
                    self.accepts_tls =
                        valid || self.attempts < super::super::TLS_CERT_ROTATION_MAX_RETRIES;
                }
            }
        }
        accepted_before_observation && self.context_available
    }
}

fn write_valid_tls_bundle_revision(path: &Path, revision: usize) {
    let trailing_whitespace = "\n".repeat(revision + 1);
    for (filename, pem) in [
        ("privkey.pem", TEST_TLS_PRIVATE_KEY_PEM),
        ("cert.pem", TEST_TLS_CERT_PEM),
        ("chain.pem", TEST_TLS_CHAIN_PEM),
    ] {
        fs::write(path.join(filename), format!("{pem}{trailing_whitespace}"))
            .expect("valid TLS bundle revision should write");
    }
}

fn write_invalid_tls_bundle_revision(path: &Path, revision: usize) {
    for filename in super::super::TLS_REQUIRED_CERT_FILENAMES {
        fs::write(
            path.join(filename),
            format!("invalid-tls-revision-{revision}-{filename}"),
        )
        .expect("invalid TLS bundle revision should write");
    }
}

fn apply_fault_step(
    path: &Path,
    step: TlsBundleFaultStep,
    complete_bundle: &mut bool,
    revision: &mut usize,
) -> TlsBundleObservation {
    if let Some(filename) = step.missing_filename() {
        let member_path = path.join(filename);
        if member_path.exists() {
            fs::remove_file(member_path).expect("selected TLS bundle member should be removable");
        }
        *complete_bundle = false;
        return TlsBundleObservation::Unavailable;
    }

    match step {
        TlsBundleFaultStep::Stable => {
            if *complete_bundle {
                TlsBundleObservation::Unchanged
            } else {
                TlsBundleObservation::Unavailable
            }
        }
        TlsBundleFaultStep::InvalidRevision => {
            *revision += 1;
            write_invalid_tls_bundle_revision(path, *revision);
            *complete_bundle = true;
            TlsBundleObservation::Changed { valid: false }
        }
        TlsBundleFaultStep::ValidRevision => {
            *revision += 1;
            write_valid_tls_bundle_revision(path, *revision);
            *complete_bundle = true;
            TlsBundleObservation::Changed { valid: true }
        }
        TlsBundleFaultStep::MissingPrivateKey
        | TlsBundleFaultStep::MissingCertificate
        | TlsBundleFaultStep::MissingChain => {
            unreachable!("missing-member steps return before the complete-bundle match")
        }
    }
}

#[test]
fn tls_rotation_real_filesystem_fault_histories_match_reference_model_without_sleeps() {
    let history_count = TLS_FAULT_STEP_COUNT.pow(TLS_FAULT_HISTORY_LENGTH as u32);
    let cert_path = temporary_directory_path("tls-snapshot-fault-histories");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("TLS fault-history directory should be creatable");

    for encoded_history in 0..history_count {
        write_valid_tls_bundle_revision(&cert_path, 0);
        let mut runtime = ServerRuntime::new();
        runtime.set_tls_cert_path(Some(cert_path.clone()));
        let mut model = TlsRotationModel {
            context_available: true,
            accepts_tls: true,
            attempts: 0,
        };
        let mut complete_bundle = true;
        let mut revision = 0;
        let mut remaining_history = encoded_history;

        for step_index in 0..TLS_FAULT_HISTORY_LENGTH {
            let step = TlsBundleFaultStep::from_digit(remaining_history % TLS_FAULT_STEP_COUNT);
            remaining_history /= TLS_FAULT_STEP_COUNT;
            let observation =
                apply_fault_step(&cert_path, step, &mut complete_bundle, &mut revision);
            let fingerprint_before = runtime.tls_certificate_bundle_fingerprint;
            let expected_start_tls = model.apply(observation);
            let client_id = format!("fault-history-{encoded_history}-step-{step_index}");

            let outbound_lines = runtime
                .handle_line(&client_id, r#"{"TLS":{"startTLS":"send"}}"#)
                .expect("generated TLS fault request should be handled");

            assert_eq!(
                tls_start_response(&outbound_lines).as_deref(),
                Some(if expected_start_tls { "true" } else { "false" }),
                "STARTTLS response diverged for history {encoded_history}, step {step_index}: {step:?}"
            );
            assert_eq!(
                has_start_tls_transport_action(&runtime.drain_transport_actions(), &client_id,),
                expected_start_tls,
                "transport action diverged for history {encoded_history}, step {step_index}: {step:?}"
            );
            assert_eq!(
                runtime.tls_context_available, model.context_available,
                "context availability diverged for history {encoded_history}, step {step_index}: {step:?}"
            );
            assert_eq!(
                runtime.server_accepts_tls, model.accepts_tls,
                "acceptability diverged for history {encoded_history}, step {step_index}: {step:?}"
            );
            assert_eq!(
                runtime.tls_rotation_attempts, model.attempts,
                "retry count diverged for history {encoded_history}, step {step_index}: {step:?}"
            );

            let should_change_fingerprint =
                matches!(observation, TlsBundleObservation::Changed { .. })
                    && fingerprint_before.is_some()
                    && model.attempts > 0;
            if should_change_fingerprint {
                assert_ne!(
                    runtime.tls_certificate_bundle_fingerprint, fingerprint_before,
                    "a complete changed revision must advance the observed fingerprint for history {encoded_history}, step {step_index}: {step:?}"
                );
            } else {
                assert_eq!(
                    runtime.tls_certificate_bundle_fingerprint, fingerprint_before,
                    "unavailable, unchanged, or gated observations must retain the fingerprint for history {encoded_history}, step {step_index}: {step:?}"
                );
            }
        }
    }

    fs::remove_dir_all(&cert_path).expect("TLS fault-history directory should be removable");
}

fn equal_length_corruption(contents: &[u8]) -> Vec<u8> {
    let mut corrupted = contents.to_vec();
    let first_non_whitespace = corrupted
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .expect("TLS fixture should contain a non-whitespace byte");
    corrupted[first_non_whitespace] = if corrupted[first_non_whitespace] == b'!' {
        b'?'
    } else {
        b'!'
    };
    assert_eq!(corrupted.len(), contents.len());
    corrupted
}

#[test]
fn tls_rotation_detects_equal_length_edits_to_every_member_with_colliding_mtimes() {
    let collision_time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);

    for filename in super::super::TLS_REQUIRED_CERT_FILENAMES {
        let cert_path = temporary_directory_path(&format!("tls-equal-length-mtime-{filename}"));
        let _ = fs::remove_dir_all(&cert_path);
        fs::create_dir_all(&cert_path).expect("TLS collision directory should be creatable");
        write_valid_tls_bundle(&cert_path);
        for member in super::super::TLS_REQUIRED_CERT_FILENAMES {
            set_file_modified_time_for_test(&cert_path.join(member), collision_time);
        }

        let mut runtime = ServerRuntime::new();
        runtime.set_tls_cert_path(Some(cert_path.clone()));
        let fingerprint_before = runtime
            .tls_certificate_bundle_fingerprint
            .expect("valid initial bundle should have a fingerprint");
        let member_path = cert_path.join(filename);
        let original = fs::read(&member_path).expect("TLS fixture member should be readable");
        let corrupted = equal_length_corruption(&original);
        fs::write(&member_path, &corrupted).expect("equal-length replacement should write");
        set_file_modified_time_for_test(&member_path, collision_time);

        assert_eq!(
            fs::metadata(&member_path)
                .expect("replaced TLS member should have metadata")
                .len(),
            u64::try_from(original.len()).expect("fixture length should fit u64"),
            "the {filename} fault must preserve file length"
        );
        let fingerprint_after = tls_certificate_bundle_fingerprint(&cert_path)
            .expect("complete corrupted bundle should still be fingerprintable");
        assert_ne!(
            fingerprint_after, fingerprint_before,
            "content identity must detect the equal-length {filename} edit"
        );

        let outbound_lines = runtime
            .handle_line(filename, r#"{"TLS":{"startTLS":"send"}}"#)
            .expect("TLS collision request should be handled");
        assert_eq!(
            tls_start_response(&outbound_lines).as_deref(),
            Some("false"),
            "the equal-length {filename} edit must invalidate the loaded context"
        );
        assert_eq!(runtime.tls_rotation_attempts, 1);
        assert!(!runtime.tls_context_available);

        fs::remove_dir_all(&cert_path).expect("TLS collision directory should be removable");
    }
}

#[test]
fn tls_rotation_retry_exhaustion_is_terminal_for_later_valid_reappearance() {
    let cert_path = temporary_directory_path("tls-real-fingerprint-retry-exhaustion");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("TLS retry directory should be creatable");
    write_valid_tls_bundle_revision(&cert_path, 0);

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
    for attempt in 1..=super::super::TLS_CERT_ROTATION_MAX_RETRIES {
        write_invalid_tls_bundle_revision(
            &cert_path,
            usize::try_from(attempt).expect("retry count should fit usize"),
        );
        let outbound_lines = runtime
            .handle_line(
                &format!("invalid-attempt-{attempt}"),
                r#"{"TLS":{"startTLS":"send"}}"#,
            )
            .expect("invalid TLS revision should be handled");
        assert_eq!(
            tls_start_response(&outbound_lines).as_deref(),
            Some("false")
        );
        assert_eq!(runtime.tls_rotation_attempts, attempt);
        assert_eq!(
            runtime.server_accepts_tls,
            attempt < super::super::TLS_CERT_ROTATION_MAX_RETRIES
        );
    }

    let terminal_fingerprint = runtime.tls_certificate_bundle_fingerprint;
    write_valid_tls_bundle_revision(&cert_path, 100);
    let outbound_lines = runtime
        .handle_line("valid-after-cap", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("post-cap TLS request should be handled");
    assert_eq!(
        tls_start_response(&outbound_lines).as_deref(),
        Some("false")
    );
    assert_eq!(
        runtime.tls_certificate_bundle_fingerprint, terminal_fingerprint,
        "the terminal acceptability gate must prevent further rotation observations"
    );
    assert_eq!(
        runtime.tls_rotation_attempts,
        super::super::TLS_CERT_ROTATION_MAX_RETRIES
    );

    fs::remove_dir_all(&cert_path).expect("TLS retry directory should be removable");
}

#[test]
fn captured_tls_snapshot_remains_loadable_after_files_are_replaced() {
    let cert_path = temporary_directory_path("tls-captured-snapshot-replacement");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("TLS snapshot directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let snapshot = read_tls_certificate_bundle_snapshot(&cert_path)
        .expect("valid TLS snapshot should be captured");
    let captured_fingerprint = snapshot.fingerprint();
    write_invalid_tls_bundle_revision(&cert_path, 1);

    assert_ne!(
        tls_certificate_bundle_fingerprint(&cert_path),
        Some(captured_fingerprint),
        "the on-disk bundle should differ after replacement"
    );
    load_tls_server_config_from_snapshot(&cert_path, &snapshot)
        .expect("rustls must parse the captured bytes rather than rereading replaced members");

    fs::remove_dir_all(&cert_path).expect("TLS snapshot directory should be removable");
}

fn mutate_certificate_signature(pem: &str) -> Vec<u8> {
    let mut mutated = pem.as_bytes().to_vec();
    let end_marker = b"-----END CERTIFICATE-----";
    let end = mutated
        .windows(end_marker.len())
        .position(|window| window == end_marker)
        .expect("certificate fixture should contain an end marker");
    let signature_byte = mutated[..end]
        .iter()
        .rposition(|byte| byte.is_ascii_alphanumeric())
        .expect("certificate fixture should contain base64 signature data");
    mutated[signature_byte] = if mutated[signature_byte] == b'A' {
        b'B'
    } else {
        b'A'
    };
    mutated
}

fn tls_generation_a() -> [Vec<u8>; 3] {
    [
        TEST_TLS_PRIVATE_KEY_PEM.as_bytes().to_vec(),
        TEST_TLS_CERT_PEM.as_bytes().to_vec(),
        TEST_TLS_CHAIN_PEM.as_bytes().to_vec(),
    ]
}

fn tls_generation_b() -> [Vec<u8>; 3] {
    let mut private_key = TEST_TLS_PRIVATE_KEY_PEM.as_bytes().to_vec();
    private_key.push(b'\n');
    [
        private_key,
        mutate_certificate_signature(TEST_TLS_CERT_PEM),
        mutate_certificate_signature(TEST_TLS_CHAIN_PEM),
    ]
}

fn snapshot_from_generation(
    path: &Path,
    generation: &[Vec<u8>; 3],
) -> super::super::TlsCertificateBundleSnapshot {
    read_tls_certificate_bundle_snapshot_with_test_reader(path, |member_path| {
        let filename = member_path
            .file_name()
            .and_then(|filename| filename.to_str())
            .expect("scheduled TLS member should have a UTF-8 filename");
        let index = super::super::TLS_REQUIRED_CERT_FILENAMES
            .iter()
            .position(|candidate| *candidate == filename)
            .expect("reader should request only required TLS members");
        Ok(generation[index].clone())
    })
    .expect("scheduled TLS generation should be readable")
}

fn test_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn generation_manifest_bytes(generation_id: &str, generation: &[Vec<u8>; 3]) -> Vec<u8> {
    let members = super::super::TLS_REQUIRED_CERT_FILENAMES
        .into_iter()
        .zip(generation)
        .map(|(filename, contents)| {
            (
                filename.to_owned(),
                json!({
                    "length": contents.len(),
                    "sha256": test_sha256(contents),
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    serde_json::to_vec(&json!({
        "schema": "sorotte-tls-bundle-v1",
        "generation": generation_id,
        "members": members,
    }))
    .expect("TLS generation manifest should serialize")
}

fn write_immutable_generation(
    root: &Path,
    generation_id: &str,
    generation: &[Vec<u8>; 3],
) -> Vec<u8> {
    let generation_root = root.join("generations").join(generation_id);
    fs::create_dir_all(&generation_root).expect("TLS generation directory should be creatable");
    for (filename, contents) in super::super::TLS_REQUIRED_CERT_FILENAMES
        .into_iter()
        .zip(generation)
    {
        fs::write(generation_root.join(filename), contents)
            .expect("TLS generation member should be writable");
    }
    generation_manifest_bytes(generation_id, generation)
}

fn select_generation(root: &Path, manifest: &[u8]) {
    fs::write(root.join("current.json"), manifest)
        .expect("TLS current-generation manifest should be replaceable");
}

#[test]
fn loose_snapshot_double_capture_rejects_cross_generation_read_boundaries() {
    let virtual_path = Path::new("scheduled-tls-bundle");
    let generation_a = tls_generation_a();
    let generation_b = tls_generation_b();
    let snapshot_a = snapshot_from_generation(virtual_path, &generation_a);
    let snapshot_b = snapshot_from_generation(virtual_path, &generation_b);
    load_tls_server_config_from_snapshot(virtual_path, &snapshot_a)
        .expect("generation A must be independently valid");
    load_tls_server_config_from_snapshot(virtual_path, &snapshot_b)
        .expect("generation B must be independently valid");
    let complete_fingerprints = [snapshot_a.fingerprint(), snapshot_b.fingerprint()];
    assert_ne!(
        complete_fingerprints[0], complete_fingerprints[1],
        "the injected generations must have distinct byte identities"
    );

    for replacement_after_read in 1..=2 {
        let mut reads = 0;
        let stable_snapshot =
            read_tls_certificate_bundle_snapshot_with_test_reader(virtual_path, |member_path| {
                let filename = member_path
                    .file_name()
                    .and_then(|filename| filename.to_str())
                    .expect("scheduled TLS member should have a UTF-8 filename");
                let index = super::super::TLS_REQUIRED_CERT_FILENAMES
                    .iter()
                    .position(|candidate| *candidate == filename)
                    .expect("reader should request only required TLS members");
                reads += 1;
                let generation = if reads <= replacement_after_read {
                    &generation_a
                } else {
                    &generation_b
                };
                Ok(generation[index].clone())
            })
            .expect("a later stable generation should be captured after the mixed observation");

        assert_eq!(
            stable_snapshot.fingerprint(),
            snapshot_b.fingerprint(),
            "boundary {replacement_after_read} must converge on complete generation B"
        );
        assert!(
            complete_fingerprints.contains(&stable_snapshot.fingerprint()),
            "boundary {replacement_after_read} installed a cross-generation fingerprint"
        );
        assert!(
            reads >= 12,
            "boundary {replacement_after_read} must reject the first mixed capture before accepting two stable captures"
        );
        load_tls_server_config_from_snapshot(virtual_path, &stable_snapshot)
            .expect("the stable complete generation must remain rustls-loadable");
    }
}

#[test]
fn atomic_manifest_switch_at_every_read_boundary_installs_only_complete_generation() {
    let root = temporary_directory_path("tls-atomic-selector-boundaries");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("generations")).expect("TLS generations root should be creatable");
    let generation_a = tls_generation_a();
    let generation_b = tls_generation_b();
    let manifest_a = write_immutable_generation(&root, "generation-A", &generation_a);
    let manifest_b = write_immutable_generation(&root, "generation-B", &generation_b);
    select_generation(&root, &manifest_a);
    let snapshot_a = read_tls_certificate_bundle_snapshot(&root).expect("generation A should load");
    select_generation(&root, &manifest_b);
    let snapshot_b = read_tls_certificate_bundle_snapshot(&root).expect("generation B should load");
    let complete_fingerprints = [snapshot_a.fingerprint(), snapshot_b.fingerprint()];
    assert_ne!(complete_fingerprints[0], complete_fingerprints[1]);

    for replacement_after_read in 1..=3 {
        select_generation(&root, &manifest_a);
        let mut switched = false;
        let snapshot =
            read_tls_certificate_bundle_snapshot_with_test_hook(&root, |member_index, _| {
                if !switched && member_index == replacement_after_read {
                    select_generation(&root, &manifest_b);
                    switched = true;
                }
            })
            .expect("selector replacement should converge on one immutable generation");

        assert!(switched, "boundary hook {replacement_after_read} must run");
        assert_eq!(
            snapshot.fingerprint(),
            snapshot_b.fingerprint(),
            "selector replacement after member {replacement_after_read} must retry generation B"
        );
        assert!(
            complete_fingerprints.contains(&snapshot.fingerprint()),
            "selector replacement after member {replacement_after_read} installed mixed bytes"
        );
        load_tls_server_config_from_snapshot(&root, &snapshot)
            .expect("the selected complete generation must remain rustls-loadable");
    }

    fs::remove_dir_all(&root).expect("TLS atomic-selector directory should be removable");
}

#[test]
fn interrupted_atomic_publication_keeps_old_generation_until_selector_switch() {
    let root = temporary_directory_path("tls-atomic-publication-interruption");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("generations")).expect("TLS generations root should be creatable");
    let generation_a = tls_generation_a();
    let generation_b = tls_generation_b();
    let manifest_a = write_immutable_generation(&root, "generation-A", &generation_a);
    select_generation(&root, &manifest_a);
    let snapshot_a = read_tls_certificate_bundle_snapshot(&root).expect("generation A should load");

    let staged_root = root.join("generations").join("unpublished-B");
    fs::create_dir(&staged_root).expect("unpublished generation should be staged");
    fs::write(staged_root.join("privkey.pem"), &generation_b[0])
        .expect("partial generation should write");
    assert_eq!(
        read_tls_certificate_bundle_snapshot(&root)
            .expect("an unreferenced partial generation must be invisible")
            .fingerprint(),
        snapshot_a.fingerprint()
    );

    fs::remove_dir_all(&staged_root).expect("partial staging directory should be removable");
    let manifest_b = write_immutable_generation(&root, "generation-B", &generation_b);
    assert_eq!(
        read_tls_certificate_bundle_snapshot(&root)
            .expect("complete but unselected generation B must remain invisible")
            .fingerprint(),
        snapshot_a.fingerprint()
    );

    select_generation(&root, &manifest_b);
    let snapshot_b =
        read_tls_certificate_bundle_snapshot(&root).expect("selected generation B should load");
    assert_ne!(snapshot_b.fingerprint(), snapshot_a.fingerprint());
    load_tls_server_config_from_snapshot(&root, &snapshot_b)
        .expect("selected generation B should be rustls-loadable");

    fs::remove_dir_all(&root).expect("TLS publication-interruption directory should be removable");
}

#[test]
fn unavailable_selected_generation_retains_active_context_without_consuming_rotation_retry() {
    let root = temporary_directory_path("tls-atomic-unavailable-selected-generation");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("generations")).expect("TLS generations root should be creatable");
    let generation_a = tls_generation_a();
    let generation_b = tls_generation_b();
    let manifest_a = write_immutable_generation(&root, "generation-A", &generation_a);
    let manifest_b = write_immutable_generation(&root, "generation-B", &generation_b);
    select_generation(&root, &manifest_a);

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(root.clone()));
    let fingerprint_a = runtime
        .tls_certificate_bundle_fingerprint
        .expect("generation A should initialize the TLS context");
    assert!(runtime.tls_context_available);
    assert!(runtime.server_accepts_tls);
    assert_eq!(runtime.tls_rotation_attempts, 0);

    let unavailable_member = root
        .join("generations")
        .join("generation-B")
        .join("cert.pem");
    fs::remove_file(&unavailable_member)
        .expect("generation B certificate should be removable for the fault");
    select_generation(&root, &manifest_b);

    let outbound = runtime
        .handle_line("unavailable-generation", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("an unavailable selected generation should retain the active context");
    assert_eq!(tls_start_response(&outbound).as_deref(), Some("true"));
    assert!(has_start_tls_transport_action(
        &runtime.drain_transport_actions(),
        "unavailable-generation"
    ));
    assert_eq!(
        runtime.tls_certificate_bundle_fingerprint,
        Some(fingerprint_a),
        "an incomplete selected generation must not replace the active identity"
    );
    assert!(runtime.tls_context_available);
    assert!(runtime.server_accepts_tls);
    assert_eq!(
        runtime.tls_rotation_attempts, 0,
        "capture instability must not consume the invalid-generation retry budget"
    );

    fs::write(&unavailable_member, &generation_b[1])
        .expect("the immutable generation member should be restored for recovery");
    let outbound = runtime
        .handle_line("recovered-generation", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("the complete selected generation should become active");
    assert_eq!(tls_start_response(&outbound).as_deref(), Some("true"));
    assert!(has_start_tls_transport_action(
        &runtime.drain_transport_actions(),
        "recovered-generation"
    ));
    assert_ne!(
        runtime.tls_certificate_bundle_fingerprint,
        Some(fingerprint_a)
    );
    assert!(runtime.tls_context_available);
    assert!(runtime.server_accepts_tls);
    assert_eq!(runtime.tls_rotation_attempts, 1);

    fs::remove_dir_all(&root).expect("TLS unavailable-generation directory should be removable");
}

#[test]
fn atomic_manifest_rejects_path_escape_digest_drift_and_duplicate_fields() {
    let root = temporary_directory_path("tls-atomic-manifest-adversarial");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("generations")).expect("TLS generations root should be creatable");
    let generation = tls_generation_a();
    let valid_manifest = write_immutable_generation(&root, "generation-A", &generation);

    let mut path_escape: Value =
        serde_json::from_slice(&valid_manifest).expect("valid manifest should parse in test");
    path_escape["generation"] = json!("../outside");
    select_generation(
        &root,
        &serde_json::to_vec(&path_escape).expect("path-escape manifest should serialize"),
    );
    let error = read_tls_certificate_bundle_snapshot(&root)
        .expect_err("generation traversal must fail closed");
    assert!(error.to_string().contains("generation"));

    let mut digest_drift: Value =
        serde_json::from_slice(&valid_manifest).expect("valid manifest should parse in test");
    digest_drift["members"]["cert.pem"]["sha256"] = json!("0".repeat(64));
    select_generation(
        &root,
        &serde_json::to_vec(&digest_drift).expect("digest-drift manifest should serialize"),
    );
    let error = read_tls_certificate_bundle_snapshot(&root)
        .expect_err("member digest drift must fail closed");
    assert!(error.to_string().contains("SHA-256 mismatch"));

    let members = &serde_json::from_slice::<Value>(&valid_manifest)
        .expect("valid manifest should parse in test")["members"];
    let duplicate = format!(
        r#"{{"schema":"sorotte-tls-bundle-v1","schema":"sorotte-tls-bundle-v1","generation":"generation-A","members":{members}}}"#
    );
    select_generation(&root, duplicate.as_bytes());
    let error = read_tls_certificate_bundle_snapshot(&root)
        .expect_err("duplicate manifest fields must fail closed");
    assert!(error.to_string().contains("duplicate field"));

    fs::remove_dir_all(&root).expect("TLS adversarial-manifest directory should be removable");
}
