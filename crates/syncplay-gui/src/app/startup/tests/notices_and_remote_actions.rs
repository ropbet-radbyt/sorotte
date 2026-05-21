use super::*;

#[test]
fn startup_notice_mentions_configuration_surface_and_grouped_sections() {
    let notice = startup_notice(&StoredClientSettingsMvp::default());

    assert!(notice.contains("[Shell App State]"));
    assert!(notice.contains("active_view=setup"));
    assert!(notice.contains("open_modal=(none)"));
    assert!(
        notice.contains("[Selection] user=0, playlist=(none), menu=0:0, media_directory=(none)")
    );
    assert!(notice.contains(
        "[Commands] busy=no, save_configuration=yes, reset_configuration=no, reload_configuration=yes, connect_saved_server=no, disconnect_session=no, connect_public_server=no, refresh_public_servers=yes, search_missing_media=no, toggle_pause=no, send_chat_message=yes"
    ));
    assert!(notice.contains("[Pending] operation=(none)"));
    assert!(notice.contains("[Control Focus] focused=(none)"));
    assert!(notice.contains("[Public Server Edit] editing=(none)"));
    assert!(notice.contains("[Text Edit] editing=(none)"));
    assert!(notice.contains("[Notifications] count=0"));
    assert!(notice.contains("[Validation] status=clean, last_action_error=(none)"));
    assert!(notice.contains("setup surface initialized"));
    assert!(notice.contains("[Connection]"));
    assert!(notice.contains("[Readiness]"));
    assert!(notice.contains("[Privacy]"));
    assert!(notice.contains("[Media Search]"));
    assert!(notice.contains("[System]"));
    assert!(notice.contains("[Room]"));
    assert!(notice.contains("[Menus & Dialogs]"));
    assert!(notice.contains("[Public Server Browser]"));
    assert!(notice.contains("[Media Search Workflow]"));
    assert!(notice.contains("Playback Controls:"));
    assert!(notice.contains("Dialog Prompts:"));
    assert!(notice.contains("Servers (0):"));
    assert!(notice.contains("Directories (0):"));
    assert!(notice.contains("unified shell app state and action reducer"));
    assert!(notice.contains("Users (1):"));
    assert!(notice.contains("- Host [text]:"));
    assert!(notice.contains("- Server Password [password]:"));
    assert!(notice.contains("room-first shell"));
    assert!(!notice.contains("bootstrap placeholder"));
    assert!(notice.contains("de/en/es"));
}

#[test]
fn shell_widget_preview_renders_tree_through_text_preview_renderer() {
    let preview = shell_widget_preview(&StoredClientSettingsMvp::default());

    assert!(!preview.contains("[Widget Tree]"));
    assert!(preview.contains("- Syncplay GUI [panel] id=shell-root"));
    assert!(preview.contains(
        "  - Setup [layout] id=configuration-root, enabled=yes, selected=yes, value=(none)"
    ));
    assert!(preview.contains(
        "    - Host [text-input] id=config:Connection:Host, enabled=yes, selected=no, value=(unset)"
    ));
    assert!(
        preview.contains(
            "  - Room [layout] id=main-window-root, enabled=yes, selected=no, value=(none)"
        )
    );
}

#[test]
fn startup_preview_includes_shell_summary_and_widget_tree_preview() {
    let preview = startup_preview(&StoredClientSettingsMvp::default());

    assert!(preview.contains("[Shell App State]"));
    assert!(preview.contains("[Widget Tree]"));
    assert!(preview.contains("- Syncplay GUI [panel] id=shell-root"));
}

#[test]
fn gui_startup_remote_actions_run_due_automatic_update_checks() {
    let settings = StoredClientSettingsMvp {
        check_for_updates_automatically: Some(true),
        update_channel: Some("dev".to_owned()),
        last_checked_for_updates: None,
        ..StoredClientSettingsMvp::default()
    };
    let expected = super::super::remote_services::LegacyUpdateCheckResult {
        status: super::super::remote_services::LegacyUpdateCheckStatus::UpdateAvailable,
        message: "Remote startup update available.".to_owned(),
        url: Some("https://syncplay.pl/download/".to_owned()),
        candidate: None,
        self_update_supported: false,
        public_servers: None,
        checked_at_utc: "2026-03-08 09:10:11.123".to_owned(),
        user_initiated: false,
    };

    let actions = super::super::gui_startup_remote_actions_with_fetchers(
        &settings,
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000),
        |_, update_channel| {
            assert_eq!(update_channel, Some("dev"));
            expected.clone()
        },
        |_| Ok(Vec::new()),
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::ApplyUpdateCheckResult(expected)]
    );
}

#[test]
fn gui_startup_remote_actions_seed_public_servers_when_cache_is_empty() {
    let settings = StoredClientSettingsMvp {
        check_for_updates_automatically: Some(true),
        last_checked_for_updates: Some("2027-01-14 09:10:11.123".to_owned()),
        ..StoredClientSettingsMvp::default()
    };

    let actions = super::super::gui_startup_remote_actions_with_fetchers(
        &settings,
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000),
        |_, _| panic!("update check should not run when the timestamp is still fresh"),
        |_| Ok(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::ApplyStartupPublicServerCache(vec![(
            "Primary".to_owned(),
            "syncplay.pl:8999".to_owned(),
        )])]
    );
}
