use super::super::*;

impl ClientSession {
    pub fn runtime_actions_for_local_playlist_index_set(
        &self,
        index: i64,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() || index < 0 {
            return Vec::new();
        }

        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        let Ok(index_usize) = usize::try_from(index) else {
            return Vec::new();
        };
        if index_usize >= playlist.files.len() {
            return Vec::new();
        }
        if !self.playlist_target_switch_allowed_legacy_compatible(&playlist.files[index_usize]) {
            return Vec::new();
        }

        vec![ClientRuntimeAction::SetPlaylistIndex { index }]
    }

    pub fn runtime_actions_for_local_playlist_next(&self) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }

        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        if playlist.files.is_empty() {
            return Vec::new();
        }
        let Some(current_index) = playlist.index.and_then(|index| usize::try_from(index).ok())
        else {
            return Vec::new();
        };
        if current_index >= playlist.files.len() {
            return Vec::new();
        }
        if self.current_user_file_name() != Some(playlist.files[current_index].as_str()) {
            return Vec::new();
        }

        if playlist.files.len() == 1 {
            if !self.loop_single_files_enabled_legacy_compatible() {
                return Vec::new();
            }
            return vec![
                ClientRuntimeAction::SetPosition(0.0),
                ClientRuntimeAction::SetPaused(false),
            ];
        }

        let Some(next_index) = current_index.checked_add(1) else {
            return Vec::new();
        };
        if next_index >= playlist.files.len() {
            if !self.loop_at_end_of_playlist_enabled_legacy_compatible() {
                return Vec::new();
            }
            if !self.playlist_target_switch_allowed_legacy_compatible(&playlist.files[0]) {
                return Vec::new();
            }
            return vec![ClientRuntimeAction::SetPlaylistIndex { index: 0 }];
        }
        if !self.playlist_target_switch_allowed_legacy_compatible(&playlist.files[next_index]) {
            return Vec::new();
        }

        vec![ClientRuntimeAction::SetPlaylistIndex {
            index: next_index as i64,
        }]
    }

    pub fn runtime_actions_for_local_playlist_queue(
        &mut self,
        file_name: String,
        select_after_queue: bool,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };

        if file_name.is_empty() {
            return Vec::new();
        }

        let (current_files, current_index) = self
            .current_room_playlist()
            .map(|playlist| {
                (
                    playlist.files.clone(),
                    playlist.index.and_then(|index| usize::try_from(index).ok()),
                )
            })
            .unwrap_or_default();
        if current_files
            .iter()
            .any(|current_file| current_file == &file_name)
        {
            return Vec::new();
        }
        let mut files = current_files.clone();
        files.push(file_name);
        self.capture_playlist_undo_snapshot_legacy_compatible(&room_name, &current_files, &files);

        let target_index = if select_after_queue {
            files.len().saturating_sub(1)
        } else {
            current_index
                .filter(|index| *index < current_files.len())
                .unwrap_or(0)
        };

        vec![
            ClientRuntimeAction::SetPlaylist { files },
            ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_delete(
        &mut self,
        index: i64,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() || index < 0 {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };

        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        let current_files = playlist.files.clone();
        let current_index = playlist
            .index
            .and_then(|current| usize::try_from(current).ok());
        let Ok(delete_index) = usize::try_from(index) else {
            return Vec::new();
        };
        if delete_index >= current_files.len() {
            return Vec::new();
        }

        let mut files = current_files.clone();
        files.remove(delete_index);
        self.capture_playlist_undo_snapshot_legacy_compatible(&room_name, &current_files, &files);

        if files.is_empty() {
            return vec![ClientRuntimeAction::SetPlaylist { files }];
        }

        let target_index = current_index
            .map(|current| {
                if current < delete_index {
                    current
                } else if current > delete_index {
                    current.saturating_sub(1)
                } else {
                    delete_index.min(files.len().saturating_sub(1))
                }
            })
            .unwrap_or(0)
            .min(files.len().saturating_sub(1));

        vec![
            ClientRuntimeAction::SetPlaylist { files },
            ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_replace(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        if files.iter().any(|file| file.is_empty()) {
            return Vec::new();
        }

        let (current_files, current_index) = self
            .current_room_playlist()
            .map(|playlist| {
                (
                    playlist.files.clone(),
                    playlist.index.and_then(|index| usize::try_from(index).ok()),
                )
            })
            .unwrap_or_default();
        let playlist_changed = files != current_files;
        if playlist_changed {
            self.capture_playlist_undo_snapshot_legacy_compatible(
                &room_name,
                &current_files,
                &files,
            );
        }
        if files.is_empty() {
            return playlist_changed
                .then_some(ClientRuntimeAction::SetPlaylist { files })
                .into_iter()
                .collect();
        }

        let target_index = selected_index
            .filter(|index| *index < files.len())
            .or_else(|| {
                Some(
                    Self::local_playlist_target_index_from_changed_playlist_legacy_compatible(
                        &current_files,
                        current_index,
                        &files,
                    )
                    .min(files.len().saturating_sub(1)),
                )
            })
            .unwrap_or(0);

        let playlist_index_changed = current_index != Some(target_index);
        if !playlist_changed && !playlist_index_changed {
            return Vec::new();
        }

        let mut actions = Vec::new();
        if playlist_changed {
            actions.push(ClientRuntimeAction::SetPlaylist { files });
        }
        if playlist_index_changed {
            actions.push(ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            });
        }
        actions
    }

    pub fn runtime_actions_for_local_playlist_undo(&mut self) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };

        let current_files = playlist.files.clone();
        let current_index = playlist.index.and_then(|index| usize::try_from(index).ok());
        let Some(previous_files) = self.playlist_undo_snapshots.get(&room_name).cloned() else {
            return Vec::new();
        };
        if previous_files == current_files {
            return Vec::new();
        }

        self.capture_playlist_undo_snapshot_legacy_compatible(
            &room_name,
            &current_files,
            &previous_files,
        );

        if previous_files.is_empty() {
            return vec![ClientRuntimeAction::SetPlaylist {
                files: previous_files,
            }];
        }

        let target_index =
            Self::local_playlist_target_index_from_changed_playlist_legacy_compatible(
                &current_files,
                current_index,
                &previous_files,
            )
            .min(previous_files.len().saturating_sub(1));

        vec![
            ClientRuntimeAction::SetPlaylist {
                files: previous_files,
            },
            ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_shuffle_remaining(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        let Some(current_index) = playlist.index.and_then(|index| usize::try_from(index).ok())
        else {
            return Vec::new();
        };

        let current_files = playlist.files.clone();
        if current_index >= current_files.len() {
            return Vec::new();
        }
        let shuffle_start = current_index.saturating_add(1);
        if shuffle_start >= current_files.len() {
            return Vec::new();
        }

        let mut shuffled_files = current_files.clone();
        let seed =
            self.next_playlist_shuffle_seed_legacy_compatible(&current_files, current_index, true);
        Self::shuffle_playlist_slice_in_place_legacy_compatible(
            &mut shuffled_files[shuffle_start..],
            seed,
        );
        if shuffled_files == current_files {
            return Vec::new();
        }

        self.capture_playlist_undo_snapshot_legacy_compatible(
            &room_name,
            &current_files,
            &shuffled_files,
        );
        vec![
            ClientRuntimeAction::SetPlaylist {
                files: shuffled_files,
            },
            ClientRuntimeAction::SetPlaylistIndex {
                index: current_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_shuffle_entire(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };

        let current_files = playlist.files.clone();
        if current_files.is_empty() {
            return Vec::new();
        }
        let current_index = playlist.index.and_then(|index| usize::try_from(index).ok());
        let mut shuffled_files = current_files.clone();
        let seed = self.next_playlist_shuffle_seed_legacy_compatible(
            &current_files,
            current_index.unwrap_or(0),
            false,
        );
        Self::shuffle_playlist_slice_in_place_legacy_compatible(&mut shuffled_files, seed);

        let playlist_changed = shuffled_files != current_files;
        if playlist_changed {
            self.capture_playlist_undo_snapshot_legacy_compatible(
                &room_name,
                &current_files,
                &shuffled_files,
            );
        }

        let mut actions = Vec::new();
        if playlist_changed {
            actions.push(ClientRuntimeAction::SetPlaylist {
                files: shuffled_files,
            });
        }
        if current_index != Some(0) || playlist_changed {
            actions.push(ClientRuntimeAction::SetPlaylistIndex { index: 0 });
        }
        actions
    }
}
