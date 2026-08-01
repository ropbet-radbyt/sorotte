use super::*;

#[derive(Clone, Copy)]
pub(super) struct MessageNormalizationOptions {
    pub(super) normalize_hello_motd: bool,
    pub(super) normalize_hello_features: bool,
    pub(super) normalize_set_user_event_features: bool,
    pub(super) normalize_set_user_features: bool,
    pub(super) normalize_list_features: bool,
    pub(super) normalize_list_position: bool,
    pub(super) normalize_list_file: bool,
    pub(super) normalize_list_is_ready_when_false_or_null: bool,
    pub(super) normalize_set_ready_not_ready_when_false_or_null: bool,
    pub(super) normalize_ping_latency_calculation: bool,
    pub(super) normalize_ping_client_latency_calculation: bool,
    pub(super) normalize_ping_client_rtt: bool,
    pub(super) normalize_ping_server_rtt: bool,
}

impl Default for MessageNormalizationOptions {
    fn default() -> Self {
        Self {
            normalize_hello_motd: true,
            normalize_hello_features: true,
            normalize_set_user_event_features: true,
            normalize_set_user_features: true,
            normalize_list_features: true,
            normalize_list_position: true,
            normalize_list_file: true,
            normalize_list_is_ready_when_false_or_null: true,
            normalize_set_ready_not_ready_when_false_or_null: false,
            normalize_ping_latency_calculation: true,
            normalize_ping_client_latency_calculation: true,
            normalize_ping_client_rtt: true,
            normalize_ping_server_rtt: true,
        }
    }
}

pub(super) fn normalization_options_for_runtime_trace_scenario(
    _scenario_name: &str,
) -> MessageNormalizationOptions {
    MessageNormalizationOptions {
        normalize_hello_motd: false,
        normalize_hello_features: false,
        normalize_set_user_event_features: true,
        normalize_set_user_features: true,
        normalize_list_features: true,
        normalize_list_position: false,
        normalize_list_file: false,
        normalize_list_is_ready_when_false_or_null: true,
        normalize_set_ready_not_ready_when_false_or_null: true,
        normalize_ping_latency_calculation: false,
        normalize_ping_client_latency_calculation: false,
        normalize_ping_client_rtt: false,
        normalize_ping_server_rtt: false,
    }
}

pub(super) fn normalization_options_for_runtime_python_scenario(
    _scenario_name: &str,
) -> MessageNormalizationOptions {
    MessageNormalizationOptions {
        normalize_hello_motd: false,
        normalize_hello_features: false,
        normalize_set_user_event_features: true,
        normalize_set_user_features: true,
        normalize_list_features: true,
        normalize_list_position: false,
        normalize_list_file: false,
        normalize_list_is_ready_when_false_or_null: false,
        normalize_set_ready_not_ready_when_false_or_null: false,
        normalize_ping_latency_calculation: false,
        normalize_ping_client_latency_calculation: false,
        normalize_ping_client_rtt: false,
        normalize_ping_server_rtt: false,
    }
}

pub(super) fn normalization_options_for_legacy_scenario(
    _scenario_name: &str,
) -> MessageNormalizationOptions {
    MessageNormalizationOptions {
        normalize_hello_motd: false,
        normalize_hello_features: false,
        normalize_set_user_event_features: false,
        normalize_set_user_features: false,
        normalize_list_features: false,
        normalize_list_position: false,
        normalize_list_file: false,
        normalize_list_is_ready_when_false_or_null: false,
        normalize_set_ready_not_ready_when_false_or_null: false,
        normalize_ping_latency_calculation: false,
        normalize_ping_client_latency_calculation: false,
        normalize_ping_client_rtt: false,
        normalize_ping_server_rtt: false,
    }
}

#[derive(Clone, Copy)]
pub(super) enum LegacyTimingSide {
    Legacy,
    Runtime,
}

#[derive(Default)]
pub(super) struct LegacyTimingCanonicalizer {
    legacy_latency_origin: Option<f64>,
    runtime_latency_origin: Option<f64>,
    legacy_server_rtt_nonzero_origin: Option<f64>,
    runtime_server_rtt_nonzero_origin: Option<f64>,
}

