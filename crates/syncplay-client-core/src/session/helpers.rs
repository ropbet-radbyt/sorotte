use super::*;

impl ClientSession {
    pub(super) fn merge_room_playstate(
        &mut self,
        room_name: String,
        playstate: PlaystatePayload,
        updated_at_seconds: f64,
    ) {
        let room_key = room_name.clone();
        let room_playstate = self.room_playstates.entry(room_name).or_default();
        if let Some(position) = playstate.position {
            room_playstate.position = Some(position);
        }
        if let Some(paused) = playstate.paused {
            room_playstate.paused = Some(paused);
        }
        room_playstate.do_seek = Some(playstate.do_seek.unwrap_or(false));
        room_playstate.set_by = playstate.set_by;
        self.room_playstate_updated_at_seconds
            .insert(room_key, updated_at_seconds);
    }

    pub(super) fn apply_inbound_ignore_counters(&mut self, state_payload: &StatePayload) {
        let Some(ignore) = state_payload.ignoring_on_the_fly.as_ref() else {
            return;
        };

        if let Some(server) = ignore.server {
            self.server_ignoring_on_the_fly = server;
            self.client_ignoring_on_the_fly = 0;
        } else if let Some(client) = ignore.client
            && client == self.client_ignoring_on_the_fly
        {
            self.client_ignoring_on_the_fly = 0;
        }
    }

    pub(super) fn has_global_playstate(&self) -> bool {
        self.current_room_playstate()
            .is_some_and(|playstate| playstate.position.is_some() && playstate.paused.is_some())
    }

    pub(super) fn effective_local_paused_state(&self, now_seconds: f64) -> bool {
        self.local_paused
            .or_else(|| {
                self.current_room_playstate_at(now_seconds)
                    .and_then(|playstate| playstate.paused)
            })
            .unwrap_or(true)
    }

    pub(super) fn shared_playlist_runtime_commands_allowed_legacy_compatible(&self) -> bool {
        self.server_chat_supported.is_some()
            && self.username.is_some()
            && self.room.is_some()
            && self.server_shared_playlists_supported != Some(false)
    }

    pub(super) fn apply_local_ready_state_optimistically(&mut self, ready: bool) {
        let Some(username) = self.username.clone() else {
            return;
        };
        self.set_user_ready_state(&username, Some(ready));
    }

    pub(super) fn runtime_actions_for_local_pause_change(
        &mut self,
        paused: bool,
        now_seconds: f64,
    ) -> Vec<ClientRuntimeAction> {
        let effective_paused = self.effective_local_paused_state(now_seconds);
        if effective_paused == paused {
            return Vec::new();
        }

        if self.username.is_none() || self.server_readiness_supported != Some(true) {
            self.local_paused = Some(paused);
            return vec![ClientRuntimeAction::SetPaused(paused)];
        }

        let local_can_control = self.local_can_control().unwrap_or(false);
        let is_playing_music = self.is_playing_music();
        let recently_advanced = self.recently_advanced(now_seconds);
        let global_paused = self
            .current_room_playstate_at(now_seconds)
            .and_then(|playstate| playstate.paused)
            .unwrap_or(true);

        if !local_can_control {
            self.local_paused = Some(global_paused);
            let mut actions = Vec::new();
            if effective_paused != global_paused {
                actions.push(ClientRuntimeAction::SetPaused(global_paused));
            }
            if (!global_paused || recently_advanced)
                && !self.recently_rewound(now_seconds, RECENT_REWIND_READINESS_SUPPRESSION_SECONDS)
            {
                let ready = !self.local_user_ready();
                self.apply_local_ready_state_optimistically(ready);
                actions.push(ClientRuntimeAction::SetReady {
                    ready,
                    manually_initiated: true,
                });
            }
            return actions;
        }

        if is_playing_music && recently_advanced {
            self.local_paused = Some(paused);
            return vec![ClientRuntimeAction::SetPaused(paused)];
        }

        if paused {
            self.local_paused = Some(true);
            let mut actions = vec![ClientRuntimeAction::SetPaused(true)];
            if self.local_user_ready() {
                self.apply_local_ready_state_optimistically(false);
                actions.push(ClientRuntimeAction::SetReady {
                    ready: false,
                    manually_initiated: false,
                });
            }
            return actions;
        }

        let instaplay = self.instaplay_conditions_met(local_can_control, is_playing_music);
        if !instaplay {
            self.local_paused = Some(true);
            let mut actions = vec![ClientRuntimeAction::SetPaused(true)];
            if !self.local_user_ready() {
                self.apply_local_ready_state_optimistically(true);
                actions.push(ClientRuntimeAction::SetReady {
                    ready: true,
                    manually_initiated: true,
                });
            }
            return actions;
        }

        if let Some(last_paused_on_leave_at_seconds) = self.last_paused_on_leave_at_seconds
            && now_seconds - last_paused_on_leave_at_seconds
                < self
                    .readiness_autoplay_config
                    .last_paused_diff_threshold_seconds
        {
            self.last_paused_on_leave_at_seconds = None;
            self.local_paused = Some(false);
            return vec![ClientRuntimeAction::SetPaused(false)];
        }

        self.local_paused = Some(false);
        let mut actions = vec![ClientRuntimeAction::SetPaused(false)];
        if !self.local_user_ready() {
            self.apply_local_ready_state_optimistically(true);
            actions.push(ClientRuntimeAction::SetReady {
                ready: true,
                manually_initiated: false,
            });
        }
        actions
    }

