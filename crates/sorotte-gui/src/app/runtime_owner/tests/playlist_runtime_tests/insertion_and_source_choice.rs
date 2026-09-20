//! Row identity and inactive-source playback regressions.
use super::*;

fn assert_insert_preserves_active_identity(
    connected: bool,
    duplicate_names: bool,
    insert_slot: usize,
) {
    let mut rig = PlaylistEditingRig::new(connected, duplicate_names);
    rig.activate(1);
    let before_ids = rig.row_ids();
    let before_path = rig.owner.player_local_file.as_ref().unwrap().path.clone();
    assert_eq!(rig.shell.main_window.active_playlist_index, Some(1));
    let third = rig._root.path().join("third.mkv");
    std::fs::write(&third, b"third").unwrap();
    rig.handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![third.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(insert_slot),
    });
    rig.pump();
    eprintln!(
        "connected={connected} duplicate_names={duplicate_names} insert_slot={insert_slot} active={:?} path_before={before_path:?} path_after={:?}",
        rig.shell.main_window.active_playlist_index,
        rig.owner.player_local_file.as_ref().unwrap().path
    );
    assert_eq!(rig.labels().len(), 3);
    let active = rig.shell.main_window.active_playlist_index.unwrap();
    let after_id = rig.row_ids()[active];
    let after_path = rig.owner.player_local_file.as_ref().unwrap().path.clone();
    let survivor_ids = rig
        .row_ids()
        .into_iter()
        .filter(|id| before_ids.contains(id))
        .collect::<Vec<_>>();

    // Follow-on editing still runs in this same session before the desired-behavior oracle.
    let third_index = rig
        .labels()
        .iter()
        .position(|name| name == "third.mkv")
        .unwrap();
    rig.remove(third_index);
    rig.undo();
    eprintln!(
        "after follow-on remove/undo: active={:?} path={:?}",
        rig.shell.main_window.active_playlist_index,
        rig.owner.player_local_file.as_ref().unwrap().path
    );
    assert_eq!(rig.labels().len(), 3);
    assert_eq!(
        survivor_ids, before_ids,
        "insertion must preserve the order of pre-existing rows"
    );
    assert_eq!(
        after_id, before_ids[1],
        "inserting must preserve the selected row's identity (connected={connected}, duplicate_names={duplicate_names}, insert_slot={insert_slot})"
    );
    assert_eq!(after_path, before_path);
    assert_eq!(
        rig.owner.player_local_file.as_ref().unwrap().path,
        before_path,
        "a later remove/undo must preserve the same active source"
    );
}

#[test]
fn detached_append_preserves_active_duplicate_identity() {
    assert_insert_preserves_active_identity(false, true, 2);
}

#[test]
fn connected_append_preserves_active_duplicate_identity() {
    assert_insert_preserves_active_identity(true, true, 2);
}

#[test]
fn duplicate_insert_preserves_active_identity_at_every_slot() {
    for connected in [false, true] {
        for slot in [0, 1, 2] {
            assert_insert_preserves_active_identity(connected, true, slot);
        }
    }
}

#[test]
fn distinct_label_insert_controls() {
    for connected in [false, true] {
        for insert_slot in [0, 1, 2] {
            assert_insert_preserves_active_identity(connected, false, insert_slot);
        }
    }
}

fn assert_active_removal_resets_physical_successor(duplicate_names: bool) {
    let mut rig = PlaylistEditingRig::new(true, duplicate_names);
    let before_ids = rig.row_ids();
    let successor_path = rig.owner.playlist_resolution.local_origins_by_row[&before_ids[1]].clone();
    rig.handle
        .push_request(GuiRuntimeRequest::SeekToPosition(30.0));
    rig.pump();
    assert_eq!(rig.owner.player_position_seconds, Some(30.0));
    assert_eq!(rig.shell.main_window.active_playlist_index, Some(0));

    rig.remove(0);
    let successor_position = rig.owner.player_position_seconds;
    let successor_room = rig
        .owner
        .session
        .as_ref()
        .unwrap()
        .current_room_playstate()
        .unwrap();
    assert_eq!(rig.row_ids(), vec![before_ids[1]]);
    assert_eq!(rig.shell.main_window.active_playlist_index, Some(0));
    assert_eq!(
        rig.owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_ref())
            .map(|path| std::fs::canonicalize(path).unwrap()),
        Some(std::fs::canonicalize(successor_path).unwrap()),
        "removing the active row loads the surviving row's exact physical file"
    );

    // An explicit replay remains valid after this compound edit in the same session.
    rig.activate(0);
    assert_eq!(rig.owner.player_position_seconds, Some(0.0));
    let replay_room = rig
        .owner
        .session
        .as_ref()
        .unwrap()
        .current_room_playstate()
        .unwrap();
    assert_eq!(replay_room.position_seconds, Some(0.0));
    assert_eq!(replay_room.paused, Some(true));
    assert_eq!(
        successor_position,
        Some(0.0),
        "deleting the active row must reset its physical successor, even when the labels match"
    );
    assert_eq!(successor_room.position_seconds, Some(0.0));
    assert_eq!(successor_room.paused, Some(true));
}

#[test]
fn connected_active_duplicate_removal_resets_physical_successor() {
    assert_active_removal_resets_physical_successor(true);
}