impl LegacyTimingCanonicalizer {
    pub(super) fn canonicalize_message(&mut self, message: &mut Value, side: LegacyTimingSide) {
        let Some(state_payload) = message.get_mut("State").and_then(Value::as_object_mut) else {
            return;
        };
        let Some(ping) = state_payload.get_mut("ping").and_then(Value::as_object_mut) else {
            return;
        };
        let Some(latency_value) = ping.get_mut("latencyCalculation") else {
            return;
        };
        let Some(latency) = latency_value.as_f64() else {
            return;
        };
        if !latency.is_finite() {
            return;
        }

        let origin_slot = match side {
            LegacyTimingSide::Legacy => &mut self.legacy_latency_origin,
            LegacyTimingSide::Runtime => &mut self.runtime_latency_origin,
        };
        let origin = *origin_slot.get_or_insert(latency);
        let canonical_latency = (latency - origin).round();
        let canonical_latency = if canonical_latency == -0.0 {
            0.0
        } else {
            canonical_latency
        };
        *latency_value = Value::from(canonical_latency);

        let Some(server_rtt_value) = ping.get_mut("serverRtt") else {
            return;
        };
        let Some(server_rtt) = server_rtt_value.as_f64() else {
            return;
        };
        if !server_rtt.is_finite() {
            return;
        }
        if server_rtt.abs() <= f64::EPSILON {
            *server_rtt_value = Value::from(0.0);
            return;
        }

        let rtt_origin_slot = match side {
            LegacyTimingSide::Legacy => &mut self.legacy_server_rtt_nonzero_origin,
            LegacyTimingSide::Runtime => &mut self.runtime_server_rtt_nonzero_origin,
        };
        let rtt_origin = *rtt_origin_slot.get_or_insert(server_rtt);
        let canonical_server_rtt = (server_rtt - rtt_origin).round();
        let canonical_server_rtt = if canonical_server_rtt == -0.0 {
            0.0
        } else {
            canonical_server_rtt
        };
        *server_rtt_value = Value::from(canonical_server_rtt);
    }
}

fn is_legacy_default_user_features(features: &serde_json::Map<String, Value>) -> bool {
    features.get("chat") == Some(&Value::Bool(false))
        && features.get("featureList") == Some(&Value::Bool(false))
        && features.get("managedRooms") == Some(&Value::Bool(false))
        && features.get("persistentRooms") == Some(&Value::Bool(false))
        && features.get("readiness") == Some(&Value::Bool(false))
        && features.get("sharedPlaylists") == Some(&Value::Bool(false))
        && features.get("uiMode") == Some(&Value::String("Unknown".to_owned()))
}

fn canonicalize_user_features_field(object: &mut serde_json::Map<String, Value>, field: &str) {
    let canonicalize_to_default = match object.get(field) {
        None => true,
        Some(Value::Null) => true,
        Some(Value::Object(features)) => is_legacy_default_user_features(features),
        _ => false,
    };
    if canonicalize_to_default {
        object.insert(
            field.to_owned(),
            Value::String("__default_user_features__".to_owned()),
        );
    }
}

pub(super) fn canonicalize_legacy_set_user_features(message: &mut Value) {
    let Some(set_payload) = message.get_mut("Set").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(users) = set_payload.get_mut("user").and_then(Value::as_object_mut) else {
        return;
    };
    for user_payload in users.values_mut() {
        let Some(user_object) = user_payload.as_object_mut() else {
            continue;
        };
        if let Some(event) = user_object.get_mut("event").and_then(Value::as_object_mut) {
            canonicalize_user_features_field(event, "features");
        }
        canonicalize_user_features_field(user_object, "features");
    }
}

fn canonicalize_user_is_ready_field(object: &mut serde_json::Map<String, Value>, field: &str) {
    let canonicalize_to_not_ready = matches!(
        object.get(field),
        None | Some(Value::Null) | Some(Value::Bool(false))
    );
    if canonicalize_to_not_ready {
        object.insert(field.to_owned(), Value::String("__not_ready__".to_owned()));
    }
}

