use super::*;

#[test]
fn load_legacy_syncplayintf_script_sends_load_script_and_option_message_when_attached() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            chat_input_enabled: true,
            chat_input_font_underline: true,
            chat_input_font_family: "serif".to_owned(),
            chat_input_relative_font_size: 18,
            chat_input_font_weight: 50,
            chat_input_font_color: "#abcdef".to_owned(),
            chat_input_position: "Bottom".to_owned(),
            chat_direct_input: true,
            chat_output_font_underline: true,
            chat_output_font_family: "monospace".to_owned(),
            chat_output_relative_font_size: 30,
            chat_output_font_weight: 75,
            chat_output_mode: "Scrolling".to_owned(),
            chat_max_lines: 9,
            chat_top_margin: 40,
            chat_left_margin: 35,
            chat_bottom_margin: 45,
            chat_move_osd: false,
            notification_timeout_ms: 4_000,
            alert_timeout_ms: 6_000,
            chat_timeout_ms: 8_000,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("legacy settings application should succeed");

    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new("C:/syncplay/syncplayintf.lua"))
        .expect("attached mpv transport should accept load-script");

    let writes = state.writes();
    assert_eq!(writes.len(), 2);
    let first_payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    let second_payload: Value = serde_json::from_str(writes[1].trim_end()).expect("valid json");
    assert_eq!(
        first_payload,
        json!({
            "command": ["load-script", "C:/syncplay/syncplayintf.lua"],
            "request_id": 1
        })
    );
    assert_eq!(
        second_payload["command"][0],
        Value::String("script-message-to".to_owned())
    );
    assert_eq!(
        second_payload["command"][1],
        Value::String("syncplayintf".to_owned())
    );
    assert_eq!(
        second_payload["command"][2],
        Value::String("set_syncplayintf_options".to_owned())
    );
    let options = second_payload["command"][3]
        .as_str()
        .expect("syncplayintf options should be a string");
    assert!(options.contains("chatInputEnabled=True"));
    assert!(options.contains("chatInputFontUnderline=True"));
    assert!(options.contains("chatInputFontFamily=serif"));
    assert!(options.contains("chatInputRelativeFontSize=18"));
    assert!(options.contains("chatInputFontWeight=50"));
    assert!(options.contains("chatInputFontColor=#abcdef"));
    assert!(options.contains("chatInputPosition=Bottom"));
    assert!(options.contains("chatOutputFontUnderline=True"));
    assert!(options.contains("chatOutputFontFamily=monospace"));
    assert!(options.contains("chatOutputRelativeFontSize=30"));
    assert!(options.contains("chatOutputFontWeight=75"));
    assert!(options.contains("chatOutputMode=Scrolling"));
    assert!(options.contains("chatMaxLines=9"));
    assert!(options.contains("chatTopMargin=40"));
    assert!(options.contains("chatLeftMargin=35"));
    assert!(options.contains("chatBottomMargin=45"));
    assert!(options.contains("chatDirectInput=True"));
    assert!(options.contains("notificationTimeout=4"));
    assert!(options.contains("alertTimeout=6"));
    assert!(options.contains("chatTimeout=8"));
    assert!(options.contains("chatOutputEnabled=True"));
}

#[test]
fn load_legacy_syncplayintf_script_targets_script_messages_to_loaded_file_stem() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/Temp/syncplay-rust-syncplayintf-702304.lua",
        ))
        .expect("attached mpv transport should accept load-script for patched temp files");

    let writes = state.writes();
    assert_eq!(writes.len(), 2);
    let second_payload: Value = serde_json::from_str(writes[1].trim_end()).expect("valid json");
    assert_eq!(
        second_payload["command"][1],
        Value::String("syncplay-rust-syncplayintf-702304".to_owned())
    );
}

#[test]
fn load_legacy_syncplayintf_script_ignores_early_option_message_failure_and_retries_later() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"error running command"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new("C:/syncplay/syncplayintf.lua"))
        .expect("initial syncplayintf load should ignore early option-message timing races");

    adapter
        .show_syncplay_legacy_chat_message("<alice> hi")
        .expect("legacy chat should retry option handoff before using the script");

    let writes = state.writes();
    assert_eq!(writes.len(), 4);
    let third_payload: Value = serde_json::from_str(writes[2].trim_end()).expect("valid json");
    let fourth_payload: Value = serde_json::from_str(writes[3].trim_end()).expect("valid json");
    assert_eq!(
        third_payload["command"],
        json!([
            "script-message-to",
            "syncplayintf",
            "set_syncplayintf_options",
            third_payload["command"][3]
                .as_str()
                .expect("options payload"),
        ])
    );
    assert_eq!(
        fourth_payload,
        json!({
            "command": ["script-message-to", "syncplayintf", "chat", "<alice> hi"],
            "request_id": 4
        })
    );
}

