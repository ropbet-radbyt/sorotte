use super::*;

#[test]
fn legacy_python_same_filename_matches_client_core_on_edge_cases() {
    let pairs = [
        ("**Hidden filename**", "anything.mkv"),
        (
            "https://example.invalid/media/Movie%20Name.mkv",
            "Movie Name.mkv",
        ),
        ("Movie Name.mkv", "a9858cb4803c"),
        ("movie-a.mkv", "movie-b.mkv"),
    ];

    let legacy_results = match run_python_same_filename_batch(&pairs) {
        Ok(results) => results,
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!(
                "legacy same-filename parity test skipped due to missing local prerequisites"
            );
            return;
        }
        Err(err) => panic!("legacy same-filename probe should succeed, got: {err}"),
    };

    assert_eq!(legacy_results.len(), pairs.len());
    for ((left, right), legacy_result) in pairs.iter().zip(legacy_results) {
        let rust_result = ClientSession::same_filename_legacy_compatible(left, right);
        assert_eq!(
            rust_result, legacy_result,
            "same-filename mismatch for pair ({left:?}, {right:?})"
        );
    }
}

#[test]
fn legacy_python_same_filesize_matches_client_core_on_edge_cases() {
    let pairs = vec![
        (json!(0), json!(123456789)),
        (json!(123456789), json!("15e2b0d3c338")),
        (json!(123456789), json!(123456789)),
        (json!(123456789), json!(987654321)),
        (json!("0"), json!(123456789)),
        (json!("ABCDEF"), json!("abcdef")),
    ];

    let legacy_results = match run_python_same_filesize_batch(&pairs) {
        Ok(results) => results,
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!(
                "legacy same-filesize parity test skipped due to missing local prerequisites"
            );
            return;
        }
        Err(err) => panic!("legacy same-filesize probe should succeed, got: {err}"),
    };

    assert_eq!(legacy_results.len(), pairs.len());
    for ((left, right), legacy_result) in pairs.iter().zip(legacy_results) {
        let rust_result = ClientSession::same_filesize_legacy_compatible(left, right);
        assert_eq!(
            rust_result, legacy_result,
            "same-filesize mismatch for pair ({left:?}, {right:?})"
        );
    }
}

#[test]
fn legacy_python_same_fileduration_matches_client_core_on_edge_cases() {
    let pairs = vec![
        (10.49, 12.49),
        (10.49, 13.49),
        (1.5, 4.5),
        (100.0, 100.0),
        (-1.5, 1.5),
    ];

    let legacy_results = match run_python_same_fileduration_batch(&pairs) {
        Ok(results) => results,
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!(
                "legacy same-fileduration parity test skipped due to missing local prerequisites"
            );
            return;
        }
        Err(err) => panic!("legacy same-fileduration probe should succeed, got: {err}"),
    };

    assert_eq!(legacy_results.len(), pairs.len());
    for ((left, right), legacy_result) in pairs.iter().zip(legacy_results) {
        let rust_result = ClientSession::same_fileduration_legacy_compatible(*left, *right);
        assert_eq!(
            rust_result, legacy_result,
            "same-fileduration mismatch for pair ({left:?}, {right:?})"
        );
    }
}

#[test]
fn legacy_python_same_fileduration_with_config_overrides_matches_client_core_on_edge_cases() {
    let pairs = vec![(10.49, 12.49), (10.49, 13.49), (1.5, 4.5)];
    let scenarios = [
        ("duration-notifications-disabled", Some(false), None),
        ("tight-threshold", Some(true), Some(1.0)),
        ("wide-threshold", Some(true), Some(3.5)),
    ];

    for (scenario_name, show_duration_notification, different_duration_threshold) in scenarios {
        let legacy_results = match run_python_same_fileduration_batch_with_overrides(
            &pairs,
            show_duration_notification,
            different_duration_threshold,
        ) {
            Ok(results) => results,
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!(
                    "legacy same-fileduration override parity test skipped due to missing local prerequisites"
                );
                return;
            }
            Err(err) => panic!(
                "legacy same-fileduration override probe should succeed for '{scenario_name}', got: {err}"
            ),
        };

        assert_eq!(legacy_results.len(), pairs.len());
        for ((left, right), legacy_result) in pairs.iter().zip(legacy_results) {
            let rust_result = ClientSession::same_fileduration_legacy_compatible_with_overrides(
                *left,
                *right,
                show_duration_notification.unwrap_or(true),
                different_duration_threshold.unwrap_or(2.5),
            );
            assert_eq!(
                rust_result, legacy_result,
                "same-fileduration override mismatch for scenario '{scenario_name}' pair ({left:?}, {right:?})"
            );
        }
    }
}