pub(super) fn canonicalize_legacy_list_fields(message: &mut Value) {
    let Some(list_payload) = message.get_mut("List").and_then(Value::as_object_mut) else {
        return;
    };
    for room_users in list_payload.values_mut() {
        let Some(room_users) = room_users.as_object_mut() else {
            continue;
        };
        for user_payload in room_users.values_mut() {
            let Some(user_object) = user_payload.as_object_mut() else {
                continue;
            };
            canonicalize_user_features_field(user_object, "features");
            canonicalize_user_is_ready_field(user_object, "isReady");
        }
    }
}

pub(super) fn canonicalize_legacy_hello_fields(message: &mut Value) {
    const SHARED_HELLO_FEATURE_KEYS: &[&str] = &[
        "chat",
        "isolateRooms",
        "managedRooms",
        "persistentRooms",
        "readiness",
        "setOthersReadiness",
    ];

    let Some(hello_payload) = message.get_mut("Hello").and_then(Value::as_object_mut) else {
        return;
    };

    let canonical_motd = match hello_payload.get("motd") {
        None | Some(Value::Null) => Some(String::new()),
        Some(Value::String(motd)) if motd.trim().is_empty() => Some(String::new()),
        _ => None,
    };
    if let Some(motd) = canonical_motd {
        hello_payload.insert("motd".to_owned(), Value::String(motd));
    }

    if let Some(features_value) = hello_payload.get_mut("features") {
        let Some(features) = features_value.as_object() else {
            return;
        };

        let mut canonical_features = serde_json::Map::new();
        for key in SHARED_HELLO_FEATURE_KEYS {
            if let Some(value) = features.get(*key) {
                canonical_features.insert((*key).to_owned(), value.clone());
            }
        }
        *features_value = Value::Object(canonical_features);
    }
}

pub(super) fn normalize_cross_impl_message(value: Value) -> Value {
    normalize_cross_impl_message_with_options(value, MessageNormalizationOptions::default())
}

