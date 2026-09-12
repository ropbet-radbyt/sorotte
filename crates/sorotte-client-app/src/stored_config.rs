use crate::runtime_config::{ClientConfig, ClientConfigIssue};
use crate::stored_settings::StoredClientSettings;
use sorotte_secret::SecretValue;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredClientSettingsRuntimeSnapshot {
    pub settings: StoredClientSettings,
    pub config: ClientConfig,
    pub validation_issues: Vec<ClientConfigIssue>,
    pub controlled_room_password_override: Option<SecretValue>,
}

pub fn parse_host_and_optional_port_from_host_arg(host_value: &str) -> (String, Option<u16>) {
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

fn normalize_controlled_room_password(password: &str) -> Option<String> {
    let normalized_password = password
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_ascii_uppercase();
    (!normalized_password.is_empty()).then_some(normalized_password)
}

fn canonical_controlled_room_name(base_name: &str, hash_suffix: &str) -> Option<String> {
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

pub fn normalize_controlled_room_input(room: String) -> (String, Option<String>) {
    let mut parts = room.rsplitn(3, ':');
    let trailing = parts.next();
    let middle = parts.next();
    let leading = parts.next();
    if let (Some(password), Some(hash_suffix), Some(base_name)) = (trailing, middle, leading)
        && let Some(canonical_room) = canonical_controlled_room_name(base_name, hash_suffix)
    {
        return (canonical_room, normalize_controlled_room_password(password));
    }

    let mut parts = room.rsplitn(2, ':');
    let hash_suffix = parts.next();
    let base_name = parts.next();
    if let (Some(hash_suffix), Some(base_name)) = (hash_suffix, base_name)
        && let Some(canonical_room) = canonical_controlled_room_name(base_name, hash_suffix)
    {
        return (canonical_room, None);
    }

    (room, None)
}

pub fn stored_client_settings_runtime_snapshot(
    settings: &StoredClientSettings,
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
        let (fallback_host, fallback_port) = parse_host_and_optional_port_from_host_arg(address);
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
        let (room, password) = normalize_controlled_room_input(room.to_owned());
        (Some(room), password)
    } else if let Some(room) = first_stored_room_list_entry(settings) {
        let (room, password) = normalize_controlled_room_input(room.to_owned());
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

fn first_stored_room_list_entry(settings: &StoredClientSettings) -> Option<&str> {
    settings
        .room_list
        .as_ref()?
        .iter()
        .map(String::as_str)
        .find(|room| !room.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use crate::stored_settings::StoredClientSettings;

    use super::{
        normalize_controlled_room_input, parse_host_and_optional_port_from_host_arg,
        stored_client_settings_runtime_snapshot,
    };

    #[test]
    fn parse_host_and_optional_port_from_host_arg_parses_expected_shapes() {
        assert_eq!(
            parse_host_and_optional_port_from_host_arg("example.org:8999"),
            ("example.org".to_owned(), Some(8999))
        );
        assert_eq!(
            parse_host_and_optional_port_from_host_arg("example.org:notaport"),
            ("example.org".to_owned(), None)
        );
        assert_eq!(
            parse_host_and_optional_port_from_host_arg("[2001:db8::1]:8999"),
            ("[2001:db8::1]".to_owned(), Some(8999))
        );
        assert_eq!(
            parse_host_and_optional_port_from_host_arg("2001:db8::1"),
            ("[2001:db8::1]".to_owned(), None)
        );
    }

    #[test]
    fn normalize_controlled_room_input_extracts_canonical_room_and_password() {
        assert_eq!(
            normalize_controlled_room_input("+room:ABCDEF123456:ab-123-456".to_owned()),
            (
                "+room:ABCDEF123456".to_owned(),
                Some("AB-123-456".to_owned())
            )
        );
        assert_eq!(
            normalize_controlled_room_input("room1".to_owned()),
            ("room1".to_owned(), None)
        );
        assert_eq!(
            normalize_controlled_room_input("room:ABCDEF123456".to_owned()),
            ("+room:ABCDEF123456".to_owned(), None)
        );
        assert_eq!(
            normalize_controlled_room_input("room:ABCDEF123456:ab-123-456".to_owned()),
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
                normalize_controlled_room_input(invalid.to_owned()),
                (invalid.to_owned(), None),
                "invalid controlled-room component was accepted: {invalid:?}"
            );
        }
        assert_eq!(
            normalize_controlled_room_input("room:ABCDEF123456:!_?".to_owned()),
            ("+room:ABCDEF123456".to_owned(), None)
        );
    }

    #[test]
    fn runtime_snapshot_discards_blank_optional_identity_values() {
        let snapshot = stored_client_settings_runtime_snapshot(&StoredClientSettings {
            host: Some(" \t ".to_owned()),
            server_password: Some(" \r\n ".into()),
            username: Some(" \n ".to_owned()),
            ..StoredClientSettings::default()
        });

        assert_eq!(snapshot.settings.host, None);
        assert_eq!(snapshot.settings.server_password, None);
        assert_eq!(snapshot.settings.username, None);
    }

    #[test]
    fn stored_runtime_config_debug_redacts_all_passwords() {
        const SERVER_MARKER: &str = "SERVER-SECRET-CANARY-91A2";
        const ROOM_MARKER: &str = "ROOM-SECRET-CANARY-73B4";
        let settings = StoredClientSettings {
            server_password: Some(SERVER_MARKER.into()),
            room: Some(format!("+room:ABCDEF123456:{ROOM_MARKER}")),
            ..StoredClientSettings::default()
        };
        let snapshot = stored_client_settings_runtime_snapshot(&settings);

        let debug = format!("{snapshot:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(SERVER_MARKER));
        assert!(!debug.contains(ROOM_MARKER));
    }

    #[test]
    fn stored_client_settings_runtime_snapshot_uses_room_list_and_public_server_fallbacks() {
        let snapshot = stored_client_settings_runtime_snapshot(&StoredClientSettings {
            room_list: Some(vec![" ".to_owned(), "+room:ABCDEF123456:ab-123".to_owned()]),
            public_servers: Some(vec![("Public".to_owned(), "example.org:8999".to_owned())]),
            ..StoredClientSettings::default()
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
    fn runtime_snapshot_fills_each_missing_endpoint_part_independently() {
        for (host, port, expected_host, expected_port) in [
            (Some("explicit.example"), None, "explicit.example", 8123),
            (None, Some(8995), "fallback.example", 8995),
        ] {
            let snapshot = stored_client_settings_runtime_snapshot(&StoredClientSettings {
                host: host.map(str::to_owned),
                port,
                public_servers: Some(vec![(
                    "Public".to_owned(),
                    "fallback.example:8123".to_owned(),
                )]),
                ..StoredClientSettings::default()
            });

            assert_eq!(snapshot.settings.host.as_deref(), Some(expected_host));
            assert_eq!(snapshot.settings.port, Some(expected_port));
        }
    }

    #[test]
    fn stored_runtime_snapshot_filters_zero_port_public_server_before_fallback() {
        let settings = StoredClientSettings {
            public_servers: Some(vec![
                ("Invalid".to_owned(), "invalid.example:0".to_owned()),
                ("Fallback".to_owned(), "fallback.example:8123".to_owned()),
            ]),
            ..StoredClientSettings::default()
        };

        let snapshot = stored_client_settings_runtime_snapshot(&settings);

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
    fn stored_client_settings_runtime_snapshot_prefers_explicit_host_and_room() {
        let snapshot = stored_client_settings_runtime_snapshot(&StoredClientSettings {
            host: Some("syncplay.example".to_owned()),
            port: Some(8995),
            room: Some("room-a".to_owned()),
            room_list: Some(vec!["room-b".to_owned()]),
            public_servers: Some(vec![(
                "Public".to_owned(),
                "fallback.example:8999".to_owned(),
            )]),
            ..StoredClientSettings::default()
        });

        assert_eq!(snapshot.settings.host.as_deref(), Some("syncplay.example"));
        assert_eq!(snapshot.settings.port, Some(8995));
        assert_eq!(snapshot.settings.room.as_deref(), Some("room-a"));
        assert_eq!(snapshot.controlled_room_password_override, None);
    }
}
