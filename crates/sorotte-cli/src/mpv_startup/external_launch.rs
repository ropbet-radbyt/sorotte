use super::*;

pub(crate) fn legacy_player_path_requests_managed_mpv_legacy_compatible(player_path: &str) -> bool {
    let trimmed = player_path.trim();
    if trimmed.is_empty() {
        return false;
    }
    let normalized = trimmed.replace('\\', "/").to_ascii_lowercase();
    if normalized.contains("mpvnet") || !normalized.contains("mpv") {
        return false;
    }

    let file_name = Path::new(&normalized)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(normalized.as_str())
        .trim()
        .to_ascii_lowercase();
    if matches!(file_name.as_str(), "mpv" | "mpv.exe" | "mpv.com") {
        return true;
    }

    let requested = Path::new(trimmed);
    let resolved = resolve_managed_mpv_launch_program_legacy_compatible(requested);
    resolved.is_file()
        || !managed_mpv_launch_program_requires_existing_file_legacy_compatible(&resolved)
}

pub(crate) fn legacy_player_path_compatibility_warning_line_legacy_compatible(
    overrides: &LegacyClientArgOverrides,
) -> Option<&'static str> {
    let player_path = overrides.player_path.as_deref()?;
    if legacy_player_path_requests_managed_mpv_legacy_compatible(player_path) {
        Some(
            "warning: legacy --player-path selects managed mpv integration for Python-style mpv paths; non-mpv values remain launch-only unmanaged fallback",
        )
    } else {
        Some(
            "warning: legacy non-mpv --player-path is launch-only unmanaged fallback; it is not adapter-integrated and is ignored when managed mpv or explicit-mpv-IPC is active",
        )
    }
}

pub(crate) fn legacy_non_mpv_player_path_ignored_by_mpv_integration_warning_line_legacy_compatible(
    overrides: &LegacyClientArgOverrides,
) -> Option<&'static str> {
    let player_path = overrides.player_path.as_deref()?;
    if legacy_player_path_requests_managed_mpv_legacy_compatible(player_path) {
        None
    } else {
        Some(
            "warning: legacy non-mpv --player-path was ignored because managed mpv or explicit-mpv-IPC integration is active",
        )
    }
}

fn should_use_automatic_managed_mpv_for_legacy_player_path_legacy_compatible(
    legacy_overrides: Option<&LegacyClientArgOverrides>,
) -> bool {
    legacy_overrides
        .and_then(|overrides| overrides.player_path.as_deref())
        .is_some_and(legacy_player_path_requests_managed_mpv_legacy_compatible)
}

pub(crate) fn should_skip_legacy_external_player_launch_due_to_mpv_integration_env() -> bool {
    explicit_mpv_ipc_path_from_env().is_some() || managed_mpv_launch_env_config_from_env().enabled
}

pub(crate) fn legacy_external_player_launch_spec_from_overrides_legacy_compatible(
    overrides: &LegacyClientArgOverrides,
) -> Option<LegacyExternalPlayerLaunchSpec> {
    let program = overrides.player_path.as_deref().map(PathBuf::from)?;
    let mut args = overrides.player_args.clone();
    if let Some(file) = overrides.file.as_deref() {
        args.push(file.to_owned());
    }
    Some(LegacyExternalPlayerLaunchSpec { program, args })
}

pub(crate) fn spawn_legacy_external_player_if_requested_legacy_compatible(
    overrides: &LegacyClientArgOverrides,
) -> anyhow::Result<bool> {
    let Some(spec) = legacy_external_player_launch_spec_from_overrides_legacy_compatible(overrides)
    else {
        return Ok(false);
    };
    if should_skip_legacy_external_player_launch_due_to_mpv_integration_env() {
        if let Some(line) =
            legacy_non_mpv_player_path_ignored_by_mpv_integration_warning_line_legacy_compatible(
                overrides,
            )
        {
            eprintln!("{line}");
        }
        return Ok(false);
    }
    if should_use_automatic_managed_mpv_for_legacy_player_path_legacy_compatible(Some(overrides)) {
        return Ok(false);
    }

    let _child = spawn_legacy_external_player_from_spec_legacy_compatible(&spec)?;
    eprintln!(
        "info: launched external player '{}' (legacy unmanaged startup path)",
        spec.program.display()
    );
    Ok(true)
}

pub(crate) fn spawn_legacy_external_player_from_spec_legacy_compatible(
    spec: &LegacyExternalPlayerLaunchSpec,
) -> anyhow::Result<Child> {
    let mut command = Command::new(&spec.program);
    if let Some(parent) = spec.program.parent()
        && !parent.as_os_str().is_empty()
    {
        command.current_dir(parent);
    }
    if !spec.args.is_empty() {
        command.args(&spec.args);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().map_err(|error| {
        anyhow!(
            "failed to launch legacy external player '{}' with args {:?}: {error}",
            spec.program.display(),
            RedactedCommandArgs::from_args(&spec.args)
        )
    })
}
