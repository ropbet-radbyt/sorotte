//! Deterministic black-box properties for controlled-room configuration.
//!
//! The oracle below is independent of the production implementation. The
//! properties exercise normalization, command presentation, INI persistence,
//! runtime resolution and application to the actual CLI startup configuration.

use proptest::{
    prelude::*,
    test_runner::{Config as ProptestConfig, RngSeed},
};
use sorotte_client_app::app_boundary::{
    commands::controlled_room_base_name,
    persistence::{
        parse_sorotte_ini_stored_client_settings, upsert_sorotte_ini_stored_client_settings,
    },
    state::{
        StoredClientSettings, TlsPolicy, normalize_controlled_room_input,
        stored_client_settings_runtime_snapshot,
    },
};

use super::configuration_composition_tests::config_values;
use super::tests::configured;

const DEFAULT_CASES: u32 = 512;
const MAX_CASES: u32 = 100_000;
const PROPERTY_SEED: u64 = 0xC0F1_700D_2026_0730;

fn configured_proptest() -> ProptestConfig {
    let cases = match std::env::var_os("PROPTEST_CASES") {
        None => DEFAULT_CASES,
        Some(raw) => raw
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value > 0)
            .map(|value| value.min(MAX_CASES))
            .unwrap_or_else(|| panic!("PROPTEST_CASES must be an integer from 1 to {MAX_CASES}")),
    };
    ProptestConfig {
        cases,
        max_shrink_iters: 20_000,
        rng_seed: RngSeed::Fixed(PROPERTY_SEED),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

fn model_password(raw: &str) -> Option<String> {
    let normalized = raw
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .flat_map(char::to_uppercase)
        .collect::<String>();
    (!normalized.is_empty()).then_some(normalized)
}

fn model_canonical_room(base: &str, hash: &str) -> Option<String> {
    let base = base.trim();
    let hash = hash.trim();
    if base.is_empty() || hash.len() != 12 || !hash.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }

    Some(if base.starts_with('+') {
        format!("{base}:{hash}")
    } else {
        format!("+{base}:{hash}")
    })
}

fn model_normalize_controlled_room(input: &str) -> (String, Option<String>) {
    if let Some(password_separator) = input.rfind(':') {
        let before_password = &input[..password_separator];
        let password = &input[password_separator + 1..];
        if let Some(hash_separator) = before_password.rfind(':') {
            let base = &before_password[..hash_separator];
            let hash = &before_password[hash_separator + 1..];
            if let Some(room) = model_canonical_room(base, hash) {
                return (room, model_password(password));
            }
        }
    }

    if let Some(hash_separator) = input.rfind(':') {
        let base = &input[..hash_separator];
        let hash = &input[hash_separator + 1..];
        if let Some(room) = model_canonical_room(base, hash) {
            return (room, None);
        }
    }

    (input.to_owned(), None)
}

fn generated_hash(words: [u64; 4]) -> String {
    const ALPHANUMERIC: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    (0..12)
        .map(|index| {
            let word = words[index % words.len()].rotate_right((index * 11) as u32);
            ALPHANUMERIC[word as usize % ALPHANUMERIC.len()] as char
        })
        .collect()
}

fn generated_password(words: [u64; 4]) -> String {
    const PIECES: [&str; 12] = [
        "A", "b", "7", "-", "_", "!", " ", ".", "\t", "\u{00e9}", "\u{03a9}", "/",
    ];
    let mut password = String::new();
    for index in 0..16 {
        let word = words[index % words.len()].rotate_left((index * 7) as u32);
        password.push_str(PIECES[word as usize % PIECES.len()]);
    }
    password
}

fn valid_controlled_room(words: [u64; 4], password: Option<&str>) -> String {
    let mut base = format!("room-{:08x}", words[0] as u32);
    if words[1] & 1 != 0 {
        base = format!("group:{base}");
    }
    if words[1] & 2 != 0 {
        base.insert(0, '+');
    }
    let hash = generated_hash(words);
    let base = if words[2] & 1 == 0 {
        base
    } else {
        format!(" \t{base}\t ")
    };
    let hash = if words[2] & 2 == 0 {
        hash
    } else {
        format!(" {hash}\t")
    };
    match password {
        Some(password) => format!("{base}:{hash}:{password}"),
        None => format!("{base}:{hash}"),
    }
}

