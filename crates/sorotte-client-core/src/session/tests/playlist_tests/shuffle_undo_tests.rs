use super::*;

fn require_bounded_test_completion(action: impl FnOnce() + Send + 'static) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action));
        let _ = sender.send(result);
    });
    let result = receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("playlist operation must complete instead of looping");
    worker
        .join()
        .expect("bounded playlist test worker should be joinable");
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn client_runtime_shuffle_remaining_playlist_preserves_prefix_and_index() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv","episode4.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let mut sent = false;
    for _ in 0..4 {
        if runtime
            .run_shuffle_remaining_playlist()
            .expect("shuffle remaining should not fail")
        {
            sent = true;
            break;
        }
    }
    assert!(
        sent,
        "shuffle remaining should eventually emit playlist change/index updates"
    );
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        None,
        "shuffle-remaining must not reload an unchanged selected row"
    );

    let (_, _, control) = runtime.into_parts();
    let outbound_messages = control.outbound_messages();
    assert_eq!(outbound_messages.len(), 2);

    let ProtocolMessage::Set(change_message) = &outbound_messages[0] else {
        panic!("first outbound shuffle-remaining message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("first outbound message should include playlistChange");
    assert_eq!(
        &playlist_change.files[..2],
        &["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
    );
    let mut expected_tail = vec!["episode3.mkv".to_owned(), "episode4.mkv".to_owned()];
    let mut actual_tail = playlist_change.files[2..].to_vec();
    expected_tail.sort();
    actual_tail.sort();
    assert_eq!(actual_tail, expected_tail);

    let ProtocolMessage::Set(index_message) = &outbound_messages[1] else {
        panic!("second outbound shuffle-remaining message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second outbound message should include playlistIndex");
    assert_eq!(playlist_index.index, 1);
}

#[test]
fn client_runtime_shuffle_entire_playlist_resets_index_to_zero() {
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
            .run_shuffle_entire_playlist()
            .expect("shuffle entire should not fail"),
        "shuffle entire should emit at least a playlist index reset"
    );
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        Some(true),
        "shuffle-entire must finalize its newly selected media before the echo"
    );
    let shuffled_files = runtime
        .session()
        .current_room_playlist()
        .expect("shuffle should project its playlist")
        .files
        .clone();
    let shuffled_files_json =
        serde_json::to_string(&shuffled_files).expect("test playlist should serialize");
    runtime
        .session_mut_for_test()
        .apply_message_json(&format!(
            r#"{{"Set":{{"playlistChange":{{"files":{shuffled_files_json},"user":"alice"}}}}}}"#
        ))
        .expect("shuffle playlist echo should apply");
    runtime
        .session_mut_for_test()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("shuffle index echo should apply");
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        None,
        "matching shuffle echoes must not create a second media reset"
    );

    let (_, _, control) = runtime.into_parts();
    let outbound_messages = control.outbound_messages();
    assert!(
        !outbound_messages.is_empty(),
        "shuffle entire should emit protocol messages"
    );

    let playlist_index = outbound_messages
        .iter()
        .find_map(|message| match message {
            ProtocolMessage::Set(set) => set.set.playlist_index.as_ref(),
            _ => None,
        })
        .expect("shuffle entire should emit Set.playlistIndex");
    assert_eq!(playlist_index.index, 0);
}

