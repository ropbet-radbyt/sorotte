//! Generated black-box contracts for migrating legacy `sorotte.ini` inputs.
//!
//! These tests deliberately start from legacy spellings and container formats
//! rather than the canonical DTOs exercised by `configuration_composition_properties`.
//! The public app boundary must parse them, preserve their meaning through an
//! in-place update, and produce an idempotent canonical rewrite.

use std::collections::BTreeMap;

use proptest::{
    prelude::*,
    test_runner::{Config as ProptestConfig, RngSeed},
};
use sorotte_client_app::app_boundary::{
    persistence::{
        parse_sorotte_ini_stored_client_settings_mvp, upsert_sorotte_ini_stored_client_settings_mvp,
    },
    state::{
        AutoplayThresholdOverride, StartSynchronizationPolicy, StoredClientSettingsV1,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    },
};
use sorotte_client_core::{PrivacyMode, UnpauseActionMode};

const DEFAULT_CASES: u32 = 512;
const MAX_CASES: u32 = 100_000;
const PROPERTY_SEED: u64 = 0xC0F1_6D1A_2026_0730;

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

fn recase_ascii(value: &str, mask: u64) -> String {
    value
        .chars()
        .enumerate()
        .map(|(index, character)| {
            if character.is_ascii_alphabetic() && (mask.rotate_right(index as u32) & 1) != 0 {
                character.to_ascii_uppercase()
            } else {
                character.to_ascii_lowercase()
            }
        })
        .collect()
}

fn bool_spelling(value: bool, selector: u64) -> &'static str {
    const TRUE: [&str; 8] = ["1", "true", "TRUE", "True", "yes", "YeS", "on", "ON"];
    const FALSE: [&str; 8] = ["0", "false", "FALSE", "False", "no", "No", "off", "OFF"];
    let spellings = if value { TRUE } else { FALSE };
    spellings[selector as usize % spellings.len()]
}

fn assignment(key: &str, value: &str, mask: u64) -> String {
    format!("\t{} \t= \t{value}\t", recase_ascii(key, mask))
}

fn section(name: &str, mask: u64) -> String {
    format!("  [{}]  ", recase_ascii(name, mask))
}

fn render_legacy_lines(lines: Vec<String>, words: [u64; 4]) -> String {
    let newline = if words[0] & 1 == 0 { "\n" } else { "\r\n" };
    let mut rendered = lines.join(newline);
    rendered.push_str(newline);
    if words[0] & 2 != 0 {
        rendered.insert(0, '\u{feff}');
    }
    rendered
}

