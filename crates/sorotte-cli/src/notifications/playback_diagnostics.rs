use super::*;

#[cfg(test)]
pub(crate) fn player_playback_telemetry_update_message(
    update: &PlayerPlaybackTelemetryUpdate,
) -> Option<String> {
    let mut fields = Vec::new();
    if let Some(paused) = update.paused {
        fields.push(format!("paused={paused}"));
    }
    if let Some(position_seconds) = update.position_seconds {
        fields.push(format!("position={position_seconds:.3}"));
    }
    if let Some(playback_rate) = update.playback_rate {
        fields.push(format!("speed={playback_rate:.3}"));
    }
    if let Some(paused_for_cache) = update.paused_for_cache {
        fields.push(format!("paused-for-cache={paused_for_cache}"));
    }
    if let Some(cache_buffering_percent) = update.cache_buffering_percent {
        fields.push(format!("cache-buffering={cache_buffering_percent:.1}%"));
    }

    if fields.is_empty() {
        None
    } else {
        Some(format!("player telemetry: {}", fields.join(" ")))
    }
}

fn localized_player_telemetry_prefix_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Player-Telemetrie",
        Some("es") => "Telemetria del reproductor",
        Some("eo") => "Ludila telemetrio",
        Some("fi") => "Soittimen telemetria",
        Some("fr") => "Telemetrie du lecteur",
        Some("it") => "Telemetria del player",
        Some("pt_PT" | "pt_BR") => "Telemetria do player",
        Some("tr") => "Oynatici telemetrisi",
        Some("ru") => "Telemetriia pleera",
        Some("zh_CN") => "Bofangqi ceju",
        Some("ko") => "Peulleieo tellemetri",
        _ => "player telemetry",
    }
}

pub(crate) fn player_playback_telemetry_update_message_localized_legacy_compatible(
    update: &PlayerPlaybackTelemetryUpdate,
    language: Option<&str>,
) -> Option<String> {
    let mut fields = Vec::new();
    if let Some(paused) = update.paused {
        fields.push(format!("paused={paused}"));
    }
    if let Some(position_seconds) = update.position_seconds {
        fields.push(format!("position={position_seconds:.3}"));
    }
    if let Some(playback_rate) = update.playback_rate {
        fields.push(format!("speed={playback_rate:.3}"));
    }
    if let Some(paused_for_cache) = update.paused_for_cache {
        fields.push(format!("paused-for-cache={paused_for_cache}"));
    }
    if let Some(cache_buffering_percent) = update.cache_buffering_percent {
        fields.push(format!("cache-buffering={cache_buffering_percent:.1}%"));
    }

    if fields.is_empty() {
        None
    } else {
        Some(format!(
            "{}: {}",
            localized_player_telemetry_prefix_legacy_compatible(language),
            fields.join(" ")
        ))
    }
}

fn emit_player_playback_telemetry_update(
    update: &PlayerPlaybackTelemetryUpdate,
) -> anyhow::Result<()> {
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let Some(message) = player_playback_telemetry_update_message_localized_legacy_compatible(
        update,
        language.as_deref(),
    ) else {
        return Ok(());
    };
    println!("{message}");
    Ok(())
}

fn localized_player_drift_prefix_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Player-Abweichung",
        Some("es") => "Desajuste del reproductor",
        Some("eo") => "Ludila diferenco",
        Some("fi") => "Soittimen poikkeama",
        Some("fr") => "Derive du lecteur",
        Some("it") => "Deriva del player",
        Some("pt_PT" | "pt_BR") => "Desvio do player",
        Some("tr") => "Oynatici kaymasi",
        Some("ru") => "Rassinkhronizatsiia pleera",
        Some("zh_CN") => "Bofangqi piancha",
        Some("ko") => "Peulleieo eotnamm",
        _ => "player drift",
    }
}

fn localized_paused_mismatch_label_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Pause-Abweichung",
        Some("es") => "desajuste de pausa",
        Some("eo") => "pauza malsamo",
        Some("fi") => "tauon ero",
        Some("fr") => "ecart de pause",
        Some("it") => "disallineamento pausa",
        Some("pt_PT" | "pt_BR") => "desalinhamento de pausa",
        Some("tr") => "duraklatma uyusmazligi",
        Some("ru") => "rassoglasovanie pausy",
        Some("zh_CN") => "zanting bu pipei",
        Some("ko") => "ilsi jeongji bul-ilchi",
        _ => "paused mismatch",
    }
}