#[test]
fn client_runtime_undo_playlist_change_toggles_between_previous_and_current_playlist() {
    require_bounded_test_completion(|| {
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
            .expect("initial playlist should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("initial playlist index should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode4.mkv","episode5.mkv"],"user":"alice"}}}"#,
            )
            .expect("updated playlist should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("updated playlist index should apply");
        assert_eq!(
            session.take_pending_playlist_index_reset_intent(),
            Some(false),
            "the remote playlist update should queue its own reset before the local undo scenario"
        );

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_undo_playlist_change()
                .expect("undo playlist should not fail"),
            "undo playlist should emit restore actions when a previous playlist exists"
        );
        assert_eq!(
            runtime
                .session_mut_for_test()
                .take_pending_playlist_index_reset_intent(),
            Some(true),
            "undo must finalize the restored media target before its echo"
        );

        {
            let outbound_messages = runtime.control().outbound_messages();
            assert_eq!(outbound_messages.len(), 3);
            let ProtocolMessage::Set(change_message) = &outbound_messages[0] else {
                panic!("first outbound undo message should be Set.playlistChange");
            };
            let playlist_change = change_message
                .set
                .playlist_change
                .as_ref()
                .expect("first outbound undo message should include playlistChange");
            assert_eq!(
                playlist_change.files,
                vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
            );

            let ProtocolMessage::Set(index_message) = &outbound_messages[1] else {
                panic!("second outbound undo message should be Set.playlistIndex");
            };
            let playlist_index = index_message
                .set
                .playlist_index
                .as_ref()
                .expect("second outbound undo message should include playlistIndex");
            assert_eq!(playlist_index.index, 0);
        }

        runtime
            .session_mut_for_test()
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("restored playlist echo should apply");
        runtime
            .session_mut_for_test()
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("restored playlist index echo should apply");
        assert_eq!(
            runtime
                .session_mut_for_test()
                .take_pending_playlist_index_reset_intent(),
            None,
            "matching undo echoes must not create a second media reset"
        );
        assert!(
            runtime
                .run_undo_playlist_change()
                .expect("second undo playlist should not fail"),
            "second undo should toggle back to the most recent playlist snapshot"
        );
        assert_eq!(
            runtime
                .session_mut_for_test()
                .take_pending_playlist_index_reset_intent(),
            Some(true),
            "toggling back to a different media target must also be finalized"
        );
        let (_, _, control) = runtime.into_parts();
        let outbound_messages = control.outbound_messages();
        assert_eq!(
            outbound_messages.len(),
            6,
            "each undo batch must retain its own trailing State behind the playlist commands it \
         describes"
        );
        assert!(
            matches!(outbound_messages[0], ProtocolMessage::Set(_))
                && matches!(outbound_messages[1], ProtocolMessage::Set(_))
                && matches!(outbound_messages[2], ProtocolMessage::State(_))
                && matches!(outbound_messages[3], ProtocolMessage::Set(_))
                && matches!(outbound_messages[4], ProtocolMessage::Set(_))
                && matches!(outbound_messages[5], ProtocolMessage::State(_)),
            "a later undo batch must not move its playlist commands ahead of the preceding batch's \
         State"
        );
        let playlist_change = outbound_messages
            .iter()
            .filter_map(|message| match message {
                ProtocolMessage::Set(set) => set.set.playlist_change.as_ref(),
                _ => None,
            })
            .find(|playlist_change| playlist_change.files == ["episode4.mkv", "episode5.mkv"])
            .expect("second undo should include its playlistChange message");
        assert_eq!(playlist_change.files, vec!["episode4.mkv", "episode5.mkv"]);
        let second_playlist_index = outbound_messages
            .iter()
            .skip(3)
            .find_map(|message| match message {
                ProtocolMessage::Set(set) => set.set.playlist_index.as_ref(),
                _ => None,
            })
            .expect("second undo should include its playlistIndex message");
        assert_eq!(second_playlist_index.index, 0);
    });
}

#[test]
fn client_runtime_undo_playlist_change_toggles_initial_empty_snapshot_without_waiting_for_echo() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"different-local-file.mkv"}}}}}"#,
        )
        .expect("local file announcement should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_undo_playlist_change()
            .expect("undo playlist should not fail"),
        "first undo should restore the initial empty snapshot"
    );
    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(change_message) = &runtime.control().outbound_messages()[0] else {
        panic!("undo playlist message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("undo playlist message should include playlistChange");
    assert!(playlist_change.files.is_empty());
    let playlist = runtime
        .session()
        .current_room_playlist()
        .expect("undo should update the current room playlist immediately");
    assert!(playlist.files.is_empty());
    assert_eq!(playlist.index, None);
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        None,
        "removing the last target must not leave an unconsumable media reset, even when the attached file fallback differs"
    );

    assert!(
        runtime
            .run_undo_playlist_change()
            .expect("second undo playlist should not fail"),
        "second undo should toggle back to the restored playlist without waiting for an echo"
    );
    assert_eq!(
        runtime
            .session_mut_for_test()
            .take_pending_playlist_index_reset_intent(),
        Some(true),
        "restoring a target from an empty playlist must finalize that media"
    );
    assert_eq!(runtime.control().outbound_messages().len(), 4);
    let ProtocolMessage::Set(change_message) = &runtime.control().outbound_messages()[1] else {
        panic!("second undo change message should be Set.playlistChange");
    };
    let playlist_change = change_message
        .set
        .playlist_change
        .as_ref()
        .expect("second undo change message should include playlistChange");
    assert_eq!(playlist_change.files, vec!["episode1.mkv"]);

    let ProtocolMessage::Set(index_message) = &runtime.control().outbound_messages()[2] else {
        panic!("second undo index message should be Set.playlistIndex");
    };
    let playlist_index = index_message
        .set
        .playlist_index
        .as_ref()
        .expect("second undo index message should include playlistIndex");
    assert_eq!(playlist_index.index, 0);
}