    #[cfg(test)]
    pub(super) fn determine_local_state_change(
        &self,
        local_paused: bool,
        local_position: f64,
    ) -> (bool, bool) {
        self.determine_local_state_change_with_global_playstate_override(
            local_paused,
            local_position,
            None,
        )
    }

    pub(super) fn determine_local_state_change_with_global_playstate_override(
        &self,
        local_paused: bool,
        local_position: f64,
        global_playstate_override: Option<RoomPlaystateView>,
    ) -> (bool, bool) {
        let global_playstate = global_playstate_override.or_else(|| {
            self.current_room_playstate_at(unix_wall_clock_time_seconds_legacy_compatible())
        });
        let global_paused = global_playstate
            .as_ref()
            .and_then(|playstate| playstate.paused)
            .unwrap_or(true);
        let global_position = global_playstate
            .as_ref()
            .and_then(|playstate| playstate.position)
            .unwrap_or(0.0);
        let player_paused = self.local_paused.unwrap_or(global_paused);
        let player_position = self.local_position.unwrap_or(global_position);

        let pause_change = player_paused != local_paused && global_paused != local_paused;
        let seeked = (player_position - local_position).abs() > SEEK_THRESHOLD_SECONDS
            && (global_position - local_position).abs() > SEEK_THRESHOLD_SECONDS;
        (pause_change, seeked)
    }

    pub(super) fn local_username_and_room(&self) -> Option<(&str, &str)> {
        let local_username = self.username.as_deref()?;
        let local_room = self.room.as_deref()?;
        Some((local_username, local_room))
    }

    pub(super) fn current_room_has_other_users(&self) -> bool {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return false;
        };

