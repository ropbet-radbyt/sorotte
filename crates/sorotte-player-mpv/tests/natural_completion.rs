#![cfg(feature = "test-support")]

use serde_json::json;
use sorotte_client_core::{ClientRuntime, ClientSession, QueuedRuntimeControl};
use sorotte_player_api::{DisconnectedPlayer, SnapshotField};
use sorotte_player_mpv::{LifecycleVerificationPlaylistEntry, MpvLifecycleVerificationHarness};

fn apply_and_ack(
    harness: &mut MpvLifecycleVerificationHarness,
    runtime: &mut ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl>,
    now: f64,
) {
    let Some(batch) = harness.take_event_batch() else {
        return;
    };
    runtime
        .apply_ordered_player_event_batch_for_verification(&batch, now)
        .expect("production ordered consumer applies the adapter batch");
    harness.acknowledge(batch.acknowledgement_token).unwrap();
    runtime.compact_acknowledged_player_event_batch_for_verification(
        batch.acknowledgement_token,
        batch.sequence_boundary,
    );
}

type Runtime = ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl>;

fn loaded_playback(position: f64) -> (MpvLifecycleVerificationHarness, Runtime) {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    session.apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice","sorottePlaylistEpoch":1}}}"#).unwrap();
    session
        .apply_message_json(
            r#"{"Set":{"playlistIndex":{"index":0,"user":"alice","sorottePlaylistEpoch":2}}}"#,
        )
        .unwrap();
    let mut runtime =
        ClientRuntime::new(session, DisconnectedPlayer, QueuedRuntimeControl::default());
    let mut harness = MpvLifecycleVerificationHarness::new();
    harness.accept_tracked_load("episode1.mkv", []);
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            77,
            Some("episode1.mkv".into()),
            true,
        )],
        Some("episode1.mkv".into()),
    );
    harness.ingest_decoded_mpv_json(json!({"event":"start-file", "playlist_entry_id":77}));
    harness.ingest_decoded_mpv_json(
        json!({"event":"property-change", "name":"path", "data":"episode1.mkv"}),
    );
    harness.ingest_decoded_mpv_json(
        json!({"event":"property-change", "name":"duration", "data":240.0}),
    );
    harness.ingest_decoded_mpv_json(json!({"event":"file-loaded"}));
    harness.ingest_decoded_mpv_json(json!({"event":"playback-restart"}));
    harness
        .ingest_decoded_mpv_json(json!({"event":"property-change", "name":"pause", "data":false}));
    harness.ingest_decoded_mpv_json(
        json!({"event":"property-change", "name":"paused-for-cache", "data":false}),
    );
    harness.ingest_decoded_mpv_json(
        json!({"event":"property-change", "name":"time-pos", "data":position}),
    );
    apply_and_ack(&mut harness, &mut runtime, 1.0);
    assert_eq!(
        runtime.session().current_room_playlist().unwrap().index,
        Some(0)
    );
    (harness, runtime)
}

fn retained_eof(harness: &mut MpvLifecycleVerificationHarness) {
    for (name, data) in [
        ("eof-reached", json!(true)),
        ("core-idle", json!(true)),
        ("time-pos", json!(240.0)),
        ("pause", json!(true)),
    ] {
        harness
            .ingest_decoded_mpv_json(json!({"event":"property-change", "name":name, "data":data}));
    }
}

