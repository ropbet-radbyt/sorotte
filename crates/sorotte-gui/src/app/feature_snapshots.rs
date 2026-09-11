//! Validated updates of feature models shared by UI and worker state.
use super::shell_state::*;
use super::support::normalized_editable_text;

fn normalize_optional_value(
    value: Option<String>,
    error_message: &'static str,
) -> Result<Option<String>, &'static str> {
    match value {
        Some(value) => {
            let Some(value) = normalized_editable_text(&value) else {
                return Err(error_message);
            };
            Ok(Some(value.to_owned()))
        }
        None => Ok(None),
    }
}

pub(super) fn apply_gui_media_index_runtime_snapshot(
    media_index_status: &mut GuiMediaIndexStatusState,
    snapshot: GuiMediaIndexRuntimeSnapshot,
) -> Result<(), &'static str> {
    let message = if snapshot.active {
        let Some(message) = snapshot
            .message
            .as_deref()
            .and_then(normalized_editable_text)
        else {
            return Err(
                "GUI media-index runtime snapshots must include a non-empty message while indexing is active.",
            );
        };
        Some(message)
    } else {
        None
    };

    media_index_status.active = snapshot.active;
    media_index_status.message = message;
    Ok(())
}

pub(super) fn apply_gui_player_setup_runtime_snapshot(
    player_setup_issue: &mut Option<GuiPlayerSetupIssue>,
    snapshot: GuiPlayerSetupRuntimeSnapshot,
) -> Result<(), &'static str> {
    let issue = match snapshot.issue {
        Some(issue) => {
            let Some(message) = normalized_editable_text(&issue.message) else {
                return Err(
                    "GUI player-setup runtime snapshots cannot contain an empty issue message.",
                );
            };
            Some(GuiPlayerSetupIssue {
                kind: issue.kind,
                message,
                retry_available: issue.retry_available,
            })
        }
        None => None,
    };

    (*player_setup_issue) = issue;
    Ok(())
}

pub(super) fn apply_gui_seek_preparation_runtime_snapshot(
    seek_preparation: &mut Option<GuiSeekPreparationState>,
    seek_preparation_degraded_reason: &mut Option<GuiSeekPreparationDegradedReason>,
    snapshot: GuiSeekPreparationRuntimeSnapshot,
) -> Result<(), &'static str> {
    if snapshot.preparation.is_some() && snapshot.degraded_reason.is_some() {
        return Err(
            "GUI seek-preparation snapshots cannot be active and terminally degraded at the same time.",
        );
    }
    let preparation = match snapshot.preparation {
        Some(preparation) => {
            if !preparation.frozen_target_seconds.is_finite()
                || preparation.frozen_target_seconds < 0.0
            {
                return Err(
                    "GUI seek-preparation snapshots require a finite, non-negative target.",
                );
            }
            if preparation
                .cache_refill_percent
                .is_some_and(|percent| !percent.is_finite() || !(0.0..=100.0).contains(&percent))
            {
                return Err(
                    "GUI seek-preparation snapshots require cache refill between 0 and 100 percent.",
                );
            }
            if preparation
                .buffered_ahead_seconds
                .is_some_and(|seconds| !seconds.is_finite() || seconds < 0.0)
            {
                return Err(
                    "GUI seek-preparation snapshots require finite, non-negative buffered-ahead time.",
                );
            }
            if preparation
                .nearest_safe_buffered_position_seconds
                .is_some_and(|seconds| !seconds.is_finite() || seconds < 0.0)
            {
                return Err(
                    "GUI seek-preparation snapshots require a finite, non-negative nearest buffered position.",
                );
            }
            if preparation.can_join_nearest_buffered
                && preparation.nearest_safe_buffered_position_seconds.is_none()
            {
                return Err(
                    "GUI seek-preparation snapshots cannot enable nearest-buffered joining without a safe position.",
                );
            }
            Some(preparation)
        }
        None => None,
    };

    (*seek_preparation) = preparation;
    (*seek_preparation_degraded_reason) = snapshot.degraded_reason;
    Ok(())
}

