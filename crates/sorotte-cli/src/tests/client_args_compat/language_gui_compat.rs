use super::*;

#[test]
fn normalized_runtime_language_tag_accepts_python_tags_and_aliases() {
    assert_eq!(crate::normalized_runtime_language_tag("fr"), Some("fr"));
    assert_eq!(
        crate::normalized_runtime_language_tag("PT-br"),
        Some("pt_BR")
    );
    assert_eq!(
        crate::normalized_runtime_language_tag("zh-cn"),
        Some("zh_CN")
    );
    assert_eq!(crate::normalized_runtime_language_tag("klingon"), None);
}

#[test]
fn resolved_runtime_language_tag_prefers_cli_and_falls_back_to_stored() {
    let overrides = SyncplayClientArgOverrides {
        language: Some("PT-br".to_owned()),
        ..Default::default()
    };
    let invalid_overrides = SyncplayClientArgOverrides {
        language: Some("klingon".to_owned()),
        ..Default::default()
    };
    let stored_settings = StoredClientSettings {
        language: Some("fr".to_owned()),
        ..Default::default()
    };

    assert_eq!(
        crate::resolved_runtime_language_tag(&overrides, Some(&stored_settings)),
        Some("pt_BR".to_owned())
    );
    assert_eq!(
        crate::resolved_runtime_language_tag(&invalid_overrides, Some(&stored_settings)),
        Some("fr".to_owned())
    );
}

#[test]
fn runtime_language_selection_line_emits_note_for_supported_values_and_warning_for_invalid_values()
{
    let supported = crate::runtime_language_selection_line(Some("PT-br"))
        .expect("supported language should emit a banner");
    let invalid = crate::runtime_language_selection_line(Some("klingon"))
        .expect("invalid language should emit a warning");

    assert!(supported.contains("pt_BR"));
    assert_eq!(
        invalid,
        "warning: unsupported --language value 'klingon' was ignored; supported values: de/en/es/eo/fi/fr/it/pt_PT/pt_BR/tr/ru/zh_CN/ko"
    );
}

#[test]
fn syncplay_force_gui_prompt_compatibility_requires_no_gui_for_headless_override() {
    let blocked = SyncplayClientArgOverrides {
        force_gui_prompt_requested: true,
        ..Default::default()
    };
    let overridden = SyncplayClientArgOverrides {
        force_gui_prompt_requested: true,
        no_gui_requested: true,
        ..Default::default()
    };

    assert!(blocked.should_halt_for_syncplay_force_gui_prompt_compatibility());
    assert!(!overridden.should_halt_for_syncplay_force_gui_prompt_compatibility());
    assert_eq!(
        crate::syncplay_force_gui_prompt_compatibility_line(&blocked),
        Some(
            "note: --force-gui-prompt requested GUI configuration flow; sorotte-cli has no GUI, so startup is halted. Re-run with --no-gui to continue headless."
        )
    );
    assert_eq!(
        crate::syncplay_force_gui_prompt_compatibility_line(&overridden),
        Some("note: --force-gui-prompt was overridden by --no-gui; continuing in headless mode")
    );
}

#[test]
fn stored_force_gui_prompt_compatibility_requires_no_gui_for_headless_override() {
    let settings = StoredClientSettings {
        force_gui_prompt: Some(true),
        ..Default::default()
    };
    let blocked = SyncplayClientArgOverrides::default();
    let overridden = SyncplayClientArgOverrides {
        no_gui_requested: true,
        ..Default::default()
    };
    let explicit_flag = SyncplayClientArgOverrides {
        force_gui_prompt_requested: true,
        ..Default::default()
    };

    assert!(crate::should_halt_for_stored_force_gui_prompt(
        &blocked, &settings
    ));
    assert!(!crate::should_halt_for_stored_force_gui_prompt(
        &overridden,
        &settings
    ));
    assert!(!crate::should_halt_for_stored_force_gui_prompt(
        &explicit_flag,
        &settings
    ));
    assert_eq!(
        crate::stored_force_gui_prompt_compatibility_line(&blocked, &settings),
        Some(
            "note: stored client_settings.forceGuiPrompt = True requested GUI configuration flow; sorotte-cli has no GUI, so startup is halted. Re-run with --no-gui or clear the stored setting to continue headless."
        )
    );
    assert_eq!(
        crate::stored_force_gui_prompt_compatibility_line(&overridden, &settings),
        Some(
            "note: stored client_settings.forceGuiPrompt = True was overridden by --no-gui; continuing in headless mode"
        )
    );
    assert_eq!(
        crate::stored_force_gui_prompt_compatibility_line(&explicit_flag, &settings),
        None
    );
}