        self.user_views.iter().any(|(username, user_view)| {
            username != local_username && user_view.room.as_deref() == Some(local_room)
        })
    }

    pub(super) fn room_playstate_has_remote_authority(
        &self,
        playstate: &RoomPlaystateView,
    ) -> bool {
        match playstate.set_by.as_deref() {
            Some(set_by) => self.username.as_deref() != Some(set_by),
            None => self.current_room_has_other_users(),
        }
    }

    pub(super) fn should_track_playlist_index_transition_for_room(
        &self,
        room_name: Option<&str>,
    ) -> bool {
        let tracked_room = self
            .pending_local_room_switch_target
            .as_deref()
            .or(self.room.as_deref());
        matches!((tracked_room, room_name), (Some(tracked_room), Some(room_name)) if tracked_room == room_name)
    }

    pub(super) fn update_local_room(&mut self, room_name: String) {
        if self.room.as_deref() != Some(room_name.as_str()) {
            self.reset_playlist_index_transition_tracking();
            self.pending_local_room_switch_target = None;
        } else if self.pending_local_room_switch_target.as_deref() == Some(room_name.as_str()) {
            self.pending_local_room_switch_target = None;
        }
        self.room = Some(room_name);
    }

    pub(super) fn is_controlled_room_name(room_name: &str) -> bool {
        if !room_name.starts_with('+') {
            return false;
        }
        let Some((_, hash)) = room_name.rsplit_once(':') else {
            return false;
        };
        hash.len() == 12 && hash.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    pub(super) fn normalize_runtime_controlled_room_input_legacy_compatible(
        room: String,
    ) -> (String, Option<String>) {
        let parts: Vec<_> = room.split(':').collect();
        if !room.starts_with('+') || parts.len() < 3 {
            return (room, None);
        }

        let canonical_room = format!("{}:{}", parts[0], parts[1]);
        let normalized_password = Self::normalize_control_password_legacy_compatible(parts[2]);
        (
            canonical_room,
            (!normalized_password.is_empty()).then_some(normalized_password),
        )
    }

    pub(super) fn normalize_control_password_legacy_compatible(password: &str) -> String {
        password
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_ascii_uppercase()
    }

    pub(super) fn local_user_ready(&self) -> bool {
        self.username
            .as_deref()
            .and_then(|username| self.user_views.get(username))
            .is_some_and(|user_view| user_view.ready == Some(true))
    }

    pub(super) fn user_ready_with_file(user_view: &ClientUserView) -> Option<bool> {
        if !user_view.has_file {
            return None;
        }
        user_view.ready
    }

    pub(super) fn legacy_json_value_truthy(value: &Value) -> bool {
        match value {
            Value::Null => false,
            Value::Bool(flag) => *flag,
            Value::Number(number) => {
                if let Some(signed) = number.as_i64() {
                    signed != 0
                } else if let Some(unsigned) = number.as_u64() {
                    unsigned != 0
                } else {
                    number.as_f64().is_some_and(|float| float != 0.0)
                }
            }
            Value::String(text) => !text.is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Object(entries) => !entries.is_empty(),
        }
    }

    pub(super) fn list_payload_has_file(file: Option<&Value>) -> bool {
        match file {
            Some(Value::Null) | None => false,
            Some(Value::Object(entries)) => !entries.is_empty(),
            Some(_) => true,
        }
    }

    pub(super) fn list_payload_file_info(
        file: Option<&Value>,
    ) -> (bool, Option<String>, Option<Value>, Option<Value>) {
        match file {
            Some(Value::Null) | None => (false, None, None, None),
            Some(Value::Object(entries)) if entries.is_empty() => (false, None, None, None),
            Some(value) => (
                Self::list_payload_has_file(Some(value)),
                Self::file_name_from_payload(value),
                Self::file_size_from_payload(value),
                Self::file_duration_from_payload(value),
            ),
        }
    }

    pub(super) fn file_name_from_payload(file: &Value) -> Option<String> {
        match file {
            Value::Object(entries) => entries
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            Value::String(name) => Some(name.to_owned()),
            _ => None,
        }
    }

    pub(super) fn file_size_from_payload(file: &Value) -> Option<Value> {
        match file {
            Value::Object(entries) => entries.get("size").cloned(),
            _ => None,
        }
    }

    pub(super) fn file_duration_from_payload(file: &Value) -> Option<Value> {
        match file {
            Value::Object(entries) => entries.get("duration").cloned(),
            _ => None,
        }
    }

    pub(super) fn file_metadata_from_payload(
        file: &Value,
    ) -> (Option<String>, Option<Value>, Option<Value>) {
        (
            Self::file_name_from_payload(file),
            Self::file_size_from_payload(file),
            Self::file_duration_from_payload(file),
        )
    }

    pub(super) fn file_difference_summary_for_users(
        current_user: &ClientUserView,
        other_user: &ClientUserView,
        session: &ClientSession,
    ) -> Option<FileDifferenceSummary> {
        if !current_user.has_file || !other_user.has_file {
            return None;
        }

        let filename = match (&current_user.file_name, &other_user.file_name) {
            (Some(current_name), Some(other_name)) => {
                !Self::same_filename_legacy_like(current_name, other_name)
            }
            (None, None) => false,
            _ => true,
        };

        let filesize = match (&current_user.file_size, &other_user.file_size) {
            (Some(current_size), Some(other_size)) => {
                !Self::same_filesize_legacy_like(current_size, other_size)
            }
            (None, None) => false,
            _ => true,
        };

        let fileduration = match (&current_user.file_duration, &other_user.file_duration) {
            (Some(current_duration), Some(other_duration)) => {
                match (current_duration.as_f64(), other_duration.as_f64()) {
                    (Some(current_duration), Some(other_duration)) => !session
                        .same_fileduration_with_readiness_autoplay_config(
                            current_duration,
                            other_duration,
                        ),
                    _ => true,
                }
            }
            (None, None) => false,
            _ => true,
        };

        Some(FileDifferenceSummary {
            filename,
            filesize,
            fileduration,
        })
    }

    pub fn current_user_file_name(&self) -> Option<&str> {
        self.username
            .as_deref()
            .and_then(|username| self.user_file_name(username))
    }

    pub(super) fn hex_value(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    pub(super) fn percent_decode_lossy(input: &str) -> String {
        let bytes = input.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;

        while index < bytes.len() {
            if bytes[index] == b'%'
                && index + 2 < bytes.len()
                && let (Some(high), Some(low)) = (
                    Self::hex_value(bytes[index + 1]),
                    Self::hex_value(bytes[index + 2]),
                )
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
            decoded.push(bytes[index]);
            index += 1;
        }

        String::from_utf8_lossy(&decoded).into_owned()
    }

    pub(super) fn strip_filename_for_compare(filename: &str, strip_url: bool) -> String {
        let decoded_filename = Self::percent_decode_lossy(filename);
        let normalized_name = if strip_url {
            let last_segment = decoded_filename
                .rsplit('/')
                .next()
                .unwrap_or(&decoded_filename);
            Self::percent_decode_lossy(last_segment)
        } else {
            decoded_filename
        };
        normalized_name
            .chars()
            .filter(|ch| {
                !matches!(
                    ch,
                    '-' | '~' | '_' | '.' | '[' | ']' | '(' | ')' | ':' | ' '
                )
            })
            .collect()
    }

    pub(super) fn same_hashed_legacy_like(
        left_raw: &str,
        left_hash: &str,
        right_raw: &str,
        right_hash: &str,
    ) -> bool {
        left_raw.to_lowercase() == right_raw.to_lowercase()
            || left_raw == right_raw
            || left_raw == right_hash
            || left_hash == right_raw
            || left_hash == right_hash
    }

    pub(super) fn is_url(filename: &str) -> bool {
        filename.contains("://")
    }

    pub(super) fn hash_filename_for_compare(filename: &str) -> String {
        format!("{:x}", Sha256::digest(filename.as_bytes()))[..12].to_owned()
    }

    pub(super) fn hash_filesize_for_compare(filesize_raw: &str) -> String {
        format!("{:x}", Sha256::digest(filesize_raw.as_bytes()))[..12].to_owned()
    }

    pub(super) fn filename_with_privacy_mode_legacy_like(
        file_name: &Value,
        privacy_mode: PrivacyMode,
    ) -> Option<String> {
        match privacy_mode {
            PrivacyMode::SendRaw => file_name.as_str().map(str::to_owned),
            PrivacyMode::SendHashed => {
                let raw_name = file_name.as_str()?;
                let strip_url = Self::is_url(raw_name);
                let stripped_name = Self::strip_filename_for_compare(raw_name, strip_url);
                Some(Self::hash_filename_for_compare(&stripped_name))
            }
            PrivacyMode::DoNotSend => Some(PRIVACY_HIDDEN_FILENAME.to_owned()),
        }
    }

    pub(super) fn filesize_raw_for_privacy(size: &Value) -> String {
        match size {
            Value::Number(number) => number.to_string(),
            Value::String(text) => text.clone(),
            Value::Bool(boolean) => boolean.to_string(),
            Value::Null => "None".to_owned(),
            Value::Array(_) | Value::Object(_) => size.to_string(),
        }
    }

    pub(super) fn filesize_with_privacy_mode_legacy_like(
        size: &Value,
        privacy_mode: PrivacyMode,
    ) -> Option<Value> {
        match privacy_mode {
            PrivacyMode::SendRaw => Some(size.clone()),
            PrivacyMode::SendHashed => {
                let raw_size = Self::filesize_raw_for_privacy(size);
                Some(Value::String(Self::hash_filesize_for_compare(&raw_size)))
            }
            PrivacyMode::DoNotSend => Some(Value::from(0)),
        }
    }

    pub(super) fn filesize_is_zero_legacy_like(filesize: &Value) -> bool {
        match filesize {
            Value::Number(number) => {
                if let Some(signed) = number.as_i64() {
                    signed == 0
                } else if let Some(unsigned) = number.as_u64() {
                    unsigned == 0
                } else {
                    number.as_f64().is_some_and(|float| float == 0.0)
                }
            }
            _ => false,
        }
    }

    pub(super) fn filesize_raw_for_compare(filesize: &Value) -> Option<String> {
        match filesize {
            Value::Number(number) => Some(number.to_string()),
            Value::String(text) => Some(text.clone()),
            _ => None,
        }
    }

    pub(super) fn same_filesize_legacy_like(left: &Value, right: &Value) -> bool {
        if Self::filesize_is_zero_legacy_like(left) || Self::filesize_is_zero_legacy_like(right) {
            return true;
        }

        let Some(left_raw) = Self::filesize_raw_for_compare(left) else {
            return false;
        };
        let Some(right_raw) = Self::filesize_raw_for_compare(right) else {
            return false;
        };

        let left_hash = Self::hash_filesize_for_compare(&left_raw);
        let right_hash = Self::hash_filesize_for_compare(&right_raw);
        Self::same_hashed_legacy_like(&left_raw, &left_hash, &right_raw, &right_hash)
    }

    pub(super) fn round_half_to_even(value: f64) -> f64 {
        let floor = value.floor();
        let fraction = value - floor;

        if fraction + ROUND_HALF_EPSILON < 0.5 {
            return floor;
        }
        if fraction - ROUND_HALF_EPSILON > 0.5 {
            return floor + 1.0;
        }

        if floor.rem_euclid(2.0) == 0.0 {
            floor
        } else {
            floor + 1.0
        }
    }

    pub(super) fn same_fileduration_legacy_like(
        left: f64,
        right: f64,
        show_duration_notification: bool,
        different_duration_threshold: f64,
    ) -> bool {
        if !show_duration_notification {
            return true;
        }

        (Self::round_half_to_even(left) - Self::round_half_to_even(right)).abs()
            < different_duration_threshold
    }

    pub(super) fn same_filename_legacy_like(left: &str, right: &str) -> bool {
        if left == PRIVACY_HIDDEN_FILENAME || right == PRIVACY_HIDDEN_FILENAME {
            return true;
        }
        let strip_url = Self::is_url(left) ^ Self::is_url(right);
        let left_stripped = Self::strip_filename_for_compare(left, strip_url);
        let right_stripped = Self::strip_filename_for_compare(right, strip_url);
        let left_hash = Self::hash_filename_for_compare(&left_stripped);
        let right_hash = Self::hash_filename_for_compare(&right_stripped);
        Self::same_hashed_legacy_like(&left_stripped, &left_hash, &right_stripped, &right_hash)
    }

    pub(super) fn all_users_in_current_room_ready(&self) -> bool {
        if !self.local_user_ready() {
            return false;
        }
        let require_same_filenames = self
            .readiness_autoplay_config
            .autoplay_require_same_filenames;
        self.all_other_users_in_current_room_ready()
            && (!require_same_filenames || self.all_users_in_current_room_match_filename())
    }

    pub(super) fn all_other_users_in_current_room_ready(&self) -> bool {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return false;
        };

        self.user_views.iter().all(|(username, user_view)| {
            if username == local_username {
                return true;
            }
            if user_view.room.as_deref() != Some(local_room) {
                return true;
            }
            Self::user_ready_with_file(user_view) != Some(false)
        })
    }

    pub(super) fn users_in_current_room_count_for_threshold(&self) -> usize {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return 0;
        };

        // Legacy usersInRoomCount adds the current user and only counts other room users
        // where isReadyWithFile() is truthy.
        let ready_others = self
            .user_views
            .iter()
            .filter(|(username, user_view)| {
                *username != local_username
                    && user_view.room.as_deref() == Some(local_room)
                    && Self::user_ready_with_file(user_view) == Some(true)
            })
            .count();
        1 + ready_others
    }

    pub(super) fn all_users_in_current_room_match_filename(&self) -> bool {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return false;
        };
        let Some(local_file_name) = self.current_user_file_name() else {
            return false;
        };

        self.user_views.iter().all(|(username, user_view)| {
            if username == local_username || user_view.room.as_deref() != Some(local_room) {
                return true;
            }
            user_view
                .file_name
                .as_deref()
                .is_some_and(|other_file_name| {
                    Self::same_filename_legacy_like(local_file_name, other_file_name)
                })
        })
    }

    pub(super) fn ready_user_count_in_current_room(&self) -> usize {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return 0;
        };

        let mut ready_count = usize::from(self.local_user_ready());
        ready_count += self
            .user_views
            .iter()
            .filter(|(username, user_view)| {
                *username != local_username
                    && user_view.room.as_deref() == Some(local_room)
                    && Self::user_ready_with_file(user_view) == Some(true)
            })
            .count();
        ready_count
    }

    pub(super) fn playlist_restore_intent_from_room_playlist(
        playlist: &RoomPlaylistView,
    ) -> Option<ReconnectPlaylistRestoreIntent> {
        if playlist.files.is_empty() {
            return None;
        }

        let index = playlist.index.filter(|index| {
            usize::try_from(*index).is_ok_and(|index| index < playlist.files.len())
        });

        Some(ReconnectPlaylistRestoreIntent {
            files: playlist.files.clone(),
            index,
        })
    }

    pub(super) fn file_payload_from_user_view(user_view: &ClientUserView) -> Option<Value> {
        if !user_view.has_file {
            return None;
        }

        let mut payload = Map::new();
        if let Some(file_name) = user_view.file_name.as_ref() {
            payload.insert("name".to_owned(), Value::String(file_name.clone()));
        }
        if let Some(file_size) = user_view.file_size.as_ref() {
            payload.insert("size".to_owned(), file_size.clone());
        }
        if let Some(file_duration) = user_view.file_duration.as_ref() {
            payload.insert("duration".to_owned(), file_duration.clone());
        }

        Some(Value::Object(payload))
    }

    pub(super) fn is_music_file_name(file_name: &str) -> bool {
        let lower_name = file_name.to_ascii_lowercase();
        MUSIC_FORMATS
            .iter()
            .any(|music_format| lower_name.ends_with(music_format))
    }

    pub(super) fn start_autoplay_countdown(&mut self) {
        if !self.autoplay_timer_running {
            self.autoplay_time_left_seconds = self.readiness_autoplay_config.autoplay_delay_seconds;
            self.autoplay_timer_running = true;
        }
    }

    pub(super) fn stop_autoplay_countdown(&mut self) {
        self.autoplay_timer_running = false;
        self.autoplay_time_left_seconds = self.readiness_autoplay_config.autoplay_delay_seconds;
    }

    pub(super) fn resolve_room_for_playlist_update(&self, set_by: Option<&str>) -> Option<String> {
        set_by
            .and_then(|username| self.user_room(username).map(str::to_owned))
            .or_else(|| self.room.clone())
    }

    pub(super) fn set_user_room(&mut self, username: &str, room_name: Option<String>) {
        let (previous_room, ready) = {
            let user_view = self.user_views.entry(username.to_owned()).or_default();
            let previous_room = user_view.room.clone();
            let ready = user_view.ready;
            user_view.room = room_name.clone();
            (previous_room, ready)
        };

        if previous_room != room_name
            && let Some(previous_room_name) = previous_room.as_deref()
        {
            let _ = self.domain.leave_room(username, previous_room_name);
        }

        if let Some(new_room_name) = room_name.as_deref() {
            self.known_rooms.insert(new_room_name.to_owned());
            self.domain.join_room(username, new_room_name);
            let _ = self
                .domain
                .set_ready(username, new_room_name, ready.unwrap_or(false));
        }
    }

    pub(super) fn set_user_ready(&mut self, username: &str, ready: bool) {
        self.set_user_ready_state(username, Some(ready));
    }

    pub(super) fn set_user_ready_state(&mut self, username: &str, ready: Option<bool>) {
        let room_name = {
            let user_view = self.user_views.entry(username.to_owned()).or_default();
            user_view.ready = ready;
            user_view.room.clone()
        };

        if let Some(room_name) = room_name {
            self.domain.join_room(username, &room_name);
            let _ = self
                .domain
                .set_ready(username, &room_name, ready.unwrap_or(false));
        }
    }

    pub(super) fn set_user_file_info(
        &mut self,
        username: &str,
        has_file: bool,
        file_name: Option<String>,
        file_size: Option<Value>,
        file_duration: Option<Value>,
    ) {
        let user_view = self.user_views.entry(username.to_owned()).or_default();
        user_view.has_file = has_file;
        user_view.file_name = if has_file { file_name } else { None };
        user_view.file_size = if has_file { file_size } else { None };
        user_view.file_duration = if has_file { file_duration } else { None };
    }

    pub(super) fn set_user_controller(&mut self, username: &str, controller: bool) {
        let user_view = self.user_views.entry(username.to_owned()).or_default();
        user_view.controller = controller;
    }

    pub(super) fn set_user_features(&mut self, username: &str, features: Option<Value>) {
        let user_view = self.user_views.entry(username.to_owned()).or_default();
        user_view.features = features;
    }

    pub(super) fn remove_user(&mut self, username: &str) {
        if let Some(user_view) = self.user_views.remove(username)
            && let Some(room_name) = user_view.room
        {
            let _ = self.domain.leave_room(username, &room_name);
        }
    }
}