fn localized_position_mismatch_label_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Positionsabweichung",
        Some("es") => "desajuste de posicion",
        Some("eo") => "pozicia malsamo",
        Some("fi") => "sijainnin ero",
        Some("fr") => "ecart de position",
        Some("it") => "disallineamento posizione",
        Some("pt_PT" | "pt_BR") => "desalinhamento de posicao",
        Some("tr") => "konum uyusmazligi",
        Some("ru") => "rassoglasovanie pozitsii",
        Some("zh_CN") => "weizhi bu pipei",
        Some("ko") => "wichi bul-ilchi",
        _ => "position mismatch",
    }
}

pub(crate) fn player_playback_drift_diagnostic_messages_localized_legacy_compatible(
    update: &PlayerPlaybackTelemetryUpdate,
    room_playstate: Option<&RoomPlaystateView>,
    language: Option<&str>,
) -> Vec<String> {
    let Some(room_playstate) = room_playstate else {
        return Vec::new();
    };

    let mut messages = Vec::new();

    if let (Some(player_paused), Some(room_paused)) = (update.paused, room_playstate.paused)
        && player_paused != room_paused
    {
        messages.push(format!(
            "{}: {} player={player_paused} room={room_paused}",
            localized_player_drift_prefix_legacy_compatible(language),
            localized_paused_mismatch_label_legacy_compatible(language),
        ));
    }

    if let (Some(player_position), Some(room_position)) =
        (update.position_seconds, room_playstate.position)
    {
        let diff = (player_position - room_position).abs();
        if diff > PLAYER_DRIFT_DIAGNOSTIC_THRESHOLD_SECONDS {
            messages.push(format!(
                "{}: {} player={player_position:.3} room={room_position:.3} diff={diff:.3}",
                localized_player_drift_prefix_legacy_compatible(language),
                localized_position_mismatch_label_legacy_compatible(language),
            ));
        }
    }

    messages
}

fn emit_player_playback_drift_diagnostic(message: &str) -> anyhow::Result<()> {
    println!("{message}");
    Ok(())
}

fn seek_target_clock_label(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "unknown target".to_owned();
    }
    let total_seconds = seconds.max(0.0).round() as u64;
    let hours = total_seconds / 3_600;
    let minutes = total_seconds % 3_600 / 60;
    let seconds = total_seconds % 60;
    if hours == 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

fn seek_target_availability_label(availability: SeekTargetAvailability) -> &'static str {
    match availability {
        SeekTargetAvailability::Cached => "cached",
        SeekTargetAvailability::FetchRequired => "fetch-required",
        SeekTargetAvailability::Unknown => "unknown",
        SeekTargetAvailability::OutsideLiveWindow => "outside-live-window",
        SeekTargetAvailability::NonSeekable => "non-seekable",
    }
}

fn seek_preparation_terminal_label(outcome: SeekPreparationTerminalOutcome) -> String {
    match outcome {
        SeekPreparationTerminalOutcome::Ready => "ready".to_owned(),
        SeekPreparationTerminalOutcome::Superseded => "superseded".to_owned(),
        SeekPreparationTerminalOutcome::Cancelled => "cancelled".to_owned(),
        SeekPreparationTerminalOutcome::Degraded(reason) => {
            format!("degraded ({reason:?})")
        }
    }
}

/// Produces an observation-only diagnostic projection. In particular, mpv's
/// cache percentage is called refill progress and no ETA is inferred from
/// approximate input-rate or cache-duration telemetry.
pub(crate) fn seek_preparation_diagnostic_messages(
    preparation: Option<&SeekPreparationSnapshot>,
    last_terminal_outcome: Option<SeekPreparationTerminalOutcome>,
) -> Vec<String> {
    let Some(preparation) = preparation else {
        return last_terminal_outcome
            .map(|outcome| {
                vec![format!(
                    "seek preparation: terminal={}",
                    seek_preparation_terminal_label(outcome)
                )]
            })
            .unwrap_or_default();
    };

    let target = seek_target_clock_label(preparation.frozen_target_seconds);
    let availability = seek_target_availability_label(preparation.availability);
    let status = match preparation.phase {
        SeekPreparationPhase::Seeking => format!("Seeking to {target}"),
        SeekPreparationPhase::Fetching => {
            format!("Fetching stream data for {target}")
        }
        SeekPreparationPhase::Refilling => preparation.cache_buffering_percent.map_or_else(
            || "Buffer refill in progress".to_owned(),
            |percent| format!("Buffer refill: {:.0}%", percent.clamp(0.0, 100.0)),
        ),
        SeekPreparationPhase::ReadyToJoin => "Ready - joining the room".to_owned(),
        SeekPreparationPhase::CatchingUp => "Catching up to the room".to_owned(),
    };
    let mut messages = vec![format!(
        "seek preparation: {status}; availability={availability}"
    )];
    if let Some(buffered_ahead) = preparation
        .buffered_ahead_seconds
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
    {
        messages.push(format!(
            "seek preparation: {buffered_ahead:.1} seconds buffered ahead"
        ));
    }

    let mut actions = Vec::new();
    if preparation.can_keep_waiting {
        actions.push("keep-waiting");
    }
    if preparation.can_join_nearest_buffered {
        actions.push("join-nearest-buffered-position");
    }
    if preparation.can_cancel_and_remain {
        actions.push("cancel-and-remain");
    }
    if !actions.is_empty() {
        messages.push(format!("seek preparation actions: {}", actions.join(",")));
    }
    messages
}

