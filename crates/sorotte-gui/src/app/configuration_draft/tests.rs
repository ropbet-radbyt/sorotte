use std::collections::BTreeMap;

use super::{
    FirstRunConfigurationDialogDraft, FirstRunConfigurationDialogState, SecretDraft, SettingId,
};
use crate::app::shell_state::{GuiSettingApplyRequirement, GuiSettingValueOrigin};

use sorotte_client_app::app_boundary::state::{AutoplayThresholdOverride, StoredClientSettingsMvp};
use sorotte_client_core::UnpauseActionMode;

#[test]
fn configuration_draft_applies_edits_and_round_trips_to_stored_settings() {
    let mut draft =
        FirstRunConfigurationDialogDraft::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(draft.apply_text_value(SettingId::ConnectionHost, "syncplay.example"));
    assert!(draft.apply_text_value(SettingId::ConnectionPort, "8995"));
    draft.begin_server_password_change();
    assert!(draft.apply_text_value(SettingId::ConnectionServerPassword, "secret"));
    assert!(draft.apply_text_value(SettingId::PlayerExecutable, "C:/Program Files/mpv/mpv.exe"));
    assert!(draft.apply_text_value(SettingId::PlayerArguments, "--profile=fast --no-border"));
    assert!(draft.apply_text_value(SettingId::ConnectionRoomHistory, "main-room\nbackup-room"));
    assert!(draft.apply_bool_value(SettingId::PlaybackAutoplay, true));
    assert!(draft.apply_bool_value(SettingId::PlaybackLoopPlaylist, true));
    assert!(draft.apply_bool_value(SettingId::PlaybackLoopSingleFiles, true));
    assert!(draft.apply_text_value(SettingId::PlaybackUnpauseAction, "Always"));
    assert!(draft.apply_text_value(SettingId::PlaybackAutoplayMinUsers, "3"));
    assert!(draft.apply_text_value(
        SettingId::PrivacyTrustedDomains,
        "youtube.com\n*.example.com/videos"
    ));
    assert!(draft.apply_text_value(SettingId::MediaLibraryDirectories, "C:/Media\nD:/Archive"));
    assert!(draft.apply_text_value(SettingId::ChatInputPosition, "Bottom"));
    assert!(draft.apply_text_value(SettingId::ChatOutputMode, "Scrolling"));
    assert!(draft.apply_text_value(SettingId::ChatInputFontSize, "24"));
    assert!(draft.apply_text_value(SettingId::ChatOutputFontWeight, "50"));
    assert!(draft.apply_text_value(SettingId::OsdNotificationTimeout, "3"));
    assert!(draft.apply_bool_value(SettingId::OsdShowSlowdown, true));
    assert!(draft.apply_bool_value(SettingId::GeneralAutosaveJoinsToList, true));
    assert!(draft.apply_bool_value(SettingId::GeneralForceGuiPrompt, true));
    assert!(draft.apply_text_value(SettingId::GeneralLanguage, "pt-br"));
    assert!(draft.apply_text_value(SettingId::GeneralUpdateChannel, "DEV"));

    let saved = draft.to_stored_settings();
    assert_eq!(saved.host.as_deref(), Some("syncplay.example"));
    assert_eq!(saved.port, Some(8995));
    assert_eq!(
        saved
            .server_password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("secret")
    );
    assert_eq!(
        saved.player_path.as_deref(),
        Some("C:/Program Files/mpv/mpv.exe")
    );
    let mut expected_arguments = BTreeMap::new();
    expected_arguments.insert(
        "C:/Program Files/mpv/mpv.exe".to_owned(),
        vec!["--profile=fast".to_owned(), "--no-border".to_owned()],
    );
    assert_eq!(saved.per_player_arguments, Some(expected_arguments));
    assert_eq!(
        saved.room_list,
        Some(vec!["backup-room".to_owned(), "main-room".to_owned()])
    );
    assert_eq!(saved.autoplay_initial_state, Some(true));
    assert_eq!(saved.loop_at_end_of_playlist, Some(true));
    assert_eq!(saved.loop_single_files, Some(true));
    assert_eq!(saved.unpause_action, Some(UnpauseActionMode::Always));
    assert_eq!(
        saved.autoplay_min_users,
        Some(AutoplayThresholdOverride::Set(3))
    );
    assert_eq!(
        saved.trusted_domains,
        Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned()
        ])
    );
    assert_eq!(
        saved.media_search_directories,
        Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()])
    );
    assert_eq!(saved.chat_input_position.as_deref(), Some("Bottom"));
    assert_eq!(saved.chat_output_mode.as_deref(), Some("Scrolling"));
    assert_eq!(saved.chat_input_relative_font_size, Some(24));
    assert_eq!(saved.chat_output_font_weight, Some(50));
    assert_eq!(saved.notification_timeout_seconds, Some(3));
    assert_eq!(saved.show_slowdown_osd, Some(true));
    assert_eq!(saved.autosave_joins_to_list, Some(true));
    assert_eq!(saved.force_gui_prompt, Some(true));
    assert_eq!(saved.language.as_deref(), Some("pt_BR"));
    assert_eq!(saved.update_channel.as_deref(), Some("dev"));
    assert_eq!(
        draft.control_value(SettingId::PrivacyTrustedDomainCount),
        Some("2")
    );
    assert_eq!(
        draft.control_value(SettingId::MediaLibraryDirectoryCount),
        Some("2")
    );
    assert_eq!(
        draft.control_value(SettingId::PlayerArguments),
        Some("--profile=fast --no-border")
    );
}

