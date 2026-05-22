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

pub(crate) fn flush_player_playback_telemetry_diagnostics(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
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

    Ok(())
}
