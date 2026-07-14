use super::*;
use crate::MAX_PENDING_LOCAL_PLAYLIST_ECHOES;

#[test]
fn queued_runtime_control_set_playlist_and_index_emit_protocol_messages() {
    let mut control = QueuedRuntimeControl::default();
    control
        .emit(ClientEffect::SetPlaylist(vec![
            "episode1.mkv".to_owned(),
            "episode2.mkv".to_owned(),
        ]))
        .expect("playlist effect should be supported");
    control
        .emit(ClientEffect::SetPlaylistIndex(1))
        .expect("playlist index effect should be supported");

    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("expected queued control playlist change to emit Set message");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("Set message should contain playlistChange payload");
    assert_eq!(playlist_change.files, vec!["episode1.mkv", "episode2.mkv"]);
    assert!(playlist_change.user.is_none());

    let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
        panic!("expected queued control playlist index to emit Set message");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("Set message should contain playlistIndex payload");
    assert_eq!(playlist_index.index, 1);
    assert!(playlist_index.user.is_none());
}

#[test]
fn plex_playlist_sidecar_outbound_keeps_syncplay_files_baseline() {
    let plex_uri =
        "plex://server/metadata/14452?title=Episode%2011&file=Episode%2011%20%5B1080p%5D.mkv";
    let mut control = QueuedRuntimeControl::default();
    control
        .emit(ClientEffect::SetPlaylist(vec![plex_uri.to_owned()]))
        .expect("playlist effect should be supported");

    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("expected queued control playlist change to emit Set message");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("Set message should contain playlistChange payload");

    assert_eq!(
        playlist_change.files,
        vec!["Episode 11 [1080p].mkv".to_owned()]
    );
    assert_eq!(
        playlist_change.extra.get("sorottePlexPlaylistUris"),
        Some(&json!([plex_uri]))
    );
}

#[test]
fn plex_playlist_sidecar_inbound_restores_canonical_playlist_uri() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    let plex_uri =
        "plex://server/metadata/14452?title=Episode%2011&file=Episode%2011%20%5B1080p%5D.mkv";
    session
        .apply_message_json(&format!(
            r#"{{"Set":{{"playlistChange":{{"files":["Episode 11 [1080p].mkv"],"user":"alice","sorottePlexPlaylistUris":["{plex_uri}"]}}}}}}"#
        ))
        .expect("playlist sidecar should apply");

    let playlist = session
        .current_room_playlist()
        .expect("playlist should be available");
    assert_eq!(playlist.files, vec![plex_uri.to_owned()]);
}

#[test]
fn playlist_reorder_index_echo_does_not_queue_reset_when_active_target_is_unchanged() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episodeA.mkv","episodeB.mkv","episodeC.mkv","episodeD.mkv"],"user":"bob"}}}"#,
            )
            .expect("initial playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#)
        .expect("initial playlist index should apply");
    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        None,
        "the first playlist index should not queue a reset"
    );

    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episodeD.mkv","episodeA.mkv","episodeB.mkv","episodeC.mkv"],"user":"bob"}}}"#,
            )
            .expect("reordered playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":2,"user":"bob"}}}"#)
        .expect("reordered playlist index should apply");
    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        None,
        "playlist reorders that preserve the active target should not queue a rewind/reset intent"
    );
}