pub(super) fn apply_gui_stream_helper_runtime_snapshot(
    stream_helper: &mut GuiStreamHelperState,
    snapshot: GuiStreamHelperRuntimeSnapshot,
) -> Result<(), &'static str> {
    let message = match snapshot.message {
        Some(message) => {
            let Some(message) = normalized_editable_text(&message) else {
                return Err(
                    "GUI stream-helper runtime snapshots cannot contain an empty issue message.",
                );
            };
            Some(message)
        }
        None => None,
    };
    if snapshot.health != GuiStreamHelperHealth::Healthy && message.is_none() {
        return Err(
            "GUI stream-helper runtime snapshots must include a non-empty message while unhealthy.",
        );
    }
    let install_location = normalize_optional_value(
        snapshot.install_location,
        "GUI stream-helper runtime snapshots cannot contain an empty install location.",
    )?;
    let downloader_status = normalize_optional_value(
        snapshot.downloader_status,
        "GUI stream-helper runtime snapshots cannot contain an empty yt-dlp status.",
    )?;
    let js_runtime_status = normalize_optional_value(
        snapshot.js_runtime_status,
        "GUI stream-helper runtime snapshots cannot contain an empty Deno status.",
    )?;

    stream_helper.health = snapshot.health;
    stream_helper.message = message;
    stream_helper.target = snapshot
        .target
        .and_then(|target| normalized_editable_text(&target));
    stream_helper.install_supported = snapshot.install_supported;
    stream_helper.integration_supported = snapshot.integration_supported;
    stream_helper.retry_available = snapshot.retry_available;
    stream_helper.install_location = install_location;
    stream_helper.downloader_status = downloader_status;
    stream_helper.js_runtime_status = js_runtime_status;
    stream_helper.open_install_location_available = snapshot.open_install_location_available;
    Ok(())
}

pub(super) fn apply_gui_stream_helper_remediation_runtime_snapshot(
    stream_helper_remediation: &mut GuiStreamHelperRemediationState,
    snapshot: GuiStreamHelperRemediationRuntimeSnapshot,
) -> Result<(), &'static str> {
    let label = match snapshot.label {
        Some(label) => {
            let Some(label) = normalized_editable_text(&label) else {
                return Err(
                    "GUI stream-helper remediation snapshots cannot contain an empty label.",
                );
            };
            Some(label)
        }
        None => None,
    };
    let detail = match snapshot.detail {
        Some(detail) => {
            let Some(detail) = normalized_editable_text(&detail) else {
                return Err(
                    "GUI stream-helper remediation snapshots cannot contain an empty detail.",
                );
            };
            Some(detail)
        }
        None => None,
    };
    if snapshot.active && label.is_none() {
        return Err(
            "GUI stream-helper remediation snapshots must include a non-empty label while active.",
        );
    }
    if !snapshot.progress_fraction.is_finite() || !(0.0..=1.0).contains(&snapshot.progress_fraction)
    {
        return Err(
            "GUI stream-helper remediation snapshots must use a progress value between 0.0 and 1.0.",
        );
    }

    stream_helper_remediation.active = snapshot.active;
    stream_helper_remediation.label = label.filter(|_| snapshot.active);
    stream_helper_remediation.detail = detail.filter(|_| snapshot.active);
    stream_helper_remediation.progress_fraction = if snapshot.active {
        snapshot.progress_fraction
    } else {
        0.0
    };
    Ok(())
}

