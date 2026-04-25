use super::*;

pub fn run_python_same_filename_batch(pairs: &[(&str, &str)]) -> Result<Vec<bool>, InteropError> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    let pairs_payload = pairs
        .iter()
        .map(|(left, right)| json!([left, right]))
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&json!({ "pairs": pairs_payload }))?;
    let stdout = run_python_probe_raw(&["--same-filename-batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let outputs = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for same-filename response".to_owned(),
            )
        })?;

    if outputs.len() != pairs.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "same-filename response count mismatch: expected {}, got {}",
            pairs.len(),
            outputs.len()
        )));
    }

    let mut results = Vec::with_capacity(outputs.len());
    for (index, output) in outputs.iter().enumerate() {
        let is_same = output.as_bool().ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(format!(
                "same-filename outputs[{index}] should be a boolean"
            ))
        })?;
        results.push(is_same);
    }
    Ok(results)
}

pub fn run_python_same_filesize_batch(pairs: &[(Value, Value)]) -> Result<Vec<bool>, InteropError> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    let pairs_payload = pairs
        .iter()
        .map(|(left, right)| json!([left, right]))
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&json!({ "pairs": pairs_payload }))?;
    let stdout = run_python_probe_raw(&["--same-filesize-batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let outputs = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for same-filesize response".to_owned(),
            )
        })?;

    if outputs.len() != pairs.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "same-filesize response count mismatch: expected {}, got {}",
            pairs.len(),
            outputs.len()
        )));
    }

    let mut results = Vec::with_capacity(outputs.len());
    for (index, output) in outputs.iter().enumerate() {
        let is_same = output.as_bool().ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(format!(
                "same-filesize outputs[{index}] should be a boolean"
            ))
        })?;
        results.push(is_same);
    }
    Ok(results)
}

pub fn run_python_same_fileduration_batch(pairs: &[(f64, f64)]) -> Result<Vec<bool>, InteropError> {
    run_python_same_fileduration_batch_with_overrides(pairs, None, None)
}

pub fn run_python_same_fileduration_batch_with_overrides(
    pairs: &[(f64, f64)],
    show_duration_notification: Option<bool>,
    different_duration_threshold: Option<f64>,
) -> Result<Vec<bool>, InteropError> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    let pairs_payload = pairs
        .iter()
        .map(|(left, right)| json!([left, right]))
        .collect::<Vec<_>>();
    let mut payload_object = serde_json::Map::new();
    payload_object.insert("pairs".to_owned(), json!(pairs_payload));
    if let Some(show_duration_notification) = show_duration_notification {
        payload_object.insert(
            "showDurationNotification".to_owned(),
            json!(show_duration_notification),
        );
    }
    if let Some(different_duration_threshold) = different_duration_threshold {
        payload_object.insert(
            "differentDurationThreshold".to_owned(),
            json!(different_duration_threshold),
        );
    }
    let payload = serde_json::to_vec(&Value::Object(payload_object))?;
    let stdout = run_python_probe_raw(&["--same-fileduration-batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let outputs = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for same-fileduration response".to_owned(),
            )
        })?;

    if outputs.len() != pairs.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "same-fileduration response count mismatch: expected {}, got {}",
            pairs.len(),
            outputs.len()
        )));
    }

    let mut results = Vec::with_capacity(outputs.len());
    for (index, output) in outputs.iter().enumerate() {
        let is_same = output.as_bool().ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(format!(
                "same-fileduration outputs[{index}] should be a boolean"
            ))
        })?;
        results.push(is_same);
    }
    Ok(results)
}
