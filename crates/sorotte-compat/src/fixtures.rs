use super::*;

pub fn protocol_fixture(name: &str) -> std::io::Result<String> {
    fs::read_to_string(fixture_path(name))
}

pub fn fixture_path(name: &str) -> PathBuf {
    protocol_fixture_dir().join(name)
}

pub fn protocol_fixture_dir() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path.push("fixtures");
    path.push("protocol");
    path
}

pub fn fixture_decodes(name: &str) -> bool {
    let Ok(contents) = protocol_fixture(name) else {
        return false;
    };
    decode_message_line(&contents).is_ok()
}

pub fn decode_fixture(name: &str) -> Option<ProtocolMessage> {
    let contents = protocol_fixture(name).ok()?;
    decode_message_line(&contents).ok()
}

pub fn all_protocol_fixture_names() -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(protocol_fixture_dir())? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    names.sort();
    Ok(names)
}

pub fn decode_protocol_file(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    decode_message_line(&contents).is_ok()
}

pub fn scenario_fixture_path(name: &str) -> PathBuf {
    scenario_fixture_dir().join(name)
}

pub fn scenario_fixture_dir() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path.push("fixtures");
    path.push("scenarios");
    path
}

pub fn load_server_runtime_scenario_fixture(
    name: &str,
) -> Result<Vec<ServerRuntimeScenarioStep>, InteropError> {
    let contents = fs::read_to_string(scenario_fixture_path(name))?;
    parse_server_runtime_scenario_steps(&contents)
}

pub fn parse_server_runtime_scenario_steps(
    json_lines: &str,
) -> Result<Vec<ServerRuntimeScenarioStep>, InteropError> {
    let mut steps = Vec::new();
    for (line_number, line) in json_lines.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Value = serde_json::from_str(trimmed)?;
        let client_id = parsed
            .get("client")
            .and_then(Value::as_str)
            .filter(|client| !client.trim().is_empty())
            .ok_or_else(|| {
                InteropError::InvalidScenarioStep(format!(
                    "line {} is missing non-empty 'client' field",
                    line_number + 1
                ))
            })?;
        let request_value = parsed.get("message").ok_or_else(|| {
            InteropError::InvalidScenarioStep(format!(
                "line {} is missing 'message' field",
                line_number + 1
            ))
        })?;
        let advance_seconds = match parsed.get("advanceSeconds") {
            Some(Value::Number(number)) => number.as_f64().ok_or_else(|| {
                InteropError::InvalidScenarioStep(format!(
                    "line {} has non-finite 'advanceSeconds' value",
                    line_number + 1
                ))
            })?,
            Some(_) => {
                return Err(InteropError::InvalidScenarioStep(format!(
                    "line {} has non-numeric 'advanceSeconds' field",
                    line_number + 1
                )));
            }
            None => 0.0,
        };
        if !advance_seconds.is_finite() || advance_seconds < 0.0 {
            return Err(InteropError::InvalidScenarioStep(format!(
                "line {} has invalid 'advanceSeconds' value",
                line_number + 1
            )));
        }
        let legacy_advance_seconds = match parsed.get("legacyAdvanceSeconds") {
            Some(Value::Number(number)) => Some(number.as_f64().ok_or_else(|| {
                InteropError::InvalidScenarioStep(format!(
                    "line {} has non-finite 'legacyAdvanceSeconds' value",
                    line_number + 1
                ))
            })?),
            Some(_) => {
                return Err(InteropError::InvalidScenarioStep(format!(
                    "line {} has non-numeric 'legacyAdvanceSeconds' field",
                    line_number + 1
                )));
            }
            None => None,
        };
        if legacy_advance_seconds.is_some_and(|seconds| !seconds.is_finite() || seconds < 0.0) {
            return Err(InteropError::InvalidScenarioStep(format!(
                "line {} has invalid 'legacyAdvanceSeconds' value",
                line_number + 1
            )));
        }
        let request_line = serde_json::to_string(request_value)?;

        // Validate each scripted request decodes as a typed protocol message.
        let _ = decode_message_line(&request_line)?;

        steps.push(ServerRuntimeScenarioStep {
            client_id: client_id.to_owned(),
            request_line,
            advance_seconds,
            legacy_advance_seconds,
        });
    }
    Ok(steps)
}