#[test]
fn client_runtime_queue_playlist_item_preserves_existing_selection_without_select_after_queue() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_queue_playlist_item("episode3.mkv", false)
            .expect("queue playlist item should not fail"),
        "queueing a playlist item should emit protocol messages"
    );

    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("local queue should update the current room playlist immediately");
    assert_eq!(
        playlist.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert_eq!(
        playlist.index,
        Some(0),
        "plain queue should preserve the existing room playlist selection"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(playlist_change_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set protocol message");
    };
    let playlist_change = playlist_change_message
        .set
        .playlist_change
        .as_ref()
        .expect("playlist change payload should be present");
    assert_eq!(
        playlist_change.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );

    let ProtocolMessage::Set(playlist_index_message) = &control.outbound_messages()[1] else {
        panic!("expected queued Set playlist index protocol message");
    };
    let playlist_index = playlist_index_message
        .set
        .playlist_index
        .as_ref()
        .expect("playlist index payload should be present");
    assert_eq!(playlist_index.index, 0);
    assert!(playlist_index.user.is_none());
}

#[test]
fn client_runtime_replace_playlist_preserves_existing_selection_without_redundant_index_update() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_replace_playlist(
                vec![
                    "episode1.mkv".to_owned(),
                    "episode2.mkv".to_owned(),
                    "episode3.mkv".to_owned(),
                ],
                Some(0),
            )
            .expect("replace playlist should not fail"),
        "playlist replace should emit a protocol message when the playlist contents change"
    );
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        None,
        "preserving the existing selection during playlist replace should not queue a reset"
    );

    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("local playlist replace should update the current room playlist immediately");
    assert_eq!(
        playlist.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert_eq!(
        playlist.index,
        Some(0),
        "playlist replace should preserve the existing room playlist selection"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(
        control.outbound_messages().len(),
        1,
        "playlist replace should omit a redundant playlistIndex update when the selected index is unchanged"
    );
    let ProtocolMessage::Set(playlist_change_message) = &control.outbound_messages()[0] else {
        panic!("expected queued Set protocol message");
    };
    let playlist_change = playlist_change_message
        .set
        .playlist_change
        .as_ref()
        .expect("playlist change payload should be present");
    assert_eq!(
        playlist_change.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert!(
        playlist_change_message.set.playlist_index.is_none(),
        "playlist replace should not send playlistIndex when the selected row is already current"
    );
}

#[test]
fn client_runtime_queue_playlist_item_dispatches_playlist_change_and_preserves_index() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_queue_playlist_item("episode3.mkv", false)
            .expect("queue command should not fail"),
        "queue command should emit playlist change/index updates"
    );
    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("queue command should update the current room playlist immediately");
    assert_eq!(
        playlist.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert_eq!(playlist.index, Some(0));

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("first outbound queue message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("first outbound message should include playlistChange");
    assert_eq!(
        playlist_change.files,
        vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
    );
    assert!(playlist_change.user.is_none());

    let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
        panic!("second outbound queue message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second outbound message should include playlistIndex");
    assert_eq!(playlist_index.index, 0);
    assert!(playlist_index.user.is_none());
}

#[test]
fn local_playlist_revision_advances_once_across_matching_server_echo() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"bob"}}}"#)
        .expect("initial remote playlist should apply");
    let initial_revision = session
        .current_room_playlist()
        .expect("initial playlist should be projected")
        .revision;
    let initial_remote_revision = session.current_room_playlist_remote_revision();

    let mut runtime = ClientRuntime::new(
        session,
        RecordingPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    assert!(
        runtime
            .run_queue_playlist_item("episode2.mkv", false)
            .expect("local queue should not fail")
    );
    let optimistic_revision = runtime
        .session()
        .current_room_playlist()
        .expect("local queue should project immediately")
        .revision;
    assert_eq!(optimistic_revision, initial_revision.wrapping_add(1));

    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
        )
        .expect("matching local server echo should apply");
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .expect("echoed playlist should remain projected")
            .revision,
        optimistic_revision,
        "a matching self-echo acknowledges the optimistic mutation without creating a new revision"
    );
    assert_eq!(
        runtime.session().current_room_playlist_remote_revision(),
        initial_remote_revision,
        "a matching self-echo must not advance the remote playlist generation"
    );

    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob"}}}"#,
        )
        .expect("same-content remote replacement should apply");
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .expect("remote playlist should remain projected")
            .revision,
        optimistic_revision.wrapping_add(1),
        "a distinguishable same-content remote replacement remains a new revision"
    );
    assert_eq!(
        runtime.session().current_room_playlist_remote_revision(),
        initial_remote_revision.wrapping_add(1),
        "a distinguishable remote replacement advances the remote playlist generation"
    );

    assert!(
        runtime
            .run_queue_playlist_item("episode3.mkv", false)
            .expect("second local queue should not fail")
    );
    let second_optimistic_revision = runtime
        .session()
        .current_room_playlist()
        .expect("second local queue should project immediately")
        .revision;
    let remote_revision_before_omitted_user_echo =
        runtime.session().current_room_playlist_remote_revision();

    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"]}}}"#,
        )
        .expect("matching omitted-user server echo should apply");
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .expect("omitted-user echo should remain projected")
            .revision,
        second_optimistic_revision,
        "a matching omitted-user echo acknowledges the optimistic mutation"
    );
    assert_eq!(
        runtime.session().current_room_playlist_remote_revision(),
        remote_revision_before_omitted_user_echo,
        "a matching omitted-user echo must not advance the remote playlist generation"
    );
}

