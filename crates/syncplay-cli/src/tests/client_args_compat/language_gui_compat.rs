use super::*;

#[test]
fn normalized_legacy_runtime_language_tag_legacy_compatible_accepts_python_tags_and_aliases() {
    assert_eq!(
        crate::normalized_legacy_runtime_language_tag_legacy_compatible("fr"),
        Some("fr")
    );
    assert_eq!(
        crate::normalized_legacy_runtime_language_tag_legacy_compatible("PT-br"),
        Some("pt_BR")
    );
    assert_eq!(
        crate::normalized_legacy_runtime_language_tag_legacy_compatible("zh-cn"),
        Some("zh_CN")
    );
    assert_eq!(
        crate::normalized_legacy_runtime_language_tag_legacy_compatible("klingon"),
        None
    );
}

#[test]
fn resolved_legacy_runtime_language_tag_legacy_compatible_prefers_cli_and_falls_back_to_stored() {
    let overrides = LegacyClientArgOverrides {
        language: Some("PT-br".to_owned()),
        ..Default::default()
    };
    let invalid_overrides = LegacyClientArgOverrides {
        language: Some("klingon".to_owned()),
        ..Default::default()
    };
    let stored_settings = StoredClientSettingsMvp {
        language: Some("fr".to_owned()),
        ..Default::default()
    };

    assert_eq!(
        crate::resolved_legacy_runtime_language_tag_legacy_compatible(
            &overrides,
            Some(&stored_settings)
        ),
        Some("pt_BR".to_owned())
    );
    assert_eq!(
        crate::resolved_legacy_runtime_language_tag_legacy_compatible(
            &invalid_overrides,
            Some(&stored_settings)
        ),
        Some("fr".to_owned())
    );
}

#[test]
fn legacy_runtime_language_selection_line_legacy_compatible_emits_note_for_supported_values_and_warning_for_invalid_values()
 {
    let supported = crate::legacy_runtime_language_selection_line_legacy_compatible(Some("PT-br"))
        .expect("supported language should emit a banner");
    let invalid = crate::legacy_runtime_language_selection_line_legacy_compatible(Some("klingon"))
        .expect("invalid language should emit a warning");

    assert!(supported.contains("pt_BR"));
    assert_eq!(
        invalid,
        "warning: unsupported legacy --language value 'klingon' was ignored; supported values: de/en/es/eo/fi/fr/it/pt_PT/pt_BR/tr/ru/zh_CN/ko"
    );
}

#[test]
fn legacy_force_gui_prompt_compatibility_requires_no_gui_for_headless_override() {
    let blocked = LegacyClientArgOverrides {
        force_gui_prompt_requested: true,
        ..Default::default()
    };
    let overridden = LegacyClientArgOverrides {
        force_gui_prompt_requested: true,
        no_gui_requested: true,
        ..Default::default()
    };

    assert!(blocked.should_halt_for_legacy_force_gui_prompt_compatibility());
    assert!(!overridden.should_halt_for_legacy_force_gui_prompt_compatibility());
    assert_eq!(
        crate::legacy_force_gui_prompt_compatibility_line_legacy_compatible(&blocked),
        Some(
            "note: legacy --force-gui-prompt requested GUI configuration flow; syncplay-cli has no GUI, so startup is halted. Re-run with --no-gui to continue headless."
        )
    );
    assert_eq!(
        crate::legacy_force_gui_prompt_compatibility_line_legacy_compatible(&overridden),
        Some(
            "note: legacy --force-gui-prompt was overridden by --no-gui; continuing in headless mode"
        )
    );
}

#[test]
fn stored_force_gui_prompt_compatibility_requires_no_gui_for_headless_override() {
    let settings = StoredClientSettingsMvp {
        force_gui_prompt: Some(true),
        ..Default::default()
    };
    let blocked = LegacyClientArgOverrides::default();
    let overridden = LegacyClientArgOverrides {
        no_gui_requested: true,
        ..Default::default()
    };
    let explicit_flag = LegacyClientArgOverrides {
        force_gui_prompt_requested: true,
        ..Default::default()
    };

    assert!(crate::should_halt_for_stored_force_gui_prompt_legacy_compatible(&blocked, &settings));
    assert!(
        !crate::should_halt_for_stored_force_gui_prompt_legacy_compatible(&overridden, &settings)
    );
    assert!(
        !crate::should_halt_for_stored_force_gui_prompt_legacy_compatible(
            &explicit_flag,
            &settings
        )
    );
    assert_eq!(
        crate::stored_force_gui_prompt_compatibility_line_legacy_compatible(&blocked, &settings),
        Some(
            "note: stored client_settings.forceGuiPrompt = True requested GUI configuration flow; syncplay-cli has no GUI, so startup is halted. Re-run with --no-gui or clear the stored setting to continue headless."
        )
    );
    assert_eq!(
        crate::stored_force_gui_prompt_compatibility_line_legacy_compatible(&overridden, &settings),
        Some(
            "note: stored client_settings.forceGuiPrompt = True was overridden by --no-gui; continuing in headless mode"
        )
    );
    assert_eq!(
        crate::stored_force_gui_prompt_compatibility_line_legacy_compatible(
            &explicit_flag,
            &settings
        ),
        None
    );
}
