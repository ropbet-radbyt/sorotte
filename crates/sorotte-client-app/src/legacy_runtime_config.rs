use crate::legacy_settings::{AutoplayThresholdOverride, StoredClientSettingsMvp};
use crate::runtime_config::{ClientConfig, ClientConfigIssue, ServerPort};
use sorotte_client_core::{PrivacyMode, UnpauseActionMode};
use sorotte_secret::SecretValue;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredClientSettingsRuntimeSnapshot {
    pub settings: StoredClientSettingsMvp,
    pub config: ClientConfig,
    pub validation_issues: Vec<ClientConfigIssue>,
    pub controlled_room_password_override: Option<SecretValue>,
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
    pub server_password: Option<SecretValue>,
    pub username: Option<String>,
    pub room: Option<String>,
    pub controlled_room_password_override: Option<SecretValue>,
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

    if host_value.contains(':') {
        return (format!("[{host_value}]"), None);
    }

    (host_value.to_owned(), None)
}

fn normalize_controlled_room_password_legacy_compatible(password: &str) -> Option<String> {
    let normalized_password = password
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_ascii_uppercase();
    (!normalized_password.is_empty()).then_some(normalized_password)
}

fn canonical_controlled_room_name_legacy_compatible(
    base_name: &str,
    hash_suffix: &str,
) -> Option<String> {
    let base_name = base_name.trim();
    let hash_suffix = hash_suffix.trim();
    if base_name.is_empty()
        || hash_suffix.len() != 12
        || !hash_suffix.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }

    let base_name = if base_name.starts_with('+') {
        base_name.to_owned()
    } else {
        format!("+{base_name}")
    };
    Some(format!("{base_name}:{hash_suffix}"))
}

pub fn normalize_controlled_room_input_legacy_compatible(room: String) -> (String, Option<String>) {
    let mut parts = room.rsplitn(3, ':');
    let trailing = parts.next();
    let middle = parts.next();
    let leading = parts.next();
    if let (Some(password), Some(hash_suffix), Some(base_name)) = (trailing, middle, leading)
        && let Some(canonical_room) =
            canonical_controlled_room_name_legacy_compatible(base_name, hash_suffix)
    {
        return (
            canonical_room,
            normalize_controlled_room_password_legacy_compatible(password),
        );
    }

    let mut parts = room.rsplitn(2, ':');
    let hash_suffix = parts.next();
    let base_name = parts.next();
    if let (Some(hash_suffix), Some(base_name)) = (hash_suffix, base_name)
        && let Some(canonical_room) =
            canonical_controlled_room_name_legacy_compatible(base_name, hash_suffix)
    {
        return (canonical_room, None);
    }

    (room, None)
}

pub fn stored_client_settings_runtime_snapshot_legacy_compatible(
    settings: &StoredClientSettingsMvp,
) -> StoredClientSettingsRuntimeSnapshot {
    let config_resolution = ClientConfig::resolve(settings);
    let mut resolved = settings.clone();
    resolved.host = resolved
        .host
        .take()
        .map(|host| host.trim().to_owned())
        .filter(|host| !host.is_empty());
    resolved.server_password = resolved
        .server_password
        .take()
        .map(|password| password.into_exposed_secret())
        .map(|password| password.trim().to_owned())
        .filter(|password| !password.is_empty())
        .map(Into::into);
    resolved.username = resolved
        .username
        .take()
        .map(|username| username.trim().to_owned())
        .filter(|username| !username.is_empty());

    if (resolved.host.is_none() || resolved.port.is_none())
        && let Some(address) = config_resolution
            .config
            .connection
            .public_servers
            .first()
            .map(|server| server.address.as_str())
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
        config: config_resolution.config,
        validation_issues: config_resolution.issues,
        controlled_room_password_override: controlled_room_password_override.map(Into::into),
    }
}