#[test]
fn older_explicit_self_echoes_preserve_newer_optimistic_playlist_and_index_state() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["A"],"user":"bob"}}}"#)
        .expect("initial playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#)
        .expect("initial index should apply");

    let initial_remote_revision = session.current_room_playlist_remote_revision();
    let mut runtime = ClientRuntime::new(
        session,
        RecordingPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    assert!(
        runtime
            .run_queue_playlist_item("B", true)
            .expect("first local mutation should apply")
    );
    assert!(
        runtime
            .run_queue_playlist_item("C", true)
            .expect("second local mutation should apply")
    );

    let latest_optimistic_playlist = runtime
        .session()
        .current_room_playlist()
        .expect("latest optimistic playlist should exist")
        .clone();
    let latest_undo_snapshot = runtime
        .session()
        .model
        .playlist
        .undo_snapshots
        .get("room1")
        .cloned();
    let active_target_state = runtime
        .session()
        .model
        .playlist
        .active_targets_before_index_update
        .clone();
    assert_eq!(latest_optimistic_playlist.files, vec!["A", "B", "C"]);
    assert_eq!(latest_optimistic_playlist.index, Some(2));
    assert_eq!(latest_optimistic_playlist.set_by.as_deref(), Some("alice"));
    assert_eq!(
        runtime.session().model.playlist.pending_local_change_echoes["room1"]
            .pending
            .len(),
        2
    );
    assert_eq!(
        runtime.session().model.playlist.pending_local_index_echoes["room1"]
            .pending
            .len(),
        2
    );

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["A","B"],"user":"alice"}}}"#)
        .expect("older explicit playlist echo should apply as an acknowledgement");
    assert_eq!(
        runtime.session().current_room_playlist(),
        Some(&latest_optimistic_playlist),
        "an older playlist echo must not roll back files, index, setter, or revision"
    );
    assert_eq!(
        runtime.session().model.playlist.undo_snapshots.get("room1"),
        latest_undo_snapshot.as_ref(),
        "an older playlist echo must not overwrite undo state"
    );
    assert_eq!(
        runtime
            .session()
            .model
            .playlist
            .active_targets_before_index_update,
        active_target_state,
        "an older playlist echo must not alter active-target bookkeeping"
    );
    assert_eq!(
        runtime.session().current_room_playlist_remote_revision(),
        initial_remote_revision
    );
    assert_eq!(
        runtime.session().model.playlist.pending_local_change_echoes["room1"]
            .pending
            .len(),
        1
    );

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("older explicit index echo should apply as an acknowledgement");
    assert_eq!(
        runtime.session().current_room_playlist(),
        Some(&latest_optimistic_playlist),
        "an older index echo must not roll back the latest optimistic index"
    );
    assert_eq!(
        runtime.session().model.playlist.pending_local_index_echoes["room1"]
            .pending
            .len(),
        1
    );

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["A","B","C"],"user":"alice"}}}"#)
        .expect("latest explicit playlist echo should acknowledge the final mutation");
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":2,"user":"alice"}}}"#)
        .expect("latest explicit index echo should acknowledge the final selection");
    assert_eq!(
        runtime.session().current_room_playlist(),
        Some(&latest_optimistic_playlist)
    );
    assert_eq!(
        runtime.session().current_room_playlist_remote_revision(),
        initial_remote_revision
    );
    assert!(
        !runtime
            .session()
            .model
            .playlist
            .pending_local_change_echoes
            .contains_key("room1"),
        "the final acknowledgement should release playlist echo tracking"
    );
    assert!(
        !runtime
            .session()
            .model
            .playlist
            .pending_local_index_echoes
            .contains_key("room1"),
        "the final acknowledgement should release index echo tracking"
    );
}

