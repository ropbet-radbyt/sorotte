use super::*;

pub fn run_python_protocol_roundtrip(
    requests: &[ProtocolMessage],
) -> Result<PythonProtocolTranscript, InteropError> {
    if requests.is_empty() {
        return Ok(PythonProtocolTranscript::default());
    }

    let mut request_lines = Vec::with_capacity(requests.len());
    for request in requests {
        request_lines.push(encode_message_line(request)?);
    }

    let payload = serde_json::to_vec(&json!({ "inputs": &request_lines }))?;
    let stdout = run_python_probe_raw(&["--batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let output_sets = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse("missing outputs array".to_owned())
        })?;

    if output_sets.len() != requests.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "response count mismatch: expected {}, got {}",
            requests.len(),
            output_sets.len()
        )));
    }

    let mut steps = Vec::with_capacity(requests.len());
    for (index, output_set) in output_sets.iter().enumerate() {
        let response_values = output_set.as_array().ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(format!(
                "outputs[{index}] should be an array of protocol messages"
            ))
        })?;

        let mut response_lines = Vec::with_capacity(response_values.len());
        let mut response_messages = Vec::with_capacity(response_values.len());
        for response_value in response_values {
            let response_line = serde_json::to_string(response_value)?;
            let response_message = decode_message_line(&response_line)?;
            response_lines.push(response_line);
            response_messages.push(response_message);
        }

        steps.push(PythonProtocolStep {
            request_line: request_lines[index].clone(),
            request_message: requests[index].clone(),
            response_lines,
            response_messages,
        });
    }

    Ok(PythonProtocolTranscript { steps })
}

pub fn run_python_handshake_roundtrip() -> Result<PythonHandshakeTranscript, InteropError> {
    run_python_handshake_roundtrip_with_hello(default_rust_client_hello_for_interop())
}

pub fn run_python_handshake_roundtrip_with_hello(
    hello: HelloPayload,
) -> Result<PythonHandshakeTranscript, InteropError> {
    let protocol_transcript = run_python_protocol_roundtrip(&[ProtocolMessage::hello(hello)])?;
    let first_step = protocol_transcript
        .steps
        .first()
        .ok_or(InteropError::EmptyPythonResponse)?;
    let response_line = first_step
        .response_lines
        .first()
        .ok_or(InteropError::EmptyPythonResponse)?
        .clone();
    let response_message = first_step
        .response_messages
        .first()
        .ok_or(InteropError::EmptyPythonResponse)?
        .clone();
    let response_hello = extract_hello_from_message(response_message.clone())?;

    Ok(PythonHandshakeTranscript {
        request_line: first_step.request_line.clone(),
        response_line,
        response_message,
        response_hello,
    })
}