#[test]
fn configuration_draft_rejects_readonly_control_edits() {
    let mut draft =
        FirstRunConfigurationDialogDraft::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!draft.apply_text_value(SettingId::ConnectionPublicServerCount, "5"));
    assert_eq!(draft.to_stored_settings().public_servers, None);
}

#[test]
fn configuration_draft_refreshes_player_arguments_when_player_path_changes() {
    let mut per_player_arguments = BTreeMap::new();
    per_player_arguments.insert("mpv".to_owned(), vec!["--idle=yes".to_owned()]);
    per_player_arguments.insert(
        "C:/Program Files/mpv/mpv.exe".to_owned(),
        vec!["--profile=fast".to_owned()],
    );
    let mut draft =
        FirstRunConfigurationDialogDraft::from_stored_settings(&StoredClientSettingsMvp {
            player_path: Some("mpv".to_owned()),
            per_player_arguments: Some(per_player_arguments),
            ..StoredClientSettingsMvp::default()
        });

    assert_eq!(
        draft.control_value(SettingId::PlayerArguments),
        Some("--idle=yes")
    );

    assert!(draft.apply_text_value(SettingId::PlayerExecutable, "C:/Program Files/mpv/mpv.exe"));

    assert_eq!(
        draft.control_value(SettingId::PlayerArguments),
        Some("--profile=fast")
    );
}