fn legacy_scalar_fixture(words: [u64; 4]) -> (String, StoredClientSettingsV1) {
    const LANGUAGES: [(&str, &str); 6] = [
        ("PT-br", "pt_BR"),
        ("pt-PT", "pt_PT"),
        ("ZH-cn", "zh_CN"),
        ("FR", "fr"),
        ("eN", "en"),
        ("Ko", "ko"),
    ];
    const UNPAUSE_ACTIONS: [(&str, UnpauseActionMode); 12] = [
        ("IfAlreadyReady", UnpauseActionMode::IfAlreadyReady),
        ("if_already_ready", UnpauseActionMode::IfAlreadyReady),
        ("if-already-ready", UnpauseActionMode::IfAlreadyReady),
        ("IfOthersReady", UnpauseActionMode::IfOthersReady),
        ("if_others_ready", UnpauseActionMode::IfOthersReady),
        ("if-others-ready", UnpauseActionMode::IfOthersReady),
        ("IfMinUsersReady", UnpauseActionMode::IfMinUsersReady),
        ("if_min_users_ready", UnpauseActionMode::IfMinUsersReady),
        ("if-min-users-ready", UnpauseActionMode::IfMinUsersReady),
        ("Always", UnpauseActionMode::Always),
        ("always", UnpauseActionMode::Always),
        ("ALWAYS", UnpauseActionMode::Always),
    ];
    const PRIVACY_MODES: [(&str, PrivacyMode); 3] = [
        ("SendRaw", PrivacyMode::SendRaw),
        ("SendHashed", PrivacyMode::SendHashed),
        ("DoNotSend", PrivacyMode::DoNotSend),
    ];
    const AUTOPLAY_MIN_USERS: [(&str, AutoplayThresholdOverride); 6] = [
        ("-7", AutoplayThresholdOverride::Disable),
        ("-1", AutoplayThresholdOverride::Disable),
        ("0", AutoplayThresholdOverride::Disable),
        ("1", AutoplayThresholdOverride::Set(1)),
        ("3", AutoplayThresholdOverride::Set(3)),
        ("32", AutoplayThresholdOverride::Set(32)),
    ];

    let (language_input, language) = LANGUAGES[words[0] as usize % LANGUAGES.len()];
    let (unpause_input, unpause_action) =
        UNPAUSE_ACTIONS[words[1] as usize % UNPAUSE_ACTIONS.len()].clone();
    let (filename_privacy_input, filename_privacy_mode) =
        PRIVACY_MODES[words[2] as usize % PRIVACY_MODES.len()];
    let (filesize_privacy_input, filesize_privacy_mode) =
        PRIVACY_MODES[words[3] as usize % PRIVACY_MODES.len()];
    let (autoplay_min_users_input, autoplay_min_users) =
        AUTOPLAY_MIN_USERS[(words[0] ^ words[3]) as usize % AUTOPLAY_MIN_USERS.len()].clone();
    let flag = |bit: u32| (words[1] & (1_u64 << bit)) != 0;
    let port = 1_024 + (words[2] % 54_000) as u16;
    let streaming_buffer_target_seconds = (8 + words[2] % 64) as f64 / 4.0;
    let streaming_memory_cache_mebibytes = 64 + words[3] % 4_096;
    let streaming_recovery_retry_budget = (words[0] % 16) as u32;
    let rewind_threshold_seconds = (1 + words[3] % 128) as f64 / 4.0;
    let chat_top_margin = (words[2] % 101) as i64;
    let host = format!("legacy-{:016x}.example", words[0]);
    let username = format!("legacy-user-{:016x}", words[1]);
    let room = format!("legacy-room-{:016x}", words[2]);
    let masks = [
        words[0],
        words[1],
        words[2],
        words[3],
        words[0] ^ words[2],
        words[1] ^ words[3],
    ];

    let lines = vec![
        "; legacy configuration with mixed casing and spacing".to_owned(),
        "line-without-an-equals-sign".to_owned(),
        section("general", masks[0]),
        assignment("language", language_input, masks[1]),
        assignment(
            "checkForUpdatesAutomatically",
            bool_spelling(flag(0), words[2]),
            masks[2],
        ),
        assignment("updateChannel", "DEV", masks[3]),
        section("server_data", masks[1]),
        assignment("host", &host, masks[2]),
        assignment("port", &port.to_string(), masks[3]),
        section("client_settings", masks[2]),
        assignment("name", &username, masks[3]),
        assignment("room", &room, masks[4]),
        assignment("streamingQualityPreset", "720P", masks[5]),
        assignment(
            "streamingBufferTarget",
            &streaming_buffer_target_seconds.to_string(),
            masks[0],
        ),
        assignment(
            "streamingMemoryCacheMiB",
            &streaming_memory_cache_mebibytes.to_string(),
            masks[1],
        ),
        assignment(
            "streamingDiskCacheEnabled",
            bool_spelling(flag(1), words[3]),
            masks[2],
        ),
        assignment("streamingRecoveryPolicy", "BALANCED", masks[3]),
        assignment(
            "streamingRecoveryRetryBudget",
            &streaming_recovery_retry_budget.to_string(),
            masks[4],
        ),
        assignment("streamingRoomBufferingPolicy", "QUORUM", masks[5]),
        assignment("forceGuiPrompt", bool_spelling(flag(2), words[0]), masks[0]),
        assignment(
            "autoplayInitialState",
            bool_spelling(flag(3), words[1]),
            masks[1],
        ),
        assignment(
            "rewindThreshold",
            &rewind_threshold_seconds.to_string(),
            masks[2],
        ),
        assignment("unpauseAction", unpause_input, masks[3]),
        assignment("autoplayMinUsers", autoplay_min_users_input, masks[4]),
        assignment("filenamePrivacyMode", filename_privacy_input, masks[5]),
        assignment("filesizePrivacyMode", filesize_privacy_input, masks[0]),
        section("gui", masks[3]),
        assignment(
            "chatInputEnabled",
            bool_spelling(flag(4), words[2]),
            masks[4],
        ),
        assignment("chatTopMargin", &chat_top_margin.to_string(), masks[5]),
        assignment(
            "showDurationNotification",
            bool_spelling(flag(5), words[3]),
            masks[0],
        ),
        section("plugins", masks[4]),
        assignment(
            "streamSupportEnabled",
            bool_spelling(flag(6), words[0]),
            masks[1],
        ),
        assignment(
            "mediaMatchingEnabled",
            bool_spelling(flag(7), words[1]),
            masks[2],
        ),
        assignment("plexEnabled", bool_spelling(flag(8), words[2]), masks[3]),
        section("plex", masks[5]),
        assignment("syncEnabled", bool_spelling(flag(9), words[3]), masks[4]),
        assignment(
            "streamingEnabled",
            bool_spelling(flag(10), words[0]),
            masks[5],
        ),
        "[future_extension]".to_owned(),
        "futureKey = preserved-but-ignored".to_owned(),
    ];

    let expected = StoredClientSettingsV1 {
        language: Some(language.to_owned()),
        check_for_updates_automatically: Some(flag(0)),
        update_channel: Some("dev".to_owned()),
        host: Some(host),
        port: Some(port),
        username: Some(username),
        room: Some(room),
        streaming_quality_preset: Some("720p".to_owned()),
        streaming_buffer_target_seconds: Some(streaming_buffer_target_seconds),
        streaming_memory_cache_mebibytes: Some(streaming_memory_cache_mebibytes),
        streaming_disk_cache_enabled: Some(flag(1)),
        streaming_recovery_policy: Some("balanced".to_owned()),
        streaming_recovery_retry_budget: Some(streaming_recovery_retry_budget),
        streaming_room_buffering_policy: Some("quorum".to_owned()),
        force_gui_prompt: Some(flag(2)),
        autoplay_initial_state: Some(flag(3)),
        rewind_threshold_seconds: Some(rewind_threshold_seconds),
        unpause_action: Some(unpause_action),
        autoplay_min_users: Some(autoplay_min_users),
        filename_privacy_mode: Some(filename_privacy_mode),
        filesize_privacy_mode: Some(filesize_privacy_mode),
        chat_input_enabled: Some(flag(4)),
        chat_top_margin: Some(chat_top_margin),
        show_duration_notification: Some(flag(5)),
        stream_support_plugin_enabled: Some(flag(6)),
        media_matching_plugin_enabled: Some(flag(7)),
        plex_plugin_enabled: Some(flag(8)),
        plex_sync_enabled: Some(flag(9)),
        plex_streaming_enabled: Some(flag(10)),
        ..StoredClientSettingsV1::default()
    };

    (render_legacy_lines(lines, words), expected)
}

