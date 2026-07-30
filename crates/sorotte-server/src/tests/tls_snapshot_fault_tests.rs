use super::*;

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

#[test]
#[should_panic(
    expected = "rustls must never install a TLS bundle assembled from multiple observed generations"
)]
fn known_defect_tls_snapshot_can_mix_members_replaced_during_observation() {
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
    let mut accepted_mixed_boundaries = Vec::new();

    for replacement_after_read in 1..=2 {
        let mut reads = 0;
        let mixed_snapshot =
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
            .expect("cross-generation read schedule should return all members");

        let mixed_load = load_tls_server_config_from_snapshot(virtual_path, &mixed_snapshot);
        if mixed_load.is_ok() && !complete_fingerprints.contains(&mixed_snapshot.fingerprint()) {
            accepted_mixed_boundaries.push(replacement_after_read);
        }
    }

    assert!(
        accepted_mixed_boundaries.is_empty(),
        "rustls must never install a TLS bundle assembled from multiple observed generations: accepted replacement boundaries {accepted_mixed_boundaries:?}"
    );
}