#[test]
fn older_omitted_user_echoes_preserve_newer_optimistic_setter_and_index() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["A"],"user":"bob"}}}"#)
        .expect("initial playlist should apply");
    let mut runtime = ClientRuntime::new(
        session,
        RecordingPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    assert!(
        runtime
            .run_queue_playlist_item("B", true)
            .expect("first local mutation should apply")
    );
    assert!(
        runtime
            .run_queue_playlist_item("C", true)
            .expect("second local mutation should apply")
    );
    let optimistic_revision = runtime
        .session()
        .current_room_playlist()
        .expect("optimistic playlist should exist")
        .revision;
    let remote_revision = runtime.session().current_room_playlist_remote_revision();

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["A","B"]}}}"#)
        .expect("older omitted-user playlist echo should be acknowledged");
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("older omitted-user index echo should be acknowledged");
    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("newer optimistic playlist should remain projected");
    assert_eq!(playlist.files, vec!["A", "B", "C"]);
    assert_eq!(playlist.index, Some(2));
    assert_eq!(playlist.set_by.as_deref(), Some("alice"));
    assert_eq!(playlist.revision, optimistic_revision);
    assert_eq!(
        runtime.session().current_room_playlist_remote_revision(),
        remote_revision
    );

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["A","B","C"]}}}"#)
        .expect("latest omitted-user playlist echo should acknowledge the final mutation");
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":2}}}"#)
        .expect("latest omitted-user index echo should acknowledge the final selection");
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .expect("final playlist should remain projected")
            .revision,
        optimistic_revision
    );
    assert_eq!(
        runtime.session().current_room_playlist_remote_revision(),
        remote_revision
    );
    assert!(
        runtime
            .session()
            .model
            .playlist
            .pending_local_change_echoes
            .is_empty()
    );
    assert!(
        runtime
            .session()
            .model
            .playlist
            .pending_local_index_echoes
            .is_empty()
    );
}

#[test]
fn local_playlist_echo_tracking_is_bounded_and_recovers_after_authoritative_overflow() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");

    let mut latest_files = Vec::new();
    for mutation in 0..=MAX_PENDING_LOCAL_PLAYLIST_ECHOES {
        latest_files = vec![format!("mutation-{mutation}.mkv")];
        session.apply_local_playlist_runtime_actions_legacy_compatible(&[
            ClientRuntimeAction::SetPlaylist {
                files: latest_files.clone(),
            },
        ]);
    }

    let optimistic_revision = session
        .current_room_playlist()
        .expect("overflowing optimistic playlist should remain projected")
        .revision;
    let remote_revision = session.current_room_playlist_remote_revision();
    let tracker = &session.model.playlist.pending_local_change_echoes["room1"];
    assert!(tracker.invalidated);
    assert!(tracker.pending.is_empty());
    let model_debug = format!("{:?}", session.model.playlist);
    assert!(model_debug.contains("invalidated_local_change_echo_room_count: 1"));
    assert!(
        !model_debug.contains("mutation-"),
        "playlist echo fingerprints and host-provided targets must remain absent from Debug"
    );

    let echoed_files_json = serde_json::to_string(&latest_files).expect("files should serialize");
    session
        .apply_message_json(&format!(
            r#"{{"Set":{{"playlistChange":{{"files":{echoed_files_json},"user":"alice"}}}}}}"#
        ))
        .expect("the first update after overflow should apply authoritatively");
    let playlist = session
        .current_room_playlist()
        .expect("authoritative overflow update should remain projected");
    assert_eq!(playlist.files, latest_files);
    assert_eq!(playlist.revision, optimistic_revision.wrapping_add(1));
    assert_eq!(
        session.current_room_playlist_remote_revision(),
        remote_revision.wrapping_add(1)
    );
    assert!(
        session
            .model
            .playlist
            .pending_local_change_echoes
            .is_empty()
    );

    let recovered_files = vec!["recovered.mkv".to_owned()];
    session.apply_local_playlist_runtime_actions_legacy_compatible(&[
        ClientRuntimeAction::SetPlaylist {
            files: recovered_files.clone(),
        },
    ]);
    let recovered_revision = session
        .current_room_playlist()
        .expect("new optimistic mutation should be tracked after overflow recovery")
        .revision;
    let recovered_remote_revision = session.current_room_playlist_remote_revision();
    assert_eq!(
        session.model.playlist.pending_local_change_echoes["room1"]
            .pending
            .len(),
        1
    );
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["recovered.mkv"],"user":"alice"}}}"#,
        )
        .expect("post-overflow matching echo should acknowledge normally");
    assert_eq!(
        session
            .current_room_playlist()
            .expect("recovered playlist should remain projected")
            .revision,
        recovered_revision
    );
    assert_eq!(
        session.current_room_playlist_remote_revision(),
        recovered_remote_revision
    );
    assert!(
        session
            .model
            .playlist
            .pending_local_change_echoes
            .is_empty()
    );
}

