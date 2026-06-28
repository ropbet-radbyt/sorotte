use super::*;

use sorotte_plex::{is_plex_playlist_uri, parse_plex_playlist_uri};

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app::runtime_owner) fn normalized_current_player_match_key(path: &str) -> String {
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

    fn playlist_target_for_index(state: &SorotteGuiShellAppState, index: usize) -> Option<String> {
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
        state: &SorotteGuiShellAppState,
    ) -> Option<String> {
        self.session
            .as_ref()
            .and_then(|session| session.current_room_playlist_index())
            .and_then(|index| Self::playlist_target_for_index(state, index))
            .or_else(|| {
                state
                    .main_window
                    .active_playlist_index
                    .and_then(|index| Self::playlist_target_for_index(state, index))
            })
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

        if is_plex_playlist_uri(target) {
            if let Some(path) = local_file.path.as_deref()
                && Self::plex_playlist_target_identity_matches(path, target)
            {
                return true;
            }
            if self
                .pending_logical_media_override
                .as_ref()
                .is_some_and(|pending| {
                    Self::plex_playlist_target_identity_matches(&pending.requested_target, target)
                        || pending.logical_file.path.as_deref().is_some_and(|path| {
                            Self::plex_playlist_target_identity_matches(path, target)
                        })
                })
            {
                return true;
            }
            if Self::plex_playlist_target_matches_local_file_hints(local_file, target) {
                return true;
            }
        }

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

    fn plex_playlist_target_identity_matches(left: &str, right: &str) -> bool {
        if !is_plex_playlist_uri(left) || !is_plex_playlist_uri(right) {
            return false;
        }
        let Ok(left) = parse_plex_playlist_uri(left) else {
            return false;
        };
        let Ok(right) = parse_plex_playlist_uri(right) else {
            return false;
        };
        left.machine_identifier
            .eq_ignore_ascii_case(&right.machine_identifier)
            && left.rating_key == right.rating_key
    }

    fn plex_playlist_target_matches_local_file_hints(
        local_file: &LocalFileUpdate,
        target: &str,
    ) -> bool {
        let Ok(target) = parse_plex_playlist_uri(target) else {
            return false;
        };
        let Some(target_size_bytes) = target.size_bytes else {
            return false;
        };
        let Some(target_file_name) = target
            .file_name
            .as_deref()
            .and_then(|file_name| Path::new(file_name).file_name())
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            return false;
        };
        let local_file_name = local_file
            .path
            .as_deref()
            .filter(|path| !browser_is_url(path) && !is_plex_playlist_uri(path))
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| local_file.name.trim());
        if local_file_name.is_empty() {
            return false;
        }
        let name_matches = if cfg!(windows) {
            local_file_name.eq_ignore_ascii_case(target_file_name)
        } else {
            local_file_name == target_file_name
        };
        if !name_matches {
            return false;
        }
        let local_size_bytes = local_file.size_bytes.or_else(|| {
            local_file
                .path
                .as_deref()
                .filter(|path| !browser_is_url(path) && !is_plex_playlist_uri(path))
                .and_then(|path| std::fs::metadata(path).ok())
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len())
        });
        local_size_bytes == Some(target_size_bytes)
    }

    pub(super) fn local_file_identity_matches(
        current: &LocalFileUpdate,
        next: &LocalFileUpdate,
    ) -> bool {
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
