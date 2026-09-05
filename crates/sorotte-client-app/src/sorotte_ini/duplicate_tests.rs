use super::*;
use crate::legacy_settings::StoredClientSettingsMvp;
use proptest::prelude::*;

#[test]
fn missing_key_is_inserted_inside_the_final_matching_section() {
    let desired = StoredClientSettingsMvp {
        username: Some("Alice".into()),
        ..Default::default()
    };
    for original in [
        "[client_settings]\n[unknown]\nkeep=yes\n",
        "[unknown]\nkeep=yes\n[client_settings]\n; retain\n[other]\nx=y\n",
        "[client_settings]\n; first\n[unknown]\nkeep=yes\n[ CLIENT_SETTINGS ]\n; last\n[other]\nx=y\n",
    ] {
        let saved = upsert_sorotte_ini_stored_client_settings_mvp(original, &desired);
        assert_eq!(
            parse_sorotte_ini_stored_client_settings_mvp(&saved),
            desired
        );
        assert!(saved.contains("keep=yes"));
        assert_eq!(
            upsert_sorotte_ini_stored_client_settings_mvp(&saved, &desired),
            saved
        );
        let expected = if original.contains("; last") {
            "; last\nname = Alice\n[other]"
        } else if original.contains("; retain") {
            "; retain\nname = Alice\n[other]"
        } else {
            "[client_settings]\nname = Alice\n[unknown]"
        };
        assert!(
            saved.contains(expected),
            "missing key must remain in its effective section"
        );
    }
}

#[test]
fn adding_a_missing_section_preserves_exact_existing_spacing() {
    let desired = StoredClientSettingsMvp {
        username: Some("Alice".into()),
        ..Default::default()
    };
    for (original, expected) in [
        ("", "[client_settings]\nname = Alice\n"),
        (
            "[unknown]\nkeep=yes\n",
            "[unknown]\nkeep=yes\n\n[client_settings]\nname = Alice\n",
        ),
        (
            "[unknown]\nkeep=yes\n\n",
            "[unknown]\nkeep=yes\n\n[client_settings]\nname = Alice\n",
        ),
    ] {
        assert_eq!(
            upsert_sorotte_ini_stored_client_settings_mvp(original, &desired),
            expected
        );
    }
}

#[test]
fn clearing_nullable_keys_removes_all_copies_and_retains_other_sections() {
    let mut lines = [
        "[other]",
        "password=keep",
        "[ SERVER_DATA ]",
        " PASSWORD = old",
        "; preserve comment",
        "unknown=keep",
        "[server_data]",
        "password=new",
        "[other]",
        "password=also-keep",
    ]
    .map(str::to_owned)
    .to_vec();
    super::helpers::remove_ini_value_legacy_compatible(&mut lines, "server_data", "password");
    let saved = lines.join("\n");
    assert!(
        parse_sorotte_ini_stored_client_settings_mvp(&saved)
            .server_password
            .is_none()
    );
    assert_eq!(
        saved,
        "[other]\npassword=keep\n[ SERVER_DATA ]\n; preserve comment\nunknown=keep\n[server_data]\n[other]\npassword=also-keep"
    );
}

#[test]
fn public_writer_replaces_every_effective_duplicate_including_credentials() {
    for contents in [
        "[client_settings]\nname=first\nname=last\n[server_data]\npassword=first\npassword=last\n[plex]\nuserToken=first\nuserToken=last\n",
        "[client_settings]\nname=first\n[server_data]\npassword=first\n[plex]\nuserToken=first\n[client_settings]\nname=last\n[server_data]\npassword=last\n[plex]\nuserToken=last\n",
        "\u{feff}; comment\n[ CLIENT_Settings ]\n NaMe = first\n[ SERVER_DATA ]\n Password = first\n[ PLEX ]\n UserToken = first\n[ client_SETTINGS ]\n NAME = last\n[ server_DATA ]\n PASSWORD = last\n[ plex ]\n USERTOKEN = last\n",
    ] {
        let contents = format!("{contents}[unknown]\nkeep=100%%\n; retain me\n");
        let desired = StoredClientSettingsMvp {
            username: Some("new%\nname".into()),
            server_password: Some("new%\rpassword".into()),
            plex_user_token: Some("new%\ttoken".into()),
            ..Default::default()
        };
        let saved = upsert_sorotte_ini_stored_client_settings_mvp(&contents, &desired);
        assert_eq!(
            parse_sorotte_ini_stored_client_settings_mvp(&saved),
            desired
        );
        assert!(saved.contains("[unknown]\nkeep=100%%\n; retain me\n"));
        assert_eq!(
            saved.starts_with('\u{feff}'),
            contents.starts_with('\u{feff}')
        );
        assert_eq!(
            upsert_sorotte_ini_stored_client_settings_mvp(&saved, &desired),
            saved
        );

        let cleared = upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity(
            &saved,
            &StoredClientSettingsMvp {
                server_password: Some("".into()),
                ..Default::default()
            },
        );
        let parsed = parse_sorotte_ini_stored_client_settings_mvp(&cleared);
        assert!(parsed.server_password.is_none());
        assert!(parsed.plex_user_token.is_none());
        assert!(!cleared.to_ascii_lowercase().contains("usertoken"));
    }
}

proptest! {
    #[test]
    fn duplicate_sections_roundtrip_and_save_idempotently(
        duplicates in 1_usize..24,
        old_values in proptest::collection::vec("[a-zA-Z0-9%]{1,16}", 1..24),
        value in "[a-zA-Z0-9%]{1,24}",
        bom in any::<bool>(),
        mixed_case in any::<bool>(),
    ) {
        let mut contents = if bom { "\u{feff}".to_owned() } else { String::new() };
        for index in 0..duplicates {
            let old = &old_values[index % old_values.len()];
            let section = if mixed_case && index % 2 == 0 { " PLEX " } else { "plex" };
            contents.push_str(&format!("[{section}]\nuserToken={old}\nUSERtoken = {old}\n; keep {index}\nunknown_{index}=yes\n"));
        }
        let desired = StoredClientSettingsMvp { plex_user_token: Some(value.into()), ..Default::default() };
        let saved = upsert_sorotte_ini_stored_client_settings_mvp(&contents, &desired);
        prop_assert_eq!(parse_sorotte_ini_stored_client_settings_mvp(&saved), desired.clone());
        prop_assert_eq!(upsert_sorotte_ini_stored_client_settings_mvp(&saved, &desired), saved.clone());
        for index in 0..duplicates {
            let expected = format!("; keep {index}\nunknown_{index}=yes");
            prop_assert!(saved.contains(&expected));
        }
    }
}