#[test]
fn local_playlist_index_echo_tracking_is_bounded_and_recovers_after_overflow() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session.apply_local_playlist_runtime_actions_legacy_compatible(&[
        ClientRuntimeAction::SetPlaylist {
            files: vec!["first.mkv".to_owned(), "second.mkv".to_owned()],
        },
    ]);
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["first.mkv","second.mkv"],"user":"alice"}}}"#,
        )
        .expect("initial playlist echo should clear file tracking");

    for mutation in 0..=MAX_PENDING_LOCAL_PLAYLIST_ECHOES {
        session.apply_local_playlist_runtime_actions_legacy_compatible(&[
            ClientRuntimeAction::SetPlaylistIndex {
                index: i64::try_from(mutation % 2).expect("bounded test index should fit"),
            },
        ]);
    }
    let playlist_before_echo = session
        .current_room_playlist()
        .expect("optimistic index should be projected")
        .clone();
    let remote_revision = session.current_room_playlist_remote_revision();
    let tracker = &session.model.playlist.pending_local_index_echoes["room1"];
    assert!(tracker.invalidated);
    assert!(tracker.pending.is_empty());
    let model_debug = format!("{:?}", session.model.playlist);
    assert!(model_debug.contains("pending_local_index_echo_count: 0"));
    assert!(model_debug.contains("invalidated_local_index_echo_room_count: 1"));
    assert!(!model_debug.contains("files_digest"));

    session
        .apply_message_json(&format!(
            r#"{{"Set":{{"playlistIndex":{{"index":{},"user":"alice"}}}}}}"#,
            playlist_before_echo
                .index
                .expect("latest optimistic index should be set")
        ))
        .expect("first index echo after overflow should apply authoritatively");
    assert_eq!(
        session.current_room_playlist(),
        Some(&playlist_before_echo),
        "an authoritative index echo does not create a playlist content revision"
    );
    assert_eq!(
        session.current_room_playlist_remote_revision(),
        remote_revision
    );
    assert!(session.model.playlist.pending_local_index_echoes.is_empty());

    let recovered_index = if playlist_before_echo.index == Some(0) {
        1
    } else {
        0
    };
    session.apply_local_playlist_runtime_actions_legacy_compatible(&[
        ClientRuntimeAction::SetPlaylistIndex {
            index: recovered_index,
        },
    ]);
    assert_eq!(
        session.model.playlist.pending_local_index_echoes["room1"]
            .pending
            .len(),
        1
    );
    session
        .apply_message_json(&format!(
            r#"{{"Set":{{"playlistIndex":{{"index":{recovered_index},"user":"alice"}}}}}}"#
        ))
        .expect("post-overflow matching index echo should acknowledge normally");
    assert_eq!(
        session
            .current_room_playlist()
            .expect("recovered index should remain projected")
            .index,
        Some(recovered_index)
    );
    assert!(session.model.playlist.pending_local_index_echoes.is_empty());
}

#[test]
fn local_playlist_echo_trackers_clear_across_room_and_reconnect_boundaries() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session.apply_local_playlist_runtime_actions_legacy_compatible(&[
        ClientRuntimeAction::SetPlaylist {
            files: vec!["room1.mkv".to_owned()],
        },
        ClientRuntimeAction::SetPlaylistIndex { index: 0 },
    ]);
    assert!(
        !session
            .model
            .playlist
            .pending_local_change_echoes
            .is_empty()
    );
    assert!(!session.model.playlist.pending_local_index_echoes.is_empty());

    session.update_local_room("room2".to_owned());
    assert!(
        session
            .model
            .playlist
            .pending_local_change_echoes
            .is_empty()
    );
    assert!(session.model.playlist.pending_local_index_echoes.is_empty());

    session.apply_local_playlist_runtime_actions_legacy_compatible(&[
        ClientRuntimeAction::SetPlaylist {
            files: vec!["room2.mkv".to_owned()],
        },
        ClientRuntimeAction::SetPlaylistIndex { index: 0 },
    ]);
    assert!(
        !session
            .model
            .playlist
            .pending_local_change_echoes
            .is_empty()
    );
    assert!(!session.model.playlist.pending_local_index_echoes.is_empty());
    session.reset_sync_state_for_reconnect();
    assert!(
        session
            .model
            .playlist
            .pending_local_change_echoes
            .is_empty()
    );
    assert!(session.model.playlist.pending_local_index_echoes.is_empty());
}