#[test]
fn legacy_python_privacy_file_payload_batch_matches_client_core_behavior() {
    let cases = vec![
        (
            json!({
                "name": "https://example.invalid/media/Movie Name.mkv",
                "size": 123456789,
                "duration": 95.5,
                "path": "C:/media/movie.mkv",
                "extra": "keep-me"
            }),
            "SendRaw",
            "SendRaw",
        ),
        (
            json!({
                "name": "https://example.invalid/media/Movie Name.mkv",
                "size": 123456789,
                "duration": 95.5,
                "path": "C:/media/movie.mkv",
                "extra": "keep-me"
            }),
            "SendHashed",
            "SendHashed",
        ),
        (
            json!({
                "name": "movie.mkv",
                "size": 123456789,
                "duration": 95.5,
                "path": "C:/media/movie.mkv"
            }),
            "DoNotSend",
            "DoNotSend",
        ),
    ];

    let legacy_results = match run_python_privacy_file_payload_batch(&cases) {
        Ok(results) => results,
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!(
                "legacy privacy file payload parity test skipped due to missing local prerequisites"
            );
            return;
        }
        Err(err) => panic!("legacy privacy file payload probe should succeed, got: {err}"),
    };

    assert_eq!(legacy_results.len(), cases.len());
    for ((file_payload, filename_privacy_mode, filesize_privacy_mode), legacy_result) in
        cases.iter().zip(legacy_results)
    {
        let mut session = ClientSession::default();
        session
                .apply_message_json(
                    r#"{"Hello":{"username":"interop-client","room":{"name":"room1"},"version":"1.2.255"}}"#,
                )
                .expect("hello should apply");

        let filename_mode = PrivacyMode::from_legacy_name(filename_privacy_mode)
            .expect("filename privacy mode should map to Rust mode");
        let filesize_mode = PrivacyMode::from_legacy_name(filesize_privacy_mode)
            .expect("filesize privacy mode should map to Rust mode");
        let actions = session.runtime_actions_for_local_file_publish_legacy_compatible(
            file_payload,
            filename_mode,
            filesize_mode,
        );
        assert_eq!(
            actions.len(),
            2,
            "local file publish should emit SetFile followed by a List refresh request"
        );
        let ClientRuntimeAction::SetFile { file } = &actions[0] else {
            panic!("local file publish should emit SetFile action");
        };
        assert!(
            matches!(actions[1], ClientRuntimeAction::RequestUserList),
            "local file publish should request a fresh user list after SetFile"
        );
        assert_eq!(
            serde_json::to_value(file).expect("typed file payload should serialize"),
            legacy_result,
            "privacy file payload mismatch for modes ({filename_privacy_mode}, {filesize_privacy_mode})"
        );
    }
}