fn effective_room_model(settings: &StoredClientSettings) -> Option<(String, Option<String>)> {
    settings
        .room
        .as_deref()
        .map(str::trim)
        .filter(|room| !room.is_empty())
        .or_else(|| {
            settings
                .room_list
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(String::as_str)
                .map(str::trim)
                .find(|room| !room.is_empty())
        })
        .map(model_normalize_controlled_room)
}

fn exposed(secret: Option<&sorotte_secret::SecretValue>) -> Option<&str> {
    secret.map(sorotte_secret::SecretValue::expose_secret)
}

fn unrelated_environment(words: [u64; 4]) -> Vec<&'static str> {
    let flag = |bit: u32| (words[bit as usize % words.len()] & (1_u64 << bit)) != 0;
    [
        ("SOROTTE_CLIENT_HOST", 0),
        ("SOROTTE_CLIENT_PORT", 1),
        ("SOROTTE_CLIENT_SERVER_PASSWORD", 2),
        ("SOROTTE_CLIENT_USERNAME", 3),
        ("SOROTTE_CLIENT_AUTOPLAY", 4),
        ("SOROTTE_CLIENT_AUTOPLAY_REQUIRE_SAME_FILENAMES", 5),
        ("SOROTTE_CLIENT_READY_AT_START", 6),
        ("SOROTTE_CLIENT_SHARED_PLAYLIST_ENABLED", 7),
        ("SOROTTE_CLIENT_PAUSE_ON_LEAVE", 8),
        ("SOROTTE_CLIENT_LOOP_AT_END_OF_PLAYLIST", 9),
        ("SOROTTE_CLIENT_LOOP_SINGLE_FILES", 10),
        ("SOROTTE_CLIENT_ONLY_SWITCH_TO_TRUSTED_DOMAINS", 11),
        ("SOROTTE_CLIENT_TRUSTED_DOMAINS", 12),
        ("SOROTTE_CLIENT_REWIND_ON_DESYNC", 13),
        ("SOROTTE_CLIENT_FASTFORWARD_ON_DESYNC", 14),
        ("SOROTTE_CLIENT_SLOW_ON_DESYNC", 15),
        ("SOROTTE_CLIENT_DONT_SLOW_DOWN_WITH_ME", 16),
        ("SOROTTE_CLIENT_REWIND_THRESHOLD_SECONDS", 17),
        ("SOROTTE_CLIENT_FASTFORWARD_THRESHOLD_SECONDS", 18),
        ("SOROTTE_CLIENT_SLOWDOWN_THRESHOLD_SECONDS", 19),
        ("SOROTTE_CLIENT_UNPAUSE_ACTION", 20),
        ("SOROTTE_CLIENT_AUTOPLAY_MIN_USERS", 21),
        ("SOROTTE_CLIENT_FILENAME_PRIVACY_MODE", 22),
        ("SOROTTE_CLIENT_FILESIZE_PRIVACY_MODE", 23),
        ("SOROTTE_CLIENT_SHOW_DURATION_NOTIFICATION", 24),
        ("SOROTTE_CLIENT_SHOW_SAME_ROOM_OSD", 25),
        ("SOROTTE_CLIENT_SHOW_OSD_WARNINGS", 26),
        ("SOROTTE_CLIENT_SHOW_NONCONTROLLER_OSD", 27),
        ("SOROTTE_CLIENT_SHOW_DIFFERENT_ROOM_OSD", 28),
    ]
    .into_iter()
    .filter_map(|(name, bit)| flag(bit).then_some(name))
    .collect()
}

fn malformed_room(selector: u8, words: [u64; 4]) -> String {
    let hash = generated_hash(words);
    let long_secret = format!("CREDENTIAL{:016X}", words[3]);
    match selector % 8 {
        0 => format!(" :{hash}:{long_secret}"),
        1 => format!("room:{}:{long_secret}", &hash[..11]),
        2 => format!("room:{hash}Z:{long_secret}"),
        3 => format!("room:{}!:{long_secret}", &hash[..11]),
        4 => format!("room:\u{00e9}{}:{long_secret}", &hash[..11]),
        5 => format!("ordinary-room-{:016x}", words[0]),
        6 => format!("room:{hash}:!_\u{00e9}\u{03a9}?"),
        _ => format!("room:{hash}"),
    }
}

