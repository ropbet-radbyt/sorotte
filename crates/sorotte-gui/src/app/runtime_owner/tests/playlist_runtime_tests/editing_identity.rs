use super::*;
use crate::app::runtime_stack::GuiOutboundProtocolDeliveryResult;

#[derive(Default)]
struct InMemoryServerDriver {
    server: sorotte_server::ServerRuntime,
}

impl GuiSessionTransportDriver for InMemoryServerDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        if let Some(delivery) = transport.take_outbound_protocol_delivery_for_driver() {
            let replies = self
                .server
                .handle_line_fanout("alice", delivery.line())
                .map_err(|error| error.to_string())?;
            transport.publish_outbound_protocol_delivery_result(
                GuiOutboundProtocolDeliveryResult::FrameWritten {
                    token: delivery.token(),
                },
            );
            transport.push_inbound_protocol_lines(
                replies
                    .into_iter()
                    .filter(|reply| reply.client_id == "alice")
                    .map(|reply| reply.line),
            );
        }
        Ok(())
    }
}

struct PlaylistEditingRig {
    _root: tempfile::TempDir,
    owner: GuiPersistedConfigRuntimeOwner,
    bridge: GuiQueuedRuntimeBridge,
    handle: GuiQueuedRuntimeBridgeHandle,
    shell: SorotteGuiShellAppState,
}

impl PlaylistEditingRig {
    fn new(connected: bool, duplicate_names: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let first_dir = root.path().join("first");
        let second_dir = root.path().join("second");
        std::fs::create_dir(&first_dir).unwrap();
        std::fs::create_dir(&second_dir).unwrap();
        let first = first_dir.join("episode.mkv");
        let second = second_dir.join(if duplicate_names {
            "episode.mkv"
        } else {
            "second.mkv"
        });
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        if connected {
            let (connected_owner, _) = owner
                .with_client_core_chat_session_runtime("alice", "room1")
                .unwrap();
            owner = connected_owner
                .with_session_transport_driver(Box::new(InMemoryServerDriver::default()));
        }
        owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
        let (bridge, handle) = GuiQueuedRuntimeBridge::new();
        let shell = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
            username: Some("alice".into()),
            room: Some("room1".into()),
            shared_playlist_enabled: Some(true),
            chat_input_enabled: Some(true),
            media_matching_plugin_enabled: Some(false),
            plex_plugin_enabled: Some(false),
            ..Default::default()
        });
        let mut rig = Self {
            _root: root,
            owner,
            bridge,
            handle,
            shell,
        };
        rig.pump();
        rig.handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
            paths: vec![
                first.to_string_lossy().into_owned(),
                second.to_string_lossy().into_owned(),
            ],
            load_into_shared_playlist: true,
            playlist_insert_slot: None,
        });
        rig.pump();
        assert_eq!(
            rig.shell.main_window.playlist.len(),
            2,
            "fixture opens two rows"
        );
        rig
    }

    fn pump(&mut self) {
        for _ in 0..12 {
            pump_and_apply_runtime_owner_actions(&mut self.owner, &self.handle, &mut self.shell);
        }
    }

    fn labels(&self) -> Vec<String> {
        self.shell.current_shared_playlist_entries()
    }

    fn row_ids(&self) -> Vec<crate::app::GuiPlaylistEntryId> {
        self.shell
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect()
    }

    fn remove(&mut self, index: usize) {
        assert!(
            self.shell
                .apply(GuiShellAction::SelectMainWindowPlaylist(index))
        );
        assert!(
            self.shell
                .apply(GuiShellAction::RemoveSelectedMainWindowPlaylist)
        );
        for action in self
            .bridge
            .actions_for_playlist_entry_removal(&self.shell, index)
        {
            self.shell.apply(action);
        }
        self.pump();
    }

    fn reorder(&mut self, from_index: usize, to_index: usize) {
        assert!(self.shell.apply(GuiShellAction::MoveMainWindowPlaylistRow {
            from_index,
            to_index
        }));
        assert!(self.shell.main_window_playlist_selection_is_local);
        for action in self
            .bridge
            .actions_for_playlist_reorder(&self.shell, self.labels(), None)
        {
            self.shell.apply(action);
        }
        self.pump();
    }

    fn undo(&mut self) {
        assert!(self.shell.apply(GuiShellAction::UndoSharedPlaylistChange));
        for action in self.bridge.actions_for_playlist_undo(&self.shell) {
            self.shell.apply(action);
        }
        self.pump();
    }

    fn activate(&mut self, index: usize) {
        assert!(
            self.shell
                .apply(GuiShellAction::ActivateMainWindowPlaylist(index))
        );
        for action in self
            .bridge
            .actions_for_playlist_activation(&self.shell, index)
        {
            self.shell.apply(action);
        }
        self.pump();
    }

    fn undo_via_chat(&mut self) {
        self.command_via_chat("/undoplaylist", |request| {
            matches!(request, GuiRuntimeRequest::UndoPlaylistChange)
        });
    }

    fn command_via_chat(&mut self, command: &str, expected: impl Fn(&GuiRuntimeRequest) -> bool) {
        let plan = crate::app::local_command_dispatch::GuiShellDispatchPlan::from_shell_actions(
            &self.shell,
            vec![GuiShellAction::BeginLocalChatSend(command.into())],
        );
        assert!(plan.runtime_requests.iter().any(expected));
        for action in plan.shell_actions {
            self.shell.apply(action);
        }
        for request in plan.runtime_requests {
            for action in self.bridge.dispatch_runtime_request(&self.shell, request) {
                self.shell.apply(action);
            }
        }
        self.pump();
    }
}

