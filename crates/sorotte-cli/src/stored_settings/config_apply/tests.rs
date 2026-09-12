use super::*;
use crate::tests::test_client_loop_config;

pub(super) fn configured(
    settings: &StoredClientSettings,
    environment: &[&str],
) -> ClientLoopConfig {
    let mut config = test_client_loop_config();
    apply_stored_client_settings(&mut config, settings, |name| {
        environment.contains(&name).then(|| "1".to_owned())
    });
    config
}

#[test]
fn stored_ports_preserve_explicit_embedded_and_public_server_precedence() {
    for (host, port, expected) in [
        ("example.org", None, 8123),
        ("example.org:8456", None, 8456),
        ("example.org:8456", Some(8789), 8789),
        ("example.org:0", None, 8999),
        ("example.org:8456", Some(0), 8999),
    ] {
        let settings = StoredClientSettings {
            host: Some(host.to_owned()),
            port,
            public_servers: Some(vec![
                ("Invalid".to_owned(), "invalid.example:0".to_owned()),
                ("Public".to_owned(), "public.example:8123".to_owned()),
            ]),
            ..StoredClientSettings::default()
        };
        let config = configured(&settings, &[]);
        assert_eq!(config.host, "example.org");
        assert_eq!(config.port, expected, "host={host}, port={port:?}");
        assert_eq!(configured(&settings, &["SOROTTE_CLIENT_PORT"]).port, 8999);
    }
}

#[test]
fn public_server_and_room_history_fallbacks_reach_the_cli() {
    let config = configured(
        &StoredClientSettings {
            room_list: Some(vec![" ".to_owned(), "+room:ABCDEF123456:ab-123".to_owned()]),
            public_servers: Some(vec![(
                "Public".to_owned(),
                "public.example:8123".to_owned(),
            )]),
            ..StoredClientSettings::default()
        },
        &[],
    );
    assert_eq!(config.host, "public.example");
    assert_eq!(config.port, 8123);
    assert_eq!(config.room, "+room:ABCDEF123456");
    assert_eq!(
        config
            .controlled_room_password_override
            .unwrap()
            .expose_secret(),
        "AB-123"
    );
}

#[test]
fn malformed_environment_values_shadow_saved_settings_except_the_port() {
    let settings = StoredClientSettings {
        port: Some(8123),
        autoplay_initial_state: Some(true),
        rewind_threshold_seconds: Some(1.25),
        ..StoredClientSettings::default()
    };
    for invalid in ["invalid", "0", "65536"] {
        let mut config = test_client_loop_config();
        apply_stored_client_settings(&mut config, &settings, |_| Some(invalid.to_owned()));
        assert_eq!(config.port, 8123);
        assert!(!config.autoplay_enabled);
        assert_eq!(config.rewind_threshold_seconds_override, None);
    }
}

#[test]
fn either_username_environment_spelling_preserves_the_cli_identity() {
    let settings = StoredClientSettings {
        username: Some("saved-user".to_owned()),
        ..StoredClientSettings::default()
    };
    assert_eq!(configured(&settings, &[]).username, "saved-user");
    for key in ["SOROTTE_CLIENT_USERNAME", "SOROTTE_CLIENT_NAME"] {
        assert_eq!(configured(&settings, &[key]).username, "cli-user");
    }
}

#[test]
fn applying_a_saved_room_preserves_an_explicit_controller_password() {
    let mut config = test_client_loop_config();
    config.controlled_room_password_override = Some("CLI-CREDENTIAL".into());
    apply_stored_client_settings(
        &mut config,
        &StoredClientSettings {
            room: Some("+room:ABCDEF123456:saved-credential".to_owned()),
            ..StoredClientSettings::default()
        },
        |_| None,
    );
    assert_eq!(config.room, "+room:ABCDEF123456");
    assert_eq!(
        config
            .controlled_room_password_override
            .unwrap()
            .expose_secret(),
        "CLI-CREDENTIAL"
    );
}