#[test]
fn playlist_undo_snapshot_capture_obeys_change_deduplication_and_room_isolation() {
    let mut session = ClientSession::default();
    let original = vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()];
    session
        .model
        .playlist
        .undo_snapshots
        .insert("other-room".to_owned(), vec!["other-room.mkv".to_owned()]);

    session.capture_playlist_undo_snapshot_legacy_compatible("room1", &original, &original);
    assert_eq!(session.model.playlist.undo_snapshots.get("room1"), None);

    session.capture_playlist_undo_snapshot_legacy_compatible(
        "room1",
        &original,
        &["episode2.mkv".to_owned(), "episode1.mkv".to_owned()],
    );
    assert_eq!(
        session.model.playlist.undo_snapshots.get("room1"),
        Some(&original)
    );

    session.capture_playlist_undo_snapshot_legacy_compatible(
        "room1",
        &original,
        &["replacement.mkv".to_owned()],
    );
    assert_eq!(
        session.model.playlist.undo_snapshots.get("room1"),
        Some(&original),
        "capturing the same current playlist twice must not replace its undo snapshot"
    );

    let replacement = vec!["replacement.mkv".to_owned()];
    session.capture_playlist_undo_snapshot_legacy_compatible(
        "room1",
        &replacement,
        &["next.mkv".to_owned()],
    );
    assert_eq!(
        session.model.playlist.undo_snapshots.get("room1"),
        Some(&replacement)
    );
    assert_eq!(
        session.model.playlist.undo_snapshots.get("other-room"),
        Some(&vec!["other-room.mkv".to_owned()]),
        "capturing one room must not disturb another room's undo snapshot"
    );
}

#[test]
fn playlist_target_index_selection_covers_forward_backward_and_boundary_rules() {
    struct Case {
        label: &'static str,
        current_files: &'static [&'static str],
        current_index: Option<usize>,
        new_files: &'static [&'static str],
        expected: usize,
    }

    let cases = [
        Case {
            label: "missing current index",
            current_files: &["alpha"],
            current_index: None,
            new_files: &["prefix", "alpha"],
            expected: 0,
        },
        Case {
            label: "forward match keeps its exact new index",
            current_files: &["prefix", "selected", "after"],
            current_index: Some(1),
            new_files: &["new-a", "new-b", "selected", "new-c"],
            expected: 2,
        },
        Case {
            label: "backward match selects the following row",
            current_files: &["ignored-zero", "kept", "removed"],
            current_index: Some(2),
            new_files: &["new-a", "kept", "new-b", "new-c"],
            expected: 2,
        },
        Case {
            label: "backward match at the final row stays in bounds",
            current_files: &["ignored-zero", "kept", "removed"],
            current_index: Some(2),
            new_files: &["new-a", "new-b", "kept"],
            expected: 2,
        },
        Case {
            label: "backward scan deliberately excludes index zero",
            current_files: &["kept-at-zero", "removed"],
            current_index: Some(1),
            new_files: &["new-a", "kept-at-zero", "new-b"],
            expected: 0,
        },
    ];

    for case in cases {
        let current_files = case
            .current_files
            .iter()
            .map(|entry| (*entry).to_owned())
            .collect::<Vec<_>>();
        let new_files = case
            .new_files
            .iter()
            .map(|entry| (*entry).to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            ClientSession::local_playlist_target_index_from_changed_playlist_legacy_compatible(
                &current_files,
                case.current_index,
                &new_files,
            ),
            case.expected,
            "{}",
            case.label
        );
    }
}

