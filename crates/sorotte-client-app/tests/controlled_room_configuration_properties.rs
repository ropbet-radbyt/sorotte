//! Deterministic black-box properties for controlled-room configuration.
//!
//! The oracle below is independent of the production implementation. The
//! properties exercise only the public client-app boundary: normalization,
//! command presentation, INI persistence, runtime resolution, and
//! environment-aware startup composition.

use proptest::{
    prelude::*,
    test_runner::{Config as ProptestConfig, RngSeed},
};
use sorotte_client_app::app_boundary::{
    commands::controlled_room_base_name_legacy_compatible,
    persistence::{
        parse_sorotte_ini_stored_client_settings_mvp, upsert_sorotte_ini_stored_client_settings_mvp,
    },
    state::{
        StoredClientSettingsEnvPresence, StoredClientSettingsV1, TlsPolicy,
        normalize_controlled_room_input_legacy_compatible,
        stored_client_settings_config_plan_legacy_compatible,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    },
};

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

fn effective_room_model(settings: &StoredClientSettingsV1) -> Option<(String, Option<String>)> {
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

fn unrelated_environment(words: [u64; 4]) -> StoredClientSettingsEnvPresence {
    let flag = |bit: u32| (words[bit as usize % words.len()] & (1_u64 << bit)) != 0;
    StoredClientSettingsEnvPresence {
        host: flag(0),
        port: flag(1),
        server_password: flag(2),
        username: flag(3),
        room: false,
        autoplay: flag(4),
        autoplay_require_same_filenames: flag(5),
        ready_at_start: flag(6),
        shared_playlist_enabled: flag(7),
        pause_on_leave: flag(8),
        loop_at_end_of_playlist: flag(9),
        loop_single_files: flag(10),
        only_switch_to_trusted_domains: flag(11),
        trusted_domains: flag(12),
        rewind_on_desync: flag(13),
        fastforward_on_desync: flag(14),
        slow_on_desync: flag(15),
        dont_slow_down_with_me: flag(16),
        rewind_threshold_seconds: flag(17),
        fastforward_threshold_seconds: flag(18),
        slowdown_threshold_seconds: flag(19),
        unpause_action: flag(20),
        autoplay_min_users: flag(21),
        filename_privacy_mode: flag(22),
        filesize_privacy_mode: flag(23),
        show_duration_notification: flag(24),
        show_same_room_osd: flag(25),
        show_osd_warnings: flag(26),
        show_noncontroller_osd: flag(27),
        show_different_room_osd: flag(28),
    }
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
        let actual = normalize_controlled_room_input_legacy_compatible(input);
        prop_assert_eq!(&actual, &expected);

        let normalized_again =
            normalize_controlled_room_input_legacy_compatible(actual.0.clone());
        prop_assert_eq!(normalized_again, (actual.0.clone(), None));

        if let Some(password) = actual.1.as_deref() {
            let reconstructed =
                normalize_controlled_room_input_legacy_compatible(format!("{}:{password}", actual.0));
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
            controlled_room_base_name_legacy_compatible(&actual.0),
            expected_base,
        );
    }

    #[test]
    fn malformed_and_passwordless_legacy_rooms_never_manufacture_credentials(
        selector in any::<u8>(),
        words in any::<[u64; 4]>(),
    ) {
        let input = malformed_room(selector, words);
        let expected = model_normalize_controlled_room(&input);
        prop_assert_eq!(expected.1.as_deref(), None);

        let actual = normalize_controlled_room_input_legacy_compatible(input.clone());
        prop_assert_eq!(&actual, &expected);

        let settings = StoredClientSettingsV1 {
            room: Some(input),
            ..StoredClientSettingsV1::default()
        };
        let snapshot = stored_client_settings_runtime_snapshot_legacy_compatible(&settings);
        let plan = stored_client_settings_config_plan_legacy_compatible(
            &settings,
            &StoredClientSettingsEnvPresence::default(),
        );
        prop_assert_eq!(snapshot.controlled_room_password_override, None);
        prop_assert_eq!(snapshot.config.connection.controlled_room_password, None);
        prop_assert_eq!(plan.controlled_room_password_override, None);
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
            0 => StoredClientSettingsV1 {
                room: Some(primary.clone()),
                room_list: Some(vec![alternate]),
                ..StoredClientSettingsV1::default()
            },
            1 => StoredClientSettingsV1 {
                room_list: Some(vec![" \t ".to_owned(), primary.clone(), alternate]),
                ..StoredClientSettingsV1::default()
            },
            2 => StoredClientSettingsV1 {
                room: Some(" \r\n ".to_owned()),
                room_list: Some(vec![primary.clone(), alternate]),
                ..StoredClientSettingsV1::default()
            },
            3 => StoredClientSettingsV1 {
                room: Some(format!("ordinary-{:016x}", words[0])),
                room_list: Some(vec![primary]),
                ..StoredClientSettingsV1::default()
            },
            _ => StoredClientSettingsV1 {
                room: Some(malformed_room(selector, words)),
                room_list: Some(vec![primary]),
                ..StoredClientSettingsV1::default()
            },
        };

        let rendered = upsert_sorotte_ini_stored_client_settings_mvp("", &settings);
        let parsed = parse_sorotte_ini_stored_client_settings_mvp(&rendered);
        let canonical = upsert_sorotte_ini_stored_client_settings_mvp("", &parsed);
        let canonical_parsed = parse_sorotte_ini_stored_client_settings_mvp(&canonical);
        prop_assert_eq!(&canonical_parsed, &parsed);
        prop_assert_eq!(
            upsert_sorotte_ini_stored_client_settings_mvp("", &canonical_parsed),
            canonical,
        );

        let expected = effective_room_model(&settings);
        let snapshot = stored_client_settings_runtime_snapshot_legacy_compatible(&settings);
        let reparsed_snapshot =
            stored_client_settings_runtime_snapshot_legacy_compatible(&parsed);
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
            stored_client_settings_config_plan_legacy_compatible(&settings, &unrelated);
        prop_assert_eq!(
            unshadowed.room.as_deref(),
            expected.as_ref().map(|(room, _)| room.as_str()),
        );
        prop_assert_eq!(
            exposed(unshadowed.controlled_room_password_override.as_ref()),
            expected.as_ref().and_then(|(_, password)| password.as_deref()),
        );

        let mut room_shadow = unrelated;
        room_shadow.room = true;
        let shadowed =
            stored_client_settings_config_plan_legacy_compatible(&settings, &room_shadow);
        let mut expected_shadowed = unshadowed;
        expected_shadowed.room = None;
        expected_shadowed.controlled_room_password_override = None;
        prop_assert_eq!(shadowed, expected_shadowed);
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
        let settings = StoredClientSettingsV1 {
            server_password: Some(server_marker.clone().into()),
            room: Some(room.clone()),
            ..StoredClientSettingsV1::default()
        };

        let snapshot = stored_client_settings_runtime_snapshot_legacy_compatible(&settings);
        let plan = stored_client_settings_config_plan_legacy_compatible(
            &settings,
            &StoredClientSettingsEnvPresence::default(),
        );
        prop_assert_eq!(
            exposed(snapshot.config.connection.controlled_room_password.as_ref()),
            Some(normalized_room_secret),
        );
        prop_assert_eq!(snapshot.config.connection.tls_policy, TlsPolicy::RequireTls);

        for (label, debug) in [
            ("settings", format!("{settings:?}")),
            ("snapshot", format!("{snapshot:?}")),
            ("config plan", format!("{plan:?}")),
        ] {
            prop_assert!(debug.contains("<redacted>"), "{label} omitted a redaction marker");
            prop_assert!(!debug.contains(&room_marker), "{label} exposed the room marker");
            prop_assert!(
                !debug.contains(normalized_room_secret),
                "{label} exposed the normalized room credential",
            );
            prop_assert!(!debug.contains(&server_marker), "{label} exposed the server marker");
        }

        let server_shadowed = stored_client_settings_config_plan_legacy_compatible(
            &settings,
            &StoredClientSettingsEnvPresence {
                server_password: true,
                ..StoredClientSettingsEnvPresence::default()
            },
        );
        prop_assert_eq!(server_shadowed.server_password, None);
        prop_assert_eq!(server_shadowed.room, plan.room.clone());
        prop_assert_eq!(
            server_shadowed.controlled_room_password_override,
            plan.controlled_room_password_override.clone(),
        );

        let room_shadowed = stored_client_settings_config_plan_legacy_compatible(
            &settings,
            &StoredClientSettingsEnvPresence {
                room: true,
                ..StoredClientSettingsEnvPresence::default()
            },
        );
        prop_assert_eq!(room_shadowed.room, None);
        prop_assert_eq!(room_shadowed.controlled_room_password_override, None);
        prop_assert_eq!(room_shadowed.server_password, plan.server_password);
    }
}
