use serde_json::json;
use sorotte_client_core::{ClientRuntime, ClientSession, QueuedRuntimeControl};
use sorotte_player_api::DisconnectedPlayer;
use sorotte_player_mpv::{LifecycleVerificationPlaylistEntry, MpvLifecycleVerificationHarness};

fn apply_and_ack(
    harness: &mut MpvLifecycleVerificationHarness,
    runtime: &mut ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl>,
    now_seconds: f64,
) {
    let batch = harness.take_event_batch().expect("player event batch");
    runtime
        .apply_ordered_player_event_batch_for_verification(&batch, now_seconds)
        .expect("production consumer should accept the batch");
    harness
        .acknowledge(batch.acknowledgement_token)
        .expect("adapter acknowledgement");
    runtime.compact_acknowledged_player_event_batch_for_verification(
        batch.acknowledgement_token,
        batch.sequence_boundary,
    );
}

fn loaded_runtime() -> (
    MpvLifecycleVerificationHarness,
    ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl>,
) {
    let mut harness = MpvLifecycleVerificationHarness::new();
    harness.accept_tracked_load("https://media.test/stream", []);
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            77,
            Some("https://media.test/stream".to_owned()),
            true,
        )],
        Some("https://media.test/stream".to_owned()),
    );
    harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": 77
    }));
    harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));

    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    apply_and_ack(&mut harness, &mut runtime, 1.0);

    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "pause",
        "data": false
    }));
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "paused-for-cache",
        "data": false
    }));
    apply_and_ack(&mut harness, &mut runtime, 2.0);
    assert_eq!(runtime.session().local_paused(), Some(false));
    (harness, runtime)
}

#[test]
fn cache_pause_does_not_become_a_logical_room_pause_in_acknowledged_mode() {
    let (mut harness, mut runtime) = loaded_runtime();

    // mpv can expose its internal cache stop first as pause=true and then
    // classify it as paused-for-cache. The acknowledged stream must preserve
    // the pre-existing invariant that this is not a user/room pause.
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "pause",
        "data": true
    }));
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "paused-for-cache",
        "data": true
    }));
    apply_and_ack(&mut harness, &mut runtime, 3.0);

    assert_eq!(
        runtime.session().local_paused(),
        Some(false),
        "cache-only pause must not overwrite the last logical playing state"
    );
    assert_eq!(runtime.session().local_paused_for_cache(), Some(true));
}

#[test]
fn explicit_pause_remains_logical_while_cache_pause_is_active() {
    let (mut harness, mut runtime) = loaded_runtime();

    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "paused-for-cache",
        "data": true
    }));
    apply_and_ack(&mut harness, &mut runtime, 3.0);
    assert_eq!(runtime.session().local_paused(), Some(false));
    assert_eq!(runtime.session().local_paused_for_cache(), Some(true));

    harness.accept_tracked_pause();
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "pause",
        "data": true
    }));
    apply_and_ack(&mut harness, &mut runtime, 4.0);

    assert_eq!(
        runtime.session().local_paused(),
        Some(true),
        "an explicitly owned pause must survive an active cache stall"
    );
    assert_eq!(runtime.session().local_paused_for_cache(), Some(true));
}