proptest! {
    #![proptest_config(configured_proptest())]

    #[test]
    fn controlled_room_normalization_matches_independent_model_and_is_idempotent(
        words in any::<[u64; 4]>(),
        with_password in any::<bool>(),
    ) {
        let password = generated_password(words);
        let input = valid_controlled_room(words, with_password.then_some(password.as_str()));
        let expected = model_normalize_controlled_room(&input);
        let actual = normalize_controlled_room_input(input);
        prop_assert_eq!(&actual, &expected);

        let normalized_again =
            normalize_controlled_room_input(actual.0.clone());
        prop_assert_eq!(normalized_again, (actual.0.clone(), None));

        if let Some(password) = actual.1.as_deref() {
            let reconstructed =
                normalize_controlled_room_input(format!("{}:{password}", actual.0));
            prop_assert_eq!(reconstructed, actual.clone());
        }

        let without_prefix = actual
            .0
            .strip_prefix('+')
            .expect("generated canonical room should have a plus prefix");
        let expected_base = without_prefix
            .rsplit_once(':')
            .expect("generated canonical room should have a hash suffix")
            .0;
        prop_assert_eq!(
            controlled_room_base_name(&actual.0),
            expected_base,
        );
    }

    #[test]
    fn malformed_and_passwordless_controlled_rooms_never_manufacture_credentials(
        selector in any::<u8>(),
        words in any::<[u64; 4]>(),
    ) {
        let input = malformed_room(selector, words);
        let expected = model_normalize_controlled_room(&input);
        prop_assert_eq!(expected.1.as_deref(), None);

        let actual = normalize_controlled_room_input(input.clone());
        prop_assert_eq!(&actual, &expected);

        let settings = StoredClientSettings {
            room: Some(input),
            ..StoredClientSettings::default()
        };
        let snapshot = stored_client_settings_runtime_snapshot(&settings);
        let config = configured(
            &settings,
            &[],
        );
        prop_assert_eq!(snapshot.controlled_room_password_override, None);
        prop_assert_eq!(snapshot.config.connection.controlled_room_password, None);
        prop_assert_eq!(config.controlled_room_password_override, None);
        prop_assert_eq!(snapshot.config.connection.tls_policy, TlsPolicy::PreferTls);
    }

    #[test]
    fn controlled_room_ini_roundtrip_composition_and_precedence_match_the_model(
        selector in any::<u8>(),
        words in any::<[u64; 4]>(),
    ) {
        let primary_password = generated_password(words);
        let primary = valid_controlled_room(words, Some(&primary_password));
        let alternate_words = [
            words[0].rotate_left(7),
            words[1].rotate_right(13),
            words[2] ^ 0xA5A5_A5A5_A5A5_A5A5,
            words[3].wrapping_add(1),
        ];
        let alternate_password = generated_password(alternate_words);
        let alternate = valid_controlled_room(alternate_words, Some(&alternate_password));
        let settings = match selector % 5 {
            0 => StoredClientSettings {
                room: Some(primary.clone()),
                room_list: Some(vec![alternate]),
                ..StoredClientSettings::default()
            },
            1 => StoredClientSettings {
                room_list: Some(vec![" \t ".to_owned(), primary.clone(), alternate]),
                ..StoredClientSettings::default()
            },
            2 => StoredClientSettings {
                room: Some(" \r\n ".to_owned()),
                room_list: Some(vec![primary.clone(), alternate]),
                ..StoredClientSettings::default()
            },
            3 => StoredClientSettings {
                room: Some(format!("ordinary-{:016x}", words[0])),
                room_list: Some(vec![primary]),
                ..StoredClientSettings::default()
            },
            _ => StoredClientSettings {
                room: Some(malformed_room(selector, words)),
                room_list: Some(vec![primary]),
                ..StoredClientSettings::default()
            },
        };

        let rendered = upsert_sorotte_ini_stored_client_settings("", &settings);
        let parsed = parse_sorotte_ini_stored_client_settings(&rendered);
        let canonical = upsert_sorotte_ini_stored_client_settings("", &parsed);
        let canonical_parsed = parse_sorotte_ini_stored_client_settings(&canonical);
        prop_assert_eq!(&canonical_parsed, &parsed);
        prop_assert_eq!(
            upsert_sorotte_ini_stored_client_settings("", &canonical_parsed),
            canonical,
        );

        let expected = effective_room_model(&settings);
        let snapshot = stored_client_settings_runtime_snapshot(&settings);
        let reparsed_snapshot =
            stored_client_settings_runtime_snapshot(&parsed);
        prop_assert_eq!(&reparsed_snapshot, &snapshot);
        prop_assert_eq!(
            snapshot.settings.room.as_deref(),
            expected.as_ref().map(|(room, _)| room.as_str()),
        );
        prop_assert_eq!(
            snapshot.config.connection.room.as_ref().map(|room| room.as_str()),
            expected.as_ref().map(|(room, _)| room.as_str()),
        );
        prop_assert_eq!(
            exposed(snapshot.controlled_room_password_override.as_ref()),
            expected.as_ref().and_then(|(_, password)| password.as_deref()),
        );
        prop_assert_eq!(
            exposed(snapshot.config.connection.controlled_room_password.as_ref()),
            expected.as_ref().and_then(|(_, password)| password.as_deref()),
        );

        let unrelated = unrelated_environment(words);
        let unshadowed =
            configured(&settings, &unrelated);
        prop_assert_eq!(
            unshadowed.room.as_str(),
            expected.as_ref().map(|(room, _)| room.as_str()).unwrap_or("cli-room"),
        );
        prop_assert_eq!(
            exposed(unshadowed.controlled_room_password_override.as_ref()),
            expected.as_ref().and_then(|(_, password)| password.as_deref()),
        );

        let mut room_shadow = unrelated;
        room_shadow.push("SOROTTE_CLIENT_ROOM");
        let shadowed =
            configured(&settings, &room_shadow);
        let mut expected_shadowed = unshadowed;
        expected_shadowed.room = "cli-room".to_owned();
        expected_shadowed.controlled_room_password_override = None;
        prop_assert_eq!(config_values(&shadowed), config_values(&expected_shadowed));
        prop_assert_eq!(shadowed.controlled_room_password_override, expected_shadowed.controlled_room_password_override);
    }

    #[test]
    fn controlled_room_credentials_are_typed_redacted_and_independently_shadowed(
        words in any::<[u64; 4]>(),
    ) {
        let room_marker = format!("ROOMSECRET{:016X}", words[0]);
        let server_marker = format!("SERVERSECRET{:016X}", words[1]);
        let room = valid_controlled_room(words, Some(&format!("!_{room_marker}-")));
        let expected = model_normalize_controlled_room(&room);
        let normalized_room_secret = expected
            .1
            .as_deref()
            .expect("generated room marker should survive normalization");
        let settings = StoredClientSettings {
            server_password: Some(server_marker.clone().into()),
            room: Some(room.clone()),
            ..StoredClientSettings::default()
        };

        let snapshot = stored_client_settings_runtime_snapshot(&settings);
        let config = configured(
            &settings,
            &[],
        );
        prop_assert_eq!(
            exposed(snapshot.config.connection.controlled_room_password.as_ref()),
            Some(normalized_room_secret),
        );
        prop_assert_eq!(snapshot.config.connection.tls_policy, TlsPolicy::RequireTls);

        for (label, debug) in [
            ("settings", format!("{settings:?}")),
            ("snapshot", format!("{snapshot:?}")),
            ("CLI config", format!("{config:?}")),
        ] {
            prop_assert!(debug.contains("<redacted>"), "{label} omitted a redaction marker");
            prop_assert!(!debug.contains(&room_marker), "{label} exposed the room marker");
            prop_assert!(
                !debug.contains(normalized_room_secret),
                "{label} exposed the normalized room credential",
            );
            prop_assert!(!debug.contains(&server_marker), "{label} exposed the server marker");
        }

        let server_shadowed = configured(
            &settings,
            &["SOROTTE_CLIENT_SERVER_PASSWORD"],
        );
        prop_assert_eq!(server_shadowed.server_password, None);
        prop_assert_eq!(server_shadowed.room, config.room.clone());
        prop_assert_eq!(
            server_shadowed.controlled_room_password_override,
            config.controlled_room_password_override.clone(),
        );

        let room_shadowed = configured(
            &settings,
            &["SOROTTE_CLIENT_ROOM"],
        );
        prop_assert_eq!(room_shadowed.room, "cli-room");
        prop_assert_eq!(room_shadowed.controlled_room_password_override, None);
        prop_assert_eq!(room_shadowed.server_password, config.server_password);
    }
}
