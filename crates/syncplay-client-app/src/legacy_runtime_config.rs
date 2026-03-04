use crate::legacy_settings::{AutoplayThresholdOverride, StoredClientSettingsMvp};
use syncplay_client_core::{PrivacyMode, UnpauseActionMode};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredClientSettingsRuntimeSnapshot {
    pub settings: StoredClientSettingsMvp,
    pub controlled_room_password_override: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredClientSettingsEnvPresence {
    pub host: bool,
    pub port: bool,
    pub server_password: bool,
    pub username: bool,
    pub room: bool,
    pub autoplay: bool,
    pub autoplay_require_same_filenames: bool,
    pub ready_at_start: bool,
    pub shared_playlist_enabled: bool,
    pub pause_on_leave: bool,
    pub loop_at_end_of_playlist: bool,
    pub loop_single_files: bool,
    pub only_switch_to_trusted_domains: bool,
    pub trusted_domains: bool,
    pub rewind_on_desync: bool,
    pub fastforward_on_desync: bool,
    pub slow_on_desync: bool,
    pub dont_slow_down_with_me: bool,
    pub rewind_threshold_seconds: bool,
    pub fastforward_threshold_seconds: bool,
    pub slowdown_threshold_seconds: bool,
    pub unpause_action: bool,
    pub autoplay_min_users: bool,
    pub filename_privacy_mode: bool,
    pub filesize_privacy_mode: bool,
    pub show_duration_notification: bool,
    pub show_same_room_osd: bool,
    pub show_osd_warnings: bool,
    pub show_noncontroller_osd: bool,
    pub show_different_room_osd: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredClientSettingsConfigPlan {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub server_password: Option<String>,
    pub username: Option<String>,
    pub room: Option<String>,
    pub controlled_room_password_override: Option<String>,
    pub autoplay_enabled: Option<bool>,
    pub autoplay_require_same_filenames: Option<bool>,
    pub ready_at_start_override: Option<bool>,
    pub shared_playlists_enabled_override: Option<bool>,
    pub pause_on_leave_override: Option<bool>,
    pub loop_at_end_of_playlist_override: Option<bool>,
    pub loop_single_files_override: Option<bool>,
    pub only_switch_to_trusted_domains_override: Option<bool>,
    pub trusted_domains_override: Option<Vec<String>>,
    pub rewind_on_desync_override: Option<bool>,
    pub fastforward_on_desync_override: Option<bool>,
    pub slow_on_desync_override: Option<bool>,
    pub dont_slow_down_with_me_override: Option<bool>,
    pub rewind_threshold_seconds_override: Option<f64>,
    pub fastforward_threshold_seconds_override: Option<f64>,
    pub slowdown_threshold_seconds_override: Option<f64>,
    pub unpause_action_override: Option<UnpauseActionMode>,
    pub auto_play_threshold_override: Option<AutoplayThresholdOverride>,
    pub filename_privacy_mode: Option<PrivacyMode>,
    pub filesize_privacy_mode: Option<PrivacyMode>,
    pub show_duration_notification_override: Option<bool>,
    pub show_same_room_osd_override: Option<bool>,
    pub show_osd_warnings_override: Option<bool>,
    pub show_noncontroller_osd_override: Option<bool>,
    pub show_different_room_osd_override: Option<bool>,
}

pub fn parse_host_and_optional_port_from_host_arg_legacy_compatible(
    host_value: &str,
) -> (String, Option<u16>) {
    if host_value.matches(':').count() == 1 {
        let mut pieces = host_value.rsplitn(2, ':');
        let maybe_port = pieces.next().unwrap_or_default();
        let maybe_host = pieces.next().unwrap_or_default();
        if let Ok(port) = maybe_port.parse::<u16>() {
            return (maybe_host.to_owned(), Some(port));
        }
        return (maybe_host.to_owned(), None);
    }

    if host_value.starts_with('[')
        && let Some(end_bracket) = host_value.find(']')
    {
        let host = &host_value[..=end_bracket];
        if let Some(port_text) = host_value
            .get(end_bracket + 1..)
            .and_then(|suffix| suffix.strip_prefix(':'))
            && let Ok(port) = port_text.parse::<u16>()
        {
            return (host.to_owned(), Some(port));
        }
        return (host.to_owned(), None);
    }

    if host_value.matches(':').count() > 1 {
        return (format!("[{host_value}]"), None);
    }

    (host_value.to_owned(), None)
}

pub fn normalize_controlled_room_input_legacy_compatible(room: String) -> (String, Option<String>) {
    if !room.starts_with('+') {
        return (room, None);
    }

    let mut parts = room.split(':');
    let Some(base_name) = parts.next() else {
        return (room, None);
    };
    let Some(hash_suffix) = parts.next() else {
        return (room, None);
    };
    let Some(password) = parts.next() else {
        return (room, None);
    };

    let normalized_password = password
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_ascii_uppercase();
    let canonical_room = format!("{base_name}:{hash_suffix}");
    if normalized_password.is_empty() {
        return (canonical_room, None);
    }
    (canonical_room, Some(normalized_password))
}

pub fn stored_client_settings_runtime_snapshot_legacy_compatible(
    settings: &StoredClientSettingsMvp,
) -> StoredClientSettingsRuntimeSnapshot {
    let mut resolved = settings.clone();
    resolved.host = resolved
        .host
        .take()
        .map(|host| host.trim().to_owned())
        .filter(|host| !host.is_empty());
    resolved.server_password = resolved
        .server_password
        .take()
        .map(|password| password.trim().to_owned())
        .filter(|password| !password.is_empty());
    resolved.username = resolved
        .username
        .take()
        .map(|username| username.trim().to_owned())
        .filter(|username| !username.is_empty());

    if (resolved.host.is_none() || resolved.port.is_none())
        && let Some(address) = first_stored_public_server_address_legacy_compatible(settings)
    {
        let (fallback_host, fallback_port) =
            parse_host_and_optional_port_from_host_arg_legacy_compatible(address);
        if resolved.host.is_none() {
            let fallback_host = fallback_host.trim();
            if !fallback_host.is_empty() {
                resolved.host = Some(fallback_host.to_owned());
            }
        }
        if resolved.port.is_none() {
            resolved.port = fallback_port;
        }
    }

    let (resolved_room, controlled_room_password_override) = if let Some(room) = settings
        .room
        .as_deref()
        .map(str::trim)
        .filter(|room| !room.is_empty())
    {
        let (room, password) = normalize_controlled_room_input_legacy_compatible(room.to_owned());
        (Some(room), password)
    } else if let Some(room) = first_stored_room_list_entry_legacy_compatible(settings) {
        let (room, password) = normalize_controlled_room_input_legacy_compatible(room.to_owned());
        (Some(room), password)
    } else {
        (None, None)
    };
    resolved.room = resolved_room;

    StoredClientSettingsRuntimeSnapshot {
        settings: resolved,
        controlled_room_password_override,
    }
}

pub fn stored_client_settings_config_plan_legacy_compatible(
    settings: &StoredClientSettingsMvp,
    env_presence: &StoredClientSettingsEnvPresence,
) -> StoredClientSettingsConfigPlan {
    let resolved = stored_client_settings_runtime_snapshot_legacy_compatible(settings);
    let resolved_settings = &resolved.settings;
    let mut plan = StoredClientSettingsConfigPlan::default();

    if !env_presence.host {
        plan.host = resolved_settings.host.clone();
    }
    if !env_presence.port {
        plan.port = resolved_settings.port;
    }
    if !env_presence.server_password {
        plan.server_password = resolved_settings.server_password.clone();
    }
    if !env_presence.username {
        plan.username = resolved_settings.username.clone();
    }
    if !env_presence.room {
        plan.room = resolved_settings.room.clone();
        plan.controlled_room_password_override = resolved.controlled_room_password_override;
    }
    if !env_presence.autoplay {
        plan.autoplay_enabled = resolved_settings.autoplay_initial_state;
    }
    if !env_presence.autoplay_require_same_filenames {
        plan.autoplay_require_same_filenames = resolved_settings.autoplay_require_same_filenames;
    }
    if !env_presence.ready_at_start {
        plan.ready_at_start_override = resolved_settings.ready_at_start;
    }
    if !env_presence.shared_playlist_enabled {
        plan.shared_playlists_enabled_override = resolved_settings.shared_playlist_enabled;
    }
    if !env_presence.pause_on_leave {
        plan.pause_on_leave_override = resolved_settings.pause_on_leave;
    }
    if !env_presence.loop_at_end_of_playlist {
        plan.loop_at_end_of_playlist_override = resolved_settings.loop_at_end_of_playlist;
    }
    if !env_presence.loop_single_files {
        plan.loop_single_files_override = resolved_settings.loop_single_files;
    }
    if !env_presence.only_switch_to_trusted_domains {
        plan.only_switch_to_trusted_domains_override =
            resolved_settings.only_switch_to_trusted_domains;
    }
    if !env_presence.trusted_domains {
        plan.trusted_domains_override = resolved_settings.trusted_domains.clone();
    }
    if !env_presence.rewind_on_desync {
        plan.rewind_on_desync_override = resolved_settings.rewind_on_desync;
    }
    if !env_presence.fastforward_on_desync {
        plan.fastforward_on_desync_override = resolved_settings.fastforward_on_desync;
    }
    if !env_presence.slow_on_desync {
        plan.slow_on_desync_override = resolved_settings.slow_on_desync;
    }
    if !env_presence.dont_slow_down_with_me {
        plan.dont_slow_down_with_me_override = resolved_settings.dont_slow_down_with_me;
    }
    if !env_presence.rewind_threshold_seconds {
        plan.rewind_threshold_seconds_override = resolved_settings.rewind_threshold_seconds;
    }
    if !env_presence.fastforward_threshold_seconds {
        plan.fastforward_threshold_seconds_override =
            resolved_settings.fastforward_threshold_seconds;
    }
    if !env_presence.slowdown_threshold_seconds {
        plan.slowdown_threshold_seconds_override = resolved_settings.slowdown_threshold_seconds;
    }
    if !env_presence.unpause_action {
        plan.unpause_action_override = resolved_settings.unpause_action.clone();
    }
    if !env_presence.autoplay_min_users {
        plan.auto_play_threshold_override = resolved_settings.autoplay_min_users.clone();
    }
    if !env_presence.filename_privacy_mode {
        plan.filename_privacy_mode = resolved_settings.filename_privacy_mode;
    }
    if !env_presence.filesize_privacy_mode {
        plan.filesize_privacy_mode = resolved_settings.filesize_privacy_mode;
    }
    if !env_presence.show_duration_notification {
        plan.show_duration_notification_override = resolved_settings.show_duration_notification;
    }
    if !env_presence.show_same_room_osd {
        plan.show_same_room_osd_override = resolved_settings.show_same_room_osd;
    }
    if !env_presence.show_osd_warnings {
        plan.show_osd_warnings_override = resolved_settings.show_osd_warnings;
    }
    if !env_presence.show_noncontroller_osd {
        plan.show_noncontroller_osd_override = resolved_settings.show_noncontroller_osd;
    }
    if !env_presence.show_different_room_osd {
        plan.show_different_room_osd_override = resolved_settings.show_different_room_osd;
    }

    plan
}

fn first_stored_room_list_entry_legacy_compatible(
    settings: &StoredClientSettingsMvp,
) -> Option<&str> {
    settings
        .room_list
        .as_ref()?
        .iter()
        .map(String::as_str)
        .find(|room| !room.trim().is_empty())
}

fn first_stored_public_server_address_legacy_compatible(
    settings: &StoredClientSettingsMvp,
) -> Option<&str> {
    settings
        .public_servers
        .as_ref()?
        .iter()
        .map(|(_, address)| address.as_str())
        .find(|address| !address.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use crate::legacy_settings::{AutoplayThresholdOverride, StoredClientSettingsMvp};
    use syncplay_client_core::{PrivacyMode, UnpauseActionMode};

    use super::{
        StoredClientSettingsEnvPresence, normalize_controlled_room_input_legacy_compatible,
        parse_host_and_optional_port_from_host_arg_legacy_compatible,
        stored_client_settings_config_plan_legacy_compatible,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    };

    #[test]
    fn parse_host_and_optional_port_from_host_arg_legacy_compatible_parses_expected_shapes() {
        assert_eq!(
            parse_host_and_optional_port_from_host_arg_legacy_compatible("example.org:8999"),
            ("example.org".to_owned(), Some(8999))
        );
        assert_eq!(
            parse_host_and_optional_port_from_host_arg_legacy_compatible("example.org:notaport"),
            ("example.org".to_owned(), None)
        );
        assert_eq!(
            parse_host_and_optional_port_from_host_arg_legacy_compatible("[2001:db8::1]:8999"),
            ("[2001:db8::1]".to_owned(), Some(8999))
        );
        assert_eq!(
            parse_host_and_optional_port_from_host_arg_legacy_compatible("2001:db8::1"),
            ("[2001:db8::1]".to_owned(), None)
        );
    }

    #[test]
    fn normalize_controlled_room_input_legacy_compatible_extracts_canonical_room_and_password() {
        assert_eq!(
            normalize_controlled_room_input_legacy_compatible(
                "+room:ABCDEF123456:ab-123-456".to_owned()
            ),
            (
                "+room:ABCDEF123456".to_owned(),
                Some("AB-123-456".to_owned())
            )
        );
        assert_eq!(
            normalize_controlled_room_input_legacy_compatible("room1".to_owned()),
            ("room1".to_owned(), None)
        );
    }

    #[test]
    fn stored_client_settings_runtime_snapshot_legacy_compatible_uses_room_list_and_public_server_fallbacks()
     {
        let snapshot =
            stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
                room_list: Some(vec![" ".to_owned(), "+room:ABCDEF123456:ab-123".to_owned()]),
                public_servers: Some(vec![("Public".to_owned(), "example.org:8999".to_owned())]),
                ..StoredClientSettingsMvp::default()
            });

        assert_eq!(snapshot.settings.host.as_deref(), Some("example.org"));
        assert_eq!(snapshot.settings.port, Some(8999));
        assert_eq!(
            snapshot.settings.room.as_deref(),
            Some("+room:ABCDEF123456")
        );
        assert_eq!(
            snapshot.controlled_room_password_override.as_deref(),
            Some("AB-123")
        );
    }

    #[test]
    fn stored_client_settings_runtime_snapshot_legacy_compatible_prefers_explicit_host_and_room() {
        let snapshot =
            stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
                host: Some("syncplay.example".to_owned()),
                port: Some(8995),
                room: Some("room-a".to_owned()),
                room_list: Some(vec!["room-b".to_owned()]),
                public_servers: Some(vec![(
                    "Public".to_owned(),
                    "fallback.example:8999".to_owned(),
                )]),
                ..StoredClientSettingsMvp::default()
            });

        assert_eq!(snapshot.settings.host.as_deref(), Some("syncplay.example"));
        assert_eq!(snapshot.settings.port, Some(8995));
        assert_eq!(snapshot.settings.room.as_deref(), Some("room-a"));
        assert_eq!(snapshot.controlled_room_password_override, None);
    }

    #[test]
    fn stored_client_settings_config_plan_legacy_compatible_applies_only_values_not_shadowed_by_env()
     {
        let plan = stored_client_settings_config_plan_legacy_compatible(
            &StoredClientSettingsMvp {
                host: Some("stored.example".to_owned()),
                port: Some(8999),
                username: Some("stored-user".to_owned()),
                autoplay_initial_state: Some(true),
                rewind_threshold_seconds: Some(1.25),
                unpause_action: Some(UnpauseActionMode::IfOthersReady),
                autoplay_min_users: Some(AutoplayThresholdOverride::Set(3)),
                filename_privacy_mode: Some(PrivacyMode::SendHashed),
                show_osd_warnings: Some(false),
                ..StoredClientSettingsMvp::default()
            },
            &StoredClientSettingsEnvPresence {
                host: true,
                username: true,
                rewind_threshold_seconds: true,
                ..StoredClientSettingsEnvPresence::default()
            },
        );

        assert_eq!(plan.host, None);
        assert_eq!(plan.port, Some(8999));
        assert_eq!(plan.username, None);
        assert_eq!(plan.autoplay_enabled, Some(true));
        assert_eq!(plan.rewind_threshold_seconds_override, None);
        assert_eq!(
            plan.unpause_action_override,
            Some(UnpauseActionMode::IfOthersReady)
        );
        assert_eq!(
            plan.auto_play_threshold_override,
            Some(AutoplayThresholdOverride::Set(3))
        );
        assert_eq!(plan.filename_privacy_mode, Some(PrivacyMode::SendHashed));
        assert_eq!(plan.show_osd_warnings_override, Some(false));
    }

    #[test]
    fn stored_client_settings_config_plan_legacy_compatible_carries_runtime_fallbacks_forward() {
        let plan = stored_client_settings_config_plan_legacy_compatible(
            &StoredClientSettingsMvp {
                room_list: Some(vec![" ".to_owned(), "+room:ABCDEF123456:AB-123".to_owned()]),
                public_servers: Some(vec![("Public".to_owned(), "example.org:8999".to_owned())]),
                ..StoredClientSettingsMvp::default()
            },
            &StoredClientSettingsEnvPresence::default(),
        );

        assert_eq!(plan.host.as_deref(), Some("example.org"));
        assert_eq!(plan.port, Some(8999));
        assert_eq!(plan.room.as_deref(), Some("+room:ABCDEF123456"));
        assert_eq!(
            plan.controlled_room_password_override.as_deref(),
            Some("AB-123")
        );
    }
}