fn run_terminal(reason: &str, position: f64) -> (bool, Option<i64>) {
    let (mut harness, mut runtime) = loaded_playback(position);
    if reason == "retained-natural-eof" {
        // Sequence captured from isolated real mpv 0.41 with keep-open=yes/always:
        // EOF while the final audio buffer drains, core idle, final position, pause.
        harness.ingest_decoded_mpv_json(
            json!({"event":"property-change", "name":"eof-reached", "data":true}),
        );
        harness.ingest_decoded_mpv_json(
            json!({"event":"property-change", "name":"core-idle", "data":true}),
        );
        harness.ingest_decoded_mpv_json(
            json!({"event":"property-change", "name":"time-pos", "data":240.0}),
        );
        harness.ingest_decoded_mpv_json(
            json!({"event":"property-change", "name":"pause", "data":true}),
        );
        // Retained physical identity must not erase or repeat the completion.
        harness.apply_authoritative_snapshot(
            [LifecycleVerificationPlaylistEntry::new(
                77,
                Some("episode1.mkv".into()),
                true,
            )],
            Some("episode1.mkv".into()),
        );
    } else {
        let mut event = json!({"event":"end-file", "playlist_entry_id":77, "reason":reason});
        if reason == "error" {
            event["file_error"] = json!("I/O error");
        }
        harness.ingest_decoded_mpv_json(event);
    }
    apply_and_ack(&mut harness, &mut runtime, 2.0);
    let advanced = runtime
        .run_advance_playlist_after_natural_completion()
        .unwrap();
    let selected = runtime.session().current_room_playlist().unwrap().index;
    let advances = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter_map(|message| {
            let value = serde_json::to_value(message).unwrap();
            value
                .get("Set")
                .and_then(|set| set.get("playlistIndex"))
                .cloned()
        })
        .collect::<Vec<_>>();
    assert_eq!(advances.len(), usize::from(advanced));
    if advanced {
        assert_eq!(advances[0]["sorotteExpectedPlaylistIndex"], 0);
        assert_eq!(advances[0]["sorotteExpectedPlaylistEpoch"], 2);
        assert!(
            !runtime
                .run_advance_playlist_after_natural_completion()
                .unwrap()
        );
    }
    (advanced, selected)
}

#[test]
fn midstream_stop_must_not_advance_shared_playlist() {
    assert_eq!(run_terminal("stop", 10.0), (false, Some(0)));
}

#[test]
fn midstream_quit_must_not_advance_shared_playlist() {
    assert_eq!(run_terminal("quit", 10.0), (false, Some(0)));
}

#[test]
fn natural_eof_positive_control_advances_shared_playlist() {
    assert_eq!(run_terminal("eof", 240.0), (true, Some(1)));
}

#[test]
fn error_negative_control_preserves_shared_playlist() {
    assert_eq!(run_terminal("error", 10.0), (false, Some(0)));
}

#[test]
fn retained_natural_eof_must_advance_shared_playlist() {
    assert_eq!(
        run_terminal("retained-natural-eof", 239.65),
        (true, Some(1))
    );
}

#[test]
fn non_eof_end_reasons_preserve_selection_even_at_duration() {
    for reason in ["stop", "quit", "redirect", "", "unknown"] {
        assert_eq!(run_terminal(reason, 240.0), (false, Some(0)), "{reason}");
    }
}

#[test]
fn authoritative_disappearance_is_not_natural_completion() {
    let (mut harness, mut runtime) = loaded_playback(240.0);
    harness.apply_authoritative_snapshot([], None);
    apply_and_ack(&mut harness, &mut runtime, 2.0);
    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );
}

#[test]
fn retained_eof_requires_uncancelled_playing_evidence() {
    for contradiction in [
        "paused-seek",
        "seeking",
        "cache",
        "restart",
        "far-from-end",
        "eof-only",
    ] {
        let (mut harness, mut runtime) = loaded_playback(100.0);
        match contradiction {
            "paused-seek" => {
                harness.ingest_decoded_mpv_json(
                    json!({"event":"property-change", "name":"pause", "data":true}),
                );
                harness.ingest_decoded_mpv_json(json!({"event":"seek"}));
                harness.ingest_decoded_mpv_json(json!({"event":"playback-restart"}));
            }
            "seeking" => harness.ingest_decoded_mpv_json(json!({"event":"seek"})),
            "cache" => harness.ingest_decoded_mpv_json(
                json!({"event":"property-change", "name":"paused-for-cache", "data":true}),
            ),
            _ => {}
        }
        harness.ingest_decoded_mpv_json(
            json!({"event":"property-change", "name":"eof-reached", "data":true}),
        );
        if contradiction == "restart" {
            harness.ingest_decoded_mpv_json(json!({"event":"playback-restart"}));
        }
        if contradiction != "eof-only" {
            harness.ingest_decoded_mpv_json(
                json!({"event":"property-change", "name":"core-idle", "data":true}),
            );
            if contradiction != "far-from-end" {
                harness.ingest_decoded_mpv_json(
                    json!({"event":"property-change", "name":"time-pos", "data":240.0}),
                );
            }
            harness.ingest_decoded_mpv_json(
                json!({"event":"property-change", "name":"pause", "data":true}),
            );
        }
        apply_and_ack(&mut harness, &mut runtime, 2.0);
        assert!(
            !runtime
                .run_advance_playlist_after_natural_completion()
                .unwrap(),
            "{contradiction}"
        );
    }
}

