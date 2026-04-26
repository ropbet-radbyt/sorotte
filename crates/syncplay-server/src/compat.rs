use super::*;

pub(crate) fn legacy_stats_snapshot_start_delay_seconds_for_port(port: u16) -> f64 {
    SERVER_STATS_DELAY_STEP_SECONDS * (f64::from(port % 10) + 1.0)
}

pub(crate) fn parse_numeric_version_components(version: &str) -> Option<Vec<u32>> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut components = Vec::new();
    for part in trimmed.split('.') {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        components.push(part.parse().ok()?);
    }

    Some(components)
}

pub(crate) fn is_client_version_outdated(client_version: &str, server_version: &str) -> bool {
    let Some(mut client_components) = parse_numeric_version_components(client_version) else {
        return false;
    };
    let Some(mut server_components) = parse_numeric_version_components(server_version) else {
        return false;
    };

    let width = client_components.len().max(server_components.len());
    client_components.resize(width, 0);
    server_components.resize(width, 0);
    client_components < server_components
}

pub(crate) fn client_version_meets_minimum(client_version: &str, minimum_version: &str) -> bool {
    let Some(mut client_components) = parse_numeric_version_components(client_version) else {
        return false;
    };
    let Some(mut minimum_components) = parse_numeric_version_components(minimum_version) else {
        return false;
    };

    let width = client_components.len().max(minimum_components.len());
    client_components.resize(width, 0);
    minimum_components.resize(width, 0);
    client_components >= minimum_components
}

pub(crate) fn render_motd_template(template: &str, client_version: &str) -> String {
    template
        .replace("{client_version}", client_version)
        .replace("{latest_version}", LEGACY_COMPAT_SERVER_VERSION)
        .replace("{upgrade_url}", LEGACY_COMPAT_UPGRADE_URL)
}

pub(crate) fn default_motd_for_client_version(client_version: &str) -> String {
    if is_client_version_outdated(client_version, LEGACY_COMPAT_SERVER_VERSION) {
        return render_motd_template(DEFAULT_OUTDATED_MOTD_TEMPLATE, client_version);
    }
    String::new()
}

pub(crate) fn motd_for_client_version(
    client_version: &str,
    motd_template_override: Option<&str>,
) -> String {
    let is_outdated = is_client_version_outdated(client_version, LEGACY_COMPAT_SERVER_VERSION);
    if let Some(template) = motd_template_override.map(str::trim) {
        if template.is_empty() {
            return String::new();
        }
        let custom_motd = render_motd_template(template, client_version);
        if is_outdated {
            let warning_motd = render_motd_template(DEFAULT_OUTDATED_MOTD_TEMPLATE, client_version);
            return format!("{warning_motd}\n{custom_motd}");
        }
        return custom_motd;
    }
    default_motd_for_client_version(client_version)
}

pub(crate) fn truncate_text_to_max_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(crate) fn truncate_file_payload_name(file: &mut Value, max_chars: usize) {
    let Some(file_object) = file.as_object_mut() else {
        return;
    };
    let Some(name_value) = file_object.get_mut("name") else {
        return;
    };
    let Some(name) = name_value.as_str() else {
        return;
    };
    *name_value = Value::String(truncate_text_to_max_chars(name, max_chars));
}

pub(crate) fn legacy_json_value_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

pub(crate) fn playlist_is_valid(files: &[String]) -> bool {
    if files.len() > DEFAULT_PLAYLIST_MAX_ITEMS {
        return false;
    }
    files.iter().map(|file| file.chars().count()).sum::<usize>() <= DEFAULT_PLAYLIST_MAX_CHARACTERS
}

pub(crate) fn hello_server_password_token(hello: &HelloPayload) -> Option<&str> {
    hello.extra.get("password").and_then(Value::as_str)
}

pub(crate) fn legacy_server_password_token_md5_hex(token: &str) -> String {
    format!("{:x}", Md5::digest(token.as_bytes()))
}