pub(super) fn normalize_cross_impl_message_with_options(
    mut value: Value,
    options: MessageNormalizationOptions,
) -> Value {
    if let Some(hello) = value.get_mut("Hello").and_then(Value::as_object_mut) {
        // Rust runtime and Python probe intentionally report different server version strings.
        hello.insert(
            "realversion".to_owned(),
            Value::String("__normalized__".to_owned()),
        );
        if options.normalize_hello_motd && hello.contains_key("motd") {
            hello.insert(
                "motd".to_owned(),
                Value::String("__normalized_motd__".to_owned()),
            );
        }
        if options.normalize_hello_features && hello.contains_key("features") {
            hello.insert(
                "features".to_owned(),
                Value::String("__normalized_features__".to_owned()),
            );
        }
    }
    if let Some(set_payload) = value.get_mut("Set").and_then(Value::as_object_mut)
        && let Some(users) = set_payload.get_mut("user").and_then(Value::as_object_mut)
    {
        for user_payload in users.values_mut() {
            let Some(user_object) = user_payload.as_object_mut() else {
                continue;
            };
            if let Some(event) = user_object.get_mut("event").and_then(Value::as_object_mut)
                && options.normalize_set_user_event_features
            {
                event.remove("features");
            }
            if options.normalize_set_user_features {
                user_object.remove("features");
            }
        }
    }
    if options.normalize_set_ready_not_ready_when_false_or_null
        && let Some(ready) = value
            .get_mut("Set")
            .and_then(Value::as_object_mut)
            .and_then(|set_payload| set_payload.get_mut("ready"))
            .and_then(Value::as_object_mut)
        && ready
            .get("manuallyInitiated")
            .is_some_and(|manual| manual == &Value::Bool(false))
        && matches!(
            ready.get("isReady"),
            None | Some(Value::Null) | Some(Value::Bool(false))
        )
    {
        ready.insert(
            "isReady".to_owned(),
            Value::String("__not_ready__".to_owned()),
        );
    }
    if let Some(list_payload) = value.get_mut("List").and_then(Value::as_object_mut) {
        for room_users in list_payload.values_mut() {
            let Some(room_users) = room_users.as_object_mut() else {
                continue;
            };
            for user_payload in room_users.values_mut() {
                let Some(user_object) = user_payload.as_object_mut() else {
                    continue;
                };
                if options.normalize_list_features {
                    user_object.remove("features");
                }
                if let Some(position_value) = user_object.get_mut("position")
                    && let Some(position) = position_value.as_f64()
                {
                    let rounded_position = (position * 1000.0).round() / 1000.0;
                    *position_value = Value::from(rounded_position);
                }
                if options.normalize_list_position {
                    user_object.remove("position");
                }
                if options.normalize_list_file {
                    user_object.remove("file");
                }
                if options.normalize_list_is_ready_when_false_or_null
                    && user_object.get("isReady").is_some_and(|is_ready| {
                        is_ready.is_null() || is_ready == &Value::Bool(false)
                    })
                {
                    user_object.remove("isReady");
                }
            }
        }
    }
    if let Some(state_payload) = value.get_mut("State").and_then(Value::as_object_mut) {
        if let Some(playstate) = state_payload
            .get_mut("playstate")
            .and_then(Value::as_object_mut)
            && let Some(position_value) = playstate.get_mut("position")
            && let Some(position) = position_value.as_f64()
        {
            let rounded_position = (position * 1000.0).round() / 1000.0;
            *position_value = Value::from(rounded_position);
        }
        if let Some(ping) = state_payload.get_mut("ping").and_then(Value::as_object_mut) {
            if let Some(latency_value) = ping.get_mut("latencyCalculation")
                && let Some(latency) = latency_value.as_f64()
            {
                let rounded_latency = (latency * 1000.0).round() / 1000.0;
                *latency_value = Value::from(rounded_latency);
            }
            if options.normalize_ping_latency_calculation && ping.contains_key("latencyCalculation")
            {
                ping.insert(
                    "latencyCalculation".to_owned(),
                    Value::String("__normalized_latency__".to_owned()),
                );
            }
            if let Some(client_latency_value) = ping.get_mut("clientLatencyCalculation")
                && let Some(client_latency) = client_latency_value.as_f64()
            {
                let canonical_client_latency = if options.normalize_ping_client_latency_calculation
                {
                    (client_latency * 1000.0).round() / 1000.0
                } else {
                    // Legacy server mutates this field slightly in-flight; compare at
                    // a stable tenth-second granularity when preserving the value.
                    (client_latency * 10.0).trunc() / 10.0
                };
                *client_latency_value = Value::from(canonical_client_latency);
            }
            if options.normalize_ping_client_latency_calculation
                && ping.contains_key("clientLatencyCalculation")
            {
                ping.insert(
                    "clientLatencyCalculation".to_owned(),
                    Value::String("__normalized_client_latency__".to_owned()),
                );
            }
            if let Some(client_rtt_value) = ping.get_mut("clientRtt")
                && let Some(client_rtt) = client_rtt_value.as_f64()
            {
                let rounded_client_rtt = (client_rtt * 1000.0).round() / 1000.0;
                *client_rtt_value = Value::from(rounded_client_rtt);
            }
            if options.normalize_ping_client_rtt && ping.contains_key("clientRtt") {
                ping.insert(
                    "clientRtt".to_owned(),
                    Value::String("__normalized_client_rtt__".to_owned()),
                );
            }
            if let Some(server_rtt_value) = ping.get_mut("serverRtt")
                && let Some(server_rtt) = server_rtt_value.as_f64()
            {
                let rounded_server_rtt = (server_rtt * 1000.0).round() / 1000.0;
                *server_rtt_value = Value::from(rounded_server_rtt);
            }
            if options.normalize_ping_server_rtt && ping.contains_key("serverRtt") {
                ping.insert(
                    "serverRtt".to_owned(),
                    Value::String("__normalized_server_rtt__".to_owned()),
                );
            }
        }
    }
    strip_null_object_fields(&mut value);
    value
}

fn strip_null_object_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.retain(|_, field_value| !field_value.is_null());
            for field_value in object.values_mut() {
                strip_null_object_fields(field_value);
            }
        }
        Value::Array(values) => {
            for field_value in values {
                strip_null_object_fields(field_value);
            }
        }
        _ => {}
    }
}