fn legacy_collection_fixture(words: [u64; 4]) -> (String, StoredClientSettingsV1) {
    let room_a = format!("room-{:08x}", words[0] as u32);
    let room_b = format!("room-{:08x}", words[1] as u32);
    let domain_a = format!("media-{:08x}.example", words[2] as u32);
    let domain_b = format!("stream-{:08x}.example", words[3] as u32);
    let directory_a = format!("Z:/Anime/{:08x}", words[0] as u32);
    let directory_b = format!("Z:/Seasonal/{:08x}", words[1] as u32);
    let player_a = format!("mpv-{:08x}", words[2] as u32);
    let player_b = format!("mpv-{:08x}", words[3] as u32);
    let argument_a = format!("--profile=legacy-{:08x}", words[0] as u32);
    let server_a = format!("sync-{:08x}.example:8999", words[2] as u32);
    let server_b = format!("sync-{:08x}.example:8998", words[3] as u32);

    let room_list = match words[0] % 3 {
        0 => format!("{room_a}; {room_b}"),
        1 => format!("[{room_a}, {room_b}]"),
        _ => format!("['{room_a}', \"{room_b}\"]"),
    };
    let trusted_domains = match words[1] % 3 {
        0 => format!("{domain_a}, {domain_b}"),
        1 => format!("[{domain_a}, {domain_b}]"),
        _ => format!("[\"{domain_a}\", '{domain_b}']"),
    };
    let per_player_arguments = if words[2] & 1 == 0 {
        format!("{{'{player_a}': ['{argument_a}', '--no-border'], \"{player_b}\": [\"--fs\"]}}")
    } else {
        format!(
            "{{ \"{player_b}\" : [ \"--fs\" ], '{player_a}' : [ '{argument_a}', '--no-border' ] }}"
        )
    };
    let public_servers = if words[3] & 1 == 0 {
        format!("[['Primary', '{server_a}'], ('Backup', \"{server_b}\")]")
    } else {
        format!("[(\"Primary\", '{server_a}'), ['Backup', \"{server_b}\"]]")
    };

    let lines = vec![
        section("client_settings", words[0]),
        assignment("roomList", &room_list, words[1]),
        assignment(
            "mediaSearchDirectories",
            &format!("{directory_a}; ; {directory_b}"),
            words[2],
        ),
        assignment("trustedDomains", &trusted_domains, words[3]),
        assignment(
            "perPlayerArguments",
            &per_player_arguments,
            words[0] ^ words[2],
        ),
        assignment("publicServers", &public_servers, words[1] ^ words[3]),
    ];

    let mut expected_arguments = BTreeMap::new();
    expected_arguments.insert(player_a, vec![argument_a, "--no-border".to_owned()]);
    expected_arguments.insert(player_b, vec!["--fs".to_owned()]);
    let expected = StoredClientSettingsV1 {
        room_list: Some(vec![room_a, room_b]),
        trusted_domains: Some(vec![domain_a, domain_b]),
        media_search_directories: Some(vec![directory_a, directory_b]),
        per_player_arguments: Some(expected_arguments),
        public_servers: Some(vec![
            ("Primary".to_owned(), server_a),
            ("Backup".to_owned(), server_b),
        ]),
        ..StoredClientSettingsV1::default()
    };

    (render_legacy_lines(lines, words), expected)
}