pub(crate) fn server_password_token_matches_legacy_compatible(
    presented_token: &str,
    configured_token: &str,
) -> bool {
    // Accept raw tokens for Rust-Rust interoperability and legacy-Python MD5 tokens for parity.
    presented_token == configured_token
        || presented_token == legacy_server_password_token_md5_hex(configured_token)
}

pub(crate) fn client_supports_persistent_rooms(advertised_features: Option<&Value>) -> bool {
    client_supports_feature(advertised_features, "persistentRooms")
}

pub(crate) fn client_supports_feature(
    advertised_features: Option<&Value>,
    feature_name: &str,
) -> bool {
    advertised_features
        .and_then(Value::as_object)
        .and_then(|features| features.get(feature_name))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(crate) fn legacy_client_feature_defaults(version: &str) -> Value {
    json!({
        "sharedPlaylists": client_version_meets_minimum(version, LEGACY_SHARED_PLAYLIST_MIN_VERSION),
        "chat": client_version_meets_minimum(version, LEGACY_CHAT_MIN_VERSION),
        "featureList": false,
        "readiness": client_version_meets_minimum(version, LEGACY_USER_READY_MIN_VERSION),
        "managedRooms": client_version_meets_minimum(version, LEGACY_CONTROLLED_ROOMS_MIN_VERSION),
        "persistentRooms": false,
        "uiMode": LEGACY_UI_MODE_UNKNOWN,
    })
}

pub(crate) fn legacy_client_features_for_version(
    version: &str,
    advertised_features: Option<Value>,
) -> Value {
    advertised_features
        .filter(legacy_json_value_truthy)
        .unwrap_or_else(|| legacy_client_feature_defaults(version))
}

pub(crate) fn persistent_rooms_notice_motd(
    base_motd: String,
    persistent_rooms_enabled: bool,
    advertised_features: Option<&Value>,
) -> String {
    if !persistent_rooms_enabled || client_supports_persistent_rooms(advertised_features) {
        return base_motd;
    }
    if base_motd.is_empty() {
        return LEGACY_PERSISTENT_ROOMS_NOTICE.to_owned();
    }
    format!("{LEGACY_PERSISTENT_ROOMS_NOTICE}\n\n{base_motd}")
}

pub(crate) fn room_name_is_marked_temporary(room_name: &str) -> bool {
    let room_name = room_name.to_ascii_lowercase();
    room_name.ends_with("-temp") || room_name.contains("-temp:")
}

pub(crate) fn playlist_as_multiline(files: &[String]) -> String {
    files.join("\n")
}

pub(crate) fn multiline_as_playlist(multiline: &str) -> Vec<String> {
    if multiline.is_empty() {
        return Vec::new();
    }
    multiline.split('\n').map(str::to_owned).collect()
}

pub(crate) fn parse_permanent_rooms_file(contents: &str) -> BTreeSet<String> {
    contents.lines().map(str::to_owned).collect()
}

pub(crate) fn feature_ui_mode(features: Option<&Value>) -> Option<&str> {
    features
        .and_then(Value::as_object)
        .and_then(|features| features.get("uiMode"))
        .and_then(Value::as_str)
}

pub(crate) fn client_is_gui_user(features: Option<&Value>) -> bool {
    let mut ui_mode = feature_ui_mode(features).unwrap_or(LEGACY_UI_MODE_UNKNOWN);
    if ui_mode == LEGACY_UI_MODE_UNKNOWN {
        ui_mode = LEGACY_UI_MODE_GRAPHICAL;
    }
    ui_mode == LEGACY_UI_MODE_GRAPHICAL
}

pub(crate) fn features_include_ui_mode(features: Option<&Value>) -> bool {
    features
        .and_then(Value::as_object)
        .is_some_and(|features| features.contains_key("uiMode"))
}

pub(crate) fn legacy_dummy_list_entry() -> ListUserEntry {
    ListUserEntry::new()
        .with_position(0.0)
        .with_file(json!({}))
        .with_controller(false)
        .with_is_ready(true)
        .with_features(json!([]))
}