#[test]
fn connected_chat_delete_then_undo_preserves_each_exact_origin() {
    for duplicate_names in [false, true] {
        let mut rig = PlaylistEditingRig::new(true, duplicate_names);
        let before_ids = rig.row_ids();
        let removed_path =
            rig.owner.playlist_resolution.local_origins_by_row[&before_ids[1]].clone();
        rig.command_via_chat("/delete 2", |request| {
            matches!(request, GuiRuntimeRequest::DeletePlaylistIndex(1))
        });
        assert_eq!(rig.labels().len(), 1);
        rig.undo_via_chat();
        assert_eq!(rig.row_ids(), before_ids);
        rig.activate(1);
        assert_eq!(
            rig.owner
                .player_local_file
                .as_ref()
                .and_then(|file| file.path.as_ref())
                .map(|path| std::fs::canonicalize(path).unwrap()),
            Some(std::fs::canonicalize(&removed_path).unwrap())
        );
    }
}

#[test]
fn connected_remove_then_undo_restores_row_identity() {
    assert_remove_then_undo_restores_origin(false, 1);
}

#[test]
fn connected_remove_active_row_then_undo_restores_origin() {
    assert_remove_then_undo_restores_origin(false, 0);
}

#[test]
fn connected_remove_duplicate_then_undo_restores_each_exact_origin() {
    assert_remove_then_undo_restores_origin(true, 1);
    assert_remove_then_undo_restores_origin(true, 0);
}

fn assert_remove_then_undo_restores_origin(duplicate_names: bool, removed_index: usize) {
    let mut rig = PlaylistEditingRig::new(true, duplicate_names);
    let before = rig.labels();
    let before_ids = rig.row_ids();
    let removed_path = rig
        .owner
        .playlist_resolution
        .local_origins_by_row
        .get(&before_ids[removed_index])
        .unwrap()
        .clone();
    rig.remove(removed_index);
    let mut after_remove = before.clone();
    after_remove.remove(removed_index);
    assert_eq!(rig.labels(), after_remove);
    rig.undo_via_chat();
    assert_eq!(rig.labels(), before);
    rig.activate(removed_index);
    let after_path = rig
        .owner
        .player_local_file
        .as_ref()
        .and_then(|file| file.path.as_ref())
        .map(|path| std::fs::canonicalize(path).unwrap());
    assert_eq!(
        after_path,
        Some(std::fs::canonicalize(&removed_path).unwrap()),
        "restored row must still open its explicitly supplied local file"
    );
    assert_eq!(
        rig.row_ids(),
        before_ids,
        "Undo must retain exact local provenance for restored rows"
    );
}

#[test]
fn detached_reorder_undo_control() {
    let mut rig = PlaylistEditingRig::new(false, false);
    let before = rig.labels();
    let before_ids = rig.row_ids();
    rig.reorder(1, 0);
    assert_eq!(rig.labels(), ["second.mkv", "episode.mkv"]);
    rig.undo();
    assert_eq!(rig.labels(), before);
    assert_eq!(rig.row_ids(), before_ids);
}

#[test]
fn duplicate_reorder_preserves_active_row() {
    let mut rig = PlaylistEditingRig::new(true, true);
    let before_ids = rig.row_ids();
    let active = rig.shell.main_window.active_playlist_index.unwrap();
    let active_id = before_ids[active];
    let before_path = rig
        .owner
        .player_local_file
        .as_ref()
        .and_then(|file| file.path.clone());
    rig.reorder(1, 0);
    let after_active = rig.shell.main_window.active_playlist_index.unwrap();
    let after_path = rig
        .owner
        .player_local_file
        .as_ref()
        .and_then(|file| file.path.clone());
    assert_eq!(
        rig.row_ids()[after_active],
        active_id,
        "reorder must follow active row identity even with duplicate labels"
    );
    assert_eq!(
        after_path, before_path,
        "reorder must not switch physical media"
    );
}

#[test]
fn distinct_reorder_preserves_active_row_control() {
    let mut rig = PlaylistEditingRig::new(true, false);
    let active_id = rig.row_ids()[rig.shell.main_window.active_playlist_index.unwrap()];
    let before_path = rig
        .owner
        .player_local_file
        .as_ref()
        .and_then(|file| file.path.clone());
    rig.reorder(1, 0);
    assert_eq!(
        rig.row_ids()[rig.shell.main_window.active_playlist_index.unwrap()],
        active_id
    );
    assert_eq!(
        rig.owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.clone()),
        before_path
    );
}

#[test]
fn connected_reorder_chat_undo_preserves_local_file_control() {
    let mut rig = PlaylistEditingRig::new(true, false);
    let before_ids = rig.row_ids();
    let second_path = rig
        .owner
        .playlist_resolution
        .local_origins_by_row
        .get(&before_ids[1])
        .unwrap()
        .clone();
    rig.reorder(1, 0);
    rig.undo_via_chat();
    assert_eq!(rig.labels(), ["episode.mkv", "second.mkv"]);
    assert_eq!(rig.row_ids(), before_ids);
    rig.activate(1);
    assert_eq!(
        rig.owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_ref())
            .map(|path| std::fs::canonicalize(path).unwrap()),
        Some(std::fs::canonicalize(&second_path).unwrap())
    );
}

#[test]
fn connected_no_removal_opens_explicit_second_file_control() {
    let mut rig = PlaylistEditingRig::new(true, false);
    let second_id = rig.row_ids()[1];
    let second_path = rig
        .owner
        .playlist_resolution
        .local_origins_by_row
        .get(&second_id)
        .unwrap()
        .clone();
    rig.activate(1);
    assert_eq!(
        rig.owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_ref())
            .map(|path| std::fs::canonicalize(path).unwrap()),
        Some(std::fs::canonicalize(&second_path).unwrap())
    );
}