pub fn stored_client_settings_config_plan_legacy_compatible(
    settings: &StoredClientSettingsMvp,
    env_presence: &StoredClientSettingsEnvPresence,
) -> StoredClientSettingsConfigPlan {
    let resolved = stored_client_settings_runtime_snapshot_legacy_compatible(settings);
    let resolved_settings = &resolved.settings;
    let config = &resolved.config;
    let mut plan = StoredClientSettingsConfigPlan::default();

    if !env_presence.host {
        plan.host = config.connection.host.clone();
    }
    let embedded_stored_port = settings
        .host
        .as_deref()
        .and_then(|host| parse_host_and_optional_port_from_host_arg_legacy_compatible(host).1);
    if !env_presence.port {
        plan.port = match settings.port {
            Some(port) => validated_stored_server_port(port),
            None => match embedded_stored_port {
                Some(port) => validated_stored_server_port(port),
                None => resolved_settings
                    .port
                    .and_then(validated_stored_server_port),
            },
        };
    }
    if !env_presence.server_password {
        plan.server_password = config.connection.server_password.clone();
    }
    if !env_presence.username {
        plan.username = config
            .connection
            .username
            .as_ref()
            .map(|username| username.as_str().to_owned());
    }
    if !env_presence.room {
        plan.room = config
            .connection
            .room
            .as_ref()
            .map(|room| room.as_str().to_owned());
        plan.controlled_room_password_override = config.connection.controlled_room_password.clone();
    }
    if !env_presence.autoplay && resolved_settings.autoplay_initial_state.is_some() {
        plan.autoplay_enabled = Some(config.readiness.autoplay_initial_state);
    }
    if !env_presence.autoplay_require_same_filenames
        && resolved_settings.autoplay_require_same_filenames.is_some()
    {
        plan.autoplay_require_same_filenames =
            Some(config.readiness.autoplay_require_same_filenames);
    }
    if !env_presence.ready_at_start && resolved_settings.ready_at_start.is_some() {
        plan.ready_at_start_override = Some(config.readiness.ready_at_start);
    }
    if !env_presence.shared_playlist_enabled && resolved_settings.shared_playlist_enabled.is_some()
    {
        plan.shared_playlists_enabled_override = Some(config.playback.shared_playlist_enabled);
    }
    if !env_presence.pause_on_leave && resolved_settings.pause_on_leave.is_some() {
        plan.pause_on_leave_override = Some(config.playback.pause_on_leave);
    }
    if !env_presence.loop_at_end_of_playlist && resolved_settings.loop_at_end_of_playlist.is_some()
    {
        plan.loop_at_end_of_playlist_override = Some(config.playback.loop_at_end_of_playlist);
    }
    if !env_presence.loop_single_files && resolved_settings.loop_single_files.is_some() {
        plan.loop_single_files_override = Some(config.playback.loop_single_files);
    }
    if !env_presence.only_switch_to_trusted_domains
        && resolved_settings.only_switch_to_trusted_domains.is_some()
    {
        plan.only_switch_to_trusted_domains_override =
            Some(config.playback.only_switch_to_trusted_domains);
    }
    if !env_presence.trusted_domains && resolved_settings.trusted_domains.is_some() {
        plan.trusted_domains_override = Some(config.playback.trusted_domains.clone());
    }
    if !env_presence.rewind_on_desync && resolved_settings.rewind_on_desync.is_some() {
        plan.rewind_on_desync_override = Some(config.synchronization.rewind_on_desync);
    }
    if !env_presence.fastforward_on_desync && resolved_settings.fastforward_on_desync.is_some() {
        plan.fastforward_on_desync_override = Some(config.synchronization.fastforward_on_desync);
    }
    if !env_presence.slow_on_desync && resolved_settings.slow_on_desync.is_some() {
        plan.slow_on_desync_override = Some(config.synchronization.slow_on_desync);
    }
    if !env_presence.dont_slow_down_with_me && resolved_settings.dont_slow_down_with_me.is_some() {
        plan.dont_slow_down_with_me_override = Some(config.synchronization.dont_slow_down_with_me);
    }
    if !env_presence.rewind_threshold_seconds
        && resolved_settings.rewind_threshold_seconds.is_some()
    {
        plan.rewind_threshold_seconds_override =
            Some(config.synchronization.rewind_threshold.get());
    }
    if !env_presence.fastforward_threshold_seconds
        && resolved_settings.fastforward_threshold_seconds.is_some()
    {
        plan.fastforward_threshold_seconds_override =
            Some(config.synchronization.fastforward_threshold.get());
    }
    if !env_presence.slowdown_threshold_seconds
        && resolved_settings.slowdown_threshold_seconds.is_some()
    {
        plan.slowdown_threshold_seconds_override =
            Some(config.synchronization.slowdown_threshold.get());
    }
    if !env_presence.unpause_action && resolved_settings.unpause_action.is_some() {
        plan.unpause_action_override = Some(config.readiness.unpause_action.clone());
    }
    if !env_presence.autoplay_min_users && resolved_settings.autoplay_min_users.is_some() {
        plan.auto_play_threshold_override = Some(config.readiness.autoplay_min_users.clone());
    }
    if !env_presence.filename_privacy_mode && resolved_settings.filename_privacy_mode.is_some() {
        plan.filename_privacy_mode = Some(config.playback.filename_privacy_mode);
    }
    if !env_presence.filesize_privacy_mode && resolved_settings.filesize_privacy_mode.is_some() {
        plan.filesize_privacy_mode = Some(config.playback.filesize_privacy_mode);
    }
    if !env_presence.show_duration_notification
        && resolved_settings.show_duration_notification.is_some()
    {
        plan.show_duration_notification_override =
            Some(config.readiness.show_duration_notification);
    }
    if !env_presence.show_same_room_osd && resolved_settings.show_same_room_osd.is_some() {
        plan.show_same_room_osd_override = Some(config.interface.show_same_room_osd);
    }
    if !env_presence.show_osd_warnings && resolved_settings.show_osd_warnings.is_some() {
        plan.show_osd_warnings_override = Some(config.interface.show_osd_warnings);
    }
    if !env_presence.show_noncontroller_osd && resolved_settings.show_noncontroller_osd.is_some() {
        plan.show_noncontroller_osd_override = Some(config.interface.show_noncontroller_osd);
    }
    if !env_presence.show_different_room_osd && resolved_settings.show_different_room_osd.is_some()
    {
        plan.show_different_room_osd_override = Some(config.interface.show_different_room_osd);
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

fn validated_stored_server_port(port: u16) -> Option<u16> {
    ServerPort::new(port).ok().map(ServerPort::get)
}

#[cfg(test)]
mod tests {
    use crate::legacy_settings::{AutoplayThresholdOverride, StoredClientSettingsMvp};
    use sorotte_client_core::{PrivacyMode, UnpauseActionMode};

    use super::{
        StoredClientSettingsConfigPlan, StoredClientSettingsEnvPresence,
        normalize_controlled_room_input_legacy_compatible,
        parse_host_and_optional_port_from_host_arg_legacy_compatible,
        stored_client_settings_config_plan_legacy_compatible,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    };

    fn stored_settings_with_every_runtime_override() -> StoredClientSettingsMvp {
        StoredClientSettingsMvp {
            host: Some("stored.example".to_owned()),
            port: Some(8123),
            server_password: Some("server-secret".into()),
            username: Some("stored-user".to_owned()),
            room: Some("+room:ABCDEF123456:AB-123".to_owned()),
            autoplay_initial_state: Some(true),
            autoplay_require_same_filenames: Some(false),
            ready_at_start: Some(true),
            shared_playlist_enabled: Some(false),
            pause_on_leave: Some(true),
            loop_at_end_of_playlist: Some(true),
            loop_single_files: Some(true),
            only_switch_to_trusted_domains: Some(false),
            trusted_domains: Some(vec![" example.org ".to_owned()]),
            rewind_on_desync: Some(false),
            fastforward_on_desync: Some(false),
            slow_on_desync: Some(false),
            dont_slow_down_with_me: Some(true),
            rewind_threshold_seconds: Some(1.25),
            fastforward_threshold_seconds: Some(4.5),
            slowdown_threshold_seconds: Some(0.75),
            unpause_action: Some(UnpauseActionMode::Always),
            autoplay_min_users: Some(AutoplayThresholdOverride::Set(7)),
            filename_privacy_mode: Some(PrivacyMode::SendHashed),
            filesize_privacy_mode: Some(PrivacyMode::DoNotSend),
            show_duration_notification: Some(false),
            show_same_room_osd: Some(false),
            show_osd_warnings: Some(false),
            show_noncontroller_osd: Some(false),
            show_different_room_osd: Some(false),
            ..StoredClientSettingsMvp::default()
        }
    }

    fn every_runtime_env_value_is_present() -> StoredClientSettingsEnvPresence {
        StoredClientSettingsEnvPresence {
            host: true,
            port: true,
            server_password: true,
            username: true,
            room: true,
            autoplay: true,
            autoplay_require_same_filenames: true,
            ready_at_start: true,
            shared_playlist_enabled: true,
            pause_on_leave: true,
            loop_at_end_of_playlist: true,
            loop_single_files: true,
            only_switch_to_trusted_domains: true,
            trusted_domains: true,
            rewind_on_desync: true,
            fastforward_on_desync: true,
            slow_on_desync: true,
            dont_slow_down_with_me: true,
            rewind_threshold_seconds: true,
            fastforward_threshold_seconds: true,
            slowdown_threshold_seconds: true,
            unpause_action: true,
            autoplay_min_users: true,
            filename_privacy_mode: true,
            filesize_privacy_mode: true,
            show_duration_notification: true,
            show_same_room_osd: true,
            show_osd_warnings: true,
            show_noncontroller_osd: true,
            show_different_room_osd: true,
        }
    }

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
        assert_eq!(
            normalize_controlled_room_input_legacy_compatible("room:ABCDEF123456".to_owned()),
            ("+room:ABCDEF123456".to_owned(), None)
        );
        assert_eq!(
            normalize_controlled_room_input_legacy_compatible(
                "room:ABCDEF123456:ab-123-456".to_owned()
            ),
            (
                "+room:ABCDEF123456".to_owned(),
                Some("AB-123-456".to_owned())
            )
        );
    }

    #[test]
    fn controlled_room_normalization_rejects_each_invalid_canonical_component() {
        for invalid in [":ABCDEF123456", "room:ABCDEF12345", "room:ABCDE!123456"] {
            assert_eq!(
                normalize_controlled_room_input_legacy_compatible(invalid.to_owned()),
                (invalid.to_owned(), None),
                "invalid controlled-room component was accepted: {invalid:?}"
            );
        }
        assert_eq!(
            normalize_controlled_room_input_legacy_compatible("room:ABCDEF123456:!_?".to_owned()),
            ("+room:ABCDEF123456".to_owned(), None)
        );
    }

    #[test]
    fn runtime_snapshot_discards_blank_optional_identity_values() {
        let snapshot =
            stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
                host: Some(" \t ".to_owned()),
                server_password: Some(" \r\n ".into()),
                username: Some(" \n ".to_owned()),
                ..StoredClientSettingsMvp::default()
            });

        assert_eq!(snapshot.settings.host, None);
        assert_eq!(snapshot.settings.server_password, None);
        assert_eq!(snapshot.settings.username, None);
    }

    #[test]
    fn legacy_runtime_config_debug_redacts_all_passwords() {
        const SERVER_MARKER: &str = "SERVER-SECRET-CANARY-91A2";
        const ROOM_MARKER: &str = "ROOM-SECRET-CANARY-73B4";
        let settings = StoredClientSettingsMvp {
            server_password: Some(SERVER_MARKER.into()),
            room: Some(format!("+room:ABCDEF123456:{ROOM_MARKER}")),
            ..StoredClientSettingsMvp::default()
        };
        let snapshot = stored_client_settings_runtime_snapshot_legacy_compatible(&settings);
        let plan = stored_client_settings_config_plan_legacy_compatible(
            &settings,
            &StoredClientSettingsEnvPresence::default(),
        );

        for debug in [format!("{snapshot:?}"), format!("{plan:?}")] {
            assert!(debug.contains("<redacted>"));
            assert!(!debug.contains(SERVER_MARKER));
            assert!(!debug.contains(ROOM_MARKER));
        }
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
            snapshot
                .controlled_room_password_override
                .as_ref()
                .map(|secret| secret.expose_secret()),
            Some("AB-123")
        );
    }

    #[test]
    fn legacy_runtime_snapshot_filters_zero_port_public_server_before_fallback() {
        let settings = StoredClientSettingsMvp {
            public_servers: Some(vec![
                ("Invalid".to_owned(), "invalid.example:0".to_owned()),
                ("Fallback".to_owned(), "fallback.example:8123".to_owned()),
            ]),
            ..StoredClientSettingsMvp::default()
        };

        let snapshot = stored_client_settings_runtime_snapshot_legacy_compatible(&settings);

        assert_eq!(snapshot.settings.host.as_deref(), Some("fallback.example"));
        assert_eq!(snapshot.settings.port, Some(8123));
        assert_eq!(snapshot.config.connection.public_servers.len(), 1);
        assert_eq!(
            snapshot
                .validation_issues
                .iter()
                .map(|issue| issue.field.as_str())
                .collect::<Vec<_>>(),
            vec!["public_servers[0].address"]
        );
    }

    #[test]
    fn legacy_config_plan_does_not_apply_invalid_embedded_or_explicit_zero_ports() {
        let embedded_settings = StoredClientSettingsMvp {
            host: Some("example.org:0".to_owned()),
            ..StoredClientSettingsMvp::default()
        };
        let embedded_snapshot =
            stored_client_settings_runtime_snapshot_legacy_compatible(&embedded_settings);
        let embedded_plan = stored_client_settings_config_plan_legacy_compatible(
            &embedded_settings,
            &StoredClientSettingsEnvPresence::default(),
        );
        assert_eq!(embedded_plan.host.as_deref(), Some("example.org"));
        assert_eq!(embedded_plan.port, None);
        assert_eq!(embedded_snapshot.validation_issues[0].field, "host");

        let explicit_settings = StoredClientSettingsMvp {
            host: Some("example.org:8123".to_owned()),
            port: Some(0),
            ..StoredClientSettingsMvp::default()
        };
        let explicit_snapshot =
            stored_client_settings_runtime_snapshot_legacy_compatible(&explicit_settings);
        let explicit_plan = stored_client_settings_config_plan_legacy_compatible(
            &explicit_settings,
            &StoredClientSettingsEnvPresence::default(),
        );
        assert_eq!(explicit_plan.port, None);
        assert_eq!(explicit_snapshot.validation_issues[0].field, "port");
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
    fn config_plan_applies_every_explicit_override_when_environment_is_absent() {
        let plan = stored_client_settings_config_plan_legacy_compatible(
            &stored_settings_with_every_runtime_override(),
            &StoredClientSettingsEnvPresence::default(),
        );

        assert_eq!(
            plan,
            StoredClientSettingsConfigPlan {
                host: Some("stored.example".to_owned()),
                port: Some(8123),
                server_password: Some("server-secret".into()),
                username: Some("stored-user".to_owned()),
                room: Some("+room:ABCDEF123456".to_owned()),
                controlled_room_password_override: Some("AB-123".into()),
                autoplay_enabled: Some(true),
                autoplay_require_same_filenames: Some(false),
                ready_at_start_override: Some(true),
                shared_playlists_enabled_override: Some(false),
                pause_on_leave_override: Some(true),
                loop_at_end_of_playlist_override: Some(true),
                loop_single_files_override: Some(true),
                only_switch_to_trusted_domains_override: Some(false),
                trusted_domains_override: Some(vec!["example.org".to_owned()]),
                rewind_on_desync_override: Some(false),
                fastforward_on_desync_override: Some(false),
                slow_on_desync_override: Some(false),
                dont_slow_down_with_me_override: Some(true),
                rewind_threshold_seconds_override: Some(1.25),
                fastforward_threshold_seconds_override: Some(4.5),
                slowdown_threshold_seconds_override: Some(0.75),
                unpause_action_override: Some(UnpauseActionMode::Always),
                auto_play_threshold_override: Some(AutoplayThresholdOverride::Set(7)),
                filename_privacy_mode: Some(PrivacyMode::SendHashed),
                filesize_privacy_mode: Some(PrivacyMode::DoNotSend),
                show_duration_notification_override: Some(false),
                show_same_room_osd_override: Some(false),
                show_osd_warnings_override: Some(false),
                show_noncontroller_osd_override: Some(false),
                show_different_room_osd_override: Some(false),
            }
        );
    }

    #[test]
    fn config_plan_suppresses_every_explicit_override_when_environment_is_present() {
        let plan = stored_client_settings_config_plan_legacy_compatible(
            &stored_settings_with_every_runtime_override(),
            &every_runtime_env_value_is_present(),
        );

        assert_eq!(plan, StoredClientSettingsConfigPlan::default());
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
            plan.controlled_room_password_override
                .as_ref()
                .map(|secret| secret.expose_secret()),
            Some("AB-123")
        );
    }

    #[test]
    fn stored_client_settings_config_plan_uses_public_server_port_with_explicit_host() {
        let plan = stored_client_settings_config_plan_legacy_compatible(
            &StoredClientSettingsMvp {
                host: Some("stored.example".to_owned()),
                public_servers: Some(vec![(
                    "Public".to_owned(),
                    "fallback.example:8123".to_owned(),
                )]),
                ..StoredClientSettingsMvp::default()
            },
            &StoredClientSettingsEnvPresence::default(),
        );

        assert_eq!(plan.host.as_deref(), Some("stored.example"));
        assert_eq!(plan.port, Some(8123));
    }
}