pub(super) fn apply_gui_media_match_runtime_snapshot(
    media_match: &mut GuiMediaMatchState,
    snapshot: GuiMediaMatchRuntimeSnapshot,
) -> Result<(), &'static str> {
    let message = match snapshot.message {
        Some(message) => {
            let Some(message) = normalized_editable_text(&message) else {
                return Err(
                    "GUI media-match runtime snapshots cannot contain an empty issue message.",
                );
            };
            Some(message)
        }
        None => None,
    };
    if snapshot.health != GuiMediaMatchToolHealth::Healthy && message.is_none() {
        return Err(
            "GUI media-match runtime snapshots must include a non-empty message while unhealthy.",
        );
    }
    let install_location = normalize_optional_value(
        snapshot.install_location,
        "GUI media-match runtime snapshots cannot contain an empty install location.",
    )?;
    let ffmpeg_status = normalize_optional_value(
        snapshot.ffmpeg_status,
        "GUI media-match runtime snapshots cannot contain an empty ffmpeg status.",
    )?;
    let ffprobe_status = normalize_optional_value(
        snapshot.ffprobe_status,
        "GUI media-match runtime snapshots cannot contain an empty ffprobe status.",
    )?;
    media_match.settings = snapshot.settings;
    media_match.health = snapshot.health;
    media_match.message = message;
    media_match.install_supported = snapshot.install_supported;
    media_match.integration_supported = snapshot.integration_supported;
    media_match.install_location = install_location;
    media_match.ffmpeg_status = ffmpeg_status;
    media_match.ffprobe_status = ffprobe_status;
    media_match.cache_status = snapshot
        .cache_status
        .and_then(|value| normalized_editable_text(&value));
    media_match.current_decision = snapshot
        .current_decision
        .and_then(|value| normalized_editable_text(&value));
    media_match.nearest_match = snapshot
        .nearest_match
        .and_then(|value| normalized_editable_text(&value));
    media_match.last_evidence = snapshot
        .last_evidence
        .and_then(|value| normalized_editable_text(&value));
    media_match.remote_status = snapshot
        .remote_status
        .and_then(|value| normalized_editable_text(&value));
    media_match.background_status = snapshot
        .background_status
        .and_then(|value| normalized_editable_text(&value));
    media_match.open_install_location_available = snapshot.open_install_location_available;
    Ok(())
}

pub(super) fn apply_gui_media_match_remediation_runtime_snapshot(
    media_match_remediation: &mut GuiMediaMatchRemediationState,
    snapshot: GuiMediaMatchRemediationRuntimeSnapshot,
) -> Result<(), &'static str> {
    let label = match snapshot.label {
        Some(label) => {
            let Some(label) = normalized_editable_text(&label) else {
                return Err("GUI media-match remediation snapshots cannot contain an empty label.");
            };
            Some(label)
        }
        None => None,
    };
    let detail = match snapshot.detail {
        Some(detail) => {
            let Some(detail) = normalized_editable_text(&detail) else {
                return Err(
                    "GUI media-match remediation snapshots cannot contain an empty detail.",
                );
            };
            Some(detail)
        }
        None => None,
    };
    if snapshot.active && label.is_none() {
        return Err(
            "GUI media-match remediation snapshots must include a non-empty label while active.",
        );
    }
    if !snapshot.progress_fraction.is_finite() || !(0.0..=1.0).contains(&snapshot.progress_fraction)
    {
        return Err(
            "GUI media-match remediation snapshots must use a progress value between 0.0 and 1.0.",
        );
    }

    media_match_remediation.active = snapshot.active;
    media_match_remediation.label = label.filter(|_| snapshot.active);
    media_match_remediation.detail = detail.filter(|_| snapshot.active);
    media_match_remediation.progress_fraction = if snapshot.active {
        snapshot.progress_fraction
    } else {
        0.0
    };
    Ok(())
}

