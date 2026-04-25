use super::*;

#[test]
fn legacy_server_live_tls_upgrade_roundtrip_supports_post_upgrade_hello_over_same_socket() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server TLS parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }

    let tls_cert_path = temporary_tls_directory_path("legacy-live-tls-upgrade");
    let _ = fs::remove_dir_all(&tls_cert_path);
    fs::create_dir_all(&tls_cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&tls_cert_path);

    let result = run_legacy_server_tls_upgrade_roundtrip_with_cert_path(&tls_cert_path);
    let _ = fs::remove_dir_all(&tls_cert_path);

    match result {
        Ok((tls_response_line, hello_response_line)) => {
            let tls_message =
                decode_message_line(&tls_response_line).expect("legacy TLS response should decode");
            match tls_message {
                ProtocolMessage::Tls(payload) => {
                    assert_eq!(payload.tls.start_tls, "true");
                }
                other => panic!(
                    "expected legacy TLS response before upgrade, got {}",
                    other.kind()
                ),
            }

            let hello_message = decode_message_line(&hello_response_line)
                .expect("post-upgrade legacy hello response should decode");
            let hello = extract_hello_from_message(hello_message)
                .expect("post-upgrade legacy response should be hello");
            assert_eq!(hello.username, "interop-client");
            assert_eq!(hello.room.name, "interop-room");
        }
        Err(err) if legacy_server_tls_prerequisites_missing(&err) => {
            if legacy_tls_parity_prerequisites_strict_enabled() {
                panic!(
                    "legacy live TLS roundtrip prerequisites should be satisfied in strict mode, got: {err}"
                );
            }
            eprintln!("legacy live TLS roundtrip test skipped due to missing prerequisites: {err}");
        }
        Err(err) => {
            panic!("legacy live TLS roundtrip should succeed over upgraded socket, got: {err}")
        }
    }
}

#[test]
fn legacy_server_live_tls_send_is_denied_for_logged_client() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server TLS parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }

    let tls_cert_path = temporary_tls_directory_path("legacy-live-tls-logged");
    let _ = fs::remove_dir_all(&tls_cert_path);
    fs::create_dir_all(&tls_cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&tls_cert_path);

    let result =
        run_legacy_server_tls_logged_client_send_denied_roundtrip_with_cert_path(&tls_cert_path);
    let _ = fs::remove_dir_all(&tls_cert_path);

    match result {
        Ok(tls_response_line) => {
            let tls_message = decode_message_line(&tls_response_line)
                .expect("legacy logged tls response should decode");
            match tls_message {
                ProtocolMessage::Tls(payload) => {
                    assert_eq!(payload.tls.start_tls, "false");
                }
                other => panic!(
                    "expected legacy TLS response for logged client probe, got {}",
                    other.kind()
                ),
            }
        }
        Err(err) if legacy_server_tls_prerequisites_missing(&err) => {
            if legacy_tls_parity_prerequisites_strict_enabled() {
                panic!(
                    "legacy logged-client TLS denial prerequisites should be satisfied in strict mode, got: {err}"
                );
            }
            eprintln!(
                "legacy logged-client TLS denial test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy logged-client TLS denial behavior should succeed with startTLS=false, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_live_tls_rotation_invalidates_subsequent_send() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server TLS parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }

    let tls_cert_path = temporary_tls_directory_path("legacy-live-tls-rotation");
    let _ = fs::remove_dir_all(&tls_cert_path);
    fs::create_dir_all(&tls_cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&tls_cert_path);

    let result =
        run_legacy_server_tls_rotation_invalidates_subsequent_send_with_cert_path(&tls_cert_path);
    let _ = fs::remove_dir_all(&tls_cert_path);

    match result {
        Ok((initial_tls_response_line, rotated_tls_response_line)) => {
            let initial_message = decode_message_line(&initial_tls_response_line)
                .expect("initial legacy tls response should decode");
            let ProtocolMessage::Tls(initial_payload) = initial_message else {
                panic!("expected initial legacy tls response payload");
            };
            assert_eq!(initial_payload.tls.start_tls, "true");

            let rotated_message = decode_message_line(&rotated_tls_response_line)
                .expect("rotated legacy tls response should decode");
            let ProtocolMessage::Tls(rotated_payload) = rotated_message else {
                panic!("expected rotated legacy tls response payload");
            };
            assert_eq!(rotated_payload.tls.start_tls, "false");
        }
        Err(err) if legacy_server_tls_prerequisites_missing(&err) => {
            if legacy_tls_parity_prerequisites_strict_enabled() {
                panic!(
                    "legacy TLS rotation invalidation prerequisites should be satisfied in strict mode, got: {err}"
                );
            }
            eprintln!(
                "legacy tls rotation invalidation test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy TLS rotation invalidation behavior should succeed with true->false progression, got: {err}"
        ),
    }
}

#[test]
fn legacy_server_live_tls_rotation_recovers_after_bundle_restored() {
    if !legacy_server_parity_assertions_enabled() {
        eprintln!(
            "legacy server TLS parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
        );
        return;
    }

    let tls_cert_path = temporary_tls_directory_path("legacy-live-tls-rotation-recovery");
    let _ = fs::remove_dir_all(&tls_cert_path);
    fs::create_dir_all(&tls_cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&tls_cert_path);

    let result = run_legacy_server_tls_rotation_recovers_after_bundle_restored_with_cert_path(
        &tls_cert_path,
    );
    let _ = fs::remove_dir_all(&tls_cert_path);

    match result {
        Ok((initial_tls_response_line, rotated_tls_response_line, recovered_tls_response_line)) => {
            let initial_message = decode_message_line(&initial_tls_response_line)
                .expect("initial legacy tls response should decode");
            let ProtocolMessage::Tls(initial_payload) = initial_message else {
                panic!("expected initial legacy tls response payload");
            };
            assert_eq!(initial_payload.tls.start_tls, "true");

            let rotated_message = decode_message_line(&rotated_tls_response_line)
                .expect("rotated legacy tls response should decode");
            let ProtocolMessage::Tls(rotated_payload) = rotated_message else {
                panic!("expected rotated legacy tls response payload");
            };
            assert_eq!(rotated_payload.tls.start_tls, "false");

            let recovered_message = decode_message_line(&recovered_tls_response_line)
                .expect("recovered legacy tls response should decode");
            let ProtocolMessage::Tls(recovered_payload) = recovered_message else {
                panic!("expected recovered legacy tls response payload");
            };
            assert_eq!(recovered_payload.tls.start_tls, "true");
        }
        Err(err) if legacy_server_tls_prerequisites_missing(&err) => {
            if legacy_tls_parity_prerequisites_strict_enabled() {
                panic!(
                    "legacy TLS rotation recovery prerequisites should be satisfied in strict mode, got: {err}"
                );
            }
            eprintln!(
                "legacy tls rotation recovery test skipped due to missing prerequisites: {err}"
            );
        }
        Err(err) => panic!(
            "legacy TLS rotation recovery behavior should succeed with true->false->true progression, got: {err}"
        ),
    }
}