#[test]
fn configuration_draft_noop_round_trip_preserves_settings_and_catalogs_every_id() {
    let settings = StoredClientSettingsMvp {
        host: Some("sync.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("main".to_owned()),
        server_password: Some("secret".into()),
        player_path: Some("mpv".to_owned()),
        autoplay_initial_state: Some(true),
        shared_playlist_enabled: Some(false),
        trusted_domains: Some(vec!["media.example".to_owned()]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        language: Some("en".to_owned()),
        update_channel: Some("stable".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let draft = FirstRunConfigurationDialogDraft::from_stored_settings(&settings);

    assert_eq!(draft.to_stored_settings(), settings);
    assert_eq!(
        draft
            .sections
            .iter()
            .flat_map(|section| &section.controls)
            .count(),
        SettingId::ALL.len()
    );
    for &id in SettingId::ALL {
        let control = draft
            .control(id)
            .expect("every SettingId must be projected");
        assert_eq!(control.id, id);
        assert_eq!(SettingId::from_automation_id(id.automation_id()), Some(id));
    }
}

#[test]
fn configuration_secret_draft_preserves_replaces_clears_and_cancels() {
    let original = StoredClientSettingsMvp {
        server_password: Some("original-secret".into()),
        ..StoredClientSettingsMvp::default()
    };
    let mut draft = FirstRunConfigurationDialogDraft::from_stored_settings(&original);

    assert_eq!(draft.server_password, SecretDraft::Unchanged);
    assert_eq!(
        draft.control_value(SettingId::ConnectionServerPassword),
        Some("")
    );
    assert_eq!(draft.to_stored_settings(), original);

    draft.begin_server_password_change();
    assert!(draft.apply_text_value(SettingId::ConnectionServerPassword, "replacement-secret"));
    assert_eq!(
        draft
            .to_stored_settings()
            .server_password
            .as_ref()
            .map(|value| value.expose_secret()),
        Some("replacement-secret")
    );

    draft.cancel_server_password_change();
    assert_eq!(draft.to_stored_settings(), original);

    draft.remove_server_password();
    assert_eq!(draft.server_password, SecretDraft::Clear);
    assert_eq!(draft.to_stored_settings().server_password, None);
}

#[test]
fn configuration_effective_defaults_retain_their_override_origin() {
    let defaults =
        FirstRunConfigurationDialogState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert_eq!(defaults.readiness.unpause_action.effective, "IfOthersReady");
    assert_eq!(
        defaults.readiness.unpause_action.origin(),
        GuiSettingValueOrigin::ApplicationDefault
    );
    assert_eq!(
        defaults.readiness.autoplay_min_users.origin(),
        GuiSettingValueOrigin::ApplicationDefault
    );

    let overridden =
        FirstRunConfigurationDialogState::from_stored_settings(&StoredClientSettingsMvp {
            unpause_action: Some(UnpauseActionMode::Always),
            autoplay_min_users: Some(AutoplayThresholdOverride::Set(2)),
            ..StoredClientSettingsMvp::default()
        });
    assert_eq!(
        overridden.readiness.unpause_action.origin(),
        GuiSettingValueOrigin::StoredOverride
    );
    assert_eq!(
        overridden.readiness.autoplay_min_users.origin(),
        GuiSettingValueOrigin::StoredOverride
    );
    assert_eq!(
        overridden
            .readiness
            .unpause_action
            .origin_against_persisted(&defaults.readiness.unpause_action),
        GuiSettingValueOrigin::DraftChange
    );
    assert_eq!(
        defaults
            .readiness
            .unpause_action
            .origin_against_persisted(&defaults.readiness.unpause_action),
        GuiSettingValueOrigin::ApplicationDefault
    );
}

#[test]
fn changed_setting_ids_include_secret_intent_and_same_length_server_replacement() {
    let original = StoredClientSettingsMvp {
        server_password: Some("saved-secret".into()),
        public_servers: Some(vec![("One".to_owned(), "one.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };
    let mut draft = FirstRunConfigurationDialogDraft::from_stored_settings(&original);
    draft.remove_server_password();
    draft.settings.public_servers = Some(vec![("Two".to_owned(), "two.example:8999".to_owned())]);

    let changed = draft.changed_setting_ids_against(&original);
    assert!(changed.contains(&SettingId::ConnectionServerPassword));
    assert!(changed.contains(&SettingId::ConnectionPublicServerCount));
    assert_eq!(
        SettingId::ConnectionServerPassword.apply_requirement(),
        GuiSettingApplyRequirement::Reconnect
    );
    assert_eq!(
        SettingId::ConnectionPublicServerCount.apply_requirement(),
        GuiSettingApplyRequirement::OnSave
    );
}