#[test]
fn legacy_client_chat_send_contract_matches_client_core_behavior() {
    let cases = vec![
        LegacyClientChatSendContractCase {
            message: "hello room".to_owned(),
            protocol_logged: true,
            server_version: "1.7.5".to_owned(),
            chat_supported: Some(true),
            max_chat_message_length: Some(150),
            derive_server_features: false,
            feature_list: None,
        },
        LegacyClientChatSendContractCase {
            message: "".to_owned(),
            protocol_logged: true,
            server_version: "1.7.5".to_owned(),
            chat_supported: Some(true),
            max_chat_message_length: Some(150),
            derive_server_features: false,
            feature_list: None,
        },
        LegacyClientChatSendContractCase {
            message: "hello\nroom\r!".to_owned(),
            protocol_logged: true,
            server_version: "1.7.5".to_owned(),
            chat_supported: Some(true),
            max_chat_message_length: Some(5),
            derive_server_features: false,
            feature_list: None,
        },
        LegacyClientChatSendContractCase {
            message: "chat disabled".to_owned(),
            protocol_logged: true,
            server_version: "1.7.5".to_owned(),
            chat_supported: Some(false),
            max_chat_message_length: Some(150),
            derive_server_features: false,
            feature_list: None,
        },
        LegacyClientChatSendContractCase {
            message: "legacy fallback disabled".to_owned(),
            protocol_logged: true,
            server_version: "1.4.9".to_owned(),
            chat_supported: None,
            max_chat_message_length: None,
            derive_server_features: true,
            feature_list: None,
        },
        LegacyClientChatSendContractCase {
            message: "x".repeat(60),
            protocol_logged: true,
            server_version: "1.7.5".to_owned(),
            chat_supported: None,
            max_chat_message_length: None,
            derive_server_features: true,
            feature_list: None,
        },
        LegacyClientChatSendContractCase {
            message: "1234567890".to_owned(),
            protocol_logged: true,
            server_version: "1.7.5".to_owned(),
            chat_supported: None,
            max_chat_message_length: None,
            derive_server_features: true,
            feature_list: Some(json!({"maxChatMessageLength": 7})),
        },
        LegacyClientChatSendContractCase {
            message: "disconnected transport".to_owned(),
            protocol_logged: false,
            server_version: "1.7.5".to_owned(),
            chat_supported: Some(true),
            max_chat_message_length: Some(150),
            derive_server_features: false,
            feature_list: None,
        },
        LegacyClientChatSendContractCase {
            message: "feature-list disabled".to_owned(),
            protocol_logged: true,
            server_version: "1.7.5".to_owned(),
            chat_supported: None,
            max_chat_message_length: None,
            derive_server_features: true,
            feature_list: Some(json!({"chat": false})),
        },
    ];

    let legacy_results = match run_python_legacy_client_chat_send_contract_batch(&cases) {
        Ok(results) => results,
        Err(err) if legacy_client_protocol_prerequisites_missing(&err) => {
            eprintln!(
                "legacy client chat-send contract test skipped due to missing local prerequisites: {err}"
            );
            return;
        }
        Err(err) => panic!("legacy client chat-send contract probe should succeed, got: {err}"),
    };

    assert_eq!(legacy_results.len(), cases.len());
    for (case, legacy_result) in cases.iter().zip(legacy_results.iter()) {
        let mut session = ClientSession::default();
        let mut features = case
            .feature_list
            .clone()
            .map(|value| {
                value.as_object().cloned().unwrap_or_else(|| {
                    panic!("feature_list should be an object when present: {value:?}")
                })
            })
            .unwrap_or_default();
        if let Some(chat_supported) = case.chat_supported {
            features
                .entry("chat".to_owned())
                .or_insert(Value::Bool(chat_supported));
        }
        if let Some(max_chat_message_length) = case.max_chat_message_length {
            features
                .entry("maxChatMessageLength".to_owned())
                .or_insert(json!(max_chat_message_length));
        }
        let hello_line = if features.is_empty() {
            json!({
                "Hello": {
                    "username": "interop-client",
                    "room": {"name": "room1"},
                    "version": case.server_version,
                }
            })
        } else {
            json!({
                "Hello": {
                    "username": "interop-client",
                    "room": {"name": "room1"},
                    "version": case.server_version,
                    "features": Value::Object(features),
                }
            })
        }
        .to_string();
        session
            .apply_message_json(&hello_line)
            .expect("hello should apply");
        if !case.protocol_logged {
            let _ = session.handle_disconnect(0.0);
        }

        let rust_messages = session
            .runtime_actions_for_outbound_chat_message(case.message.clone())
            .into_iter()
            .filter_map(|action| match action {
                ClientRuntimeAction::SendChat { message } => Some(message),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            rust_messages, legacy_result.sent_messages,
            "outbound chat mismatch for case: {case:?}",
        );
        if session.server_chat_supported() == Some(false) {
            assert!(
                !legacy_result.error_messages.is_empty(),
                "legacy client should emit a not-supported error when chat is disabled: {case:?}"
            );
        }
    }
}

#[test]
fn legacy_client_set_file_contract_matches_client_core_behavior() {
    let legacy_contract = match run_python_legacy_client_set_file_contract_probe() {
        Ok(contract) => contract,
        Err(err) if legacy_client_protocol_prerequisites_missing(&err) => {
            eprintln!(
                "legacy client set-file contract test skipped due to missing local prerequisites: {err}"
            );
            return;
        }
        Err(err) => panic!("legacy client set-file contract probe should succeed, got: {err}"),
    };

    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(
            r#"{"Set":{"file":{"name":"movie.mkv","duration":95.5,"size":123456789}}}"#,
        )
        .expect("set file should parse");
    let rust_file_payload_ignored =
        session.user_has_file("alice") == Some(false) && session.user_file_name("alice").is_none();

    session
        .apply_message_json(r#"{"Set":{"file":{}}}"#)
        .expect("empty set file should parse");
    let rust_empty_payload_ignored =
        session.user_has_file("alice") == Some(false) && session.user_file_name("alice").is_none();

    assert_eq!(
        rust_file_payload_ignored, legacy_contract.file_payload_ignored,
        "Rust top-level Set.file handling diverges from legacy client contract"
    );
    assert_eq!(
        rust_empty_payload_ignored, legacy_contract.empty_payload_ignored,
        "Rust empty top-level Set.file handling diverges from legacy client contract"
    );
    assert!(
        legacy_contract.file_payload_calls.is_empty(),
        "legacy probe expected zero calls for top-level file payload, got: {:?}",
        legacy_contract.file_payload_calls
    );
    assert!(
        legacy_contract.empty_payload_calls.is_empty(),
        "legacy probe expected zero calls for top-level empty file payload, got: {:?}",
        legacy_contract.empty_payload_calls
    );
}

#[test]
fn legacy_client_user_file_metadata_contract_matches_client_core_behavior() {
    let legacy_probe = match run_python_legacy_client_user_file_metadata_probe() {
        Ok(probe) => probe,
        Err(err) if legacy_client_protocol_prerequisites_missing(&err) => {
            eprintln!(
                "legacy client user-file metadata test skipped due to missing local prerequisites: {err}"
            );
            return;
        }
        Err(err) => panic!("legacy client user-file metadata probe should succeed, got: {err}"),
    };

    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"interop-client","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"**Hidden filename**","size":"15e2b0d3c338","duration":95}},"bob":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("set mixed user metadata should apply");
    let after_set_mixed = rust_user_file_snapshot(&session, &["alice", "bob", "charlie"]);

    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{}}}}}"#)
        .expect("set empty file payload should apply");
    let after_set_empty = rust_user_file_snapshot(&session, &["alice", "bob", "charlie"]);

    session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"file":{"name":"**Hidden filename**","size":"15e2b0d3c338","duration":95},"controller":false,"isReady":true,"features":{}},"bob":{"file":{"name":"movie.mkv","size":123456789,"duration":95.5},"controller":false,"isReady":false,"features":{}}},"room2":{"charlie":{"file":{"name":"a9858cb4803c","size":"15e2b0d3c338","duration":95.0},"controller":true,"isReady":true,"features":{}}}}}"#,
            )
            .expect("list mixed metadata payload should apply");
    let after_list_mixed = rust_user_file_snapshot(&session, &["alice", "bob", "charlie"]);

    session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"file":{"name":"**Hidden filename**","size":"15e2b0d3c338","duration":95},"controller":false,"isReady":true,"features":{}},"bob":{"file":{},"controller":false,"isReady":false,"features":{}}},"room2":{"charlie":{"file":{"name":"a9858cb4803c","size":"15e2b0d3c338","duration":95.0},"controller":true,"isReady":true,"features":{}}}}}"#,
            )
            .expect("list empty file payload should apply");
    let after_list_clears = rust_user_file_snapshot(&session, &["alice", "bob", "charlie"]);

    assert_eq!(
        after_set_mixed, legacy_probe.after_set_mixed,
        "Rust Set.user mixed file metadata snapshot diverges from legacy client behavior"
    );
    assert_eq!(
        after_set_empty, legacy_probe.after_set_empty,
        "Rust Set.user empty file payload semantics diverge from legacy client behavior"
    );
    assert_eq!(
        after_list_mixed, legacy_probe.after_list_mixed,
        "Rust List mixed file metadata snapshot diverges from legacy client behavior"
    );
    assert_eq!(
        after_list_clears, legacy_probe.after_list_clears,
        "Rust List empty file payload semantics diverge from legacy client behavior"
    );
}
