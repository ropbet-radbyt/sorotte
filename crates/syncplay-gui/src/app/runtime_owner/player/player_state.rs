use super::*;

impl GuiPersistedConfigRuntimeOwner {
    fn normalized_current_player_match_key(path: &str) -> String {
        let mut key = path.trim().replace('\\', "/");
        while key.ends_with('/') && key.len() > 1 {
            key.pop();
        }
        if cfg!(windows) {
            key.to_ascii_lowercase()
        } else {
            key
        }
    }

    fn local_media_target_has_path_context(target: &str) -> bool {
        if browser_is_url(target) {
            return true;
        }
        let target = target.trim();
        target.contains('/')
            || target.contains('\\')
            || Path::new(target).is_absolute()
            || Path::new(target).components().count() > 1
    }

    fn playlist_target_for_index(state: &SyncplayGuiShellAppState, index: usize) -> Option<String> {
        if !state.main_window.shared_playlist_enabled {
            return None;
        }

        state
            .main_window
            .playlist
            .get(index)
            .and_then(|target| normalized_editable_text(&target.label))
    }

    pub(in crate::app::runtime_owner) fn current_shared_playlist_target(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Option<String> {
        self.session
            .as_ref()
            .and_then(|session| session.current_room_playlist_index())
            .and_then(|index| Self::playlist_target_for_index(state, index))
            .or_else(|| {
                self.active_shared_playlist_index
                    .and_then(|index| Self::playlist_target_for_index(state, index))
            })
    }

    pub(in crate::app::runtime_owner) fn current_player_matches_media_target(
        &self,
        target: &str,
    ) -> bool {
        let Some(local_file) = self.player_local_file.as_ref() else {
            return false;
        };

        if let Some(path) = local_file.path.as_deref()
            && Self::normalized_current_player_match_key(path)
                == Self::normalized_current_player_match_key(target)
        {
            return true;
        }

        if browser_is_url(target) {
            return if cfg!(windows) {
                local_file.name.eq_ignore_ascii_case(target)
            } else {
                local_file.name == target
            };
        }
        if Self::local_media_target_has_path_context(target) {
            return false;
        }

        let target_name = if browser_is_url(target) {
            Some(target)
        } else {
            Path::new(target).file_name().and_then(|name| name.to_str())
        };
        target_name.is_some_and(|target_name| {
            if cfg!(windows) {
                local_file.name.eq_ignore_ascii_case(target_name)
            } else {
                local_file.name == target_name
            }
        })
    }

    fn local_file_identity_matches(current: &LocalFileUpdate, next: &LocalFileUpdate) -> bool {
        let current_path = current
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let next_path = next
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty());
        if let (Some(current_path), Some(next_path)) = (current_path, next_path) {
            return if cfg!(windows) {
                current_path.eq_ignore_ascii_case(next_path)
            } else {
                current_path == next_path
            };
        }

        let current_name = current.name.trim();
        let next_name = next.name.trim();
        if current_name.is_empty() || next_name.is_empty() {
            return false;
        }

        if cfg!(windows) {
            current_name.eq_ignore_ascii_case(next_name)
        } else {
            current_name == next_name
        }
    }

    pub(super) fn local_file_update_replaces_current_file(
        current: Option<&LocalFileUpdate>,
        next: &LocalFileUpdate,
    ) -> bool {
        match current {
            Some(current) => !Self::local_file_identity_matches(current, next),
            None => true,
        }
    }

    pub(in crate::app::runtime_owner) fn player_target_position_seconds_for_global_position_impl(
        &self,
        global_position_seconds: f64,
    ) -> f64 {
        (global_position_seconds + self.user_offset_seconds).max(0.0)
    }

    pub(super) fn current_player_file_duration_seconds(&self) -> Option<f64> {
        self.player_local_file
            .as_ref()
            .and_then(|local_file| local_file.duration_seconds)
            .filter(|duration_seconds| duration_seconds.is_finite() && *duration_seconds >= 0.0)
    }

    pub(super) fn clamp_player_position_to_file_duration(&mut self) {
        let Some(duration_seconds) = self.current_player_file_duration_seconds() else {
            return;
        };
        let Some(position_seconds) = self
            .player_position_seconds
            .filter(|position_seconds| position_seconds.is_finite())
        else {
            return;
        };
        self.player_position_seconds = Some(position_seconds.clamp(0.0, duration_seconds));
    }
}
