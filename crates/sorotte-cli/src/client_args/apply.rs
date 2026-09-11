use super::*;

pub(crate) fn apply_syncplay_client_arg_overrides(
    config: &mut ClientLoopConfig,
    overrides: &SyncplayClientArgOverrides,
) {
    if let Some(host) = overrides.host.as_deref() {
        config.host = host.to_owned();
    }
    if let Some(port) = overrides.port {
        config.port = port;
    }
    if let Some(username) = overrides.username.as_deref() {
        config.username = username.to_owned();
    }
    if let Some(room) = overrides.room.as_deref() {
        let (normalized_room, normalized_password) =
            normalize_controlled_room_input(room.to_owned());
        config.room = normalized_room;
        config.controlled_room_password_override = normalized_password.map(SecretValue::from);
    }
    if let Some(password) = overrides.controlled_room_password_override.as_ref()
        && !password.is_empty()
    {
        config.controlled_room_password_override = Some(password.clone());
    }
}

pub(crate) fn validate_composed_client_endpoint(
    config: &ClientLoopConfig,
) -> Result<(), HostArgumentError> {
    if config.host.trim().is_empty() {
        return Err(HostArgumentError::EmptyHost);
    }
    if config.port == 0 {
        return Err(HostArgumentError::PortOutOfRange);
    }
    Ok(())
}

pub(crate) fn emit_syncplay_client_arg_compatibility_warnings(
    overrides: &SyncplayClientArgOverrides,
) {
    if overrides.debug_requested {
        eprintln!("note: --debug enables sorotte-cli diagnostics output");
    }
    if let Some(line) = syncplay_force_gui_prompt_compatibility_line(overrides) {
        eprintln!("{line}");
    }
    if let Some(line) = runtime_language_selection_line(overrides.language.as_deref()) {
        eprintln!("{line}");
    }
    if let Some(line) = player_path_compatibility_warning_line(overrides) {
        eprintln!("{line}");
    }
    if overrides.file.is_some() {
        eprintln!(
            "warning: positional file is routed to managed mpv preload, unmanaged external launch, and explicit mpv IPC startup open-file; sorotte-cli intentionally keeps broader ConfigurationGetter side effects such as relative config handling out of this path"
        );
    }
    if !overrides.player_args.is_empty() {
        eprintln!(
            "warning: player arguments after [file] are forwarded for managed mpv and unmanaged external launch; explicit-mpv-IPC applies the runtime property subset plus generic --name=value / --profile attach commands, and only remaining launch-only tokens are warned"
        );
    }
}

pub(crate) fn syncplay_unrecognized_arguments_diagnostic_line(
    unknown_options: &[SyncplayClientArgumentIssue],
) -> String {
    let redacted_options = unknown_options
        .iter()
        .map(SyncplayClientArgumentIssue::diagnostic_fragment)
        .collect::<Vec<_>>();
    format!(
        "error: unrecognized arguments: {}",
        redacted_options.join(" ")
    )
}
