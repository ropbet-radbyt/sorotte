use super::*;

pub(crate) fn derive_runtime_loop_inputs(
    runtime: &ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    config: &ClientLoopConfig,
    now_seconds: f64,
) -> RuntimeLoopInputs {
    let session = runtime.session();
    let readiness_supported = config
        .readiness_supported_override
        .or_else(|| session.server_readiness_supported())
        .unwrap_or(true);
    let local_can_control = config
        .local_can_control_override
        .or_else(|| session.local_can_control())
        .unwrap_or(true);
    let is_playing_music = config
        .is_playing_music_override
        .unwrap_or_else(|| session.is_playing_music());
    let recently_advanced = config
        .recently_advanced_override
        .unwrap_or_else(|| session.recently_advanced(now_seconds));

    RuntimeLoopInputs {
        readiness_supported,
        local_can_control,
        is_playing_music,
        recently_advanced,
    }
}

pub(crate) fn shared_playlists_enabled_cli_legacy_compatible(config: &ClientLoopConfig) -> bool {
    config.shared_playlists_enabled_override.unwrap_or(true)
}

pub(crate) fn client_hello_features_legacy_compatible(config: &ClientLoopConfig) -> Value {
    let mut features = Map::new();
    features.insert(
        "sharedPlaylists".to_owned(),
        Value::Bool(shared_playlists_enabled_cli_legacy_compatible(config)),
    );
    features.insert("chat".to_owned(), Value::Bool(true));
    features.insert("uiMode".to_owned(), Value::String("CLI".to_owned()));
    features.insert("featureList".to_owned(), Value::Bool(true));
    features.insert("readiness".to_owned(), Value::Bool(true));
    features.insert("managedRooms".to_owned(), Value::Bool(true));
    features.insert("persistentRooms".to_owned(), Value::Bool(true));
    features.insert("setOthersReadiness".to_owned(), Value::Bool(true));
    Value::Object(features)
}
