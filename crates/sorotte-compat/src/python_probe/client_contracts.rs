use super::*;

pub fn run_python_privacy_file_payload_batch(
    cases: &[(Value, &str, &str)],
) -> Result<Vec<Value>, InteropError> {
    if cases.is_empty() {
        return Ok(Vec::new());
    }

    let cases_payload = cases
        .iter()
        .map(|(file, filename_privacy_mode, filesize_privacy_mode)| {
            json!({
                "file": file,
                "filenamePrivacyMode": filename_privacy_mode,
                "filesizePrivacyMode": filesize_privacy_mode,
            })
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&json!({ "cases": cases_payload }))?;
    let stdout = run_python_probe_raw(&["--privacy-file-payload-batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let outputs = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for privacy file payload response".to_owned(),
            )
        })?;

    if outputs.len() != cases.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "privacy file payload response count mismatch: expected {}, got {}",
            cases.len(),
            outputs.len()
        )));
    }

    outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            if output.is_object() {
                Ok(output.clone())
            } else {
                Err(InteropError::InvalidPythonBatchResponse(format!(
                    "privacy file payload outputs[{index}] should be an object"
                )))
            }
        })
        .collect::<Result<Vec<_>, _>>()
}

pub fn run_python_legacy_client_set_file_contract_probe()
-> Result<LegacyClientSetFileContractProbe, InteropError> {
    let stdout = run_python_probe_raw(&["--client-set-file-contract"], b"")?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;

    let parse_string_array = |field_name: &str| -> Result<Vec<String>, InteropError> {
        let values = parsed
            .get(field_name)
            .and_then(Value::as_array)
            .ok_or_else(|| {
                InteropError::InvalidPythonBatchResponse(format!(
                    "missing {field_name} array for client set-file contract response"
                ))
            })?;
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.as_str().map(str::to_owned).ok_or_else(|| {
                    InteropError::InvalidPythonBatchResponse(format!(
                        "{field_name}[{index}] should be a string"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()
    };

    let file_payload_ignored = parsed
        .get("filePayloadIgnored")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing filePayloadIgnored bool for client set-file contract response".to_owned(),
            )
        })?;
    let empty_payload_ignored = parsed
        .get("emptyPayloadIgnored")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing emptyPayloadIgnored bool for client set-file contract response".to_owned(),
            )
        })?;

    Ok(LegacyClientSetFileContractProbe {
        file_payload_ignored,
        empty_payload_ignored,
        file_payload_calls: parse_string_array("filePayloadCalls")?,
        empty_payload_calls: parse_string_array("emptyPayloadCalls")?,
    })
}

pub fn run_python_legacy_client_user_file_metadata_probe()
-> Result<LegacyClientUserFileMetadataProbe, InteropError> {
    let stdout = run_python_probe_raw(&["--client-user-file-metadata-contract"], b"")?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;

    let parse_snapshot_map =
        |field_name: &str| -> Result<BTreeMap<String, Option<Value>>, InteropError> {
            let values = parsed
                .get(field_name)
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    InteropError::InvalidPythonBatchResponse(format!(
                        "missing {field_name} object for client user file metadata response"
                    ))
                })?;

            values
                .iter()
                .map(|(username, value)| {
                    let file_value = if value.is_null() {
                        None
                    } else if value.is_object() {
                        Some(value.clone())
                    } else {
                        return Err(InteropError::InvalidPythonBatchResponse(format!(
                            "{field_name}.{username} should be an object or null"
                        )));
                    };
                    Ok((username.clone(), file_value))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()
        };

    Ok(LegacyClientUserFileMetadataProbe {
        after_set_mixed: parse_snapshot_map("afterSetMixed")?,
        after_set_empty: parse_snapshot_map("afterSetEmpty")?,
        after_list_mixed: parse_snapshot_map("afterListMixed")?,
        after_list_clears: parse_snapshot_map("afterListClears")?,
    })
}

pub fn run_python_legacy_client_chat_send_contract_batch(
    cases: &[LegacyClientChatSendContractCase],
) -> Result<Vec<LegacyClientChatSendContractResult>, InteropError> {
    if cases.is_empty() {
        return Ok(Vec::new());
    }

    let cases_payload = cases
        .iter()
        .map(|case| {
            json!({
                "message": case.message,
                "chatSupported": case.chat_supported,
                "protocolLogged": case.protocol_logged,
                "serverVersion": case.server_version,
                "maxChatMessageLength": case.max_chat_message_length,
                "deriveServerFeatures": case.derive_server_features,
                "featureList": case.feature_list,
            })
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&json!({ "cases": cases_payload }))?;
    let stdout = run_python_probe_raw(&["--client-chat-send-contract-batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let outputs = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for client chat send contract response".to_owned(),
            )
        })?;

    if outputs.len() != cases.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "client chat send contract response count mismatch: expected {}, got {}",
            cases.len(),
            outputs.len()
        )));
    }

    let parse_string_array =
        |value: &Value, field_name: &str| -> Result<Vec<String>, InteropError> {
            let values = value
                .get(field_name)
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    InteropError::InvalidPythonBatchResponse(format!(
                        "missing {field_name} array for client chat send contract response"
                    ))
                })?;
            values
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    entry.as_str().map(str::to_owned).ok_or_else(|| {
                        InteropError::InvalidPythonBatchResponse(format!(
                            "{field_name}[{index}] should be a string"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        };

    outputs
        .iter()
        .map(|output| {
            if !output.is_object() {
                return Err(InteropError::InvalidPythonBatchResponse(
                    "client chat send contract output should be an object".to_owned(),
                ));
            }
            Ok(LegacyClientChatSendContractResult {
                sent_messages: parse_string_array(output, "sentMessages")?,
                error_messages: parse_string_array(output, "errorMessages")?,
                debug_messages: parse_string_array(output, "debugMessages")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()
}
