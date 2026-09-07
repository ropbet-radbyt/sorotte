//! Independent native fake-server wire contract. No production state reducer is imported.
//! The real-server conversation test compares this boundary against a live server.

use std::io::Write;

pub fn validated_client_playstate_transition(
    value: &serde_json::Value,
) -> Result<Option<(bool, serde_json::Value)>, String> {
    let Some(state) = value.get("State") else {
        return Ok(None);
    };
    let Some(playstate) = state.get("playstate") else {
        return Ok(None);
    };
    let top_level = value.as_object().ok_or_else(|| {
        "playlist-echo mock TCP server received a non-object playstate frame".to_owned()
    })?;
    if top_level.len() != 1 {
        return Err(
            "playlist-echo mock TCP server received a widened playstate top-level schema"
                .to_owned(),
        );
    }
    let state = state.as_object().ok_or_else(|| {
        "playlist-echo mock TCP server received a non-object State playstate frame".to_owned()
    })?;
    if state
        .keys()
        .any(|key| !matches!(key.as_str(), "playstate" | "ping" | "ignoringOnTheFly"))
    {
        return Err(
            "playlist-echo mock TCP server received an unknown field beside client playstate"
                .to_owned(),
        );
    }
    let playstate = playstate.as_object().ok_or_else(|| {
        "playlist-echo mock TCP server received a non-object client playstate".to_owned()
    })?;
    if playstate.keys().any(|key| {
        !matches!(
            key.as_str(),
            "position" | "paused" | "doSeek" | "sorotteTransportRevision"
        )
    }) {
        return Err(
            "playlist-echo mock TCP server received a widened client playstate schema".to_owned(),
        );
    }
    let position = playstate
        .get("position")
        .and_then(serde_json::Value::as_f64)
        .filter(|position| position.is_finite() && *position >= 0.0)
        .ok_or_else(|| {
            "playlist-echo mock TCP server client playstate omitted a finite non-negative position"
                .to_owned()
        })?;
    let paused = playstate
        .get("paused")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| {
            "playlist-echo mock TCP server client playstate omitted a boolean paused state"
                .to_owned()
        })?;
    if playstate
        .get("doSeek")
        .is_some_and(|do_seek| !do_seek.is_null() && do_seek.as_bool().is_none())
    {
        return Err(
            "playlist-echo mock TCP server client playstate used a non-boolean doSeek".to_owned(),
        );
    }
    if playstate
        .get("sorotteTransportRevision")
        .is_some_and(|revision| revision.as_u64().is_none_or(|revision| revision == 0))
    {
        return Err(
            "playlist-echo mock TCP server client playstate used an invalid transport revision"
                .to_owned(),
        );
    }

    let mut authoritative_playstate = serde_json::Map::new();
    authoritative_playstate.insert("position".to_owned(), serde_json::json!(position));
    authoritative_playstate.insert("paused".to_owned(), serde_json::json!(paused));
    authoritative_playstate.insert(
        "doSeek".to_owned(),
        playstate
            .get("doSeek")
            .cloned()
            .filter(|value| !value.is_null())
            .unwrap_or(serde_json::Value::Bool(false)),
    );
    if let Some(revision) = playstate.get("sorotteTransportRevision") {
        authoritative_playstate.insert("sorotteTransportRevision".to_owned(), revision.clone());
    }
    Ok(Some((
        paused,
        serde_json::Value::Object(authoritative_playstate),
    )))
}

pub fn validated_client_ignore_counter(value: &serde_json::Value) -> Result<Option<u32>, String> {
    let Some(counters) = value.pointer("/State/ignoringOnTheFly") else {
        return Ok(None);
    };
    let counters = counters.as_object().ok_or_else(|| {
        "playlist-echo mock TCP server received invalid ignoringOnTheFly counters".to_owned()
    })?;
    if counters.iter().any(|(key, value)| {
        !matches!(key.as_str(), "client" | "server")
            || value
                .as_u64()
                .is_none_or(|counter| counter > u64::from(u32::MAX))
    }) {
        return Err(
            "playlist-echo mock TCP server received invalid ignoringOnTheFly counters".to_owned(),
        );
    }
    Ok(counters
        .get("client")
        .and_then(serde_json::Value::as_u64)
        .and_then(|counter| u32::try_from(counter).ok())
        .filter(|counter| *counter != 0))
}

pub fn write_playlist_echo_counter_ack(
    stream: &mut std::net::TcpStream,
    counter: Option<u32>,
) -> Result<(), String> {
    if let Some(counter) = counter {
        let acknowledgement = serde_json::json!({"State":{"ignoringOnTheFly":{"client":counter}}});
        writeln!(stream, "{acknowledgement}")
            .and_then(|()| stream.flush())
            .map_err(|error| {
                format!(
                    "playlist-echo mock TCP server could not acknowledge client counter: {error}"
                )
            })?;
    }
    Ok(())
}