#[derive(Debug, Default)]
pub(crate) struct SeekPreparationNotificationState {
    last_fingerprint: Option<String>,
}

pub(crate) fn next_seek_preparation_notification_messages(
    preparation: Option<&SeekPreparationSnapshot>,
    last_terminal: Option<&SeekPreparationSnapshot>,
    state: &mut SeekPreparationNotificationState,
) -> Vec<String> {
    let (fingerprint, messages) = if let Some(preparation) = preparation {
        let messages = seek_preparation_diagnostic_messages(Some(preparation), None);
        (
            Some(format!(
                "active:{}:{}:{}:{messages:?}",
                preparation.media_generation, preparation.load_attempt, preparation.id,
            )),
            messages,
        )
    } else if let Some(terminal) = last_terminal {
        let outcome = terminal.terminal_outcome;
        let messages = seek_preparation_diagnostic_messages(None, outcome);
        (
            outcome.map(|outcome| {
                format!(
                    "terminal:{}:{}:{}:{outcome:?}",
                    terminal.media_generation, terminal.load_attempt, terminal.id,
                )
            }),
            messages,
        )
    } else {
        (None, Vec::new())
    };

    if state.last_fingerprint == fingerprint {
        return Vec::new();
    }
    state.last_fingerprint = fingerprint;
    messages
}

pub(crate) fn flush_seek_preparation_notifications(
    runtime: &ClientApplication<MpvAdapter>,
    state: &mut SeekPreparationNotificationState,
) {
    let coordination = runtime.playback_coordination_snapshot();
    for message in next_seek_preparation_notification_messages(
        coordination.seek_preparation.as_ref(),
        coordination.last_seek_preparation_terminal.as_ref(),
        state,
    ) {
        println!("{message}");
    }
}

pub(crate) fn flush_player_playback_telemetry_diagnostics(
    runtime: &mut ClientApplication<MpvAdapter>,
    log_telemetry: bool,
    log_drift: bool,
) -> anyhow::Result<()> {
    if !log_telemetry && !log_drift {
        return Ok(());
    }

    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let room_playstate = runtime.session().current_room_playstate().cloned();
    let updates = runtime.drain_player_playback_telemetry_updates();
    for update in &updates {
        if log_telemetry {
            emit_player_playback_telemetry_update(update)?;
        }
        if log_drift {
            for message in player_playback_drift_diagnostic_messages_localized_legacy_compatible(
                update,
                room_playstate.as_ref(),
                language.as_deref(),
            ) {
                emit_player_playback_drift_diagnostic(&message)?;
            }
        }
    }
    if log_telemetry {
        let coordination = runtime.playback_coordination_snapshot();
        let recovery = coordination
            .recovery_episode
            .as_ref()
            .map(|episode| format!("episode-{}", episode.id))
            .unwrap_or_else(|| "none".to_owned());
        println!(
            "playback coordinator: phase={:?} recovery={} degraded={:?} buffer-episodes={} hard-seeks={}",
            coordination.diagnostic,
            recovery,
            coordination.last_degraded_reason,
            coordination.metrics.buffer_episode_count,
            coordination.metrics.hard_seek_count,
        );
        if let Some(suggestion) = runtime.streaming_quality_downgrade_suggestion(None) {
            println!(
                "stream quality suggestion: current={} recommended={} reason={:?} (no automatic change was made)",
                suggestion.current.config_value(),
                suggestion.recommended.config_value(),
                suggestion.reason,
            );
        }
    }

    Ok(())
}