#[test]
fn retained_completion_preserves_physical_load_and_accepts_a_later_replay() {
    let (mut harness, mut runtime) = loaded_playback(239.65);
    retained_eof(&mut harness);
    apply_and_ack(&mut harness, &mut runtime, 2.0);
    assert!(
        runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );
    assert_eq!(
        harness.projection().physical_file_loaded,
        SnapshotField::Known(true)
    );

    // A repeated property notification cannot request a second advancement
    // for the same completed selection.
    retained_eof(&mut harness);
    apply_and_ack(&mut harness, &mut runtime, 3.0);
    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );

    // Select the completed item again and seek its still-loaded physical file.
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistIndex":{"index":0,"user":"alice","sorottePlaylistEpoch":4}}}"#,
        )
        .unwrap();
    for event in [
        json!({"event":"seek"}),
        json!({"event":"property-change","name":"eof-reached","data":false}),
        json!({"event":"property-change","name":"time-pos","data":30.0}),
        json!({"event":"playback-restart"}),
        json!({"event":"property-change","name":"core-idle","data":false}),
        json!({"event":"property-change","name":"pause","data":false}),
        json!({"event":"property-change","name":"time-pos","data":239.65}),
    ] {
        harness.ingest_decoded_mpv_json(event);
    }
    apply_and_ack(&mut harness, &mut runtime, 4.0);
    retained_eof(&mut harness);
    apply_and_ack(&mut harness, &mut runtime, 5.0);
    assert!(
        runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );
}

#[test]
fn accepted_successor_prevents_retained_predecessor_completion() {
    let (mut harness, mut runtime) = loaded_playback(239.65);
    harness.accept_tracked_load("episode2.mkv", [77]);
    retained_eof(&mut harness);
    apply_and_ack(&mut harness, &mut runtime, 2.0);
    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );
}

#[test]
fn retained_completion_and_following_end_file_advance_only_once() {
    let (mut harness, mut runtime) = loaded_playback(239.65);
    retained_eof(&mut harness);
    apply_and_ack(&mut harness, &mut runtime, 2.0);
    assert!(
        runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );
    harness
        .ingest_decoded_mpv_json(json!({"event":"end-file","playlist_entry_id":77,"reason":"eof"}));
    apply_and_ack(&mut harness, &mut runtime, 3.0);
    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );
}

#[test]
fn seek_back_before_consuming_retained_completion_cancels_advancement() {
    let (mut harness, mut runtime) = loaded_playback(239.65);
    retained_eof(&mut harness);
    harness.ingest_decoded_mpv_json(json!({"event":"seek"}));
    harness
        .ingest_decoded_mpv_json(json!({"event":"property-change","name":"time-pos","data":30.0}));
    apply_and_ack(&mut harness, &mut runtime, 2.0);
    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );
}

#[test]
fn retained_eof_corroboration_is_independent_of_property_notification_order() {
    let properties = [
        ("eof-reached", json!(true)),
        ("core-idle", json!(true)),
        ("time-pos", json!(240.0)),
        ("pause", json!(true)),
    ];
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let order = [a, b, c, d];
                    if order
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != 4
                    {
                        continue;
                    }
                    let (mut harness, mut runtime) = loaded_playback(239.65);
                    for index in order {
                        let (name, data) = &properties[index];
                        harness.ingest_decoded_mpv_json(
                            json!({"event":"property-change","name":name,"data":data}),
                        );
                    }
                    apply_and_ack(&mut harness, &mut runtime, 2.0);
                    assert!(
                        runtime
                            .run_advance_playlist_after_natural_completion()
                            .unwrap(),
                        "notification order {order:?}"
                    );
                    assert!(
                        !runtime
                            .run_advance_playlist_after_natural_completion()
                            .unwrap()
                    );
                }
            }
        }
    }
}