#[test]
fn remote_playlist_change_invalidates_an_unacknowledged_local_echo() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["initial.mkv"],"user":"bob"}}}"#)
        .expect("initial playlist should apply");
    let mut runtime = ClientRuntime::new(
        session,
        RecordingPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    assert!(
        runtime
            .run_queue_playlist_item("local.mkv", false)
            .expect("local queue should apply optimistically")
    );
    assert!(
        runtime
            .session()
            .model
            .playlist
            .pending_local_change_echoes
            .contains_key("room1")
    );
    assert!(
        runtime
            .session()
            .model
            .playlist
            .pending_local_index_echoes
            .contains_key("room1")
    );

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["remote.mkv"]}}}"#)
        .expect("intervening omitted-user remote replacement should apply");
    let remote_revision = runtime.session().current_room_playlist_remote_revision();
    let total_revision = runtime
        .session()
        .current_room_playlist()
        .expect("remote playlist should be projected")
        .revision;
    assert!(
        !runtime
            .session()
            .model
            .playlist
            .pending_local_change_echoes
            .contains_key("room1")
    );
    assert!(
        !runtime
            .session()
            .model
            .playlist
            .pending_local_index_echoes
            .contains_key("room1"),
        "an authoritative playlist replacement should invalidate stale index echoes too"
    );

    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["initial.mkv","local.mkv"],"user":"alice"}}}"#,
        )
        .expect("late stale self-authored update should apply authoritatively");
    assert_eq!(
        runtime.session().current_room_playlist_remote_revision(),
        remote_revision.wrapping_add(1),
        "an intervening remote replacement must invalidate the queued self-echo acknowledgement"
    );
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .expect("late update should remain projected")
            .revision,
        total_revision.wrapping_add(1)
    );
}

#[test]
fn client_runtime_queue_playlist_item_preserves_whitespace_only_file_name() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_queue_playlist_item(" ", false)
            .expect("queue command should not fail"),
        "queue command should preserve whitespace-only file names"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("first outbound queue message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("first outbound message should include playlistChange");
    assert_eq!(
        playlist_change.files,
        vec!["episode1.mkv", "episode2.mkv", " "]
    );
    assert!(playlist_change.user.is_none());
}

#[test]
fn client_runtime_queue_and_select_playlist_item_sets_new_item_index() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_queue_playlist_item("episode3.mkv", true)
            .expect("queue-and-select command should not fail"),
        "queue-and-select command should emit playlist change/index updates"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
        panic!("second outbound queue-and-select message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second outbound message should include playlistIndex");
    assert_eq!(playlist_index.index, 2);
    assert!(playlist_index.user.is_none());
}

#[test]
fn client_runtime_queue_playlist_item_is_omitted_before_server_hello() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_queue_playlist_item("episode1.mkv", false)
            .expect("queue command should not fail"),
        "queue command should be suppressed before server hello"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_queue_playlist_item_omits_duplicate_entries() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_queue_playlist_item("episode2.mkv", false)
            .expect("duplicate queue command should not fail"),
        "duplicate queue requests should be suppressed"
    );
    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("duplicate queue should preserve the current room playlist");
    assert_eq!(playlist.files, vec!["episode1.mkv", "episode2.mkv"]);
    assert_eq!(playlist.index, Some(0));
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_delete_playlist_index_dispatches_playlist_change_and_index() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":2,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_delete_playlist_index(1)
            .expect("delete command should not fail"),
        "delete command should emit playlist change/index updates"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 2);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("first outbound delete message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("first outbound message should include playlistChange");
    assert_eq!(playlist_change.files, vec!["episode1.mkv", "episode3.mkv"]);
    assert!(playlist_change.user.is_none());

    let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
        panic!("second outbound delete message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second outbound message should include playlistIndex");
    assert_eq!(playlist_index.index, 1);
    assert!(playlist_index.user.is_none());
}

#[test]
fn client_runtime_delete_playlist_index_last_item_emits_only_playlist_change() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_delete_playlist_index(0)
            .expect("delete command should not fail"),
        "delete command should emit playlist change for last item removal"
    );

    let (_, _, control) = runtime.into_parts();
    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
        panic!("outbound delete message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("outbound message should include playlistChange");
    assert!(playlist_change.files.is_empty());
    assert!(playlist_change.user.is_none());
}

#[test]
fn client_runtime_delete_playlist_index_is_omitted_for_invalid_index() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_delete_playlist_index(3)
            .expect("delete command should not fail"),
        "delete command should be suppressed for invalid index"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}
