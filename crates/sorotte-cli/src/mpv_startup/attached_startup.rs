use super::*;

pub(crate) fn apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible<
    P,
>(
    player: &mut P,
    overrides: &LegacyClientArgOverrides,
) -> anyhow::Result<bool>
where
    P: PlayerAdapter,
{
    if explicit_mpv_ipc_path_from_env().is_none() {
        return Ok(false);
    }
    let startup_arg_analysis =
        analyze_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible(
            &overrides.player_args,
        );
    let startup_args = &startup_arg_analysis.parsed;
    let mut applied = false;
    let mut applied_supported_commands = 0usize;
    for command in &startup_arg_analysis.runtime_commands {
        match command {
            LegacyExplicitMpvIpcStartupPlayerCommand::SetOptionString { name, value } => {
                retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
                    player.set_option_string(name, value)
                })
                .map_err(|error| {
                    anyhow!(
                        "failed applying legacy explicit-mpv-IPC startup option {:?}: {error}",
                        RedactedCommandArgs::from_option_names(std::iter::once(name))
                    )
                })?;
            }
            LegacyExplicitMpvIpcStartupPlayerCommand::ApplyProfile { profile } => {
                retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
                    player.apply_profile(profile)
                })
                .map_err(|error| {
                    anyhow!(
                        "failed applying legacy explicit-mpv-IPC startup profile argument {:?}: {error}",
                        RedactedCommandArgs::from_count(1)
                    )
                })?;
            }
        }
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(file) = overrides.file.as_deref() {
        player.open_file(file).map_err(|error| {
            anyhow!("failed opening legacy startup file via attached player: {error}")
        })?;
        applied = true;
    }
    if let Some(start_position_seconds) = startup_args.start_position_seconds {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_position(start_position_seconds)
        })
        .map_err(|error| {
            anyhow!("failed applying legacy explicit-mpv-IPC startup '--start' override: {error}")
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(paused) = startup_args.paused {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_paused(paused)
        })
        .map_err(|error| {
            anyhow!("failed applying legacy explicit-mpv-IPC startup '--pause' override: {error}")
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(playback_rate) = startup_args.playback_rate {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_playback_rate(playback_rate)
        })
        .map_err(|error| {
            anyhow!("failed applying legacy explicit-mpv-IPC startup '--speed' override: {error}")
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(volume) = startup_args.volume {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_volume(volume)
        })
        .map_err(|error| {
            anyhow!("failed applying legacy explicit-mpv-IPC startup '--volume' override: {error}")
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(muted) = startup_args.muted {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| player.set_muted(muted))
            .map_err(|error| {
                anyhow!(
                    "failed applying legacy explicit-mpv-IPC startup '--mute' override: {error}"
                )
            })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(deinterlace) = startup_args.deinterlace {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_deinterlace(deinterlace)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--deinterlace' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(keepaspect) = startup_args.keepaspect {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_keepaspect(keepaspect)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--keepaspect' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(keepaspect_window) = startup_args.keepaspect_window {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_keepaspect_window(keepaspect_window)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--keepaspect-window' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(fullscreen) = startup_args.fullscreen {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_fullscreen(fullscreen)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--fullscreen' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(ontop) = startup_args.ontop {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| player.set_ontop(ontop))
            .map_err(|error| {
                anyhow!(
                    "failed applying legacy explicit-mpv-IPC startup '--ontop' override: {error}"
                )
            })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(border) = startup_args.border {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_border(border)
        })
        .map_err(|error| {
            anyhow!("failed applying legacy explicit-mpv-IPC startup '--border' override: {error}")
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(force_window) = startup_args.force_window {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_force_window(force_window)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--force-window' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(keep_open) = startup_args.keep_open {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_keep_open(keep_open)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--keep-open' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(keep_open_pause) = startup_args.keep_open_pause {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_keep_open_pause(keep_open_pause)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--keep-open-pause' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(cursor_autohide_fs_only) = startup_args.cursor_autohide_fs_only {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_cursor_autohide_fs_only(cursor_autohide_fs_only)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--cursor-autohide-fs-only' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(stop_screensaver) = startup_args.stop_screensaver {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_stop_screensaver(stop_screensaver)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--stop-screensaver' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(sub_visibility) = startup_args.sub_visibility {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_sub_visibility(sub_visibility)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--sub-visibility' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(osd_bar) = startup_args.osd_bar {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_osd_bar(osd_bar)
        })
        .map_err(|error| {
            anyhow!("failed applying legacy explicit-mpv-IPC startup '--osd-bar' override: {error}")
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(window_maximized) = startup_args.window_maximized {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_window_maximized(window_maximized)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--window-maximized' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    if let Some(window_minimized) = startup_args.window_minimized {
        retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(|| {
            player.set_window_minimized(window_minimized)
        })
        .map_err(|error| {
            anyhow!(
                "failed applying legacy explicit-mpv-IPC startup '--window-minimized' override: {error}"
            )
        })?;
        applied_supported_commands += 1;
        applied = true;
    }
    emit_legacy_explicit_mpv_ipc_startup_player_arg_diagnostics_legacy_compatible(
        &startup_arg_analysis.diagnostics,
        applied_supported_commands,
    );
    Ok(applied)
}

fn should_retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(
    error: &PlayerError,
) -> bool {
    let PlayerError::OperationFailed(message) = error else {
        return false;
    };
    let lower = message.to_ascii_lowercase();
    lower.contains("property unavailable") || lower.contains("no file loaded")
}

pub(crate) fn retry_explicit_mpv_ipc_startup_player_command_legacy_compatible<F>(
    mut operation: F,
) -> Result<(), PlayerError>
where
    F: FnMut() -> Result<(), PlayerError>,
{
    const MAX_ATTEMPTS: usize = 20;
    const RETRY_DELAY: Duration = Duration::from_millis(25);

    let mut last_error = None;
    for attempt in 0..MAX_ATTEMPTS {
        match operation() {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < MAX_ATTEMPTS
                    && should_retry_explicit_mpv_ipc_startup_player_command_legacy_compatible(
                        &error,
                    ) =>
            {
                last_error = Some(error);
                std::thread::sleep(RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        PlayerError::OperationFailed(
            "startup command retry unexpectedly exhausted without error".to_owned(),
        )
    }))
}
