use super::*;

pub(in crate::tests) fn legacy_client_protocol_prerequisites_missing(error: &InteropError) -> bool {
    match error {
        InteropError::LegacySyncplayCheckoutMissing(_) | InteropError::PythonSpawn { .. } => true,
        InteropError::PythonProbeFailed { stdout, stderr, .. } => {
            let lowered = format!("{stdout}\n{stderr}").to_ascii_lowercase();
            lowered.contains("legacy-client-protocol-import-failed")
                || lowered.contains("legacy-client-chat-import-failed")
                || lowered.contains("no module named 'twisted'")
                || lowered.contains("unable import twisted")
                || lowered.contains("unable to import twisted")
        }
        _ => false,
    }
}

pub(in crate::tests) fn rust_file_payload_for_user(
    session: &ClientSession,
    username: &str,
) -> Option<Value> {
    match session.user_has_file(username) {
        Some(true) => {
            let mut file = serde_json::Map::new();
            if let Some(name) = session.user_file_name(username) {
                file.insert("name".to_owned(), json!(name));
            }
            if let Some(size) = session.user_file_size(username) {
                file.insert("size".to_owned(), size.to_json_value());
            }
            if let Some(duration) = session.user_file_duration_wire(username) {
                file.insert("duration".to_owned(), duration.to_json_value());
            }
            Some(Value::Object(file))
        }
        Some(false) => None,
        None => None,
    }
}

pub(in crate::tests) fn rust_user_file_snapshot(
    session: &ClientSession,
    usernames: &[&str],
) -> BTreeMap<String, Option<Value>> {
    let mut snapshot = BTreeMap::new();
    for username in usernames {
        if session.user_room(username).is_none() {
            continue;
        }
        snapshot.insert(
            (*username).to_owned(),
            rust_file_payload_for_user(session, username),
        );
    }
    snapshot
}