#[test]
fn legacy_osd_falls_back_to_show_text_while_syncplayintf_initialization_is_still_pending() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"error running command"}"#,
        r#"{"request_id":3,"error":"error running command"}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new("C:/syncplay/syncplayintf.lua"))
        .expect("load-script should still succeed");

    adapter
        .show_syncplay_legacy_message("room updated", LegacySyncplayOsdKind::Notification)
        .expect("legacy OSD should fall back to show-text if the script is not ready yet");

    let writes = state.writes();
    assert_eq!(writes.len(), 4);
    let fourth_payload: Value = serde_json::from_str(writes[3].trim_end()).expect("valid json");
    assert_eq!(
        fourth_payload,
        json!({
            "command": ["show-text", "room updated", 3_000, 1],
            "request_id": 4
        })
    );
}

#[test]
fn configure_legacy_syncplay_ui_settings_applies_osd_position_when_needed() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings::default())
        .expect("legacy settings application should succeed");

    let writes = state.writes();
    assert_eq!(writes.len(), 2);
    let first_payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    let second_payload: Value = serde_json::from_str(writes[1].trim_end()).expect("valid json");
    assert_eq!(
        first_payload,
        json!({
            "command": ["set_property", "osd-align-y", "bottom"],
            "request_id": 1
        })
    );
    assert_eq!(
        second_payload,
        json!({
            "command": ["set_property", "osd-margin-y", 110],
            "request_id": 2
        })
    );
    assert_eq!(
        adapter.legacy_syncplay_ui_settings(),
        &LegacySyncplayUiSettings::default()
    );
}

#[test]
fn configure_legacy_syncplay_ui_settings_skips_osd_position_when_disabled() {
    let (transport, state) = fake_transport_with_reads(&[]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            chat_move_osd: false,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("legacy settings application should succeed");

    assert!(state.writes().is_empty());
}

#[test]
fn show_syncplay_legacy_message_uses_script_message_when_syncplayintf_is_loaded() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new("C:/syncplay/syncplayintf.lua"))
        .expect("attached mpv transport should accept load-script");

    adapter
        .show_syncplay_legacy_message("room updated", LegacySyncplayOsdKind::Notification)
        .expect("syncplayintf notification should succeed");

    let writes = state.writes();
    assert_eq!(writes.len(), 3);
    let payload: Value = serde_json::from_str(writes[2].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["script-message-to", "syncplayintf", "notification-osd-neutral", "room updated"],
            "request_id": 3
        })
    );
}

#[test]
fn show_syncplay_legacy_message_uses_notification_timeout_when_osd_is_enabled() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            chat_move_osd: false,
            notification_timeout_ms: 4_500,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("legacy settings application should succeed");

    adapter
        .show_syncplay_legacy_message("room updated", LegacySyncplayOsdKind::Notification)
        .expect("show-text notification should succeed");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["show-text", "room updated", 4_500, 1],
            "request_id": 1
        })
    );
}

#[test]
fn show_syncplay_legacy_message_uses_alert_timeout_when_requested() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            chat_move_osd: false,
            alert_timeout_ms: 6_000,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("legacy settings application should succeed");

    adapter
        .show_syncplay_legacy_message("autoplay", LegacySyncplayOsdKind::Alert)
        .expect("show-text alert should succeed");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["show-text", "autoplay", 6_000, 1],
            "request_id": 1
        })
    );
}

#[test]
fn show_syncplay_legacy_chat_message_uses_script_message_when_syncplayintf_is_loaded() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new("C:/syncplay/syncplayintf.lua"))
        .expect("attached mpv transport should accept load-script");

    adapter
        .show_syncplay_legacy_chat_message("<alice> hi")
        .expect("syncplayintf chat should succeed");

    let writes = state.writes();
    assert_eq!(writes.len(), 3);
    let payload: Value = serde_json::from_str(writes[2].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["script-message-to", "syncplayintf", "chat", "<alice> hi"],
            "request_id": 3
        })
    );
}

#[test]
fn show_syncplay_legacy_chat_message_uses_chat_timeout_even_when_show_osd_is_disabled() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            show_osd: false,
            chat_move_osd: false,
            chat_timeout_ms: 8_000,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("legacy settings application should succeed");

    adapter
        .show_syncplay_legacy_chat_message("<alice> hi")
        .expect("chat show-text should succeed");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["show-text", "<alice> hi", 8_000, 1],
            "request_id": 1
        })
    );
}

#[test]
fn show_syncplay_legacy_chat_message_falls_back_to_notification_timeout_when_chat_output_is_disabled()
 {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            chat_output_enabled: false,
            chat_move_osd: false,
            notification_timeout_ms: 2_500,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("legacy settings application should succeed");

    adapter
        .show_syncplay_legacy_chat_message("<alice> hi")
        .expect("chat fallback show-text should succeed");

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        payload,
        json!({
            "command": ["show-text", "<alice> hi", 2_500, 1],
            "request_id": 1
        })
    );
}