fn malformed_fixture(selectors: [u8; 8], words: [u64; 4]) -> String {
    const BAD_BOOLS: [&str; 8] = ["", "2", "-1", "1.0", "truthy", "null", "tru e", "yes!"];
    const BAD_PORTS: [&str; 8] = ["", "0", "-1", "65536", "1.5", "NaN", "8999x", "+-1"];
    const BAD_FLOATS: [&str; 8] = ["", "-1", "NaN", "inf", "-inf", "number", "1.2.3", "--1"];
    const BAD_U64S: [&str; 8] = [
        "",
        "-1",
        "1.5",
        "NaN",
        "18446744073709551616",
        "64MiB",
        "0x40",
        "+-1",
    ];
    const BAD_U32S: [&str; 8] = ["", "-1", "1.5", "NaN", "4294967296", "three", "0x3", "+-1"];
    const BAD_I64S: [&str; 8] = [
        "",
        "1.5",
        "NaN",
        "9223372036854775808",
        "-9223372036854775809",
        "margin",
        "0x10",
        "+-1",
    ];
    const BAD_UNPAUSE: [&str; 8] = [
        "",
        "sometimes",
        "if ready",
        "IfAllReady",
        "0",
        "null",
        "always!",
        "IfMinUsers",
    ];
    const BAD_PRIVACY: [&str; 8] = [
        "",
        "sendraw",
        "SENDRAW",
        "Raw",
        "Hash",
        "None",
        "null",
        "DoNotSend!",
    ];
    const BAD_MAPS: [&str; 8] = [
        "",
        "not-a-map",
        "{",
        "{'mpv': nope}",
        "{'mpv' ['--fs']}",
        "{'mpv': ['--fs']} trailing",
        "[]",
        "{mpv: ['--fs']}",
    ];
    const BAD_SERVERS: [&str; 8] = [
        "",
        "not-a-list",
        "[",
        "[['OnlyLabel']]",
        "[['Label', 'host'] trailing]",
        "[('Label' 'host')]",
        "{}",
        "[Label, host]",
    ];

    let pick = |values: &[&'static str], selector: u8| values[selector as usize % values.len()];
    let lines = vec![
        section("general", words[0]),
        assignment("language", "not-a-supported-language", words[1]),
        assignment(
            "checkForUpdatesAutomatically",
            pick(&BAD_BOOLS, selectors[0]),
            words[2],
        ),
        section("server_data", words[1]),
        assignment("port", pick(&BAD_PORTS, selectors[1]), words[2]),
        section("client_settings", words[2]),
        assignment("name", "valid-sentinel-user", words[3]),
        assignment(
            "streamingBufferTarget",
            pick(&BAD_FLOATS, selectors[2]),
            words[0],
        ),
        assignment(
            "streamingMemoryCacheMiB",
            pick(&BAD_U64S, selectors[3]),
            words[1],
        ),
        assignment(
            "streamingRecoveryRetryBudget",
            pick(&BAD_U32S, selectors[4]),
            words[2],
        ),
        assignment("unpauseAction", pick(&BAD_UNPAUSE, selectors[5]), words[3]),
        assignment(
            "filenamePrivacyMode",
            pick(&BAD_PRIVACY, selectors[6]),
            words[0],
        ),
        assignment(
            "perPlayerArguments",
            pick(&BAD_MAPS, selectors[7]),
            words[1],
        ),
        assignment(
            "publicServers",
            pick(&BAD_SERVERS, selectors[0] ^ selectors[7]),
            words[2],
        ),
        section("gui", words[3]),
        assignment(
            "chatTopMargin",
            pick(&BAD_I64S, selectors[1] ^ selectors[6]),
            words[0],
        ),
    ];
    render_legacy_lines(lines, words)
}

fn assert_canonical_migration(
    legacy: &str,
    expected: &StoredClientSettingsV1,
) -> Result<(), TestCaseError> {
    let parsed = parse_sorotte_ini_stored_client_settings_mvp(legacy);
    prop_assert_eq!(&parsed, expected);

    let in_place = upsert_sorotte_ini_stored_client_settings_mvp(legacy, &parsed);
    let in_place_parsed = parse_sorotte_ini_stored_client_settings_mvp(&in_place);
    prop_assert_eq!(&in_place_parsed, expected);

    let canonical = upsert_sorotte_ini_stored_client_settings_mvp("", &parsed);
    let canonical_parsed = parse_sorotte_ini_stored_client_settings_mvp(&canonical);
    prop_assert_eq!(&canonical_parsed, expected);
    prop_assert_eq!(
        upsert_sorotte_ini_stored_client_settings_mvp("", &canonical_parsed),
        canonical
    );

    let before = stored_client_settings_runtime_snapshot_legacy_compatible(&parsed);
    let after = stored_client_settings_runtime_snapshot_legacy_compatible(&canonical_parsed);
    prop_assert_eq!(after, before);
    Ok(())
}

proptest! {
    #![proptest_config(configured_proptest())]

    #[test]
    fn legacy_scalar_spellings_migrate_without_semantic_drift(
        words in any::<[u64; 4]>(),
    ) {
        let (legacy, expected) = legacy_scalar_fixture(words);
        assert_canonical_migration(&legacy, &expected)?;

        let snapshot = stored_client_settings_runtime_snapshot_legacy_compatible(&expected);
        prop_assert!(
            snapshot.validation_issues.is_empty(),
            "generated valid legacy settings produced issues: {:?}",
            snapshot.validation_issues,
        );
        prop_assert_eq!(
            snapshot.config.playback.streaming.start_synchronization.policy,
            StartSynchronizationPolicy::Immediate,
            "omitting the post-legacy start policy must preserve immediate startup",
        );
    }

    #[test]
    fn legacy_collection_formats_migrate_to_one_idempotent_representation(
        words in any::<[u64; 4]>(),
    ) {
        let (legacy, expected) = legacy_collection_fixture(words);
        assert_canonical_migration(&legacy, &expected)?;
    }

    #[test]
    fn malformed_typed_values_do_not_manufacture_persisted_settings(
        selectors in any::<[u8; 8]>(),
        words in any::<[u64; 4]>(),
    ) {
        let legacy = malformed_fixture(selectors, words);
        let expected = StoredClientSettingsV1 {
            username: Some("valid-sentinel-user".to_owned()),
            ..StoredClientSettingsV1::default()
        };
        assert_canonical_migration(&legacy, &expected)?;
    }
}
