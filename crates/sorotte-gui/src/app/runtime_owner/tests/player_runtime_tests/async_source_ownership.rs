use super::*;
use crate::app::testing::support::pump_worker_state;
use crate::app::{GuiMediaSourceProviderId, GuiPlaylistSourcePolicy};
use std::sync::{Arc, Mutex, atomic::AtomicBool, mpsc};

#[derive(Default)]
struct RecordingPlayer(Arc<Mutex<Vec<String>>>);

impl PlayerAdapter for RecordingPlayer {
    fn name(&self) -> &'static str {
        "async-source-recorder"
    }

    fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
        self.0.lock().unwrap().push(path.to_owned());
        Ok(())
    }

    fn set_position(&mut self, _: f64) -> Result<(), sorotte_player_api::PlayerError> {
        Ok(())
    }

    fn set_paused(&mut self, _: bool) -> Result<(), sorotte_player_api::PlayerError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
enum SelectionAction {
    Unchanged,
    Other,
    Reselect,
    ReplacePlaylist,
}

fn late_local_resolution_after_selection_change(
    connected: bool,
    selection_action: SelectionAction,
) -> Vec<String> {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let nested = root.join("nested");
    std::fs::create_dir(&nested).unwrap();
    let late = nested.join("late.mkv");
    let now = root.join("now.mkv");
    std::fs::write(&late, b"late").unwrap();
    std::fs::write(&now, b"now").unwrap();
    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(root);
    let (result_tx, result_rx) = mpsc::channel();
    let opened = Arc::new(Mutex::new(Vec::new()));
    let (mut owner, transport) = if connected {
        let (owner, transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_recording_chat_session_runtime("alice", "room1")
            .unwrap();
        (owner, Some(transport))
    } else {
        (GuiPersistedConfigRuntimeOwner::with_config_path(None), None)
    };
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayer(
        opened.clone(),
    ))));
    owner.active_shared_playlist_index = Some(0);
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![root_key.clone()],
        root_indexes_by_key: Default::default(),
        roots_requiring_refresh: std::collections::BTreeSet::from([root_key.clone()]),
    });
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        roots: vec![root_key.clone()],
        cancel_flag: Arc::new(AtomicBool::new(false)),
        latest_progress: Arc::new(Mutex::new(None)),
        result_rx,
    });
    let mut state =
        crate::app::runtime_state::GuiRuntimeState::from_stored_settings(&StoredClientSettings {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            shared_playlist_enabled: Some(true),
            media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
            media_matching_plugin_enabled: Some(false),
            plex_plugin_enabled: Some(false),
            ..StoredClientSettings::default()
        });
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    if let Some(transport) = transport.as_ref() {
        pump_worker_state(&mut owner, &handle, &mut state);
        transport.push_inbound_protocol_lines([
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sharedPlaylists":true}}}"#.to_owned(),
            r#"{"Set":{"playlistChange":{"files":["late.mkv","now.mkv"],"user":"bob"}}}"#.to_owned(),
            r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
        ]);
        pump_worker_state(&mut owner, &handle, &mut state);
    } else {
        state.apply_shared_playlist_entries(
            vec!["late.mkv".into(), "now.mkv".into()],
            Some(0),
            false,
        );
        state.playlist.main_window.active_playlist_index = Some(0);
    }
    if connected {
        // The real source-picker command applies its shell reducer before the
        // queued runtime request. Preserve that UserOverride/ForceLocal state.
        let mut shell =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
        shell.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
            crate::app::shell_state::MainWindowRuntimeSnapshot::from_shell_state(
                &state.playlist.main_window,
            ),
        ));
        assert!(shell.apply(GuiShellAction::SelectMainWindowPlaylistSource {
            index: 0,
            provider_id: GuiMediaSourceProviderId::local(),
        }));
        state.apply_main_window_runtime_snapshot(
            crate::app::shell_state::MainWindowRuntimeSnapshot::from_shell_state(
                &shell.main_window,
            ),
        );
        assert_eq!(
            state.playlist.main_window.playlist[0].source_state.policy,
            GuiPlaylistSourcePolicy::ForceLocal
        );
    }
    handle.push_request(GuiRuntimeRequest::ResolvePlaylistSource {
        index: 0,
        provider_id: GuiMediaSourceProviderId::local(),
    });
    pump_worker_state(&mut owner, &handle, &mut state);
    assert!(
        owner.pending_playlist_source_resolution.is_some(),
        "local resolution must be pending on its in-flight search"
    );
    assert!(opened.lock().unwrap().is_empty());

    match selection_action {
        SelectionAction::Unchanged => {}
        SelectionAction::Other => handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(1)),
        SelectionAction::Reselect => handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(0)),
        SelectionAction::ReplacePlaylist => {
            transport.as_ref().unwrap().push_inbound_protocol_lines([
                r#"{"Set":{"playlistChange":{"files":["now.mkv"],"user":"bob"}}}"#.to_owned(),
                r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
            ])
        }
    }
    if !matches!(selection_action, SelectionAction::Unchanged) {
        pump_worker_state(&mut owner, &handle, &mut state);
    }
    if matches!(
        selection_action,
        SelectionAction::Other | SelectionAction::ReplacePlaylist
    ) {
        let expected_index = if matches!(selection_action, SelectionAction::Other) {
            1
        } else {
            0
        };
        assert_eq!(
            state.playlist.main_window.active_playlist_index,
            Some(expected_index)
        );
        if let Some(session) = owner.session.as_ref() {
            assert_eq!(session.current_room_playlist_index(), Some(expected_index));
        }
        assert!(
            opened
                .lock()
                .unwrap()
                .iter()
                .any(|path| std::path::Path::new(path) == now),
            "successor should open before old search completes"
        );
        assert!(
            owner.pending_shared_playlist_open.is_none(),
            "selection receipt must be delivered before worker completion"
        );
        opened.lock().unwrap().clear();
    }
    result_tx
        .send(GuiAttachedMediaSearchBuildStatus::Completed(vec![
            GuiAttachedMediaSearchRootRefreshResult {
                root_key: root_key.clone(),
                index: Some(GuiAttachedMediaSearchRootIndex {
                    root_key,
                    root_path: root.to_owned(),
                    built_at_unix_ms: 1,
                    candidates_by_name: std::collections::HashMap::from([
                        ("late.mkv".to_owned(), vec!["nested/late.mkv".to_owned()]),
                        ("now.mkv".to_owned(), vec!["now.mkv".to_owned()]),
                    ]),
                }),
                error: None,
            },
        ]))
        .unwrap();
    pump_worker_state(&mut owner, &handle, &mut state);
    for _ in 0..3 {
        pump_worker_state(&mut owner, &handle, &mut state);
    }
    opened
        .lock()
        .unwrap()
        .iter()
        .map(|path| {
            std::path::Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>()
}

#[test]
fn late_local_source_completion_does_not_reopen_unselected_row() {
    let opened = late_local_resolution_after_selection_change(true, SelectionAction::Other);
    assert!(
        !opened.iter().any(|path| path == "late.mkv"),
        "a superseded source request must not open the previous row after a new selection"
    );
}

#[test]
fn late_local_source_completion_current_selection_control() {
    let opened = late_local_resolution_after_selection_change(true, SelectionAction::Unchanged);
    assert_eq!(
        opened,
        ["late.mkv"],
        "current pending selection should open exactly once after discovery"
    );
}

#[test]
fn late_local_source_completion_reselection_control() {
    let opened = late_local_resolution_after_selection_change(true, SelectionAction::Reselect);
    assert_eq!(opened, ["late.mkv"]);
}

#[test]
fn late_local_source_completion_playlist_replacement_control() {
    let opened =
        late_local_resolution_after_selection_change(true, SelectionAction::ReplacePlaylist);
    assert!(
        opened.is_empty(),
        "completion from the replaced playlist must not reopen any row"
    );
}

#[test]
fn late_local_source_completion_detached_does_not_reopen_unselected_row() {
    let opened = late_local_resolution_after_selection_change(false, SelectionAction::Other);
    assert!(!opened.iter().any(|path| path == "late.mkv"));
}

#[test]
fn late_local_source_completion_detached_retries_after_reply_consumed_by_other_resolver() {
    let opened = late_local_resolution_after_selection_change(false, SelectionAction::Unchanged);
    assert_eq!(
        opened,
        ["late.mkv"],
        "unchanged detached source request should complete after discovery"
    );
}