#[test]
fn connected_active_distinct_removal_resets_physical_successor_control() {
    assert_active_removal_resets_physical_successor(false);
}

struct RecordingTestPlayer {
    inner: GuiTestPlayerAdapter,
    opens: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl sorotte_player_api::PlayerAdapter for RecordingTestPlayer {
    fn name(&self) -> &'static str {
        "test"
    }
    fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
        self.opens.lock().unwrap().push(path.to_owned());
        self.inner.open_file(path)
    }
    fn execute_tracked(
        &mut self,
        command: sorotte_player_api::PlayerCommand,
    ) -> Result<sorotte_player_api::PlayerCommandId, sorotte_player_api::PlayerError> {
        if let sorotte_player_api::PlayerCommand::OpenFile(path) = &command {
            self.opens.lock().unwrap().push(path.clone());
        }
        self.inner.execute_tracked(command)
    }
    fn supports_transport_telemetry(&self) -> bool {
        self.inner.supports_transport_telemetry()
    }
    fn set_paused(&mut self, value: bool) -> Result<(), sorotte_player_api::PlayerError> {
        self.inner.set_paused(value)
    }
    fn set_position(&mut self, value: f64) -> Result<(), sorotte_player_api::PlayerError> {
        self.inner.set_position(value)
    }
    fn set_playback_rate(&mut self, value: f64) -> Result<(), sorotte_player_api::PlayerError> {
        self.inner.set_playback_rate(value)
    }
    fn unload(&mut self) -> Result<(), sorotte_player_api::PlayerError> {
        self.inner.unload()
    }
    fn take_player_event_batch(&mut self) -> Option<sorotte_player_api::PlayerEventBatch> {
        self.inner.take_player_event_batch()
    }
    fn acknowledge_player_event_batch(
        &mut self,
        token: sorotte_player_api::PlayerEventAcknowledgementToken,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        self.inner.acknowledge_player_event_batch(token)
    }
}

fn assert_source_choice_preserves_playback(connected: bool, source_index: usize, seek_first: bool) {
    let mut rig = PlaylistEditingRig::new(connected, false);
    if seek_first {
        rig.handle
            .push_request(GuiRuntimeRequest::SeekToPosition(30.0));
        rig.pump();
    }
    let before_position = rig.owner.player_position_seconds;
    if seek_first {
        assert_eq!(
            before_position,
            Some(30.0),
            "fixture must establish an actual nonzero playback position"
        );
    }
    let before_path = rig.owner.player_local_file.as_ref().unwrap().path.clone();
    assert_eq!(rig.shell.main_window.active_playlist_index, Some(0));
    let GuiOwnedPlayer::Test(inner) = rig.owner.player.take().unwrap() else {
        panic!("fixture player should be simulated");
    };
    let opens = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    rig.owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingTestPlayer {
        inner,
        opens: opens.clone(),
    })));
    let plan = crate::app::local_command_dispatch::GuiShellDispatchPlan::from_shell_actions(
        &rig.shell,
        vec![GuiShellAction::SelectMainWindowPlaylistSource {
            index: source_index,
            provider_id: crate::app::GuiMediaSourceProviderId::local(),
        }],
    );
    for action in plan.shell_actions {
        rig.shell.apply(action);
    }
    for request in plan.runtime_requests {
        for action in rig.bridge.dispatch_runtime_request(&rig.shell, request) {
            rig.shell.apply(action);
        }
    }
    rig.pump();
    eprintln!(
        "active={:?} path_before={before_path:?} path_after={:?}",
        rig.shell.main_window.active_playlist_index,
        rig.owner.player_local_file.as_ref().unwrap().path
    );
    assert_eq!(rig.shell.main_window.active_playlist_index, Some(0));
    assert_eq!(
        rig.owner.player_local_file.as_ref().unwrap().path,
        before_path,
        "changing the provider of an inactive row must not replace the room's active media"
    );
    eprintln!("physical OpenFile commands: {:?}", *opens.lock().unwrap());
    eprintln!(
        "connected={connected} source_index={source_index} position_before={before_position:?} position_after={:?}",
        rig.owner.player_position_seconds
    );
    assert!(
        opens.lock().unwrap().is_empty(),
        "changing an inactive source must not briefly load B then reload A"
    );
    assert_eq!(
        rig.shell.main_window.playlist[source_index]
            .source_state
            .policy,
        crate::app::GuiPlaylistSourcePolicy::ForceLocal
    );
    if source_index == 1 {
        rig.activate(source_index);
        assert_eq!(rig.shell.main_window.active_playlist_index, Some(1));
        assert_eq!(
            rig.owner
                .player_local_file
                .as_ref()
                .unwrap()
                .path
                .as_deref()
                .map(|path| std::fs::canonicalize(path).unwrap()),
            Some(std::fs::canonicalize(rig._root.path().join("second/second.mkv")).unwrap())
        );
        assert_eq!(
            opens.lock().unwrap().len(),
            1,
            "activation opens the chosen source once"
        );
    }
}

#[test]
fn nonactive_source_choice_does_not_transiently_open_another_item() {
    assert_source_choice_preserves_playback(true, 1, true);
}

#[test]
fn nonactive_source_choice_detached() {
    assert_source_choice_preserves_playback(false, 1, false);
}

#[test]
fn active_source_choice_controls() {
    for connected in [false, true] {
        assert_source_choice_preserves_playback(connected, 0, true);
    }
}