pub(super) fn apply_gui_plex_runtime_snapshot(
    plex: &mut GuiPlexState,
    snapshot: GuiPlexRuntimeSnapshot,
) -> Result<(), &'static str> {
    if snapshot.status.trim().is_empty() {
        return Err("GUI Plex runtime snapshots cannot contain empty status.");
    }
    plex.enabled = snapshot.enabled;
    plex.streaming_enabled = snapshot.streaming_enabled;
    plex.authenticated = snapshot.authenticated;
    plex.authenticating = snapshot.authenticating;
    plex.auth_code = snapshot
        .auth_code
        .and_then(|value| normalized_editable_text(&value));
    plex.auth_url = snapshot
        .auth_url
        .and_then(|value| normalized_editable_text(&value));
    plex.selected_server_id = snapshot
        .selected_server_id
        .and_then(|value| normalized_editable_text(&value));
    plex.selected_server_url = snapshot
        .selected_server_url
        .and_then(|value| normalized_editable_text(&value));
    plex.servers = snapshot
        .servers
        .into_iter()
        .filter_map(|server| {
            Some(GuiPlexServerRow {
                name: normalized_editable_text(&server.name)?,
                machine_identifier: normalized_editable_text(&server.machine_identifier)?,
                uri: normalized_editable_text(&server.uri)?,
                reachability: server.reachability,
                connection_kind: server.connection_kind,
                has_local_connection: server.has_local_connection,
                owned: server.owned,
                selected: server.selected,
            })
        })
        .collect();
    plex.status = snapshot.status;
    plex.current_item = snapshot
        .current_item
        .and_then(|value| normalized_editable_text(&value));
    plex.last_report = snapshot
        .last_report
        .and_then(|value| normalized_editable_text(&value));
    plex.last_error = snapshot
        .last_error
        .and_then(|value| normalized_editable_text(&value));
    Ok(())
}

pub(super) fn complete_plex_playlist_search(
    playlist_search: &mut Option<GuiPlexPlaylistSearchState>,
    query: String,
    results: Vec<GuiPlexPlaylistSearchResult>,
    error: Option<String>,
) -> bool {
    let Some(search) = (*playlist_search).as_mut() else {
        return false;
    };
    if !search.searching || search.query.as_str() != query.as_str() {
        return false;
    }
    search.query = query;
    search.searching = false;
    search.adding_rating_key = None;
    search.error = error.and_then(|message| normalized_editable_text(&message));
    if search.error.is_some() {
        search.results.clear();
        search.selected_index = None;
    } else {
        search.results = results;
        search.selected_index = if search.results.is_empty() {
            None
        } else {
            Some(
                search
                    .selected_index
                    .unwrap_or(0)
                    .min(search.results.len().saturating_sub(1)),
            )
        };
    }
    true
}
pub(super) fn complete_plex_playlist_item_resolve(
    playlist_search: &mut Option<GuiPlexPlaylistSearchState>,
    rating_key: String,
    error: Option<String>,
) -> bool {
    let Some(search) = (*playlist_search).as_mut() else {
        return false;
    };
    if search.adding_rating_key.as_deref() != Some(rating_key.as_str()) {
        return false;
    }
    search.adding_rating_key = None;
    search.error = error.and_then(|message| normalized_editable_text(&message));
    true
}

pub(super) fn apply_menu_dialog_snapshot(
    menus: &mut MenuDialogShellState,
    overrides: &mut Vec<MenuActionRuntimeOverride>,
    settings: &sorotte_client_app::app_boundary::state::StoredClientSettings,
    snapshot: MenuDialogRuntimeSnapshot,
) -> Result<(), String> {
    let baseline = MenuDialogShellState::from_stored_settings(settings);
    for action_override in snapshot.action_overrides {
        if let Some(action) = baseline.action(action_override.id) {
            if action.enabled == action_override.enabled {
                overrides.retain(|current| current.id != action_override.id);
            } else if let Some(current) = overrides
                .iter_mut()
                .find(|current| current.id == action_override.id)
            {
                *current = action_override.clone();
            } else {
                overrides.push(action_override.clone());
            }
        }
        let Some(action) = menus.action_mut(action_override.id) else {
            return Err(format!(
                "No menu action exists for '{}' in the runtime snapshot.",
                action_override.id.automation_id()
            ));
        };
        action.enabled = action_override.enabled;
    }
    menus.tls_prompt_expected = snapshot.tls_prompt_expected;
    menus.update_notice_expected = snapshot.update_notice_expected;
    menus.about_dialog_available = snapshot.about_dialog_available;
    Ok(())
}