#[test]
fn playlist_shuffle_seed_has_stable_scope_index_nonce_and_filename_framing() {
    struct Case {
        label: &'static str,
        remaining: bool,
        current_index: usize,
        nonce: u64,
        files: &'static [&'static str],
        expected: u64,
    }

    let cases = [
        Case {
            label: "base",
            remaining: false,
            current_index: 2,
            nonce: 0,
            files: &["alpha.mkv", "beta.mkv"],
            expected: 0x23cd_6ca5_3b7e_f524,
        },
        Case {
            label: "remaining scope",
            remaining: true,
            current_index: 2,
            nonce: 0,
            files: &["alpha.mkv", "beta.mkv"],
            expected: 0x0983_5015_a3b5_7127,
        },
        Case {
            label: "index",
            remaining: false,
            current_index: 3,
            nonce: 0,
            files: &["alpha.mkv", "beta.mkv"],
            expected: 0xa733_5e8f_87bd_a523,
        },
        Case {
            label: "nonce",
            remaining: false,
            current_index: 2,
            nonce: 1,
            files: &["alpha.mkv", "beta.mkv"],
            expected: 0x3280_a74b_2607_cd63,
        },
        Case {
            label: "filename order",
            remaining: false,
            current_index: 2,
            nonce: 0,
            files: &["beta.mkv", "alpha.mkv"],
            expected: 0x53ed_c8a3_7d01_10b1,
        },
        Case {
            label: "nul-delimited filename framing a plus bc",
            remaining: false,
            current_index: 3,
            nonce: 7,
            files: &["a", "bc"],
            expected: 0x814e_f795_20b0_53dd,
        },
        Case {
            label: "nul-delimited filename framing ab plus c",
            remaining: false,
            current_index: 3,
            nonce: 7,
            files: &["ab", "c"],
            expected: 0x5a2a_7bc9_5bd6_6aa7,
        },
    ];

    for case in cases {
        let mut session = ClientSession::default();
        session.model.playlist.shuffle_nonce = case.nonce;
        let files = case
            .files
            .iter()
            .map(|entry| (*entry).to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            session.next_playlist_shuffle_seed_legacy_compatible(
                &files,
                case.current_index,
                case.remaining,
            ),
            case.expected,
            "{}",
            case.label
        );
        assert_eq!(
            session.model.playlist.shuffle_nonce,
            case.nonce.wrapping_add(1),
            "{} nonce transition",
            case.label
        );
    }
}

#[test]
fn playlist_shuffle_prng_transition_matches_independent_golden_vectors() {
    let cases = [
        (0_u64, 0x1405_7b7e_f767_814f_u64),
        (1_u64, 0x6c57_6fac_43fd_007c_u64),
        (u64::MAX, 0xbbb3_8751_aad2_0222_u64),
        (0x0123_4567_89ab_cdef_u64, 0x2ce3_2d23_35df_4552_u64),
    ];

    for (initial, expected) in cases {
        let mut state = initial;
        let returned = ClientSession::next_shuffle_state_legacy_compatible(&mut state);
        assert_eq!(
            returned, expected,
            "return value for initial {initial:#018x}"
        );
        assert_eq!(state, expected, "stored state for initial {initial:#018x}");
    }
}

#[test]
fn playlist_fisher_yates_shuffle_matches_golden_permutations_and_preserves_members() {
    let cases = [
        (0_u64, ["D", "A", "B", "E", "C"]),
        (1_u64, ["A", "B", "E", "D", "C"]),
        (0x0123_4567_89ab_cdef_u64, ["A", "E", "C", "B", "D"]),
        (u64::MAX, ["C", "E", "A", "B", "D"]),
    ];

    for (seed, expected) in cases {
        let mut files = ["A", "B", "C", "D", "E"].map(str::to_owned);
        ClientSession::shuffle_playlist_slice_in_place_legacy_compatible(&mut files, seed);
        assert_eq!(
            files,
            expected.map(str::to_owned),
            "golden permutation for seed {seed:#018x}"
        );
        let mut sorted = files;
        sorted.sort();
        assert_eq!(sorted, ["A", "B", "C", "D", "E"].map(str::to_owned));
    }

    let mut empty: [String; 0] = [];
    ClientSession::shuffle_playlist_slice_in_place_legacy_compatible(&mut empty, 7);
    assert!(empty.is_empty());
    let mut singleton = ["only".to_owned()];
    ClientSession::shuffle_playlist_slice_in_place_legacy_compatible(&mut singleton, 7);
    assert_eq!(singleton, ["only".to_owned()]);
}

#[test]
fn playlist_fisher_yates_shuffle_is_a_permutation_across_fixed_seed_stress() {
    let expected = (0..17)
        .map(|index| format!("episode-{index:02}.mkv"))
        .collect::<Vec<_>>();
    for seed in 0..512_u64 {
        let mut shuffled = expected.clone();
        ClientSession::shuffle_playlist_slice_in_place_legacy_compatible(&mut shuffled, seed);
        shuffled.sort();
        assert_eq!(shuffled, expected, "permutation invariant for seed {seed}");
    }
}
